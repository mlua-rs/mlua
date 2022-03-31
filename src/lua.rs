use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::cell::{Ref, RefCell, RefMut, UnsafeCell};
use std::collections::HashMap;
use std::ffi::CString;
use std::fmt;
use std::marker::PhantomData;
use std::os::raw::{c_char, c_int, c_void};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe, Location};
use std::sync::{Arc, Mutex};
use std::{mem, ptr, str};

use rustc_hash::FxHashMap;

use crate::chunk::{AsChunk, Chunk, ChunkMode};
use crate::error::{Error, Result};
use crate::ffi;
use crate::function::Function;
use crate::hook::Debug;
use crate::scope::Scope;
use crate::stdlib::StdLib;
use crate::string::String;
use crate::table::Table;
use crate::thread::Thread;
use crate::types::{
    Callback, CallbackUpvalue, DestructedUserdataMT, Integer, LightUserData, LuaRef, MaybeSend,
    Number, RegistryKey,
};
use crate::userdata::{AnyUserData, UserData, UserDataCell};
use crate::userdata_impl::{StaticUserDataFields, StaticUserDataMethods};
use crate::util::{
    self, assert_stack, callback_error, check_stack, get_destructed_userdata_metatable,
    get_gc_metatable, get_gc_userdata, get_main_state, get_userdata, init_error_registry,
    init_gc_metatable, init_userdata_metatable, pop_error, push_gc_userdata, push_string,
    push_table, rawset_field, safe_pcall, safe_xpcall, StackGuard, WrappedFailure,
};
use crate::value::{FromLua, FromLuaMulti, MultiValue, Nil, ToLua, ToLuaMulti, Value};

#[cfg(not(feature = "lua54"))]
use crate::util::push_userdata;
#[cfg(feature = "lua54")]
use {
    crate::{types::WarnCallback, userdata::USER_VALUE_MAXSLOT, util::push_userdata_uv},
    std::ffi::CStr,
};

#[cfg(not(feature = "luau"))]
use crate::{hook::HookTriggers, types::HookCallback};

#[cfg(feature = "luau")]
use crate::types::{InterruptCallback, VmState};

#[cfg(feature = "async")]
use {
    crate::types::{AsyncCallback, AsyncCallbackUpvalue, AsyncPollUpvalue},
    futures_core::{
        future::{Future, LocalBoxFuture},
        task::{Context, Poll, Waker},
    },
    futures_task::noop_waker,
    futures_util::future::{self, TryFutureExt},
};

#[cfg(feature = "serialize")]
use serde::Serialize;

/// Top level Lua struct which holds the Lua state itself.
pub struct Lua {
    pub(crate) state: *mut ffi::lua_State,
    main_state: Option<*mut ffi::lua_State>,
    extra: Arc<UnsafeCell<ExtraData>>,
    ephemeral: bool,
    safe: bool,
    // Lua has lots of interior mutability, should not be RefUnwindSafe
    _no_ref_unwind_safe: PhantomData<UnsafeCell<()>>,
}

// Data associated with the Lua.
struct ExtraData {
    registered_userdata: FxHashMap<TypeId, c_int>,
    registered_userdata_mt: FxHashMap<*const c_void, Option<TypeId>>,
    registry_unref_list: Arc<Mutex<Option<Vec<c_int>>>>,

    #[cfg(not(feature = "send"))]
    app_data: RefCell<HashMap<TypeId, Box<dyn Any>>>,
    #[cfg(feature = "send")]
    app_data: RefCell<HashMap<TypeId, Box<dyn Any + Send>>>,

    libs: StdLib,
    mem_info: Option<ptr::NonNull<MemoryInfo>>,
    safe: bool, // Same as in the Lua struct

    ref_thread: *mut ffi::lua_State,
    ref_stack_size: c_int,
    ref_stack_top: c_int,
    ref_free: Vec<c_int>,

    // Cache of `WrappedFailure` enums on the ref thread (as userdata)
    wrapped_failures_cache: Vec<c_int>,
    // Cache of recycled `MultiValue` containers
    multivalue_cache: Vec<MultiValue<'static>>,
    // Cache of recycled `Thread`s (coroutines)
    #[cfg(feature = "async")]
    recycled_thread_cache: Vec<c_int>,

    // Index of `Option<Waker>` userdata on the ref thread
    #[cfg(feature = "async")]
    ref_waker_idx: c_int,

    #[cfg(not(feature = "luau"))]
    hook_callback: Option<HookCallback>,
    #[cfg(feature = "lua54")]
    warn_callback: Option<WarnCallback>,
    #[cfg(feature = "luau")]
    interrupt_callback: Option<InterruptCallback>,

    #[cfg(feature = "luau")]
    sandboxed: bool,
}

#[cfg_attr(any(feature = "lua51", feature = "luajit"), allow(dead_code))]
struct MemoryInfo {
    used_memory: isize,
    memory_limit: isize,
}

/// Mode of the Lua garbage collector (GC).
///
/// In Lua 5.4 GC can work in two modes: incremental and generational.
/// Previous Lua versions support only incremental GC.
///
/// More information can be found in the Lua [documentation].
///
/// [documentation]: https://www.lua.org/manual/5.4/manual.html#2.5
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GCMode {
    Incremental,
    /// Requires `feature = "lua54"`
    #[cfg(any(feature = "lua54"))]
    Generational,
}

/// Controls Lua interpreter behavior such as Rust panics handling.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct LuaOptions {
    /// Catch Rust panics when using [`pcall`]/[`xpcall`].
    ///
    /// If disabled, wraps these functions and automatically resumes panic if found.
    /// Also in Lua 5.1 adds ability to provide arguments to [`xpcall`] similar to Lua >= 5.2.
    ///
    /// If enabled, keeps [`pcall`]/[`xpcall`] unmodified.
    /// Panics are still automatically resumed if returned to the Rust side.
    ///
    /// Default: **true**
    ///
    /// [`pcall`]: https://www.lua.org/manual/5.4/manual.html#pdf-pcall
    /// [`xpcall`]: https://www.lua.org/manual/5.4/manual.html#pdf-xpcall
    pub catch_rust_panics: bool,

    /// Max size of thread (coroutine) object cache used to execute asynchronous functions.
    ///
    /// It works on Lua 5.4, LuaJIT (vendored) and Luau, where [`lua_resetthread`] function
    /// is available and allows to reuse old coroutines with reset state.
    ///
    /// Default: **0** (disabled)
    ///
    /// [`lua_resetthread`]: https://www.lua.org/manual/5.4/manual.html#lua_resetthread
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub thread_cache_size: usize,
}

impl Default for LuaOptions {
    fn default() -> Self {
        LuaOptions::new()
    }
}

impl LuaOptions {
    /// Returns a new instance of `LuaOptions` with default parameters.
    pub const fn new() -> Self {
        LuaOptions {
            catch_rust_panics: true,
            #[cfg(feature = "async")]
            thread_cache_size: 0,
        }
    }

    /// Sets [`catch_rust_panics`] option.
    ///
    /// [`catch_rust_panics`]: #structfield.catch_rust_panics
    #[must_use]
    pub const fn catch_rust_panics(mut self, enabled: bool) -> Self {
        self.catch_rust_panics = enabled;
        self
    }

    /// Sets [`thread_cache_size`] option.
    ///
    /// [`thread_cache_size`]: #structfield.thread_cache_size
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    #[must_use]
    pub const fn thread_cache_size(mut self, size: usize) -> Self {
        self.thread_cache_size = size;
        self
    }
}

#[cfg(feature = "async")]
pub(crate) static ASYNC_POLL_PENDING: u8 = 0;
pub(crate) static EXTRA_REGISTRY_KEY: u8 = 0;

const WRAPPED_FAILURES_CACHE_SIZE: usize = 32;
const MULTIVALUE_CACHE_SIZE: usize = 32;

/// Requires `feature = "send"`
#[cfg(feature = "send")]
#[cfg_attr(docsrs, doc(cfg(feature = "send")))]
unsafe impl Send for Lua {}

impl Drop for Lua {
    fn drop(&mut self) {
        unsafe {
            if !self.ephemeral {
                let extra = &mut *self.extra.get();
                let drain_iter = extra.wrapped_failures_cache.drain(..);
                #[cfg(feature = "async")]
                let drain_iter = drain_iter.chain(extra.recycled_thread_cache.drain(..));
                for index in drain_iter {
                    ffi::lua_pushnil(extra.ref_thread);
                    ffi::lua_replace(extra.ref_thread, index);
                    extra.ref_free.push(index);
                }
                #[cfg(feature = "async")]
                {
                    // Destroy Waker slot
                    ffi::lua_pushnil(extra.ref_thread);
                    ffi::lua_replace(extra.ref_thread, extra.ref_waker_idx);
                    extra.ref_free.push(extra.ref_waker_idx);
                }
                #[cfg(feature = "luau")]
                {
                    let callbacks = ffi::lua_callbacks(self.state);
                    let extra_ptr = (*callbacks).userdata as *mut Arc<UnsafeCell<ExtraData>>;
                    drop(Box::from_raw(extra_ptr));
                    (*callbacks).userdata = ptr::null_mut();
                }
                mlua_debug_assert!(
                    ffi::lua_gettop(extra.ref_thread) == extra.ref_stack_top
                        && extra.ref_stack_top as usize == extra.ref_free.len(),
                    "reference leak detected"
                );
                ffi::lua_close(mlua_expect!(self.main_state, "main_state is null"));
            }
        }
    }
}

impl Drop for ExtraData {
    fn drop(&mut self) {
        *mlua_expect!(self.registry_unref_list.lock(), "unref list poisoned") = None;
        if let Some(mem_info) = self.mem_info {
            drop(unsafe { Box::from_raw(mem_info.as_ptr()) });
        }
    }
}

impl fmt::Debug for Lua {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Lua({:p})", self.state)
    }
}

impl Lua {
    /// Creates a new Lua state and loads the **safe** subset of the standard libraries.
    ///
    /// # Safety
    /// The created Lua state would have _some_ safety guarantees and would not allow to load unsafe
    /// standard libraries or C modules.
    ///
    /// See [`StdLib`] documentation for a list of unsafe modules that cannot be loaded.
    ///
    /// [`StdLib`]: crate::StdLib
    #[allow(clippy::new_without_default)]
    pub fn new() -> Lua {
        mlua_expect!(
            Self::new_with(StdLib::ALL_SAFE, LuaOptions::default()),
            "can't create new safe Lua state"
        )
    }

    /// Creates a new Lua state and loads all the standard libraries.
    ///
    /// # Safety
    /// The created Lua state would not have safety guarantees and would allow to load C modules.
    pub unsafe fn unsafe_new() -> Lua {
        Self::unsafe_new_with(StdLib::ALL, LuaOptions::default())
    }

    /// Creates a new Lua state and loads the specified safe subset of the standard libraries.
    ///
    /// Use the [`StdLib`] flags to specify the libraries you want to load.
    ///
    /// # Safety
    /// The created Lua state would have _some_ safety guarantees and would not allow to load unsafe
    /// standard libraries or C modules.
    ///
    /// See [`StdLib`] documentation for a list of unsafe modules that cannot be loaded.
    ///
    /// [`StdLib`]: crate::StdLib
    pub fn new_with(libs: StdLib, options: LuaOptions) -> Result<Lua> {
        if libs.contains(StdLib::DEBUG) {
            return Err(Error::SafetyError(
                "the unsafe `debug` module can't be loaded using safe `new_with`".to_string(),
            ));
        }
        #[cfg(feature = "luajit")]
        {
            if libs.contains(StdLib::FFI) {
                return Err(Error::SafetyError(
                    "the unsafe `ffi` module can't be loaded using safe `new_with`".to_string(),
                ));
            }
        }

        let mut lua = unsafe { Self::inner_new(libs, options) };

        #[cfg(not(feature = "luau"))]
        if libs.contains(StdLib::PACKAGE) {
            mlua_expect!(lua.disable_c_modules(), "Error during disabling C modules");
        }
        lua.safe = true;
        unsafe { (*lua.extra.get()).safe = true };

        Ok(lua)
    }

    /// Creates a new Lua state and loads the specified subset of the standard libraries.
    ///
    /// Use the [`StdLib`] flags to specify the libraries you want to load.
    ///
    /// # Safety
    /// The created Lua state will not have safety guarantees and allow to load C modules.
    ///
    /// [`StdLib`]: crate::StdLib
    pub unsafe fn unsafe_new_with(libs: StdLib, options: LuaOptions) -> Lua {
        #[cfg(not(feature = "luau"))]
        ffi::keep_lua_symbols();
        Self::inner_new(libs, options)
    }

    unsafe fn inner_new(libs: StdLib, options: LuaOptions) -> Lua {
        #[cfg_attr(
            any(feature = "lua51", feature = "luajit", feature = "luau"),
            allow(dead_code)
        )]
        unsafe extern "C" fn allocator(
            extra_data: *mut c_void,
            ptr: *mut c_void,
            osize: usize,
            nsize: usize,
        ) -> *mut c_void {
            use std::alloc;

            let mem_info = &mut *(extra_data as *mut MemoryInfo);

            if nsize == 0 {
                // Free memory
                if !ptr.is_null() {
                    let layout =
                        alloc::Layout::from_size_align_unchecked(osize, ffi::SYS_MIN_ALIGN);
                    alloc::dealloc(ptr as *mut u8, layout);
                    mem_info.used_memory -= osize as isize;
                }
                return ptr::null_mut();
            }

            // Are we fit to the memory limits?
            let mut mem_diff = nsize as isize;
            if !ptr.is_null() {
                mem_diff -= osize as isize;
            }
            let new_used_memory = mem_info.used_memory + mem_diff;
            if mem_info.memory_limit > 0 && new_used_memory > mem_info.memory_limit {
                return ptr::null_mut();
            }

            let new_layout = alloc::Layout::from_size_align_unchecked(nsize, ffi::SYS_MIN_ALIGN);

            if ptr.is_null() {
                // Allocate new memory
                let new_ptr = alloc::alloc(new_layout) as *mut c_void;
                if !new_ptr.is_null() {
                    mem_info.used_memory += mem_diff;
                }
                return new_ptr;
            }

            // Reallocate memory
            let old_layout = alloc::Layout::from_size_align_unchecked(osize, ffi::SYS_MIN_ALIGN);
            let new_ptr = alloc::realloc(ptr as *mut u8, old_layout, nsize) as *mut c_void;

            if !new_ptr.is_null() {
                mem_info.used_memory += mem_diff;
            } else if !ptr.is_null() && nsize < osize {
                // Should not happen
                alloc::handle_alloc_error(new_layout);
            }

            new_ptr
        }

        #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
        let mem_info = Box::into_raw(Box::new(MemoryInfo {
            used_memory: 0,
            memory_limit: 0,
        }));

        #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
        let state = ffi::lua_newstate(allocator, mem_info as *mut c_void);
        #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
        let state = ffi::luaL_newstate();

        ffi::luaL_requiref(state, cstr!("_G"), ffi::luaopen_base, 1);
        ffi::lua_pop(state, 1);

        let mut lua = Lua::init_from_ptr(state);
        lua.ephemeral = false;

        let extra = &mut *lua.extra.get();

        #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
        {
            extra.mem_info = ptr::NonNull::new(mem_info);
        }

        mlua_expect!(
            load_from_std_lib(state, libs),
            "Error during loading standard libraries"
        );
        extra.libs |= libs;

        if !options.catch_rust_panics {
            mlua_expect!(
                (|| -> Result<()> {
                    let _sg = StackGuard::new(lua.state);

                    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
                    ffi::lua_rawgeti(lua.state, ffi::LUA_REGISTRYINDEX, ffi::LUA_RIDX_GLOBALS);
                    #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
                    ffi::lua_pushvalue(lua.state, ffi::LUA_GLOBALSINDEX);

                    ffi::lua_pushcfunction(lua.state, safe_pcall);
                    rawset_field(lua.state, -2, "pcall")?;

                    ffi::lua_pushcfunction(lua.state, safe_xpcall);
                    rawset_field(lua.state, -2, "xpcall")?;

                    Ok(())
                })(),
                "Error during applying option `catch_rust_panics`"
            )
        }

        #[cfg(feature = "async")]
        if options.thread_cache_size > 0 {
            extra.recycled_thread_cache = Vec::with_capacity(options.thread_cache_size);
        }

        #[cfg(feature = "luau")]
        mlua_expect!(lua.prepare_luau_state(), "Error preparing Luau state");

        lua
    }

    /// Constructs a new Lua instance from an existing raw state.
    ///
    /// Once called, a returned Lua state is cached in the registry and can be retrieved
    /// by calling this function again.
    #[allow(clippy::missing_safety_doc)]
    pub unsafe fn init_from_ptr(state: *mut ffi::lua_State) -> Lua {
        let maybe_main_state = get_main_state(state);
        let main_state = maybe_main_state.unwrap_or(state);
        let main_state_top = ffi::lua_gettop(main_state);

        if let Some(lua) = Lua::make_from_ptr(state) {
            return lua;
        }

        mlua_expect!(
            (|state| {
                init_error_registry(state)?;

                // Create the internal metatables and place them in the registry
                // to prevent them from being garbage collected.

                init_gc_metatable::<Arc<UnsafeCell<ExtraData>>>(state, None)?;
                init_gc_metatable::<Callback>(state, None)?;
                init_gc_metatable::<CallbackUpvalue>(state, None)?;
                #[cfg(feature = "async")]
                {
                    init_gc_metatable::<AsyncCallback>(state, None)?;
                    init_gc_metatable::<AsyncCallbackUpvalue>(state, None)?;
                    init_gc_metatable::<AsyncPollUpvalue>(state, None)?;
                    init_gc_metatable::<Option<Waker>>(state, None)?;
                }

                // Init serde metatables
                #[cfg(feature = "serialize")]
                crate::serde::init_metatables(state)?;

                Ok::<_, Error>(())
            })(main_state),
            "Error during Lua construction",
        );

        // Create ref stack thread and place it in the registry to prevent it from being garbage
        // collected.
        let ref_thread = mlua_expect!(
            protect_lua!(state, 0, 0, |state| {
                let thread = ffi::lua_newthread(state);
                ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX);
                thread
            }),
            "Error while creating ref thread",
        );

        // Create empty Waker slot on the ref thread
        #[cfg(feature = "async")]
        let ref_waker_idx = {
            mlua_expect!(
                push_gc_userdata::<Option<Waker>>(ref_thread, None),
                "Error while creating Waker slot"
            );
            ffi::lua_gettop(ref_thread)
        };
        let ref_stack_top = ffi::lua_gettop(ref_thread);

        // Create ExtraData

        let extra = Arc::new(UnsafeCell::new(ExtraData {
            registered_userdata: FxHashMap::default(),
            registered_userdata_mt: FxHashMap::default(),
            registry_unref_list: Arc::new(Mutex::new(Some(Vec::new()))),
            app_data: RefCell::new(HashMap::new()),
            ref_thread,
            libs: StdLib::NONE,
            mem_info: None,
            safe: false,
            // We need 1 extra stack space to move values in and out of the ref stack.
            ref_stack_size: ffi::LUA_MINSTACK - 1,
            ref_stack_top,
            ref_free: Vec::new(),
            wrapped_failures_cache: Vec::with_capacity(WRAPPED_FAILURES_CACHE_SIZE),
            multivalue_cache: Vec::with_capacity(MULTIVALUE_CACHE_SIZE),
            #[cfg(feature = "async")]
            recycled_thread_cache: Vec::new(),
            #[cfg(feature = "async")]
            ref_waker_idx,
            #[cfg(not(feature = "luau"))]
            hook_callback: None,
            #[cfg(feature = "lua54")]
            warn_callback: None,
            #[cfg(feature = "luau")]
            interrupt_callback: None,
            #[cfg(feature = "luau")]
            sandboxed: false,
        }));

        mlua_expect!(
            (|state| {
                push_gc_userdata(state, Arc::clone(&extra))?;
                protect_lua!(main_state, 1, 0, fn(state) {
                    let extra_key = &EXTRA_REGISTRY_KEY as *const u8 as *const c_void;
                    ffi::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, extra_key);
                })
            })(main_state),
            "Error while storing extra data",
        );

        // Register `DestructedUserdataMT` type
        get_destructed_userdata_metatable(main_state);
        let destructed_mt_ptr = ffi::lua_topointer(main_state, -1);
        (*extra.get()).registered_userdata_mt.insert(
            destructed_mt_ptr,
            Some(TypeId::of::<DestructedUserdataMT>()),
        );
        ffi::lua_pop(main_state, 1);

        mlua_debug_assert!(
            ffi::lua_gettop(main_state) == main_state_top,
            "stack leak during creation"
        );
        assert_stack(main_state, ffi::LUA_MINSTACK);

        // Set Luau callbacks userdata to extra data
        // We can use global callbacks userdata since we don't allow C modules in Luau
        #[cfg(feature = "luau")]
        {
            let extra_raw = Box::into_raw(Box::new(Arc::clone(&extra)));
            (*ffi::lua_callbacks(main_state)).userdata = extra_raw as *mut c_void;
        }

        Lua {
            state,
            main_state: maybe_main_state,
            extra,
            ephemeral: true,
            safe: false,
            _no_ref_unwind_safe: PhantomData,
        }
    }

    /// Loads the specified subset of the standard libraries into an existing Lua state.
    ///
    /// Use the [`StdLib`] flags to specify the libraries you want to load.
    ///
    /// [`StdLib`]: crate::StdLib
    pub fn load_from_std_lib(&self, libs: StdLib) -> Result<()> {
        if self.safe && libs.contains(StdLib::DEBUG) {
            return Err(Error::SafetyError(
                "the unsafe `debug` module can't be loaded in safe mode".to_string(),
            ));
        }
        #[cfg(feature = "luajit")]
        {
            if self.safe && libs.contains(StdLib::FFI) {
                return Err(Error::SafetyError(
                    "the unsafe `ffi` module can't be loaded in safe mode".to_string(),
                ));
            }
        }

        let state = self.main_state.unwrap_or(self.state);
        let res = unsafe { load_from_std_lib(state, libs) };

        // If `package` library loaded into a safe lua state then disable C modules
        let extra = unsafe { &mut *self.extra.get() };
        #[cfg(not(feature = "luau"))]
        {
            let curr_libs = extra.libs;
            if self.safe && (curr_libs ^ (curr_libs | libs)).contains(StdLib::PACKAGE) {
                mlua_expect!(self.disable_c_modules(), "Error during disabling C modules");
            }
        }
        extra.libs |= libs;

        res
    }

    /// Loads module `modname` into an existing Lua state using the specified entrypoint
    /// function.
    ///
    /// Internally calls the Lua function `func` with the string `modname` as an argument,
    /// sets the call result to `package.loaded[modname]` and returns copy of the result.
    ///
    /// If `package.loaded[modname]` value is not nil, returns copy of the value without
    /// calling the function.
    ///
    /// If the function does not return a non-nil value then this method assigns true to
    /// `package.loaded[modname]`.
    ///
    /// Behavior is similar to Lua's [`require`] function.
    ///
    /// [`require`]: https://www.lua.org/manual/5.4/manual.html#pdf-require
    pub fn load_from_function<'lua, S, T>(
        &'lua self,
        modname: &S,
        func: Function<'lua>,
    ) -> Result<T>
    where
        S: AsRef<[u8]> + ?Sized,
        T: FromLua<'lua>,
    {
        let loaded = unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 2)?;
            protect_lua!(self.state, 0, 1, fn(state) {
                ffi::luaL_getsubtable(state, ffi::LUA_REGISTRYINDEX, cstr!("_LOADED"));
            })?;
            Table(self.pop_ref())
        };

        let modname = self.create_string(modname)?;
        let value = match loaded.raw_get(modname.clone())? {
            Value::Nil => {
                let result = match func.call(modname.clone())? {
                    Value::Nil => Value::Boolean(true),
                    res => res,
                };
                loaded.raw_set(modname, result.clone())?;
                result
            }
            res => res,
        };
        T::from_lua(value, self)
    }

    /// Unloads module `modname`.
    ///
    /// Removes module from the [`package.loaded`] table which allows to load it again.
    /// It does not support unloading binary Lua modules since they are internally cached and can be
    /// unloaded only by closing Lua state.
    ///
    /// [`package.loaded`]: https://www.lua.org/manual/5.4/manual.html#pdf-package.loaded
    pub fn unload<S>(&self, modname: &S) -> Result<()>
    where
        S: AsRef<[u8]> + ?Sized,
    {
        let loaded = unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 2)?;
            protect_lua!(self.state, 0, 1, fn(state) {
                ffi::luaL_getsubtable(state, ffi::LUA_REGISTRYINDEX, cstr!("_LOADED"));
            })?;
            Table(self.pop_ref())
        };

        let modname = self.create_string(modname)?;
        loaded.raw_remove(modname)?;
        Ok(())
    }

    // Executes module entrypoint function, which returns only one Value.
    // The returned value then pushed onto the stack.
    #[doc(hidden)]
    #[cfg(not(tarpaulin_include))]
    pub unsafe fn entrypoint<'lua, A, R, F>(self, func: F) -> Result<c_int>
    where
        A: FromLuaMulti<'lua>,
        R: ToLua<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua, A) -> Result<R>,
    {
        let entrypoint_inner = |lua: &'lua Lua, func: F| {
            let nargs = ffi::lua_gettop(lua.state);
            check_stack(lua.state, 3)?;

            let mut args = MultiValue::new();
            args.reserve(nargs as usize);
            for _ in 0..nargs {
                args.push_front(lua.pop_value());
            }

            // We create callback rather than call `func` directly to catch errors
            // with attached stacktrace.
            let callback = lua.create_callback(Box::new(move |lua, args| {
                func(lua, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
            }))?;
            callback.call(args)
        };

        match entrypoint_inner(mem::transmute(&self), func) {
            Ok(res) => {
                self.push_value(res)?;
                Ok(1)
            }
            Err(err) => {
                self.push_value(Value::Error(err))?;
                let state = self.state;
                // Lua (self) must be dropped before triggering longjmp
                drop(self);
                ffi::lua_error(state)
            }
        }
    }

    // A simple module entrypoint without arguments
    #[doc(hidden)]
    #[cfg(not(tarpaulin_include))]
    pub unsafe fn entrypoint1<'lua, R, F>(self, func: F) -> Result<c_int>
    where
        R: ToLua<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua) -> Result<R>,
    {
        self.entrypoint(move |lua, _: ()| func(lua))
    }

    /// Enables (or disables) sandbox mode on this Lua instance.
    ///
    /// This method, in particular:
    /// - Set all libraries to read-only
    /// - Set all builtin metatables to read-only
    /// - Set globals to read-only (and activates safeenv)
    /// - Setup local environment table that performs writes locally and proxies reads
    ///   to the global environment.
    ///
    /// # Examples
    ///
    /// ```
    /// # use mlua::{Lua, Result};
    /// # fn main() -> Result<()> {
    /// let lua = Lua::new();
    ///
    /// lua.sandbox(true)?;
    /// lua.load("var = 123").exec()?;
    /// assert_eq!(lua.globals().get::<_, u32>("var")?, 123);
    ///
    /// // Restore the global environment (clear changes made in sandbox)
    /// lua.sandbox(false)?;
    /// assert_eq!(lua.globals().get::<_, Option<u32>>("var")?, None);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Requires `feature = "luau"`
    #[cfg(feature = "luau")]
    pub fn sandbox(&self, enabled: bool) -> Result<()> {
        unsafe {
            let extra = &mut *self.extra.get();
            if extra.sandboxed != enabled {
                let state = self.main_state.ok_or(Error::MainThreadNotAvailable)?;
                check_stack(state, 3)?;
                protect_lua!(state, 0, 0, |state| {
                    if enabled {
                        ffi::luaL_sandbox(state, 1);
                        ffi::luaL_sandboxthread(state);
                    } else {
                        // Restore original `LUA_GLOBALSINDEX`
                        self.ref_thread_exec(|ref_thread| {
                            ffi::lua_xpush(ref_thread, state, ffi::LUA_GLOBALSINDEX);
                            ffi::lua_replace(state, ffi::LUA_GLOBALSINDEX);
                        });
                        ffi::luaL_sandbox(state, 0);
                    }
                })?;
                extra.sandboxed = enabled;
            }
            Ok(())
        }
    }

    /// Sets a 'hook' function that will periodically be called as Lua code executes.
    ///
    /// When exactly the hook function is called depends on the contents of the `triggers`
    /// parameter, see [`HookTriggers`] for more details.
    ///
    /// The provided hook function can error, and this error will be propagated through the Lua code
    /// that was executing at the time the hook was triggered. This can be used to implement a
    /// limited form of execution limits by setting [`HookTriggers.every_nth_instruction`] and
    /// erroring once an instruction limit has been reached.
    ///
    /// # Example
    ///
    /// Shows each line number of code being executed by the Lua interpreter.
    ///
    /// ```
    /// # use mlua::{Lua, HookTriggers, Result};
    /// # fn main() -> Result<()> {
    /// let lua = Lua::new();
    /// lua.set_hook(HookTriggers::every_line(), |_lua, debug| {
    ///     println!("line {}", debug.curr_line());
    ///     Ok(())
    /// })?;
    ///
    /// lua.load(r#"
    ///     local x = 2 + 3
    ///     local y = x * 63
    ///     local z = string.len(x..", "..y)
    /// "#).exec()
    /// # }
    /// ```
    ///
    /// [`HookTriggers`]: crate::HookTriggers
    /// [`HookTriggers.every_nth_instruction`]: crate::HookTriggers::every_nth_instruction
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub fn set_hook<F>(&self, triggers: HookTriggers, callback: F) -> Result<()>
    where
        F: 'static + MaybeSend + Fn(&Lua, Debug) -> Result<()>,
    {
        unsafe extern "C" fn hook_proc(state: *mut ffi::lua_State, ar: *mut ffi::lua_Debug) {
            let lua = match Lua::make_from_ptr(state) {
                Some(lua) => lua,
                None => return,
            };
            let extra = lua.extra.get();
            callback_error_ext(state, extra, move |_| {
                let debug = Debug::new(&lua, ar);
                let hook_cb = (*lua.extra.get()).hook_callback.clone();
                let hook_cb = mlua_expect!(hook_cb, "no hook callback set in hook_proc");
                if Arc::strong_count(&hook_cb) > 2 {
                    return Ok(()); // Don't allow recursion
                }
                hook_cb(&lua, debug)
            })
        }

        let state = self.main_state.ok_or(Error::MainThreadNotAvailable)?;
        unsafe {
            (*self.extra.get()).hook_callback = Some(Arc::new(callback));
            ffi::lua_sethook(state, Some(hook_proc), triggers.mask(), triggers.count());
        }
        Ok(())
    }

    /// Removes any hook previously set by `set_hook`.
    ///
    /// This function has no effect if a hook was not previously set.
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub fn remove_hook(&self) {
        // If main_state is not available, then sethook wasn't called.
        let state = match self.main_state {
            Some(state) => state,
            None => return,
        };
        unsafe {
            (*self.extra.get()).hook_callback = None;
            ffi::lua_sethook(state, None, 0, 0);
        }
    }

    /// Sets an 'interrupt' function that will periodically be called by Luau VM.
    ///
    /// Any Luau code is guaranteed to call this handler "eventually"
    /// (in practice this can happen at any function call or at any loop iteration).
    ///
    /// The provided interrupt function can error, and this error will be propagated through
    /// the Luau code that was executing at the time the interrupt was triggered.
    /// Also this can be used to implement continuous execution limits by instructing Luau VM to yield
    /// by returning [`VmState::Yield`].
    ///
    /// This is similar to [`Lua::set_hook`] but in more simplified form.
    ///
    /// # Example
    ///
    /// Periodically yield Luau VM to suspend execution.
    ///
    /// ```
    /// # use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
    /// # use mlua::{Lua, Result, ThreadStatus, VmState};
    /// # fn main() -> Result<()> {
    /// let lua = Lua::new();
    /// let count = Arc::new(AtomicU64::new(0));
    /// lua.set_interrupt(move |_lua| {
    ///     if count.fetch_add(1, Ordering::Relaxed) % 2 == 0 {
    ///         return Ok(VmState::Yield);
    ///     }
    ///     Ok(VmState::Continue)
    /// });
    ///
    /// let co = lua.create_thread(
    ///     lua.load(r#"
    ///         local b = 0
    ///         for _, x in ipairs({1, 2, 3}) do b += x end
    ///     "#)
    ///     .into_function()?,
    /// )?;
    /// while co.status() == ThreadStatus::Resumable {
    ///     co.resume(())?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "luau")]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn set_interrupt<F>(&self, callback: F)
    where
        F: 'static + MaybeSend + Fn(&Lua) -> Result<VmState>,
    {
        unsafe extern "C" fn interrupt_proc(state: *mut ffi::lua_State, gc: c_int) {
            if gc != -1 {
                // We don't support GC interrupts since they cannot survive Lua exceptions
                return;
            }
            // TODO: think about not using drop types here
            let lua = match Lua::make_from_ptr(state) {
                Some(lua) => lua,
                None => return,
            };
            let extra = lua.extra.get();
            let result = callback_error_ext(state, extra, move |_| {
                let interrupt_cb = (*extra).interrupt_callback.clone();
                let interrupt_cb =
                    mlua_expect!(interrupt_cb, "no interrupt callback set in interrupt_proc");
                if Arc::strong_count(&interrupt_cb) > 2 {
                    return Ok(VmState::Continue); // Don't allow recursion
                }
                interrupt_cb(&lua)
            });
            match result {
                VmState::Continue => {}
                VmState::Yield => {
                    ffi::lua_yield(state, 0);
                }
            }
        }

        let state = mlua_expect!(self.main_state, "Luau should always has main state");
        unsafe {
            (*self.extra.get()).interrupt_callback = Some(Arc::new(callback));
            (*ffi::lua_callbacks(state)).interrupt = Some(interrupt_proc);
        }
    }

    /// Removes any 'interrupt' previously set by `set_interrupt`.
    ///
    /// This function has no effect if an 'interrupt' was not previously set.
    #[cfg(feature = "luau")]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn remove_interrupt(&self) {
        let state = mlua_expect!(self.main_state, "Luau should always has main state");
        unsafe {
            (*self.extra.get()).interrupt_callback = None;
            (*ffi::lua_callbacks(state)).interrupt = None;
        }
    }

    /// Sets the warning function to be used by Lua to emit warnings.
    ///
    /// Requires `feature = "lua54"`
    #[cfg(feature = "lua54")]
    pub fn set_warning_function<F>(&self, callback: F)
    where
        F: 'static + MaybeSend + Fn(&Lua, &CStr, bool) -> Result<()>,
    {
        unsafe extern "C" fn warn_proc(ud: *mut c_void, msg: *const c_char, tocont: c_int) {
            let state = ud as *mut ffi::lua_State;
            let lua = match Lua::make_from_ptr(state) {
                Some(lua) => lua,
                None => return,
            };
            let extra = lua.extra.get();
            callback_error_ext(state, extra, move |_| {
                let cb = mlua_expect!(
                    (*lua.extra.get()).warn_callback.as_ref(),
                    "no warning callback set in warn_proc"
                );
                let msg = CStr::from_ptr(msg);
                cb(&lua, msg, tocont != 0)
            });
        }

        let state = self.main_state.unwrap_or(self.state);
        unsafe {
            (*self.extra.get()).warn_callback = Some(Box::new(callback));
            ffi::lua_setwarnf(state, Some(warn_proc), state as *mut c_void);
        }
    }

    /// Removes warning function previously set by `set_warning_function`.
    ///
    /// This function has no effect if a warning function was not previously set.
    ///
    /// Requires `feature = "lua54"`
    #[cfg(feature = "lua54")]
    pub fn remove_warning_function(&self) {
        let state = self.main_state.unwrap_or(self.state);
        unsafe {
            (*self.extra.get()).warn_callback = None;
            ffi::lua_setwarnf(state, None, ptr::null_mut());
        }
    }

    /// Emits a warning with the given message.
    ///
    /// A message in a call with `tocont` set to `true` should be continued in another call to this function.
    ///
    /// Requires `feature = "lua54"`
    #[cfg(feature = "lua54")]
    pub fn warning<S: Into<Vec<u8>>>(&self, msg: S, tocont: bool) -> Result<()> {
        let msg = CString::new(msg).map_err(|err| Error::RuntimeError(err.to_string()))?;
        unsafe { ffi::lua_warning(self.state, msg.as_ptr(), if tocont { 1 } else { 0 }) };
        Ok(())
    }

    /// Gets information about the interpreter runtime stack.
    ///
    /// This function returns [`Debug`] structure that can be used to get information about the function
    /// executing at a given level. Level `0` is the current running function, whereas level `n+1` is the
    /// function that has called level `n` (except for tail calls, which do not count in the stack).
    ///
    /// [`Debug`]: crate::hook::Debug
    pub fn inspect_stack(&self, level: usize) -> Option<Debug> {
        unsafe {
            let mut ar: ffi::lua_Debug = mem::zeroed();
            let level = level as c_int;
            #[cfg(not(feature = "luau"))]
            if ffi::lua_getstack(self.state, level, &mut ar) == 0 {
                return None;
            }
            #[cfg(feature = "luau")]
            if ffi::lua_getinfo(self.state, level, cstr!(""), &mut ar) == 0 {
                return None;
            }
            Some(Debug::new_owned(self, level, ar))
        }
    }

    /// Returns the amount of memory (in bytes) currently used inside this Lua state.
    pub fn used_memory(&self) -> usize {
        unsafe {
            let state = self.main_state.unwrap_or(self.state);
            match (*self.extra.get()).mem_info.map(|x| x.as_ref()) {
                Some(mem_info) => mem_info.used_memory as usize,
                None => {
                    // Get data from the Lua GC
                    let used_kbytes = ffi::lua_gc(state, ffi::LUA_GCCOUNT, 0);
                    let used_kbytes_rem = ffi::lua_gc(state, ffi::LUA_GCCOUNTB, 0);
                    (used_kbytes as usize) * 1024 + (used_kbytes_rem as usize)
                }
            }
        }
    }

    /// Sets a memory limit (in bytes) on this Lua state.
    ///
    /// Once an allocation occurs that would pass this memory limit,
    /// a `Error::MemoryError` is generated instead.
    /// Returns previous limit (zero means no limit).
    ///
    /// Does not work on module mode where Lua state is managed externally.
    ///
    /// Requires `feature = "lua54/lua53/lua52"`
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    pub fn set_memory_limit(&self, memory_limit: usize) -> Result<usize> {
        unsafe {
            match (*self.extra.get()).mem_info.map(|mut x| x.as_mut()) {
                Some(mem_info) => {
                    let prev_limit = mem_info.memory_limit as usize;
                    mem_info.memory_limit = memory_limit as isize;
                    Ok(prev_limit)
                }
                None => Err(Error::MemoryLimitNotAvailable),
            }
        }
    }

    /// Returns true if the garbage collector is currently running automatically.
    ///
    /// Requires `feature = "lua54/lua53/lua52/luau"`
    #[cfg(any(
        feature = "lua54",
        feature = "lua53",
        feature = "lua52",
        feature = "luau"
    ))]
    pub fn gc_is_running(&self) -> bool {
        let state = self.main_state.unwrap_or(self.state);
        unsafe { ffi::lua_gc(state, ffi::LUA_GCISRUNNING, 0) != 0 }
    }

    /// Stop the Lua GC from running
    pub fn gc_stop(&self) {
        let state = self.main_state.unwrap_or(self.state);
        unsafe { ffi::lua_gc(state, ffi::LUA_GCSTOP, 0) };
    }

    /// Restarts the Lua GC if it is not running
    pub fn gc_restart(&self) {
        let state = self.main_state.unwrap_or(self.state);
        unsafe { ffi::lua_gc(state, ffi::LUA_GCRESTART, 0) };
    }

    /// Perform a full garbage-collection cycle.
    ///
    /// It may be necessary to call this function twice to collect all currently unreachable
    /// objects. Once to finish the current gc cycle, and once to start and finish the next cycle.
    pub fn gc_collect(&self) -> Result<()> {
        let state = self.main_state.unwrap_or(self.state);
        unsafe {
            check_stack(state, 3)?;
            protect_lua!(state, 0, 0, fn(state) ffi::lua_gc(state, ffi::LUA_GCCOLLECT, 0))
        }
    }

    /// Steps the garbage collector one indivisible step.
    ///
    /// Returns true if this has finished a collection cycle.
    pub fn gc_step(&self) -> Result<bool> {
        self.gc_step_kbytes(0)
    }

    /// Steps the garbage collector as though memory had been allocated.
    ///
    /// if `kbytes` is 0, then this is the same as calling `gc_step`. Returns true if this step has
    /// finished a collection cycle.
    pub fn gc_step_kbytes(&self, kbytes: c_int) -> Result<bool> {
        let state = self.main_state.unwrap_or(self.state);
        unsafe {
            check_stack(state, 3)?;
            protect_lua!(state, 0, 0, |state| {
                ffi::lua_gc(state, ffi::LUA_GCSTEP, kbytes) != 0
            })
        }
    }

    /// Sets the 'pause' value of the collector.
    ///
    /// Returns the previous value of 'pause'. More information can be found in the Lua
    /// [documentation][lua_doc].
    ///
    /// [lua_doc]: https://www.lua.org/manual/5.4/manual.html#2.5
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub fn gc_set_pause(&self, pause: c_int) -> c_int {
        let state = self.main_state.unwrap_or(self.state);
        unsafe { ffi::lua_gc(state, ffi::LUA_GCSETPAUSE, pause) }
    }

    /// Sets the 'step multiplier' value of the collector.
    ///
    /// Returns the previous value of the 'step multiplier'. More information can be found in the
    /// Lua [documentation][lua_doc].
    ///
    /// [lua_doc]: https://www.lua.org/manual/5.4/manual.html#2.5
    pub fn gc_set_step_multiplier(&self, step_multiplier: c_int) -> c_int {
        let state = self.main_state.unwrap_or(self.state);
        unsafe { ffi::lua_gc(state, ffi::LUA_GCSETSTEPMUL, step_multiplier) }
    }

    /// Changes the collector to incremental mode with the given parameters.
    ///
    /// Returns the previous mode (always `GCMode::Incremental` in Lua < 5.4).
    /// More information can be found in the Lua [documentation][lua_doc].
    ///
    /// [lua_doc]: https://www.lua.org/manual/5.4/manual.html#2.5.1
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub fn gc_inc(&self, pause: c_int, step_multiplier: c_int, step_size: c_int) -> GCMode {
        let state = self.main_state.unwrap_or(self.state);

        #[cfg(any(
            feature = "lua53",
            feature = "lua52",
            feature = "lua51",
            feature = "luajit"
        ))]
        {
            if pause > 0 {
                unsafe { ffi::lua_gc(state, ffi::LUA_GCSETPAUSE, pause) };
            }
            if step_multiplier > 0 {
                unsafe { ffi::lua_gc(state, ffi::LUA_GCSETSTEPMUL, step_multiplier) };
            }
            let _ = step_size; // Ignored
            GCMode::Incremental
        }

        #[cfg(feature = "lua54")]
        let prev_mode =
            unsafe { ffi::lua_gc(state, ffi::LUA_GCINC, pause, step_multiplier, step_size) };
        #[cfg(feature = "lua54")]
        match prev_mode {
            ffi::LUA_GCINC => GCMode::Incremental,
            ffi::LUA_GCGEN => GCMode::Generational,
            _ => unreachable!(),
        }
    }

    /// Changes the collector to generational mode with the given parameters.
    ///
    /// Returns the previous mode. More information about the generational GC
    /// can be found in the Lua 5.4 [documentation][lua_doc].
    ///
    /// Requires `feature = "lua54"`
    ///
    /// [lua_doc]: https://www.lua.org/manual/5.4/manual.html#2.5.2
    #[cfg(any(feature = "lua54"))]
    pub fn gc_gen(&self, minor_multiplier: c_int, major_multiplier: c_int) -> GCMode {
        let state = self.main_state.unwrap_or(self.state);
        let prev_mode =
            unsafe { ffi::lua_gc(state, ffi::LUA_GCGEN, minor_multiplier, major_multiplier) };
        match prev_mode {
            ffi::LUA_GCGEN => GCMode::Generational,
            ffi::LUA_GCINC => GCMode::Incremental,
            _ => unreachable!(),
        }
    }

    /// Returns Lua source code as a `Chunk` builder type.
    ///
    /// In order to actually compile or run the resulting code, you must call [`Chunk::exec`] or
    /// similar on the returned builder. Code is not even parsed until one of these methods is
    /// called.
    ///
    /// [`Chunk::exec`]: crate::Chunk::exec
    #[track_caller]
    pub fn load<'lua, 'a, S>(&'lua self, source: &'a S) -> Chunk<'lua, 'a>
    where
        S: AsChunk<'lua> + ?Sized,
    {
        Chunk {
            lua: self,
            source: Cow::Borrowed(source.source()),
            name: match source.name() {
                Some(name) => Some(name),
                None => CString::new(Location::caller().to_string()).ok(),
            },
            env: source.env(self),
            mode: source.mode(),
            #[cfg(feature = "luau")]
            compiler: None,
        }
    }

    pub(crate) fn load_chunk<'lua>(
        &'lua self,
        source: &[u8],
        name: Option<&CString>,
        env: Option<Value<'lua>>,
        mode: Option<ChunkMode>,
    ) -> Result<Function<'lua>> {
        unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 1)?;

            let mode_str = match mode {
                Some(ChunkMode::Binary) => cstr!("b"),
                Some(ChunkMode::Text) => cstr!("t"),
                None => cstr!("bt"),
            };

            match ffi::luaL_loadbufferx(
                self.state,
                source.as_ptr() as *const c_char,
                source.len(),
                name.map(|n| n.as_ptr()).unwrap_or_else(ptr::null),
                mode_str,
            ) {
                ffi::LUA_OK => {
                    if let Some(env) = env {
                        self.push_value(env)?;
                        #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
                        ffi::lua_setupvalue(self.state, -2, 1);
                        #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
                        ffi::lua_setfenv(self.state, -2);
                    }
                    Ok(Function(self.pop_ref()))
                }
                err => Err(pop_error(self.state, err)),
            }
        }
    }

    /// Create and return an interned Lua string. Lua strings can be arbitrary [u8] data including
    /// embedded nulls, so in addition to `&str` and `&String`, you can also pass plain `&[u8]`
    /// here.
    pub fn create_string<S>(&self, s: &S) -> Result<String>
    where
        S: AsRef<[u8]> + ?Sized,
    {
        unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 3)?;
            push_string(self.state, s)?;
            Ok(String(self.pop_ref()))
        }
    }

    /// Creates and returns a new empty table.
    pub fn create_table(&self) -> Result<Table> {
        unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 2)?;
            protect_lua!(self.state, 0, 1, fn(state) ffi::lua_newtable(state))?;
            Ok(Table(self.pop_ref()))
        }
    }

    /// Creates and returns a new empty table, with the specified capacity.
    /// `narr` is a hint for how many elements the table will have as a sequence;
    /// `nrec` is a hint for how many other elements the table will have.
    /// Lua may use these hints to preallocate memory for the new table.
    pub fn create_table_with_capacity(&self, narr: c_int, nrec: c_int) -> Result<Table> {
        unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 3)?;
            push_table(self.state, narr, nrec)?;
            Ok(Table(self.pop_ref()))
        }
    }

    /// Creates a table and fills it with values from an iterator.
    pub fn create_table_from<'lua, K, V, I>(&'lua self, iter: I) -> Result<Table<'lua>>
    where
        K: ToLua<'lua>,
        V: ToLua<'lua>,
        I: IntoIterator<Item = (K, V)>,
    {
        unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 6)?;

            let iter = iter.into_iter();
            let lower_bound = iter.size_hint().0;
            push_table(self.state, 0, lower_bound as c_int)?;
            for (k, v) in iter {
                self.push_value(k.to_lua(self)?)?;
                self.push_value(v.to_lua(self)?)?;
                protect_lua!(self.state, 3, 1, fn(state) ffi::lua_rawset(state, -3))?;
            }

            Ok(Table(self.pop_ref()))
        }
    }

    /// Creates a table from an iterator of values, using `1..` as the keys.
    pub fn create_sequence_from<'lua, T, I>(&'lua self, iter: I) -> Result<Table<'lua>>
    where
        T: ToLua<'lua>,
        I: IntoIterator<Item = T>,
    {
        unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 5)?;

            let iter = iter.into_iter();
            let lower_bound = iter.size_hint().0;
            push_table(self.state, lower_bound as c_int, 0)?;
            for (i, v) in iter.enumerate() {
                self.push_value(v.to_lua(self)?)?;
                protect_lua!(self.state, 2, 1, |state| {
                    ffi::lua_rawseti(state, -2, (i + 1) as Integer);
                })?;
            }

            Ok(Table(self.pop_ref()))
        }
    }

    /// Wraps a Rust function or closure, creating a callable Lua function handle to it.
    ///
    /// The function's return value is always a `Result`: If the function returns `Err`, the error
    /// is raised as a Lua error, which can be caught using `(x)pcall` or bubble up to the Rust code
    /// that invoked the Lua code. This allows using the `?` operator to propagate errors through
    /// intermediate Lua code.
    ///
    /// If the function returns `Ok`, the contained value will be converted to one or more Lua
    /// values. For details on Rust-to-Lua conversions, refer to the [`ToLua`] and [`ToLuaMulti`]
    /// traits.
    ///
    /// # Examples
    ///
    /// Create a function which prints its argument:
    ///
    /// ```
    /// # use mlua::{Lua, Result};
    /// # fn main() -> Result<()> {
    /// # let lua = Lua::new();
    /// let greet = lua.create_function(|_, name: String| {
    ///     println!("Hello, {}!", name);
    ///     Ok(())
    /// });
    /// # let _ = greet;    // used
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Use tuples to accept multiple arguments:
    ///
    /// ```
    /// # use mlua::{Lua, Result};
    /// # fn main() -> Result<()> {
    /// # let lua = Lua::new();
    /// let print_person = lua.create_function(|_, (name, age): (String, u8)| {
    ///     println!("{} is {} years old!", name, age);
    ///     Ok(())
    /// });
    /// # let _ = print_person;    // used
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`ToLua`]: crate::ToLua
    /// [`ToLuaMulti`]: crate::ToLuaMulti
    pub fn create_function<'lua, 'callback, A, R, F>(&'lua self, func: F) -> Result<Function<'lua>>
    where
        'lua: 'callback,
        A: FromLuaMulti<'callback>,
        R: ToLuaMulti<'callback>,
        F: 'static + MaybeSend + Fn(&'callback Lua, A) -> Result<R>,
    {
        self.create_callback(Box::new(move |lua, args| {
            func(lua, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
        }))
    }

    /// Wraps a Rust mutable closure, creating a callable Lua function handle to it.
    ///
    /// This is a version of [`create_function`] that accepts a FnMut argument. Refer to
    /// [`create_function`] for more information about the implementation.
    ///
    /// [`create_function`]: #method.create_function
    pub fn create_function_mut<'lua, 'callback, A, R, F>(
        &'lua self,
        func: F,
    ) -> Result<Function<'lua>>
    where
        'lua: 'callback,
        A: FromLuaMulti<'callback>,
        R: ToLuaMulti<'callback>,
        F: 'static + MaybeSend + FnMut(&'callback Lua, A) -> Result<R>,
    {
        let func = RefCell::new(func);
        self.create_function(move |lua, args| {
            (*func
                .try_borrow_mut()
                .map_err(|_| Error::RecursiveMutCallback)?)(lua, args)
        })
    }

    /// Wraps a C function, creating a callable Lua function handle to it.
    ///
    /// # Safety
    /// This function is unsafe because provides a way to execute unsafe C function.
    pub unsafe fn create_c_function(&self, func: ffi::lua_CFunction) -> Result<Function> {
        check_stack(self.state, 1)?;
        ffi::lua_pushcfunction(self.state, func);
        Ok(Function(self.pop_ref()))
    }

    /// Wraps a Rust async function or closure, creating a callable Lua function handle to it.
    ///
    /// While executing the function Rust will poll Future and if the result is not ready, call
    /// `yield()` passing internal representation of a `Poll::Pending` value.
    ///
    /// The function must be called inside Lua coroutine ([`Thread`]) to be able to suspend its execution.
    /// An executor should be used to poll [`AsyncThread`] and mlua will take a provided Waker
    /// in that case. Otherwise noop waker will be used if try to call the function outside of Rust
    /// executors.
    ///
    /// The family of `call_async()` functions takes care about creating [`Thread`].
    ///
    /// Requires `feature = "async"`
    ///
    /// # Examples
    ///
    /// Non blocking sleep:
    ///
    /// ```
    /// use std::time::Duration;
    /// use futures_timer::Delay;
    /// use mlua::{Lua, Result};
    ///
    /// async fn sleep(_lua: &Lua, n: u64) -> Result<&'static str> {
    ///     Delay::new(Duration::from_millis(n)).await;
    ///     Ok("done")
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let lua = Lua::new();
    ///     lua.globals().set("sleep", lua.create_async_function(sleep)?)?;
    ///     let res: String = lua.load("return sleep(...)").call_async(100).await?; // Sleep 100ms
    ///     assert_eq!(res, "done");
    ///     Ok(())
    /// }
    /// ```
    ///
    /// [`Thread`]: crate::Thread
    /// [`AsyncThread`]: crate::AsyncThread
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn create_async_function<'lua, 'callback, A, R, F, FR>(
        &'lua self,
        func: F,
    ) -> Result<Function<'lua>>
    where
        'lua: 'callback,
        A: FromLuaMulti<'callback>,
        R: ToLuaMulti<'callback>,
        F: 'static + MaybeSend + Fn(&'callback Lua, A) -> FR,
        FR: 'lua + Future<Output = Result<R>>,
    {
        self.create_async_callback(Box::new(move |lua, args| {
            let args = match A::from_lua_multi(args, lua) {
                Ok(args) => args,
                Err(e) => return Box::pin(future::err(e)),
            };
            Box::pin(func(lua, args).and_then(move |ret| future::ready(ret.to_lua_multi(lua))))
        }))
    }

    /// Wraps a Lua function into a new thread (or coroutine).
    ///
    /// Equivalent to `coroutine.create`.
    pub fn create_thread<'lua>(&'lua self, func: Function<'lua>) -> Result<Thread<'lua>> {
        unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 3)?;

            let thread_state = protect_lua!(self.state, 0, 1, |state| ffi::lua_newthread(state))?;
            self.push_ref(&func.0);
            ffi::lua_xmove(self.state, thread_state, 1);

            Ok(Thread(self.pop_ref()))
        }
    }

    /// Wraps a Lua function into a new or recycled thread (coroutine).
    #[cfg(feature = "async")]
    pub(crate) fn create_recycled_thread<'lua>(
        &'lua self,
        func: Function<'lua>,
    ) -> Result<Thread<'lua>> {
        #[cfg(any(
            feature = "lua54",
            all(feature = "luajit", feature = "vendored"),
            feature = "luau",
        ))]
        unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 1)?;

            let extra = &mut *self.extra.get();
            if let Some(index) = extra.recycled_thread_cache.pop() {
                let thread_state = ffi::lua_tothread(extra.ref_thread, index);
                self.push_ref(&func.0);
                ffi::lua_xmove(self.state, thread_state, 1);

                #[cfg(feature = "luau")]
                {
                    // Inherit `LUA_GLOBALSINDEX` from the caller
                    ffi::lua_xpush(self.state, thread_state, ffi::LUA_GLOBALSINDEX);
                    ffi::lua_replace(thread_state, ffi::LUA_GLOBALSINDEX);
                }

                return Ok(Thread(LuaRef { lua: self, index }));
            }
        };
        self.create_thread(func)
    }

    /// Resets thread (coroutine) and returns to the cache for later use.
    #[cfg(feature = "async")]
    #[cfg(any(
        feature = "lua54",
        all(feature = "luajit", feature = "vendored"),
        feature = "luau",
    ))]
    pub(crate) unsafe fn recycle_thread(&self, thread: &mut Thread) {
        let extra = &mut *self.extra.get();
        let thread_state = ffi::lua_tothread(extra.ref_thread, thread.0.index);
        if extra.recycled_thread_cache.len() < extra.recycled_thread_cache.capacity() {
            #[cfg(feature = "lua54")]
            let status = ffi::lua_resetthread(thread_state);
            #[cfg(feature = "lua54")]
            if status != ffi::LUA_OK {
                return;
            }
            #[cfg(all(feature = "luajit", feature = "vendored"))]
            ffi::lua_resetthread(self.state, thread_state);
            #[cfg(feature = "luau")]
            ffi::lua_resetthread(thread_state);
            extra.recycled_thread_cache.push(thread.0.index);
            thread.0.index = 0;
        }
    }

    /// Create a Lua userdata object from a custom userdata type.
    pub fn create_userdata<T>(&self, data: T) -> Result<AnyUserData>
    where
        T: 'static + MaybeSend + UserData,
    {
        unsafe { self.make_userdata(UserDataCell::new(data)) }
    }

    /// Create a Lua userdata object from a custom serializable userdata type.
    ///
    /// Requires `feature = "serialize"`
    #[cfg(feature = "serialize")]
    #[cfg_attr(docsrs, doc(cfg(feature = "serialize")))]
    pub fn create_ser_userdata<T>(&self, data: T) -> Result<AnyUserData>
    where
        T: 'static + MaybeSend + UserData + Serialize,
    {
        unsafe { self.make_userdata(UserDataCell::new_ser(data)) }
    }

    /// Returns a handle to the global environment.
    pub fn globals(&self) -> Table {
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 1);
            #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
            ffi::lua_rawgeti(self.state, ffi::LUA_REGISTRYINDEX, ffi::LUA_RIDX_GLOBALS);
            #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
            ffi::lua_pushvalue(self.state, ffi::LUA_GLOBALSINDEX);
            Table(self.pop_ref())
        }
    }

    /// Returns a handle to the active `Thread`. For calls to `Lua` this will be the main Lua thread,
    /// for parameters given to a callback, this will be whatever Lua thread called the callback.
    pub fn current_thread(&self) -> Thread {
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 1);
            ffi::lua_pushthread(self.state);
            Thread(self.pop_ref())
        }
    }

    /// Calls the given function with a `Scope` parameter, giving the function the ability to create
    /// userdata and callbacks from rust types that are !Send or non-'static.
    ///
    /// The lifetime of any function or userdata created through `Scope` lasts only until the
    /// completion of this method call, on completion all such created values are automatically
    /// dropped and Lua references to them are invalidated. If a script accesses a value created
    /// through `Scope` outside of this method, a Lua error will result. Since we can ensure the
    /// lifetime of values created through `Scope`, and we know that `Lua` cannot be sent to another
    /// thread while `Scope` is live, it is safe to allow !Send datatypes and whose lifetimes only
    /// outlive the scope lifetime.
    ///
    /// Inside the scope callback, all handles created through Scope will share the same unique 'lua
    /// lifetime of the parent `Lua`. This allows scoped and non-scoped values to be mixed in
    /// API calls, which is very useful (e.g. passing a scoped userdata to a non-scoped function).
    /// However, this also enables handles to scoped values to be trivially leaked from the given
    /// callback. This is not dangerous, though!  After the callback returns, all scoped values are
    /// invalidated, which means that though references may exist, the Rust types backing them have
    /// dropped. `Function` types will error when called, and `AnyUserData` will be typeless. It
    /// would be impossible to prevent handles to scoped values from escaping anyway, since you
    /// would always be able to smuggle them through Lua state.
    pub fn scope<'lua, 'scope, R, F>(&'lua self, f: F) -> Result<R>
    where
        'lua: 'scope,
        R: 'static,
        F: FnOnce(&Scope<'lua, 'scope>) -> Result<R>,
    {
        f(&Scope::new(self))
    }

    /// An asynchronous version of [`scope`] that allows to create scoped async functions and
    /// execute them.
    ///
    /// Requires `feature = "async"`
    ///
    /// [`scope`]: #method.scope
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn async_scope<'lua, 'scope, R, F, FR>(
        &'lua self,
        f: F,
    ) -> LocalBoxFuture<'scope, Result<R>>
    where
        'lua: 'scope,
        R: 'static,
        F: FnOnce(Scope<'lua, 'scope>) -> FR,
        FR: 'scope + Future<Output = Result<R>>,
    {
        Box::pin(f(Scope::new(self)))
    }

    /// Attempts to coerce a Lua value into a String in a manner consistent with Lua's internal
    /// behavior.
    ///
    /// To succeed, the value must be a string (in which case this is a no-op), an integer, or a
    /// number.
    pub fn coerce_string<'lua>(&'lua self, v: Value<'lua>) -> Result<Option<String<'lua>>> {
        Ok(match v {
            Value::String(s) => Some(s),
            v => unsafe {
                let _sg = StackGuard::new(self.state);
                check_stack(self.state, 4)?;

                self.push_value(v)?;
                let res = protect_lua!(self.state, 1, 1, |state| {
                    ffi::lua_tolstring(state, -1, ptr::null_mut())
                })?;
                if !res.is_null() {
                    Some(String(self.pop_ref()))
                } else {
                    None
                }
            },
        })
    }

    /// Attempts to coerce a Lua value into an integer in a manner consistent with Lua's internal
    /// behavior.
    ///
    /// To succeed, the value must be an integer, a floating point number that has an exact
    /// representation as an integer, or a string that can be converted to an integer. Refer to the
    /// Lua manual for details.
    pub fn coerce_integer(&self, v: Value) -> Result<Option<Integer>> {
        Ok(match v {
            Value::Integer(i) => Some(i),
            v => unsafe {
                let _sg = StackGuard::new(self.state);
                check_stack(self.state, 2)?;

                self.push_value(v)?;
                let mut isint = 0;
                let i = ffi::lua_tointegerx(self.state, -1, &mut isint);
                if isint == 0 {
                    None
                } else {
                    Some(i)
                }
            },
        })
    }

    /// Attempts to coerce a Lua value into a Number in a manner consistent with Lua's internal
    /// behavior.
    ///
    /// To succeed, the value must be a number or a string that can be converted to a number. Refer
    /// to the Lua manual for details.
    pub fn coerce_number(&self, v: Value) -> Result<Option<Number>> {
        Ok(match v {
            Value::Number(n) => Some(n),
            v => unsafe {
                let _sg = StackGuard::new(self.state);
                check_stack(self.state, 2)?;

                self.push_value(v)?;
                let mut isnum = 0;
                let n = ffi::lua_tonumberx(self.state, -1, &mut isnum);
                if isnum == 0 {
                    None
                } else {
                    Some(n)
                }
            },
        })
    }

    /// Converts a value that implements `ToLua` into a `Value` instance.
    pub fn pack<'lua, T: ToLua<'lua>>(&'lua self, t: T) -> Result<Value<'lua>> {
        t.to_lua(self)
    }

    /// Converts a `Value` instance into a value that implements `FromLua`.
    pub fn unpack<'lua, T: FromLua<'lua>>(&'lua self, value: Value<'lua>) -> Result<T> {
        T::from_lua(value, self)
    }

    /// Converts a value that implements `ToLuaMulti` into a `MultiValue` instance.
    pub fn pack_multi<'lua, T: ToLuaMulti<'lua>>(&'lua self, t: T) -> Result<MultiValue<'lua>> {
        t.to_lua_multi(self)
    }

    /// Converts a `MultiValue` instance into a value that implements `FromLuaMulti`.
    pub fn unpack_multi<'lua, T: FromLuaMulti<'lua>>(
        &'lua self,
        value: MultiValue<'lua>,
    ) -> Result<T> {
        T::from_lua_multi(value, self)
    }

    /// Set a value in the Lua registry based on a string name.
    ///
    /// This value will be available to rust from all `Lua` instances which share the same main
    /// state.
    pub fn set_named_registry_value<'lua, S, T>(&'lua self, name: &S, t: T) -> Result<()>
    where
        S: AsRef<[u8]> + ?Sized,
        T: ToLua<'lua>,
    {
        let t = t.to_lua(self)?;
        unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 5)?;

            self.push_value(t)?;
            rawset_field(self.state, ffi::LUA_REGISTRYINDEX, name)
        }
    }

    /// Get a value from the Lua registry based on a string name.
    ///
    /// Any Lua instance which shares the underlying main state may call this method to
    /// get a value previously set by [`set_named_registry_value`].
    ///
    /// [`set_named_registry_value`]: #method.set_named_registry_value
    pub fn named_registry_value<'lua, S, T>(&'lua self, name: &S) -> Result<T>
    where
        S: AsRef<[u8]> + ?Sized,
        T: FromLua<'lua>,
    {
        let value = unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 3)?;

            push_string(self.state, name)?;
            ffi::lua_rawget(self.state, ffi::LUA_REGISTRYINDEX);

            self.pop_value()
        };
        T::from_lua(value, self)
    }

    /// Removes a named value in the Lua registry.
    ///
    /// Equivalent to calling [`set_named_registry_value`] with a value of Nil.
    ///
    /// [`set_named_registry_value`]: #method.set_named_registry_value
    pub fn unset_named_registry_value<S>(&self, name: &S) -> Result<()>
    where
        S: AsRef<[u8]> + ?Sized,
    {
        self.set_named_registry_value(name, Nil)
    }

    /// Place a value in the Lua registry with an auto-generated key.
    ///
    /// This value will be available to Rust from all `Lua` instances which share the same main
    /// state.
    ///
    /// Be warned, garbage collection of values held inside the registry is not automatic, see
    /// [`RegistryKey`] for more details.
    /// However, dropped [`RegistryKey`]s automatically reused to store new values.
    ///
    /// [`RegistryKey`]: crate::RegistryKey
    pub fn create_registry_value<'lua, T: ToLua<'lua>>(&'lua self, t: T) -> Result<RegistryKey> {
        let t = t.to_lua(self)?;
        unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 4)?;

            let unref_list = (*self.extra.get()).registry_unref_list.clone();
            self.push_value(t)?;

            // Try to reuse previously allocated RegistryKey
            let unref_list2 = unref_list.clone();
            let mut unref_list2 = mlua_expect!(unref_list2.lock(), "unref list poisoned");
            if let Some(registry_id) = unref_list2.as_mut().and_then(|x| x.pop()) {
                // It must be safe to replace the value without triggering memory error
                ffi::lua_rawseti(self.state, ffi::LUA_REGISTRYINDEX, registry_id as Integer);
                return Ok(RegistryKey {
                    registry_id,
                    unref_list,
                });
            }

            // Allocate a new RegistryKey
            let registry_id = protect_lua!(self.state, 1, 0, |state| {
                ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX)
            })?;

            Ok(RegistryKey {
                registry_id,
                unref_list,
            })
        }
    }

    /// Get a value from the Lua registry by its `RegistryKey`
    ///
    /// Any Lua instance which shares the underlying main state may call this method to get a value
    /// previously placed by [`create_registry_value`].
    ///
    /// [`create_registry_value`]: #method.create_registry_value
    pub fn registry_value<'lua, T: FromLua<'lua>>(&'lua self, key: &RegistryKey) -> Result<T> {
        if !self.owns_registry_value(key) {
            return Err(Error::MismatchedRegistryKey);
        }

        let value = unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 1)?;

            ffi::lua_rawgeti(
                self.state,
                ffi::LUA_REGISTRYINDEX,
                key.registry_id as Integer,
            );
            self.pop_value()
        };
        T::from_lua(value, self)
    }

    /// Removes a value from the Lua registry.
    ///
    /// You may call this function to manually remove a value placed in the registry with
    /// [`create_registry_value`]. In addition to manual `RegistryKey` removal, you can also call
    /// [`expire_registry_values`] to automatically remove values from the registry whose
    /// `RegistryKey`s have been dropped.
    ///
    /// [`create_registry_value`]: #method.create_registry_value
    /// [`expire_registry_values`]: #method.expire_registry_values
    pub fn remove_registry_value(&self, key: RegistryKey) -> Result<()> {
        if !self.owns_registry_value(&key) {
            return Err(Error::MismatchedRegistryKey);
        }
        unsafe {
            ffi::luaL_unref(self.state, ffi::LUA_REGISTRYINDEX, key.take());
        }
        Ok(())
    }

    /// Replaces a value in the Lua registry by its `RegistryKey`.
    ///
    /// See [`create_registry_value`] for more details.
    ///
    /// [`create_registry_value`]: #method.create_registry_value
    pub fn replace_registry_value<'lua, T: ToLua<'lua>>(
        &'lua self,
        key: &RegistryKey,
        t: T,
    ) -> Result<()> {
        if !self.owns_registry_value(key) {
            return Err(Error::MismatchedRegistryKey);
        }

        let t = t.to_lua(self)?;
        unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 2)?;

            self.push_value(t)?;
            // It must be safe to replace the value without triggering memory error
            ffi::lua_rawseti(
                self.state,
                ffi::LUA_REGISTRYINDEX,
                key.registry_id as Integer,
            );

            Ok(())
        }
    }

    /// Returns true if the given `RegistryKey` was created by a `Lua` which shares the underlying
    /// main state with this `Lua` instance.
    ///
    /// Other than this, methods that accept a `RegistryKey` will return
    /// `Error::MismatchedRegistryKey` if passed a `RegistryKey` that was not created with a
    /// matching `Lua` state.
    pub fn owns_registry_value(&self, key: &RegistryKey) -> bool {
        let registry_unref_list = unsafe { &(*self.extra.get()).registry_unref_list };
        Arc::ptr_eq(&key.unref_list, registry_unref_list)
    }

    /// Remove any registry values whose `RegistryKey`s have all been dropped.
    ///
    /// Unlike normal handle values, `RegistryKey`s do not automatically remove themselves on Drop,
    /// but you can call this method to remove any unreachable registry values not manually removed
    /// by `Lua::remove_registry_value`.
    pub fn expire_registry_values(&self) {
        unsafe {
            let mut unref_list = mlua_expect!(
                (*self.extra.get()).registry_unref_list.lock(),
                "unref list poisoned"
            );
            let unref_list = mem::replace(&mut *unref_list, Some(Vec::new()));
            for id in mlua_expect!(unref_list, "unref list not set") {
                ffi::luaL_unref(self.state, ffi::LUA_REGISTRYINDEX, id);
            }
        }
    }

    /// Sets or replaces an application data object of type `T`.
    ///
    /// Application data could be accessed at any time by using [`Lua::app_data_ref()`] or [`Lua::app_data_mut()`]
    /// methods where `T` is the data type.
    ///
    /// # Examples
    ///
    /// ```
    /// use mlua::{Lua, Result};
    ///
    /// fn hello(lua: &Lua, _: ()) -> Result<()> {
    ///     let mut s = lua.app_data_mut::<&str>().unwrap();
    ///     assert_eq!(*s, "hello");
    ///     *s = "world";
    ///     Ok(())
    /// }
    ///
    /// fn main() -> Result<()> {
    ///     let lua = Lua::new();
    ///     lua.set_app_data("hello");
    ///     lua.create_function(hello)?.call(())?;
    ///     let s = lua.app_data_ref::<&str>().unwrap();
    ///     assert_eq!(*s, "world");
    ///     Ok(())
    /// }
    /// ```
    pub fn set_app_data<T: 'static + MaybeSend>(&self, data: T) {
        let extra = unsafe { &mut (*self.extra.get()) };
        extra
            .app_data
            .try_borrow_mut()
            .expect("cannot borrow mutably app data container")
            .insert(TypeId::of::<T>(), Box::new(data));
    }

    /// Gets a reference to an application data object stored by [`Lua::set_app_data()`] of type `T`.
    pub fn app_data_ref<T: 'static>(&self) -> Option<Ref<T>> {
        let extra = unsafe { &(*self.extra.get()) };
        let app_data = extra
            .app_data
            .try_borrow()
            .expect("cannot borrow app data container");
        let value = app_data.get(&TypeId::of::<T>())?.downcast_ref::<T>()? as *const _;
        Some(Ref::map(app_data, |_| unsafe { &*value }))
    }

    /// Gets a mutable reference to an application data object stored by [`Lua::set_app_data()`] of type `T`.
    pub fn app_data_mut<T: 'static>(&self) -> Option<RefMut<T>> {
        let extra = unsafe { &(*self.extra.get()) };
        let mut app_data = extra
            .app_data
            .try_borrow_mut()
            .expect("cannot mutably borrow app data container");
        let value = app_data.get_mut(&TypeId::of::<T>())?.downcast_mut::<T>()? as *mut _;
        Some(RefMut::map(app_data, |_| unsafe { &mut *value }))
    }

    /// Removes an application data of type `T`.
    pub fn remove_app_data<T: 'static>(&self) -> Option<T> {
        let extra = unsafe { &mut (*self.extra.get()) };
        extra
            .app_data
            .try_borrow_mut()
            .expect("cannot mutably borrow app data container")
            .remove(&TypeId::of::<T>())
            .and_then(|data| data.downcast().ok().map(|data| *data))
    }

    // Uses 2 stack spaces, does not call checkstack
    pub(crate) unsafe fn push_value(&self, value: Value) -> Result<()> {
        match value {
            Value::Nil => {
                ffi::lua_pushnil(self.state);
            }

            Value::Boolean(b) => {
                ffi::lua_pushboolean(self.state, if b { 1 } else { 0 });
            }

            Value::LightUserData(ud) => {
                ffi::lua_pushlightuserdata(self.state, ud.0);
            }

            Value::Integer(i) => {
                ffi::lua_pushinteger(self.state, i);
            }

            Value::Number(n) => {
                ffi::lua_pushnumber(self.state, n);
            }

            #[cfg(feature = "luau")]
            Value::Vector(x, y, z) => {
                ffi::lua_pushvector(self.state, x, y, z);
            }

            Value::String(s) => {
                self.push_ref(&s.0);
            }

            Value::Table(t) => {
                self.push_ref(&t.0);
            }

            Value::Function(f) => {
                self.push_ref(&f.0);
            }

            Value::Thread(t) => {
                self.push_ref(&t.0);
            }

            Value::UserData(ud) => {
                self.push_ref(&ud.0);
            }

            Value::Error(err) => {
                push_gc_userdata(self.state, WrappedFailure::Error(err))?;
            }
        }

        Ok(())
    }

    // Uses 2 stack spaces, does not call checkstack
    pub(crate) unsafe fn pop_value(&self) -> Value {
        let state = self.state;
        match ffi::lua_type(state, -1) {
            ffi::LUA_TNIL => {
                ffi::lua_pop(state, 1);
                Nil
            }

            ffi::LUA_TBOOLEAN => {
                let b = Value::Boolean(ffi::lua_toboolean(state, -1) != 0);
                ffi::lua_pop(state, 1);
                b
            }

            ffi::LUA_TLIGHTUSERDATA => {
                let ud = Value::LightUserData(LightUserData(ffi::lua_touserdata(state, -1)));
                ffi::lua_pop(state, 1);
                ud
            }

            ffi::LUA_TNUMBER => {
                if ffi::lua_isinteger(state, -1) != 0 {
                    let i = Value::Integer(ffi::lua_tointeger(state, -1));
                    ffi::lua_pop(state, 1);
                    i
                } else {
                    let n = Value::Number(ffi::lua_tonumber(state, -1));
                    ffi::lua_pop(state, 1);
                    n
                }
            }

            #[cfg(feature = "luau")]
            ffi::LUA_TVECTOR => {
                let v = ffi::lua_tovector(state, -1);
                mlua_debug_assert!(!v.is_null(), "vector is null");
                let vec = Value::Vector(*v, *v.add(1), *v.add(2));
                ffi::lua_pop(state, 1);
                vec
            }

            ffi::LUA_TSTRING => Value::String(String(self.pop_ref())),

            ffi::LUA_TTABLE => Value::Table(Table(self.pop_ref())),

            ffi::LUA_TFUNCTION => Value::Function(Function(self.pop_ref())),

            ffi::LUA_TUSERDATA => {
                // We must prevent interaction with userdata types other than UserData OR a WrappedError.
                // WrappedPanics are automatically resumed.
                match get_gc_userdata::<WrappedFailure>(state, -1).as_mut() {
                    Some(WrappedFailure::Error(err)) => {
                        let err = err.clone();
                        ffi::lua_pop(state, 1);
                        Value::Error(err)
                    }
                    Some(WrappedFailure::Panic(panic)) => {
                        if let Some(panic) = panic.take() {
                            ffi::lua_pop(state, 1);
                            resume_unwind(panic);
                        }
                        // Previously resumed panic?
                        ffi::lua_pop(state, 1);
                        Nil
                    }
                    _ => Value::UserData(AnyUserData(self.pop_ref())),
                }
            }

            ffi::LUA_TTHREAD => Value::Thread(Thread(self.pop_ref())),

            #[cfg(feature = "luajit")]
            ffi::LUA_TCDATA => {
                ffi::lua_pop(state, 1);
                // TODO: Fix this in a next major release
                panic!("cdata objects cannot be handled by mlua yet");
            }

            _ => mlua_panic!("LUA_TNONE in pop_value"),
        }
    }

    // Pushes a LuaRef value onto the stack, uses 1 stack space, does not call checkstack
    pub(crate) unsafe fn push_ref(&self, lref: &LuaRef) {
        assert!(
            Arc::ptr_eq(&lref.lua.extra, &self.extra),
            "Lua instance passed Value created from a different main Lua state"
        );
        let extra = &*self.extra.get();
        #[cfg(not(feature = "luau"))]
        {
            ffi::lua_pushvalue(extra.ref_thread, lref.index);
            ffi::lua_xmove(extra.ref_thread, self.state, 1);
        }
        #[cfg(feature = "luau")]
        ffi::lua_xpush(extra.ref_thread, self.state, lref.index);
    }

    // Pops the topmost element of the stack and stores a reference to it. This pins the object,
    // preventing garbage collection until the returned `LuaRef` is dropped.
    //
    // References are stored in the stack of a specially created auxiliary thread that exists only
    // to store reference values. This is much faster than storing these in the registry, and also
    // much more flexible and requires less bookkeeping than storing them directly in the currently
    // used stack. The implementation is somewhat biased towards the use case of a relatively small
    // number of short term references being created, and `RegistryKey` being used for long term
    // references.
    pub(crate) unsafe fn pop_ref(&self) -> LuaRef {
        let extra = &mut *self.extra.get();
        ffi::lua_xmove(self.state, extra.ref_thread, 1);
        let index = ref_stack_pop(extra);
        LuaRef { lua: self, index }
    }

    pub(crate) fn clone_ref<'lua>(&'lua self, lref: &LuaRef<'lua>) -> LuaRef<'lua> {
        unsafe {
            let extra = &mut *self.extra.get();
            ffi::lua_pushvalue(extra.ref_thread, lref.index);
            let index = ref_stack_pop(extra);
            LuaRef { lua: self, index }
        }
    }

    pub(crate) fn drop_ref(&self, lref: &LuaRef) {
        unsafe {
            let extra = &mut *self.extra.get();
            ffi::lua_pushnil(extra.ref_thread);
            ffi::lua_replace(extra.ref_thread, lref.index);
            extra.ref_free.push(lref.index);
        }
    }

    /// Executes the function provided on the ref thread
    #[inline]
    pub(crate) unsafe fn ref_thread_exec<F, R>(&self, f: F) -> R
    where
        F: FnOnce(*mut ffi::lua_State) -> R,
    {
        let ref_thread = (*self.extra.get()).ref_thread;
        f(ref_thread)
    }

    unsafe fn push_userdata_metatable<T: 'static + UserData>(&self) -> Result<()> {
        let extra = &mut *self.extra.get();

        let type_id = TypeId::of::<T>();
        if let Some(&table_id) = extra.registered_userdata.get(&type_id) {
            ffi::lua_rawgeti(self.state, ffi::LUA_REGISTRYINDEX, table_id as Integer);
            return Ok(());
        }

        let _sg = StackGuard::new_extra(self.state, 1);
        check_stack(self.state, 13)?;

        let mut fields = StaticUserDataFields::default();
        let mut methods = StaticUserDataMethods::default();
        T::add_fields(&mut fields);
        T::add_methods(&mut methods);

        // Prepare metatable, add meta methods first and then meta fields
        let metatable_nrec = methods.meta_methods.len() + fields.meta_fields.len();
        #[cfg(feature = "async")]
        let metatable_nrec = metatable_nrec + methods.async_meta_methods.len();
        push_table(self.state, 0, metatable_nrec as c_int)?;
        for (k, m) in methods.meta_methods {
            self.push_value(Value::Function(self.create_callback(m)?))?;
            rawset_field(self.state, -2, k.validate()?.name())?;
        }
        #[cfg(feature = "async")]
        for (k, m) in methods.async_meta_methods {
            self.push_value(Value::Function(self.create_async_callback(m)?))?;
            rawset_field(self.state, -2, k.validate()?.name())?;
        }
        for (k, f) in fields.meta_fields {
            self.push_value(f(self)?)?;
            rawset_field(self.state, -2, k.validate()?.name())?;
        }
        let metatable_index = ffi::lua_absindex(self.state, -1);

        let mut extra_tables_count = 0;

        let mut field_getters_index = None;
        let field_getters_nrec = fields.field_getters.len();
        if field_getters_nrec > 0 {
            push_table(self.state, 0, field_getters_nrec as c_int)?;
            for (k, m) in fields.field_getters {
                self.push_value(Value::Function(self.create_callback(m)?))?;
                rawset_field(self.state, -2, &k)?;
            }
            field_getters_index = Some(ffi::lua_absindex(self.state, -1));
            extra_tables_count += 1;
        }

        let mut field_setters_index = None;
        let field_setters_nrec = fields.field_setters.len();
        if field_setters_nrec > 0 {
            push_table(self.state, 0, field_setters_nrec as c_int)?;
            for (k, m) in fields.field_setters {
                self.push_value(Value::Function(self.create_callback(m)?))?;
                rawset_field(self.state, -2, &k)?;
            }
            field_setters_index = Some(ffi::lua_absindex(self.state, -1));
            extra_tables_count += 1;
        }

        let mut methods_index = None;
        let methods_nrec = methods.methods.len();
        #[cfg(feature = "async")]
        let methods_nrec = methods_nrec + methods.async_methods.len();
        if methods_nrec > 0 {
            push_table(self.state, 0, methods_nrec as c_int)?;
            for (k, m) in methods.methods {
                self.push_value(Value::Function(self.create_callback(m)?))?;
                rawset_field(self.state, -2, &k)?;
            }
            #[cfg(feature = "async")]
            for (k, m) in methods.async_methods {
                self.push_value(Value::Function(self.create_async_callback(m)?))?;
                rawset_field(self.state, -2, &k)?;
            }
            methods_index = Some(ffi::lua_absindex(self.state, -1));
            extra_tables_count += 1;
        }

        init_userdata_metatable::<UserDataCell<T>>(
            self.state,
            metatable_index,
            field_getters_index,
            field_setters_index,
            methods_index,
        )?;

        // Pop extra tables to get metatable on top of the stack
        ffi::lua_pop(self.state, extra_tables_count);

        let mt_ptr = ffi::lua_topointer(self.state, -1);
        ffi::lua_pushvalue(self.state, -1);
        let id = protect_lua!(self.state, 1, 0, |state| {
            ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX)
        })?;

        extra.registered_userdata.insert(type_id, id);
        extra.registered_userdata_mt.insert(mt_ptr, Some(type_id));

        Ok(())
    }

    pub(crate) unsafe fn register_userdata_metatable(
        &self,
        ptr: *const c_void,
        type_id: Option<TypeId>,
    ) {
        let extra = &mut *self.extra.get();
        extra.registered_userdata_mt.insert(ptr, type_id);
    }

    pub(crate) unsafe fn deregister_userdata_metatable(&self, ptr: *const c_void) {
        (*self.extra.get()).registered_userdata_mt.remove(&ptr);
    }

    // Pushes a LuaRef value onto the stack, checking that it's a registered
    // and not destructed UserData.
    // Uses 2 stack spaces, does not call checkstack.
    pub(crate) unsafe fn push_userdata_ref(&self, lref: &LuaRef) -> Result<Option<TypeId>> {
        self.push_ref(lref);
        if ffi::lua_getmetatable(self.state, -1) == 0 {
            return Err(Error::UserDataTypeMismatch);
        }
        let mt_ptr = ffi::lua_topointer(self.state, -1);
        ffi::lua_pop(self.state, 1);

        let extra = &*self.extra.get();
        match extra.registered_userdata_mt.get(&mt_ptr) {
            Some(&type_id) if type_id == Some(TypeId::of::<DestructedUserdataMT>()) => {
                Err(Error::UserDataDestructed)
            }
            Some(&type_id) => Ok(type_id),
            None => Err(Error::UserDataTypeMismatch),
        }
    }

    // Creates a Function out of a Callback containing a 'static Fn. This is safe ONLY because the
    // Fn is 'static, otherwise it could capture 'callback arguments improperly. Without ATCs, we
    // cannot easily deal with the "correct" callback type of:
    //
    // Box<for<'lua> Fn(&'lua Lua, MultiValue<'lua>) -> Result<MultiValue<'lua>>)>
    //
    // So we instead use a caller provided lifetime, which without the 'static requirement would be
    // unsafe.
    pub(crate) fn create_callback<'lua, 'callback>(
        &'lua self,
        func: Callback<'callback, 'static>,
    ) -> Result<Function<'lua>>
    where
        'lua: 'callback,
    {
        unsafe extern "C" fn call_callback(state: *mut ffi::lua_State) -> c_int {
            let extra = match ffi::lua_type(state, ffi::lua_upvalueindex(1)) {
                ffi::LUA_TUSERDATA => {
                    let upvalue = get_userdata::<CallbackUpvalue>(state, ffi::lua_upvalueindex(1));
                    (*upvalue).lua.extra.get()
                }
                _ => ptr::null_mut(),
            };
            callback_error_ext(state, extra, |nargs| {
                let upvalue_idx = ffi::lua_upvalueindex(1);
                if ffi::lua_type(state, upvalue_idx) == ffi::LUA_TNIL {
                    return Err(Error::CallbackDestructed);
                }
                let upvalue = get_userdata::<CallbackUpvalue>(state, upvalue_idx);

                if nargs < ffi::LUA_MINSTACK {
                    check_stack(state, ffi::LUA_MINSTACK - nargs)?;
                }

                let mut lua = (*upvalue).lua.clone();
                lua.state = state;

                let mut args = MultiValue::new_or_cached(&lua);
                args.reserve(nargs as usize);
                for _ in 0..nargs {
                    args.push_front(lua.pop_value());
                }

                let mut results = ((*upvalue).func)(&lua, args)?;
                let nresults = results.len() as c_int;

                check_stack(state, nresults)?;
                for r in results.drain_all() {
                    lua.push_value(r)?;
                }
                lua.cache_multivalue(results);

                Ok(nresults)
            })
        }

        unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 4)?;

            let lua = self.clone();
            let func = mem::transmute(func);
            push_gc_userdata(self.state, CallbackUpvalue { lua, func })?;
            protect_lua!(self.state, 1, 1, fn(state) {
                ffi::lua_pushcclosure(state, call_callback, 1);
            })?;

            Ok(Function(self.pop_ref()))
        }
    }

    #[cfg(feature = "async")]
    pub(crate) fn create_async_callback<'lua, 'callback>(
        &'lua self,
        func: AsyncCallback<'callback, 'static>,
    ) -> Result<Function<'lua>>
    where
        'lua: 'callback,
    {
        #[cfg(any(
            feature = "lua54",
            feature = "lua53",
            feature = "lua52",
            feature = "luau"
        ))]
        unsafe {
            let libs = (*self.extra.get()).libs;
            if !libs.contains(StdLib::COROUTINE) {
                self.load_from_std_lib(StdLib::COROUTINE)?;
            }
        }

        struct StateGuard(*mut Lua, *mut ffi::lua_State);

        impl StateGuard {
            unsafe fn new(lua: *mut Lua, state: *mut ffi::lua_State) -> Self {
                let orig_state = (*lua).state;
                (*lua).state = state;
                Self(lua, orig_state)
            }
        }

        impl Drop for StateGuard {
            fn drop(&mut self) {
                unsafe { (*self.0).state = self.1 }
            }
        }

        unsafe extern "C" fn call_callback(state: *mut ffi::lua_State) -> c_int {
            let extra = match ffi::lua_type(state, ffi::lua_upvalueindex(1)) {
                ffi::LUA_TUSERDATA => {
                    let upvalue =
                        get_userdata::<AsyncCallbackUpvalue>(state, ffi::lua_upvalueindex(1));
                    (*upvalue).lua.extra.get()
                }
                _ => ptr::null_mut(),
            };
            callback_error_ext(state, extra, |nargs| {
                let upvalue_idx = ffi::lua_upvalueindex(1);
                if ffi::lua_type(state, upvalue_idx) == ffi::LUA_TNIL {
                    return Err(Error::CallbackDestructed);
                }
                let upvalue = get_userdata::<AsyncCallbackUpvalue>(state, upvalue_idx);

                if nargs < ffi::LUA_MINSTACK {
                    check_stack(state, ffi::LUA_MINSTACK - nargs)?;
                }

                let lua = &mut (*upvalue).lua;
                let _guard = StateGuard::new(lua, state);

                let mut args = MultiValue::new_or_cached(lua);
                args.reserve(nargs as usize);
                for _ in 0..nargs {
                    args.push_front(lua.pop_value());
                }

                let fut = ((*upvalue).func)(lua, args);
                let lua = lua.clone();
                push_gc_userdata(state, AsyncPollUpvalue { lua, fut })?;
                protect_lua!(state, 1, 1, fn(state) {
                    ffi::lua_pushcclosure(state, poll_future, 1);
                })?;

                Ok(1)
            })
        }

        unsafe extern "C" fn poll_future(state: *mut ffi::lua_State) -> c_int {
            let extra = match ffi::lua_type(state, ffi::lua_upvalueindex(1)) {
                ffi::LUA_TUSERDATA => {
                    let upvalue = get_userdata::<AsyncPollUpvalue>(state, ffi::lua_upvalueindex(1));
                    (*upvalue).lua.extra.get()
                }
                _ => ptr::null_mut(),
            };
            callback_error_ext(state, extra, |nargs| {
                let upvalue_idx = ffi::lua_upvalueindex(1);
                if ffi::lua_type(state, upvalue_idx) == ffi::LUA_TNIL {
                    return Err(Error::CallbackDestructed);
                }
                let upvalue = get_userdata::<AsyncPollUpvalue>(state, upvalue_idx);

                if nargs < ffi::LUA_MINSTACK {
                    check_stack(state, ffi::LUA_MINSTACK - nargs)?;
                }

                let lua = &mut (*upvalue).lua;
                lua.state = state;

                // Try to get an outer poll waker
                let waker = lua.waker().unwrap_or_else(noop_waker);
                let mut ctx = Context::from_waker(&waker);

                let fut = &mut (*upvalue).fut;
                match fut.as_mut().poll(&mut ctx) {
                    Poll::Pending => {
                        check_stack(state, 1)?;
                        ffi::lua_pushboolean(state, 0);
                        Ok(1)
                    }
                    Poll::Ready(results) => {
                        let results = results?;
                        let nresults = results.len() as Integer;
                        let results = lua.create_sequence_from(results)?;
                        check_stack(state, 3)?;
                        ffi::lua_pushboolean(state, 1);
                        lua.push_value(Value::Table(results))?;
                        lua.push_value(Value::Integer(nresults))?;
                        Ok(3)
                    }
                }
            })
        }

        let get_poll = unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 4)?;

            let lua = self.clone();
            let func = mem::transmute(func);
            push_gc_userdata(self.state, AsyncCallbackUpvalue { lua, func })?;
            protect_lua!(self.state, 1, 1, fn(state) {
                ffi::lua_pushcclosure(state, call_callback, 1);
            })?;

            Function(self.pop_ref())
        };

        unsafe extern "C" fn unpack(state: *mut ffi::lua_State) -> c_int {
            let len = ffi::lua_tointeger(state, 2);
            ffi::luaL_checkstack(state, len as c_int, ptr::null());
            for i in 1..=len {
                ffi::lua_rawgeti(state, 1, i);
            }
            len as c_int
        }

        let coroutine = self.globals().get::<_, Table>("coroutine")?;

        let env = self.create_table_with_capacity(0, 4)?;
        env.set("get_poll", get_poll)?;
        env.set("yield", coroutine.get::<_, Function>("yield")?)?;
        unsafe {
            env.set("unpack", self.create_c_function(unpack)?)?;
        }
        env.set("pending", {
            LightUserData(&ASYNC_POLL_PENDING as *const u8 as *mut c_void)
        })?;

        // We set `poll` variable in the env table to be able to destroy upvalues
        self.load(
            r#"
            poll = get_poll(...)
            local poll, pending, yield, unpack = poll, pending, yield, unpack
            while true do
                local ready, res, nres = poll()
                if ready then
                    return unpack(res, nres)
                end
                yield(pending)
            end
            "#,
        )
        .set_name("_mlua_async_poll")?
        .set_environment(env)?
        .into_function()
    }

    #[cfg(feature = "async")]
    #[inline]
    pub(crate) unsafe fn waker(&self) -> Option<Waker> {
        let extra = &*self.extra.get();
        (*get_userdata::<Option<Waker>>(extra.ref_thread, extra.ref_waker_idx)).clone()
    }

    #[cfg(feature = "async")]
    #[inline]
    pub(crate) unsafe fn set_waker(&self, waker: Option<Waker>) -> Option<Waker> {
        let extra = &*self.extra.get();
        let waker_slot = &mut *get_userdata::<Option<Waker>>(extra.ref_thread, extra.ref_waker_idx);
        match waker {
            Some(waker) => waker_slot.replace(waker),
            None => waker_slot.take(),
        }
    }

    pub(crate) unsafe fn make_userdata<T>(&self, data: UserDataCell<T>) -> Result<AnyUserData>
    where
        T: 'static + UserData,
    {
        let _sg = StackGuard::new(self.state);
        check_stack(self.state, 3)?;

        // We push metatable first to ensure having correct metatable with `__gc` method
        ffi::lua_pushnil(self.state);
        self.push_userdata_metatable::<T>()?;
        #[cfg(not(feature = "lua54"))]
        push_userdata(self.state, data)?;
        #[cfg(feature = "lua54")]
        push_userdata_uv(self.state, data, USER_VALUE_MAXSLOT as c_int)?;
        ffi::lua_replace(self.state, -3);
        ffi::lua_setmetatable(self.state, -2);

        // Set empty environment for Lua 5.1
        #[cfg(any(feature = "lua51", feature = "luajit"))]
        protect_lua!(self.state, 1, 1, fn(state) {
            ffi::lua_newtable(state);
            ffi::lua_setuservalue(state, -2);
        })?;

        Ok(AnyUserData(self.pop_ref()))
    }

    #[inline]
    pub(crate) fn clone(&self) -> Self {
        Lua {
            state: self.state,
            main_state: self.main_state,
            extra: Arc::clone(&self.extra),
            ephemeral: true,
            safe: self.safe,
            _no_ref_unwind_safe: PhantomData,
        }
    }

    #[cfg(not(feature = "luau"))]
    fn disable_c_modules(&self) -> Result<()> {
        let package: Table = self.globals().get("package")?;

        package.set(
            "loadlib",
            self.create_function(|_, ()| -> Result<()> {
                Err(Error::SafetyError(
                    "package.loadlib is disabled in safe mode".to_string(),
                ))
            })?,
        )?;

        #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
        let searchers: Table = package.get("searchers")?;
        #[cfg(any(feature = "lua51", feature = "luajit"))]
        let searchers: Table = package.get("loaders")?;

        let loader = self.create_function(|_, ()| Ok("\n\tcan't load C modules in safe mode"))?;

        // The third and fourth searchers looks for a loader as a C library
        searchers.raw_set(3, loader.clone())?;
        searchers.raw_remove(4)?;

        Ok(())
    }

    pub(crate) unsafe fn make_from_ptr(state: *mut ffi::lua_State) -> Option<Self> {
        let _sg = StackGuard::new(state);
        assert_stack(state, 1);

        let extra = extra_data(state)?;
        let safe = (*extra.get()).safe;
        Some(Lua {
            state,
            main_state: get_main_state(state),
            extra,
            ephemeral: true,
            safe,
            _no_ref_unwind_safe: PhantomData,
        })
    }

    #[inline]
    pub(crate) fn new_or_cached_multivalue(&self) -> MultiValue {
        unsafe {
            let extra = &mut *self.extra.get();
            extra.multivalue_cache.pop().unwrap_or_default()
        }
    }

    #[inline]
    pub(crate) fn cache_multivalue(&self, mut multivalue: MultiValue) {
        unsafe {
            let extra = &mut *self.extra.get();
            if extra.multivalue_cache.len() < MULTIVALUE_CACHE_SIZE {
                multivalue.clear();
                extra.multivalue_cache.push(mem::transmute(multivalue));
            }
        }
    }
}

#[cfg(feature = "luau")]
unsafe fn extra_data(state: *mut ffi::lua_State) -> Option<Arc<UnsafeCell<ExtraData>>> {
    let extra_ptr = (*ffi::lua_callbacks(state)).userdata as *mut Arc<UnsafeCell<ExtraData>>;
    if extra_ptr.is_null() {
        return None;
    }
    Some(Arc::clone(&*extra_ptr))
}

#[cfg(not(feature = "luau"))]
unsafe fn extra_data(state: *mut ffi::lua_State) -> Option<Arc<UnsafeCell<ExtraData>>> {
    let extra_key = &EXTRA_REGISTRY_KEY as *const u8 as *const c_void;
    if ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, extra_key) != ffi::LUA_TUSERDATA {
        return None;
    }
    let extra_ptr = ffi::lua_touserdata(state, -1) as *mut Arc<UnsafeCell<ExtraData>>;
    let extra = Arc::clone(&*extra_ptr);
    ffi::lua_pop(state, 1);
    Some(extra)
}

// Creates required entries in the metatable cache (see `util::METATABLE_CACHE`)
pub(crate) fn init_metatable_cache(cache: &mut FxHashMap<TypeId, u8>) {
    cache.insert(TypeId::of::<Arc<UnsafeCell<ExtraData>>>(), 0);
    cache.insert(TypeId::of::<Callback>(), 0);
    cache.insert(TypeId::of::<CallbackUpvalue>(), 0);

    #[cfg(feature = "async")]
    {
        cache.insert(TypeId::of::<AsyncCallback>(), 0);
        cache.insert(TypeId::of::<AsyncCallbackUpvalue>(), 0);
        cache.insert(TypeId::of::<AsyncPollUpvalue>(), 0);
        cache.insert(TypeId::of::<Option<Waker>>(), 0);
    }
}

// An optimized version of `callback_error` that does not allocate `WrappedFailure` userdata
// and instead reuses unsed and cached values from previous calls (or allocates new).
// It requires `get_extra` function to return `ExtraData` value.
unsafe fn callback_error_ext<F, R>(state: *mut ffi::lua_State, extra: *mut ExtraData, f: F) -> R
where
    F: FnOnce(c_int) -> Result<R>,
{
    if extra.is_null() {
        return callback_error(state, f);
    }
    let extra = &mut *extra;

    let nargs = ffi::lua_gettop(state);

    // We need 2 extra stack spaces to store preallocated memory and error/panic metatable.
    let extra_stack = if nargs < 2 { 2 - nargs } else { 1 };
    ffi::luaL_checkstack(
        state,
        extra_stack,
        cstr!("not enough stack space for callback error handling"),
    );

    enum PreallocatedFailure {
        New(*mut WrappedFailure),
        Cached(i32),
    }

    // We cannot shadow Rust errors with Lua ones, so we need to obtain pre-allocated memory
    // to store a wrapped failure (error or panic) *before* we proceed.
    let prealloc_failure = match extra.wrapped_failures_cache.pop() {
        Some(index) => PreallocatedFailure::Cached(index),
        None => {
            let ud = WrappedFailure::new_userdata(state);
            ffi::lua_rotate(state, 1, 1);
            PreallocatedFailure::New(ud)
        }
    };

    let mut get_wrapped_failure = || match prealloc_failure {
        PreallocatedFailure::New(ud) => {
            ffi::lua_settop(state, 1);
            ud
        }
        PreallocatedFailure::Cached(index) => {
            ffi::lua_settop(state, 0);
            ffi::lua_pushvalue(extra.ref_thread, index);
            ffi::lua_xmove(extra.ref_thread, state, 1);
            ffi::lua_pushnil(extra.ref_thread);
            ffi::lua_replace(extra.ref_thread, index);
            extra.ref_free.push(index);
            ffi::lua_touserdata(state, -1) as *mut WrappedFailure
        }
    };

    match catch_unwind(AssertUnwindSafe(|| f(nargs))) {
        Ok(Ok(r)) => {
            // Return unused WrappedFailure to the cache
            match prealloc_failure {
                PreallocatedFailure::New(_)
                    if extra.wrapped_failures_cache.len() < WRAPPED_FAILURES_CACHE_SIZE =>
                {
                    ffi::lua_rotate(state, 1, -1);
                    ffi::lua_xmove(state, extra.ref_thread, 1);
                    let index = ref_stack_pop(extra);
                    extra.wrapped_failures_cache.push(index);
                }
                PreallocatedFailure::New(_) => {
                    ffi::lua_remove(state, 1);
                }
                PreallocatedFailure::Cached(index)
                    if extra.wrapped_failures_cache.len() < WRAPPED_FAILURES_CACHE_SIZE =>
                {
                    extra.wrapped_failures_cache.push(index);
                }
                PreallocatedFailure::Cached(index) => {
                    ffi::lua_pushnil(extra.ref_thread);
                    ffi::lua_replace(extra.ref_thread, index);
                    extra.ref_free.push(index);
                }
            }
            r
        }
        Ok(Err(err)) => {
            let wrapped_error = get_wrapped_failure();

            // Build `CallbackError` with traceback
            let traceback = if ffi::lua_checkstack(state, ffi::LUA_TRACEBACK_STACK) != 0 {
                ffi::luaL_traceback(state, state, ptr::null(), 0);
                let traceback = util::to_string(state, -1);
                ffi::lua_pop(state, 1);
                traceback
            } else {
                "<not enough stack space for traceback>".to_string()
            };
            let cause = Arc::new(err);
            ptr::write(
                wrapped_error,
                WrappedFailure::Error(Error::CallbackError { traceback, cause }),
            );
            get_gc_metatable::<WrappedFailure>(state);
            ffi::lua_setmetatable(state, -2);

            ffi::lua_error(state)
        }
        Err(p) => {
            let wrapped_panic = get_wrapped_failure();
            ptr::write(wrapped_panic, WrappedFailure::Panic(Some(p)));
            get_gc_metatable::<WrappedFailure>(state);
            ffi::lua_setmetatable(state, -2);
            ffi::lua_error(state)
        }
    }
}

// Uses 3 stack spaces
unsafe fn load_from_std_lib(state: *mut ffi::lua_State, libs: StdLib) -> Result<()> {
    #[inline(always)]
    pub unsafe fn requiref<S: AsRef<[u8]> + ?Sized>(
        state: *mut ffi::lua_State,
        modname: &S,
        openf: ffi::lua_CFunction,
        glb: c_int,
    ) -> Result<()> {
        let modname = mlua_expect!(CString::new(modname.as_ref()), "modname contains nil bytes");
        protect_lua!(state, 0, 1, |state| {
            ffi::luaL_requiref(state, modname.as_ptr() as *const c_char, openf, glb)
        })
    }

    #[cfg(feature = "luajit")]
    struct GcGuard(*mut ffi::lua_State);

    #[cfg(feature = "luajit")]
    impl GcGuard {
        fn new(state: *mut ffi::lua_State) -> Self {
            // Stop collector during library initialization
            unsafe { ffi::lua_gc(state, ffi::LUA_GCSTOP, 0) };
            GcGuard(state)
        }
    }

    #[cfg(feature = "luajit")]
    impl Drop for GcGuard {
        fn drop(&mut self) {
            unsafe { ffi::lua_gc(self.0, ffi::LUA_GCRESTART, -1) };
        }
    }

    // Stop collector during library initialization
    #[cfg(feature = "luajit")]
    let _gc_guard = GcGuard::new(state);

    #[cfg(any(
        feature = "lua54",
        feature = "lua53",
        feature = "lua52",
        feature = "luau"
    ))]
    {
        if libs.contains(StdLib::COROUTINE) {
            requiref(state, ffi::LUA_COLIBNAME, ffi::luaopen_coroutine, 1)?;
            ffi::lua_pop(state, 1);
        }
    }

    if libs.contains(StdLib::TABLE) {
        requiref(state, ffi::LUA_TABLIBNAME, ffi::luaopen_table, 1)?;
        ffi::lua_pop(state, 1);
    }

    #[cfg(not(feature = "luau"))]
    if libs.contains(StdLib::IO) {
        requiref(state, ffi::LUA_IOLIBNAME, ffi::luaopen_io, 1)?;
        ffi::lua_pop(state, 1);
    }

    if libs.contains(StdLib::OS) {
        requiref(state, ffi::LUA_OSLIBNAME, ffi::luaopen_os, 1)?;
        ffi::lua_pop(state, 1);
    }

    if libs.contains(StdLib::STRING) {
        requiref(state, ffi::LUA_STRLIBNAME, ffi::luaopen_string, 1)?;
        ffi::lua_pop(state, 1);
    }

    #[cfg(any(feature = "lua54", feature = "lua53", feature = "luau"))]
    {
        if libs.contains(StdLib::UTF8) {
            requiref(state, ffi::LUA_UTF8LIBNAME, ffi::luaopen_utf8, 1)?;
            ffi::lua_pop(state, 1);
        }
    }

    #[cfg(any(feature = "lua52", feature = "luau"))]
    {
        if libs.contains(StdLib::BIT) {
            requiref(state, ffi::LUA_BITLIBNAME, ffi::luaopen_bit32, 1)?;
            ffi::lua_pop(state, 1);
        }
    }

    #[cfg(feature = "luajit")]
    {
        if libs.contains(StdLib::BIT) {
            requiref(state, ffi::LUA_BITLIBNAME, ffi::luaopen_bit, 1)?;
            ffi::lua_pop(state, 1);
        }
    }

    if libs.contains(StdLib::MATH) {
        requiref(state, ffi::LUA_MATHLIBNAME, ffi::luaopen_math, 1)?;
        ffi::lua_pop(state, 1);
    }

    if libs.contains(StdLib::DEBUG) {
        requiref(state, ffi::LUA_DBLIBNAME, ffi::luaopen_debug, 1)?;
        ffi::lua_pop(state, 1);
    }

    #[cfg(not(feature = "luau"))]
    if libs.contains(StdLib::PACKAGE) {
        requiref(state, ffi::LUA_LOADLIBNAME, ffi::luaopen_package, 1)?;
        ffi::lua_pop(state, 1);
    }

    #[cfg(feature = "luajit")]
    {
        if libs.contains(StdLib::JIT) {
            requiref(state, ffi::LUA_JITLIBNAME, ffi::luaopen_jit, 1)?;
            ffi::lua_pop(state, 1);
        }

        if libs.contains(StdLib::FFI) {
            requiref(state, ffi::LUA_FFILIBNAME, ffi::luaopen_ffi, 1)?;
            ffi::lua_pop(state, 1);
        }
    }

    Ok(())
}

unsafe fn ref_stack_pop(extra: &mut ExtraData) -> c_int {
    if let Some(free) = extra.ref_free.pop() {
        ffi::lua_replace(extra.ref_thread, free);
        return free;
    }

    // Try to grow max stack size
    if extra.ref_stack_top >= extra.ref_stack_size {
        let mut inc = extra.ref_stack_size; // Try to double stack size
        while inc > 0 && ffi::lua_checkstack(extra.ref_thread, inc) == 0 {
            inc /= 2;
        }
        if inc == 0 {
            // Pop item on top of the stack to avoid stack leaking and successfully run destructors
            // during unwinding.
            ffi::lua_pop(extra.ref_thread, 1);
            let top = extra.ref_stack_top;
            // It is a user error to create enough references to exhaust the Lua max stack size for
            // the ref thread.
            panic!(
                "cannot create a Lua reference, out of auxiliary stack space (used {} slots)",
                top
            );
        }
        extra.ref_stack_size += inc;
    }
    extra.ref_stack_top += 1;
    extra.ref_stack_top
}
