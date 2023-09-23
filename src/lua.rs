use std::prelude::v1::*;

use std::any::TypeId;
use std::cell::{RefCell, UnsafeCell};
use std::ffi::{c_char, c_int, c_void};
use std::ffi::{CStr, CString};
use std::fmt;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::ops::Deref;
#[cfg(feature = "panic-safety")]
use std::panic::{catch_unwind, resume_unwind};
use std::panic::{AssertUnwindSafe, Location};
use std::ptr::NonNull;
use std::result::Result as StdResult;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::{mem, ptr, str};

#[cfg(not(any(feature = "std", target_has_atomic = "ptr")))]
use std::rc::Rc as Arc;
#[cfg(any(feature = "std", target_has_atomic = "ptr"))]
use std::sync::Arc;

#[cfg(feature = "std")]
use rustc_hash::FxHashMap;
#[cfg(not(feature = "std"))]
type FxHashMap<K, V> =
    std::collections::HashMap<K, V, core::hash::BuildHasherDefault<rustc_hash::FxHasher>>;

#[cfg(any(
    feature = "lua52",
    feature = "lua51",
    feature = "luajit",
    feature = "luau"
))]
use num_traits::Float;

use crate::chunk::{AsChunk, Chunk, ChunkMode};
use crate::error::{Error, Result};
use crate::function::Function;
use crate::hook::Debug;
use crate::memory::{MemoryState, ALLOCATOR};
use crate::scope::Scope;
use crate::stdlib::StdLib;
use crate::string::String;
use crate::table::Table;
use crate::thread::Thread;
use crate::types::{
    AppData, AppDataRef, AppDataRefMut, Callback, CallbackUpvalue, DestructedUserdata, Integer,
    LightUserData, LuaRef, MaybeSend, Number, RegistryKey, UnrefList,
};
use crate::userdata::{AnyUserData, MetaMethod, UserData, UserDataCell};
use crate::userdata_impl::{UserDataProxy, UserDataRegistry};
use crate::util::{
    self, assert_stack, check_stack, get_destructed_userdata_metatable, get_gc_metatable,
    get_gc_userdata, get_main_state, get_userdata, init_error_registry, init_gc_metatable,
    init_userdata_metatable, pop_error, push_gc_userdata, push_string, push_table, rawset_field,
    short_type_name, StackGuard, WrappedFailure,
};
#[cfg(feature = "panic-safety")]
use crate::util::{safe_pcall, safe_xpcall};
use crate::value::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, MultiValue, Nil, Value};

#[cfg(not(feature = "lua54"))]
use crate::util::push_userdata;
#[cfg(feature = "lua54")]
use crate::{types::WarnCallback, userdata::USER_VALUE_MAXSLOT, util::push_userdata_uv};

#[cfg(not(feature = "luau"))]
use crate::{hook::HookTriggers, types::HookCallback};

#[cfg(feature = "luau")]
use crate::types::InterruptCallback;
#[cfg(any(feature = "luau", doc))]
use crate::{
    chunk::Compiler,
    types::{Vector, VmState},
};

#[cfg(feature = "async")]
use {
    crate::types::{AsyncCallback, AsyncCallbackUpvalue, AsyncPollUpvalue},
    futures_util::future::{self, Future},
    futures_util::task::{noop_waker_ref, Context, Poll, Waker},
};

#[cfg(feature = "serialize")]
use serde::Serialize;

/// Top level Lua struct which represents an instance of Lua VM.
#[repr(transparent)]
pub struct Lua(Arc<LuaInner>);

/// An inner Lua struct which holds a raw Lua state.
pub struct LuaInner {
    // The state is dynamic and depends on context
    state: AtomicPtr<ffi::lua_State>,
    main_state: *mut ffi::lua_State,
    extra: Arc<UnsafeCell<ExtraData>>,
}

// Data associated with the Lua.
pub(crate) struct ExtraData {
    // Same layout as `Lua`
    inner: MaybeUninit<Arc<LuaInner>>,

    registered_userdata: FxHashMap<TypeId, c_int>,
    registered_userdata_mt: FxHashMap<*const c_void, Option<TypeId>>,
    last_checked_userdata_mt: (*const c_void, Option<TypeId>),

    // When Lua instance dropped, setting `None` would prevent collecting `RegistryKey`s
    registry_unref_list: UnrefList,

    // Container to store arbitrary data (extensions)
    app_data: AppData,

    safe: bool,
    libs: StdLib,
    mem_state: Option<NonNull<MemoryState>>,
    #[cfg(feature = "module")]
    skip_memory_check: bool,

    // Auxiliary thread to store references
    ref_thread: *mut ffi::lua_State,
    ref_stack_size: c_int,
    ref_stack_top: c_int,
    ref_free: Vec<c_int>,

    // Pool of `WrappedFailure` enums in the ref thread (as userdata)
    wrapped_failure_pool: Vec<c_int>,
    // Pool of `MultiValue` containers
    multivalue_pool: Vec<Vec<Value<'static>>>,
    // Pool of `Thread`s (coroutines) for async execution
    #[cfg(feature = "async")]
    thread_pool: Vec<c_int>,

    // Address of `WrappedFailure` metatable
    wrapped_failure_mt_ptr: *const c_void,

    // Waker for polling futures
    #[cfg(feature = "async")]
    waker: NonNull<Waker>,

    #[cfg(not(feature = "luau"))]
    hook_callback: Option<HookCallback>,
    #[cfg(not(feature = "luau"))]
    hook_thread: *mut ffi::lua_State,
    #[cfg(feature = "lua54")]
    warn_callback: Option<WarnCallback>,
    #[cfg(feature = "luau")]
    interrupt_callback: Option<InterruptCallback>,

    #[cfg(feature = "luau")]
    sandboxed: bool,
    #[cfg(feature = "luau")]
    compiler: Option<Compiler>,
    #[cfg(feature = "luau-jit")]
    enable_jit: bool,
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
    #[cfg(feature = "lua54")]
    #[cfg_attr(docsrs, doc(cfg(feature = "lua54")))]
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
    #[cfg(feature = "panic-safety")]
    pub catch_rust_panics: bool,

    /// Max size of thread (coroutine) object pool used to execute asynchronous functions.
    ///
    /// It works on Lua 5.4 and Luau, where [`lua_resetthread`] function
    /// is available and allows to reuse old coroutines after resetting their state.
    ///
    /// Default: **0** (disabled)
    ///
    /// [`lua_resetthread`]: https://www.lua.org/manual/5.4/manual.html#lua_resetthread
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub thread_pool_size: usize,
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
            #[cfg(feature = "panic-safety")]
            catch_rust_panics: true,
            #[cfg(feature = "async")]
            thread_pool_size: 0,
        }
    }

    /// Sets [`catch_rust_panics`] option.
    ///
    /// [`catch_rust_panics`]: #structfield.catch_rust_panics
    #[must_use]
    #[cfg(feature = "panic-safety")]
    pub const fn catch_rust_panics(mut self, enabled: bool) -> Self {
        self.catch_rust_panics = enabled;
        self
    }

    /// Sets [`thread_pool_size`] option.
    ///
    /// [`thread_pool_size`]: #structfield.thread_pool_size
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    #[must_use]
    pub const fn thread_pool_size(mut self, size: usize) -> Self {
        self.thread_pool_size = size;
        self
    }
}

#[cfg(feature = "async")]
pub(crate) static ASYNC_POLL_PENDING: u8 = 0;
pub(crate) static EXTRA_REGISTRY_KEY: u8 = 0;

const WRAPPED_FAILURE_POOL_SIZE: usize = 64;
const MULTIVALUE_POOL_SIZE: usize = 64;

/// Requires `feature = "send"`
#[cfg(feature = "send")]
#[cfg_attr(docsrs, doc(cfg(feature = "send")))]
unsafe impl Send for Lua {}

#[cfg(not(feature = "module"))]
impl Drop for Lua {
    fn drop(&mut self) {
        let _ = self.gc_collect();
    }
}

#[cfg(not(feature = "module"))]
impl Drop for LuaInner {
    fn drop(&mut self) {
        unsafe {
            #[cfg(feature = "luau")]
            {
                (*ffi::lua_callbacks(self.state())).userdata = ptr::null_mut();
            }
            ffi::lua_close(self.main_state);
        }
    }
}

impl Drop for ExtraData {
    fn drop(&mut self) {
        #[cfg(feature = "module")]
        unsafe {
            self.inner.assume_init_drop();
        }

        #[cfg(feature = "std")]
        {
            *mlua_expect!(self.registry_unref_list.lock(), "unref list poisoned") = None;
        }
        #[cfg(not(feature = "std"))]
        {
            *mlua_expect!(
                self.registry_unref_list.try_borrow_mut(),
                "unref list poisoned"
            ) = None;
        }
        if let Some(mem_state) = self.mem_state {
            drop(unsafe { Box::from_raw(mem_state.as_ptr()) });
        }
    }
}

impl fmt::Debug for Lua {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Lua({:p})", self.state())
    }
}

impl Deref for Lua {
    type Target = LuaInner;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Default for Lua {
    #[inline]
    fn default() -> Self {
        Lua::new()
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
    pub fn new() -> Lua {
        mlua_expect!(
            Self::new_with(StdLib::ALL_SAFE, LuaOptions::default()),
            "Cannot create new safe Lua state"
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
        #[cfg(not(feature = "luau"))]
        if libs.contains(StdLib::DEBUG) {
            return Err(Error::SafetyError(
                "The unsafe `debug` module can't be loaded using safe `new_with`".to_string(),
            ));
        }
        #[cfg(feature = "luajit")]
        if libs.contains(StdLib::FFI) {
            return Err(Error::SafetyError(
                "The unsafe `ffi` module can't be loaded using safe `new_with`".to_string(),
            ));
        }

        let lua = unsafe { Self::inner_new(libs, options) };

        #[cfg(not(feature = "luau"))]
        if libs.contains(StdLib::PACKAGE) {
            mlua_expect!(lua.disable_c_modules(), "Error during disabling C modules");
        }
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
        {
            // Workaround to avoid stripping a few unused Lua symbols that could be imported
            // by C modules in unsafe mode
            let mut _symbols: Vec<*const extern "C-unwind" fn()> = vec![
                ffi::lua_atpanic as _,
                ffi::lua_isuserdata as _,
                ffi::lua_tocfunction as _,
                ffi::luaL_loadstring as _,
                ffi::luaL_openlibs as _,
            ];
            #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
            {
                _symbols.push(ffi::lua_getglobal as _);
                _symbols.push(ffi::lua_setglobal as _);
                _symbols.push(ffi::luaL_setfuncs as _);
            }
        }

        Self::inner_new(libs, options)
    }

    /// Creates a new Lua state with required `libs` and `options`
    unsafe fn inner_new(libs: StdLib, options: LuaOptions) -> Lua {
        #[cfg(not(feature = "panic-safety"))]
        let _ = options;

        let mut mem_state: *mut MemoryState = Box::into_raw(Box::default());
        let mut state = ffi::lua_newstate(ALLOCATOR, mem_state as *mut c_void);
        // If state is null then switch to Lua internal allocator
        if state.is_null() {
            drop(Box::from_raw(mem_state));
            mem_state = ptr::null_mut();
            state = ffi::luaL_newstate();
        }
        assert!(!state.is_null(), "Failed to instantiate Lua VM");

        ffi::luaL_requiref(state, cstr!("_G"), ffi::luaopen_base, 1);
        ffi::lua_pop(state, 1);

        // Init Luau code generator (jit)
        #[cfg(feature = "luau-jit")]
        if ffi::luau_codegen_supported() != 0 {
            ffi::luau_codegen_create(state);
        }

        let lua = Lua::init_from_ptr(state);
        let extra = lua.extra.get();
        (*extra).mem_state = NonNull::new(mem_state);

        mlua_expect!(
            load_from_std_lib(state, libs),
            "Error during loading standard libraries"
        );
        (*extra).libs |= libs;

        #[cfg(feature = "panic-safety")]
        if !options.catch_rust_panics {
            mlua_expect!(
                (|| -> Result<()> {
                    let _sg = StackGuard::new(state);

                    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
                    ffi::lua_rawgeti(state, ffi::LUA_REGISTRYINDEX, ffi::LUA_RIDX_GLOBALS);
                    #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
                    ffi::lua_pushvalue(state, ffi::LUA_GLOBALSINDEX);

                    ffi::lua_pushcfunction(state, safe_pcall);
                    rawset_field(state, -2, "pcall")?;

                    ffi::lua_pushcfunction(state, safe_xpcall);
                    rawset_field(state, -2, "xpcall")?;

                    Ok(())
                })(),
                "Error during applying option `catch_rust_panics`"
            )
        }

        #[cfg(feature = "async")]
        if options.thread_pool_size > 0 {
            (*extra).thread_pool.reserve_exact(options.thread_pool_size);
        }

        #[cfg(feature = "luau")]
        mlua_expect!(lua.prepare_luau_state(), "Error preparing Luau state");

        lua
    }

    /// Constructs a new Lua instance from an existing raw state.
    ///
    /// Once called, a returned Lua state is cached in the registry and can be retrieved
    /// by calling this function again.
    #[allow(clippy::missing_safety_doc, clippy::arc_with_non_send_sync)]
    pub unsafe fn init_from_ptr(state: *mut ffi::lua_State) -> Lua {
        assert!(!state.is_null(), "Lua state is NULL");
        if let Some(lua) = Lua::try_from_ptr(state) {
            return lua;
        }

        let main_state = get_main_state(state).unwrap_or(state);
        let main_state_top = ffi::lua_gettop(main_state);

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
            protect_lua!(main_state, 0, 0, |state| {
                let thread = ffi::lua_newthread(state);
                ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX);
                thread
            }),
            "Error while creating ref thread",
        );

        let wrapped_failure_mt_ptr = {
            get_gc_metatable::<WrappedFailure>(main_state);
            let ptr = ffi::lua_topointer(main_state, -1);
            ffi::lua_pop(main_state, 1);
            ptr
        };

        // Create ExtraData
        let extra = Arc::new(UnsafeCell::new(ExtraData {
            inner: MaybeUninit::uninit(),
            registered_userdata: FxHashMap::default(),
            registered_userdata_mt: FxHashMap::default(),
            last_checked_userdata_mt: (ptr::null(), None),
            registry_unref_list: UnrefList::default(),
            app_data: AppData::default(),
            safe: false,
            libs: StdLib::NONE,
            mem_state: None,
            #[cfg(feature = "module")]
            skip_memory_check: false,
            ref_thread,
            // We need 1 extra stack space to move values in and out of the ref stack.
            ref_stack_size: ffi::LUA_MINSTACK - 1,
            ref_stack_top: ffi::lua_gettop(ref_thread),
            ref_free: Vec::new(),
            wrapped_failure_pool: Vec::with_capacity(WRAPPED_FAILURE_POOL_SIZE),
            multivalue_pool: Vec::with_capacity(MULTIVALUE_POOL_SIZE),
            #[cfg(feature = "async")]
            thread_pool: Vec::new(),
            wrapped_failure_mt_ptr,
            #[cfg(feature = "async")]
            waker: NonNull::from(noop_waker_ref()),
            #[cfg(not(feature = "luau"))]
            hook_callback: None,
            #[cfg(not(feature = "luau"))]
            hook_thread: ptr::null_mut(),
            #[cfg(feature = "lua54")]
            warn_callback: None,
            #[cfg(feature = "luau")]
            interrupt_callback: None,
            #[cfg(feature = "luau")]
            sandboxed: false,
            #[cfg(feature = "luau")]
            compiler: None,
            #[cfg(feature = "luau-jit")]
            enable_jit: true,
        }));

        // Store it in the registry
        mlua_expect!(
            (|state| {
                push_gc_userdata(state, Arc::clone(&extra), true)?;
                protect_lua!(state, 1, 0, fn(state) {
                    let extra_key = &EXTRA_REGISTRY_KEY as *const u8 as *const c_void;
                    ffi::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, extra_key);
                })
            })(main_state),
            "Error while storing extra data",
        );

        // Register `DestructedUserdata` type
        get_destructed_userdata_metatable(main_state);
        let destructed_mt_ptr = ffi::lua_topointer(main_state, -1);
        let destructed_ud_typeid = TypeId::of::<DestructedUserdata>();
        (*extra.get())
            .registered_userdata_mt
            .insert(destructed_mt_ptr, Some(destructed_ud_typeid));
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
            (*ffi::lua_callbacks(main_state)).userdata = extra.get() as *mut c_void;
        }

        let inner = Arc::new(LuaInner {
            state: AtomicPtr::new(state),
            main_state,
            extra: Arc::clone(&extra),
        });

        (*extra.get()).inner.write(Arc::clone(&inner));
        #[cfg(not(feature = "module"))]
        Arc::decrement_strong_count(Arc::as_ptr(&inner));

        Lua(inner)
    }

    /// Loads the specified subset of the standard libraries into an existing Lua state.
    ///
    /// Use the [`StdLib`] flags to specify the libraries you want to load.
    ///
    /// [`StdLib`]: crate::StdLib
    pub fn load_from_std_lib(&self, libs: StdLib) -> Result<()> {
        #[cfg(not(feature = "luau"))]
        let is_safe = unsafe { (*self.extra.get()).safe };

        #[cfg(not(feature = "luau"))]
        if is_safe && libs.contains(StdLib::DEBUG) {
            return Err(Error::SafetyError(
                "the unsafe `debug` module can't be loaded in safe mode".to_string(),
            ));
        }
        #[cfg(feature = "luajit")]
        if is_safe && libs.contains(StdLib::FFI) {
            return Err(Error::SafetyError(
                "the unsafe `ffi` module can't be loaded in safe mode".to_string(),
            ));
        }

        let res = unsafe { load_from_std_lib(self.main_state, libs) };

        // If `package` library loaded into a safe lua state then disable C modules
        #[cfg(not(feature = "luau"))]
        {
            let curr_libs = unsafe { (*self.extra.get()).libs };
            if is_safe && (curr_libs ^ (curr_libs | libs)).contains(StdLib::PACKAGE) {
                mlua_expect!(self.disable_c_modules(), "Error during disabling C modules");
            }
        }
        unsafe { (*self.extra.get()).libs |= libs };

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
    pub fn load_from_function<'lua, T>(&'lua self, modname: &str, func: Function<'lua>) -> Result<T>
    where
        T: FromLua<'lua>,
    {
        let state = self.state();
        let loaded = unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 2)?;
            protect_lua!(state, 0, 1, fn(state) {
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
    pub fn unload(&self, modname: &str) -> Result<()> {
        let state = self.state();
        let loaded = unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 2)?;
            protect_lua!(state, 0, 1, fn(state) {
                ffi::luaL_getsubtable(state, ffi::LUA_REGISTRYINDEX, cstr!("_LOADED"));
            })?;
            Table(self.pop_ref())
        };

        let modname = self.create_string(modname)?;
        loaded.raw_remove(modname)?;
        Ok(())
    }

    /// Consumes and leaks `Lua` object, returning a static reference `&'static Lua`.
    ///
    /// This function is useful when the `Lua` object is supposed to live for the remainder
    /// of the program's life.
    ///
    /// Dropping the returned reference will cause a memory leak. If this is not acceptable,
    /// the reference should first be wrapped with the [`Lua::from_static`] function producing a `Lua`.
    /// This `Lua` object can then be dropped which will properly release the allocated memory.
    ///
    /// [`Lua::from_static`]: #method.from_static
    #[doc(hidden)]
    pub fn into_static(self) -> &'static Self {
        Box::leak(Box::new(self))
    }

    /// Constructs a `Lua` from a static reference to it.
    ///
    /// # Safety
    /// This function is unsafe because improper use may lead to memory problems or undefined behavior.
    #[doc(hidden)]
    pub unsafe fn from_static(lua: &'static Lua) -> Self {
        *Box::from_raw(lua as *const Lua as *mut Lua)
    }

    // Executes module entrypoint function, which returns only one Value.
    // The returned value then pushed onto the stack.
    #[doc(hidden)]
    #[cfg(not(tarpaulin_include))]
    pub unsafe fn entrypoint<'lua, A, R, F>(self, func: F) -> c_int
    where
        A: FromLuaMulti<'lua>,
        R: IntoLua<'lua>,
        F: Fn(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
    {
        let (state, extra) = (self.state(), self.extra.get());
        // It must be safe to drop `self` as in the module mode we keep strong reference to `Lua` in the registry
        drop(self);

        callback_error_ext(state, extra, move |nargs| {
            let lua: &Lua = mem::transmute((*extra).inner.assume_init_ref());
            let args = A::from_stack_args(nargs, 1, None, lua)?;
            func(lua, args)?.push_into_stack(lua)?;
            Ok(1)
        })
    }

    // A simple module entrypoint without arguments
    #[doc(hidden)]
    #[cfg(not(tarpaulin_include))]
    pub unsafe fn entrypoint1<'lua, R, F>(self, func: F) -> c_int
    where
        R: IntoLua<'lua>,
        F: Fn(&'lua Lua) -> Result<R> + MaybeSend + 'static,
    {
        self.entrypoint(move |lua, _: ()| func(lua))
    }

    /// Skips memory checks for some operations.
    #[doc(hidden)]
    #[cfg(feature = "module")]
    pub fn skip_memory_check(&self, skip: bool) {
        unsafe { (*self.extra.get()).skip_memory_check = skip };
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
    #[cfg(any(feature = "luau", docsrs))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn sandbox(&self, enabled: bool) -> Result<()> {
        unsafe {
            if (*self.extra.get()).sandboxed != enabled {
                let state = self.main_state;
                check_stack(state, 3)?;
                protect_lua!(state, 0, 0, |state| {
                    if enabled {
                        ffi::luaL_sandbox(state, 1);
                        ffi::luaL_sandboxthread(state);
                    } else {
                        // Restore original `LUA_GLOBALSINDEX`
                        ffi::lua_xpush(self.ref_thread(), state, ffi::LUA_GLOBALSINDEX);
                        ffi::lua_replace(state, ffi::LUA_GLOBALSINDEX);
                        ffi::luaL_sandbox(state, 0);
                    }
                })?;
                (*self.extra.get()).sandboxed = enabled;
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
    /// This method sets a hook function for the current thread of this Lua instance.
    /// If you want to set a hook function for another thread (coroutine), use [`Thread::set_hook()`] instead.
    ///
    /// Please note you cannot have more than one hook function set at a time for this Lua instance.
    ///
    /// # Example
    ///
    /// Shows each line number of code being executed by the Lua interpreter.
    ///
    /// ```
    /// # use mlua::{Lua, HookTriggers, Result};
    /// # fn main() -> Result<()> {
    /// let lua = Lua::new();
    /// lua.set_hook(HookTriggers::EVERY_LINE, |_lua, debug| {
    ///     println!("line {}", debug.curr_line());
    ///     Ok(())
    /// });
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
    pub fn set_hook<F>(&self, triggers: HookTriggers, callback: F)
    where
        F: Fn(&Lua, Debug) -> Result<()> + MaybeSend + 'static,
    {
        unsafe { self.set_thread_hook(self.state(), triggers, callback) };
    }

    /// Sets a 'hook' function for a thread (coroutine).
    #[cfg(not(feature = "luau"))]
    pub(crate) unsafe fn set_thread_hook<F>(
        &self,
        state: *mut ffi::lua_State,
        triggers: HookTriggers,
        callback: F,
    ) where
        F: Fn(&Lua, Debug) -> Result<()> + MaybeSend + 'static,
    {
        unsafe extern "C-unwind" fn hook_proc(state: *mut ffi::lua_State, ar: *mut ffi::lua_Debug) {
            let extra = extra_data(state);
            if (*extra).hook_thread != state {
                // Hook was destined for a different thread, ignore
                ffi::lua_sethook(state, None, 0, 0);
                return;
            }
            callback_error_ext(state, extra, move |_| {
                let hook_cb = (*extra).hook_callback.clone();
                let hook_cb = mlua_expect!(hook_cb, "no hook callback set in hook_proc");
                if Arc::strong_count(&hook_cb) > 2 {
                    return Ok(()); // Don't allow recursion
                }
                let lua: &Lua = mem::transmute((*extra).inner.assume_init_ref());
                let _guard = StateGuard::new(&lua.0, state);
                let debug = Debug::new(lua, ar);
                hook_cb(lua, debug)
            })
        }

        (*self.extra.get()).hook_callback = Some(Arc::new(callback));
        (*self.extra.get()).hook_thread = state; // Mark for what thread the hook is set
        ffi::lua_sethook(state, Some(hook_proc), triggers.mask(), triggers.count());
    }

    /// Removes any hook previously set by [`Lua::set_hook()`] or [`Thread::set_hook()`].
    ///
    /// This function has no effect if a hook was not previously set.
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub fn remove_hook(&self) {
        unsafe {
            let state = self.state();
            ffi::lua_sethook(state, None, 0, 0);
            match get_main_state(self.main_state) {
                Some(main_state) if !ptr::eq(state, main_state) => {
                    // If main_state is different from state, remove hook from it too
                    ffi::lua_sethook(main_state, None, 0, 0);
                }
                _ => {}
            };
            (*self.extra.get()).hook_callback = None;
            (*self.extra.get()).hook_thread = ptr::null_mut();
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
    /// lua.set_interrupt(move |_| {
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
    #[cfg(any(feature = "luau", docsrs))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn set_interrupt<F>(&self, callback: F)
    where
        F: Fn(&Lua) -> Result<VmState> + MaybeSend + 'static,
    {
        unsafe extern "C-unwind" fn interrupt_proc(state: *mut ffi::lua_State, gc: c_int) {
            if gc >= 0 {
                // We don't support GC interrupts since they cannot survive Lua exceptions
                return;
            }
            let extra = extra_data(state);
            let result = callback_error_ext(state, extra, move |_| {
                let interrupt_cb = (*extra).interrupt_callback.clone();
                let interrupt_cb =
                    mlua_expect!(interrupt_cb, "no interrupt callback set in interrupt_proc");
                if Arc::strong_count(&interrupt_cb) > 2 {
                    return Ok(VmState::Continue); // Don't allow recursion
                }
                let lua: &Lua = mem::transmute((*extra).inner.assume_init_ref());
                let _guard = StateGuard::new(&lua.0, state);
                interrupt_cb(lua)
            });
            match result {
                VmState::Continue => {}
                VmState::Yield => {
                    ffi::lua_yield(state, 0);
                }
            }
        }

        unsafe {
            (*self.extra.get()).interrupt_callback = Some(Arc::new(callback));
            (*ffi::lua_callbacks(self.main_state)).interrupt = Some(interrupt_proc);
        }
    }

    /// Removes any 'interrupt' previously set by `set_interrupt`.
    ///
    /// This function has no effect if an 'interrupt' was not previously set.
    #[cfg(any(feature = "luau", docsrs))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn remove_interrupt(&self) {
        unsafe {
            (*self.extra.get()).interrupt_callback = None;
            (*ffi::lua_callbacks(self.main_state)).interrupt = None;
        }
    }

    /// Sets the warning function to be used by Lua to emit warnings.
    ///
    /// Requires `feature = "lua54"`
    #[cfg(feature = "lua54")]
    #[cfg_attr(docsrs, doc(cfg(feature = "lua54")))]
    pub fn set_warning_function<F>(&self, callback: F)
    where
        F: Fn(&Lua, &str, bool) -> Result<()> + MaybeSend + 'static,
    {
        unsafe extern "C-unwind" fn warn_proc(ud: *mut c_void, msg: *const c_char, tocont: c_int) {
            let extra = ud as *mut ExtraData;
            let lua: &Lua = mem::transmute((*extra).inner.assume_init_ref());
            callback_error_ext(lua.state(), extra, |_| {
                let cb = mlua_expect!(
                    (*extra).warn_callback.as_ref(),
                    "no warning callback set in warn_proc"
                );
                let msg = std::string::String::from_utf8_lossy(CStr::from_ptr(msg).to_bytes());
                cb(lua, &msg, tocont != 0)
            });
        }

        let state = self.main_state;
        unsafe {
            (*self.extra.get()).warn_callback = Some(Box::new(callback));
            ffi::lua_setwarnf(state, Some(warn_proc), self.extra.get() as *mut c_void);
        }
    }

    /// Removes warning function previously set by `set_warning_function`.
    ///
    /// This function has no effect if a warning function was not previously set.
    ///
    /// Requires `feature = "lua54"`
    #[cfg(feature = "lua54")]
    #[cfg_attr(docsrs, doc(cfg(feature = "lua54")))]
    pub fn remove_warning_function(&self) {
        unsafe {
            (*self.extra.get()).warn_callback = None;
            ffi::lua_setwarnf(self.main_state, None, ptr::null_mut());
        }
    }

    /// Emits a warning with the given message.
    ///
    /// A message in a call with `incomplete` set to `true` should be continued in
    /// another call to this function.
    ///
    /// Requires `feature = "lua54"`
    #[cfg(feature = "lua54")]
    #[cfg_attr(docsrs, doc(cfg(feature = "lua54")))]
    pub fn warning(&self, msg: impl AsRef<str>, incomplete: bool) {
        let msg = msg.as_ref();
        let mut bytes = vec![0; msg.len() + 1];
        bytes[..msg.len()].copy_from_slice(msg.as_bytes());
        let real_len = bytes.iter().position(|&c| c == 0).unwrap();
        bytes.truncate(real_len);
        unsafe {
            ffi::lua_warning(
                self.state(),
                bytes.as_ptr() as *const c_char,
                incomplete as c_int,
            );
        }
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
            if ffi::lua_getstack(self.state(), level, &mut ar) == 0 {
                return None;
            }
            #[cfg(feature = "luau")]
            if ffi::lua_getinfo(self.state(), level, cstr!(""), &mut ar) == 0 {
                return None;
            }
            Some(Debug::new_owned(self, level, ar))
        }
    }

    /// Returns the amount of memory (in bytes) currently used inside this Lua state.
    pub fn used_memory(&self) -> usize {
        unsafe {
            match (*self.extra.get()).mem_state.map(|x| x.as_ref()) {
                Some(mem_state) => mem_state.used_memory(),
                None => {
                    // Get data from the Lua GC
                    let used_kbytes = ffi::lua_gc(self.main_state, ffi::LUA_GCCOUNT, 0);
                    let used_kbytes_rem = ffi::lua_gc(self.main_state, ffi::LUA_GCCOUNTB, 0);
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
    /// Does not work in module mode where Lua state is managed externally.
    pub fn set_memory_limit(&self, limit: usize) -> Result<usize> {
        unsafe {
            match (*self.extra.get()).mem_state.map(|mut x| x.as_mut()) {
                Some(mem_state) => Ok(mem_state.set_memory_limit(limit)),
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
        unsafe { ffi::lua_gc(self.main_state, ffi::LUA_GCISRUNNING, 0) != 0 }
    }

    /// Stop the Lua GC from running
    pub fn gc_stop(&self) {
        unsafe { ffi::lua_gc(self.main_state, ffi::LUA_GCSTOP, 0) };
    }

    /// Restarts the Lua GC if it is not running
    pub fn gc_restart(&self) {
        unsafe { ffi::lua_gc(self.main_state, ffi::LUA_GCRESTART, 0) };
    }

    /// Perform a full garbage-collection cycle.
    ///
    /// It may be necessary to call this function twice to collect all currently unreachable
    /// objects. Once to finish the current gc cycle, and once to start and finish the next cycle.
    pub fn gc_collect(&self) -> Result<()> {
        unsafe {
            check_stack(self.main_state, 2)?;
            protect_lua!(self.main_state, 0, 0, fn(state) ffi::lua_gc(state, ffi::LUA_GCCOLLECT, 0))
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
        unsafe {
            check_stack(self.main_state, 3)?;
            protect_lua!(self.main_state, 0, 0, |state| {
                ffi::lua_gc(state, ffi::LUA_GCSTEP, kbytes) != 0
            })
        }
    }

    /// Sets the 'pause' value of the collector.
    ///
    /// Returns the previous value of 'pause'. More information can be found in the Lua
    /// [documentation].
    ///
    /// For Luau this parameter sets GC goal
    ///
    /// [documentation]: https://www.lua.org/manual/5.4/manual.html#2.5
    pub fn gc_set_pause(&self, pause: c_int) -> c_int {
        unsafe {
            #[cfg(not(feature = "luau"))]
            return ffi::lua_gc(self.main_state, ffi::LUA_GCSETPAUSE, pause);
            #[cfg(feature = "luau")]
            return ffi::lua_gc(self.main_state, ffi::LUA_GCSETGOAL, pause);
        }
    }

    /// Sets the 'step multiplier' value of the collector.
    ///
    /// Returns the previous value of the 'step multiplier'. More information can be found in the
    /// Lua [documentation].
    ///
    /// [documentation]: https://www.lua.org/manual/5.4/manual.html#2.5
    pub fn gc_set_step_multiplier(&self, step_multiplier: c_int) -> c_int {
        unsafe { ffi::lua_gc(self.main_state, ffi::LUA_GCSETSTEPMUL, step_multiplier) }
    }

    /// Changes the collector to incremental mode with the given parameters.
    ///
    /// Returns the previous mode (always `GCMode::Incremental` in Lua < 5.4).
    /// More information can be found in the Lua [documentation].
    ///
    /// [documentation]: https://www.lua.org/manual/5.4/manual.html#2.5.1
    pub fn gc_inc(&self, pause: c_int, step_multiplier: c_int, step_size: c_int) -> GCMode {
        let state = self.main_state;

        #[cfg(any(
            feature = "lua53",
            feature = "lua52",
            feature = "lua51",
            feature = "luajit",
            feature = "luau"
        ))]
        unsafe {
            if pause > 0 {
                #[cfg(not(feature = "luau"))]
                ffi::lua_gc(state, ffi::LUA_GCSETPAUSE, pause);
                #[cfg(feature = "luau")]
                ffi::lua_gc(state, ffi::LUA_GCSETGOAL, pause);
            }

            if step_multiplier > 0 {
                ffi::lua_gc(state, ffi::LUA_GCSETSTEPMUL, step_multiplier);
            }

            #[cfg(feature = "luau")]
            if step_size > 0 {
                ffi::lua_gc(state, ffi::LUA_GCSETSTEPSIZE, step_size);
            }
            #[cfg(not(feature = "luau"))]
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
    #[cfg(feature = "lua54")]
    #[cfg_attr(docsrs, doc(cfg(feature = "lua54")))]
    pub fn gc_gen(&self, minor_multiplier: c_int, major_multiplier: c_int) -> GCMode {
        let state = self.main_state;
        let prev_mode =
            unsafe { ffi::lua_gc(state, ffi::LUA_GCGEN, minor_multiplier, major_multiplier) };
        match prev_mode {
            ffi::LUA_GCGEN => GCMode::Generational,
            ffi::LUA_GCINC => GCMode::Incremental,
            _ => unreachable!(),
        }
    }

    /// Sets a default Luau compiler (with custom options).
    ///
    /// This compiler will be used by default to load all Lua chunks
    /// including via `require` function.
    ///
    /// See [`Compiler`] for details and possible options.
    ///
    /// Requires `feature = "luau"`
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn set_compiler(&self, compiler: Compiler) {
        unsafe { (*self.extra.get()).compiler = Some(compiler) };
    }

    /// Toggles JIT compilation mode for new chunks of code.
    ///
    /// By default JIT is enabled. Changing this option does not have any effect on
    /// already loaded functions.
    #[cfg(any(feature = "luau-jit", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau-jit")))]
    pub fn enable_jit(&self, enable: bool) {
        unsafe { (*self.extra.get()).enable_jit = enable };
    }

    /// Returns Lua source code as a `Chunk` builder type.
    ///
    /// In order to actually compile or run the resulting code, you must call [`Chunk::exec`] or
    /// similar on the returned builder. Code is not even parsed until one of these methods is
    /// called.
    ///
    /// [`Chunk::exec`]: crate::Chunk::exec
    #[track_caller]
    pub fn load<'lua, 'a>(&'lua self, chunk: impl AsChunk<'lua, 'a>) -> Chunk<'lua, 'a> {
        let caller = Location::caller();
        Chunk {
            lua: self,
            name: chunk.name().unwrap_or_else(|| caller.to_string()),
            env: chunk.environment(self),
            mode: chunk.mode(),
            source: chunk.source(),
            #[cfg(feature = "luau")]
            compiler: unsafe { (*self.extra.get()).compiler.clone() },
        }
    }

    pub(crate) fn load_chunk<'lua>(
        &'lua self,
        name: Option<&CStr>,
        env: Option<Table>,
        mode: Option<ChunkMode>,
        source: &[u8],
    ) -> Result<Function<'lua>> {
        let state = self.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 1)?;

            let mode_str = match mode {
                Some(ChunkMode::Binary) => cstr!("b"),
                Some(ChunkMode::Text) => cstr!("t"),
                None => cstr!("bt"),
            };

            match ffi::luaL_loadbufferx(
                state,
                source.as_ptr() as *const c_char,
                source.len(),
                name.map(|n| n.as_ptr()).unwrap_or_else(ptr::null),
                mode_str,
            ) {
                ffi::LUA_OK => {
                    if let Some(env) = env {
                        self.push_ref(&env.0);
                        #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
                        ffi::lua_setupvalue(state, -2, 1);
                        #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
                        ffi::lua_setfenv(state, -2);
                    }

                    #[cfg(feature = "luau-jit")]
                    if (*self.extra.get()).enable_jit && ffi::luau_codegen_supported() != 0 {
                        ffi::luau_codegen_compile(state, -1);
                    }

                    Ok(Function(self.pop_ref()))
                }
                err => Err(pop_error(state, err)),
            }
        }
    }

    /// Create and return an interned Lua string. Lua strings can be arbitrary [u8] data including
    /// embedded nulls, so in addition to `&str` and `&String`, you can also pass plain `&[u8]`
    /// here.
    pub fn create_string(&self, s: impl AsRef<[u8]>) -> Result<String> {
        let state = self.state();
        unsafe {
            if self.unlikely_memory_error() {
                push_string(self.ref_thread(), s.as_ref(), false)?;
                return Ok(String(self.pop_ref_thread()));
            }

            let _sg = StackGuard::new(state);
            check_stack(state, 3)?;
            push_string(state, s.as_ref(), true)?;
            Ok(String(self.pop_ref()))
        }
    }

    /// Creates and returns a new empty table.
    pub fn create_table(&self) -> Result<Table> {
        self.create_table_with_capacity(0, 0)
    }

    /// Creates and returns a new empty table, with the specified capacity.
    /// `narr` is a hint for how many elements the table will have as a sequence;
    /// `nrec` is a hint for how many other elements the table will have.
    /// Lua may use these hints to preallocate memory for the new table.
    pub fn create_table_with_capacity(&self, narr: usize, nrec: usize) -> Result<Table> {
        let state = self.state();
        unsafe {
            if self.unlikely_memory_error() {
                push_table(self.ref_thread(), narr, nrec, false)?;
                return Ok(Table(self.pop_ref_thread()));
            }

            let _sg = StackGuard::new(state);
            check_stack(state, 3)?;
            push_table(state, narr, nrec, true)?;
            Ok(Table(self.pop_ref()))
        }
    }

    /// Creates a table and fills it with values from an iterator.
    pub fn create_table_from<'lua, K, V, I>(&'lua self, iter: I) -> Result<Table<'lua>>
    where
        K: IntoLua<'lua>,
        V: IntoLua<'lua>,
        I: IntoIterator<Item = (K, V)>,
    {
        let state = self.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 6)?;

            let iter = iter.into_iter();
            let lower_bound = iter.size_hint().0;
            let protect = !self.unlikely_memory_error();
            push_table(state, 0, lower_bound, protect)?;
            for (k, v) in iter {
                self.push_value(k.into_lua(self)?)?;
                self.push_value(v.into_lua(self)?)?;
                if protect {
                    protect_lua!(state, 3, 1, fn(state) ffi::lua_rawset(state, -3))?;
                } else {
                    ffi::lua_rawset(state, -3);
                }
            }

            Ok(Table(self.pop_ref()))
        }
    }

    /// Creates a table from an iterator of values, using `1..` as the keys.
    pub fn create_sequence_from<'lua, T, I>(&'lua self, iter: I) -> Result<Table<'lua>>
    where
        T: IntoLua<'lua>,
        I: IntoIterator<Item = T>,
    {
        let state = self.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 5)?;

            let iter = iter.into_iter();
            let lower_bound = iter.size_hint().0;
            let protect = !self.unlikely_memory_error();
            push_table(state, lower_bound, 0, protect)?;
            for (i, v) in iter.enumerate() {
                self.push_value(v.into_lua(self)?)?;
                if protect {
                    protect_lua!(state, 2, 1, |state| {
                        ffi::lua_rawseti(state, -2, (i + 1) as Integer);
                    })?;
                } else {
                    ffi::lua_rawseti(state, -2, (i + 1) as Integer);
                }
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
    /// values. For details on Rust-to-Lua conversions, refer to the [`IntoLua`] and [`IntoLuaMulti`]
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
    /// [`IntoLua`]: crate::IntoLua
    /// [`IntoLuaMulti`]: crate::IntoLuaMulti
    pub fn create_function<'lua, A, R, F>(&'lua self, func: F) -> Result<Function<'lua>>
    where
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
        F: Fn(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
    {
        self.create_callback(Box::new(move |lua, nargs| unsafe {
            let args = A::from_stack_args(nargs, 1, None, lua)?;
            func(lua, args)?.push_into_stack_multi(lua)
        }))
    }

    /// Wraps a Rust mutable closure, creating a callable Lua function handle to it.
    ///
    /// This is a version of [`create_function`] that accepts a FnMut argument. Refer to
    /// [`create_function`] for more information about the implementation.
    ///
    /// [`create_function`]: #method.create_function
    pub fn create_function_mut<'lua, A, R, F>(&'lua self, func: F) -> Result<Function<'lua>>
    where
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
        F: FnMut(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
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
        let state = self.state();
        check_stack(state, 1)?;
        ffi::lua_pushcfunction(state, func);
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
    /// use mlua::{Lua, Result};
    ///
    /// async fn sleep(_lua: &Lua, n: u64) -> Result<&'static str> {
    ///     tokio::time::sleep(Duration::from_millis(n)).await;
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
    pub fn create_async_function<'lua, A, R, F, FR>(&'lua self, func: F) -> Result<Function<'lua>>
    where
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
        F: Fn(&'lua Lua, A) -> FR + MaybeSend + 'static,
        FR: Future<Output = Result<R>> + 'lua,
    {
        self.create_async_callback(Box::new(move |lua, args| unsafe {
            let args = match A::from_lua_args(args, 1, None, lua) {
                Ok(args) => args,
                Err(e) => return Box::pin(future::err(e)),
            };
            let fut = func(lua, args);
            Box::pin(async move { fut.await?.push_into_stack_multi(lua) })
        }))
    }

    /// Wraps a Lua function into a new thread (or coroutine).
    ///
    /// Equivalent to `coroutine.create`.
    pub fn create_thread<'lua>(&'lua self, func: Function) -> Result<Thread<'lua>> {
        self.create_thread_inner(&func)
    }

    /// Wraps a Lua function into a new thread (or coroutine).
    ///
    /// Takes function by reference.
    fn create_thread_inner<'lua>(&'lua self, func: &Function) -> Result<Thread<'lua>> {
        let state = self.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 3)?;

            let thread_state = if self.unlikely_memory_error() {
                ffi::lua_newthread(state)
            } else {
                protect_lua!(state, 0, 1, |state| ffi::lua_newthread(state))?
            };
            self.push_ref(&func.0);
            ffi::lua_xmove(state, thread_state, 1);

            Ok(Thread::new(self.pop_ref()))
        }
    }

    /// Wraps a Lua function into a new or recycled thread (coroutine).
    #[cfg(feature = "async")]
    pub(crate) fn create_recycled_thread<'lua>(
        &'lua self,
        func: &Function,
    ) -> Result<Thread<'lua>> {
        #[cfg(any(feature = "lua54", feature = "luau"))]
        unsafe {
            let state = self.state();
            let _sg = StackGuard::new(state);
            check_stack(state, 1)?;

            if let Some(index) = (*self.extra.get()).thread_pool.pop() {
                let thread_state = ffi::lua_tothread(self.ref_thread(), index);
                self.push_ref(&func.0);
                ffi::lua_xmove(state, thread_state, 1);

                #[cfg(feature = "luau")]
                {
                    // Inherit `LUA_GLOBALSINDEX` from the caller
                    ffi::lua_xpush(state, thread_state, ffi::LUA_GLOBALSINDEX);
                    ffi::lua_replace(thread_state, ffi::LUA_GLOBALSINDEX);
                }

                return Ok(Thread::new(LuaRef::new(self, index)));
            }
        };
        self.create_thread_inner(func)
    }

    /// Resets thread (coroutine) and returns to the pool for later use.
    #[cfg(feature = "async")]
    #[cfg(any(feature = "lua54", feature = "luau"))]
    pub(crate) unsafe fn recycle_thread(&self, thread: &mut Thread) -> bool {
        let extra = &mut *self.extra.get();
        if extra.thread_pool.len() < extra.thread_pool.capacity() {
            let thread_state = ffi::lua_tothread(extra.ref_thread, thread.0.index);
            #[cfg(all(feature = "lua54", not(feature = "vendored")))]
            let status = ffi::lua_resetthread(thread_state);
            #[cfg(all(feature = "lua54", feature = "vendored"))]
            let status = ffi::lua_closethread(thread_state, self.state());
            #[cfg(feature = "lua54")]
            if status != ffi::LUA_OK {
                // Error object is on top, drop it
                ffi::lua_settop(thread_state, 0);
            }
            #[cfg(feature = "luau")]
            ffi::lua_resetthread(thread_state);
            extra.thread_pool.push(thread.0.index);
            thread.0.drop = false;
            return true;
        }
        false
    }

    /// Creates a Lua userdata object from a custom userdata type.
    ///
    /// All userdata instances of the same type `T` shares the same metatable.
    #[inline]
    pub fn create_userdata<T>(&self, data: T) -> Result<AnyUserData>
    where
        T: UserData + MaybeSend + 'static,
    {
        unsafe { self.make_userdata(UserDataCell::new(data)) }
    }

    /// Creates a Lua userdata object from a custom serializable userdata type.
    ///
    /// Requires `feature = "serialize"`
    #[cfg(feature = "serialize")]
    #[cfg_attr(docsrs, doc(cfg(feature = "serialize")))]
    #[inline]
    pub fn create_ser_userdata<T>(&self, data: T) -> Result<AnyUserData>
    where
        T: UserData + Serialize + MaybeSend + 'static,
    {
        unsafe { self.make_userdata(UserDataCell::new_ser(data)) }
    }

    /// Creates a Lua userdata object from a custom Rust type.
    ///
    /// You can register the type using [`Lua::register_userdata_type()`] to add fields or methods
    /// _before_ calling this method.
    /// Otherwise, the userdata object will have an empty metatable.
    ///
    /// All userdata instances of the same type `T` shares the same metatable.
    #[inline]
    pub fn create_any_userdata<T>(&self, data: T) -> Result<AnyUserData>
    where
        T: MaybeSend + 'static,
    {
        unsafe { self.make_any_userdata(UserDataCell::new(data)) }
    }

    /// Registers a custom Rust type in Lua to use in userdata objects.
    ///
    /// This methods provides a way to add fields or methods to userdata objects of a type `T`.
    pub fn register_userdata_type<T: 'static>(
        &self,
        f: impl FnOnce(&mut UserDataRegistry<T>),
    ) -> Result<()> {
        let mut registry = UserDataRegistry::new();
        f(&mut registry);

        unsafe {
            // Deregister the type if it already registered
            let type_id = TypeId::of::<T>();
            if let Some(&table_id) = (*self.extra.get()).registered_userdata.get(&type_id) {
                ffi::luaL_unref(self.state(), ffi::LUA_REGISTRYINDEX, table_id);
            }

            // Register the type
            self.register_userdata_metatable(registry)?;
            Ok(())
        }
    }

    /// Create a Lua userdata "proxy" object from a custom userdata type.
    ///
    /// Proxy object is an empty userdata object that has `T` metatable attached.
    /// The main purpose of this object is to provide access to static fields and functions
    /// without creating an instance of type `T`.
    ///
    /// You can get or set uservalues on this object but you cannot borrow any Rust type.
    ///
    /// # Examples
    ///
    /// ```
    /// # use mlua::{Lua, Result, UserData, UserDataFields, UserDataMethods};
    /// # fn main() -> Result<()> {
    /// # let lua = Lua::new();
    /// struct MyUserData(i32);
    ///
    /// impl UserData for MyUserData {
    ///     fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
    ///         fields.add_field_method_get("val", |_, this| Ok(this.0));
    ///     }
    ///
    ///     fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
    ///         methods.add_function("new", |_, value: i32| Ok(MyUserData(value)));
    ///     }
    /// }
    ///
    /// lua.globals().set("MyUserData", lua.create_proxy::<MyUserData>()?)?;
    ///
    /// lua.load("assert(MyUserData.new(321).val == 321)").exec()?;
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn create_proxy<T>(&self) -> Result<AnyUserData>
    where
        T: UserData + 'static,
    {
        unsafe { self.make_userdata(UserDataCell::new(UserDataProxy::<T>(PhantomData))) }
    }

    /// Sets the metatable for a Luau builtin vector type.
    #[cfg(any(all(feature = "luau", feature = "unstable"), doc))]
    #[cfg_attr(docsrs, doc(cfg(all(feature = "luau", feature = "unstable"))))]
    pub fn set_vector_metatable(&self, metatable: Option<Table>) {
        unsafe {
            let state = self.state();
            let _sg = StackGuard::new(state);
            assert_stack(state, 2);

            #[cfg(not(feature = "luau-vector4"))]
            ffi::lua_pushvector(state, 0., 0., 0.);
            #[cfg(feature = "luau-vector4")]
            ffi::lua_pushvector(state, 0., 0., 0., 0.);
            match metatable {
                Some(metatable) => self.push_ref(&metatable.0),
                None => ffi::lua_pushnil(state),
            };
            ffi::lua_setmetatable(state, -2);
        }
    }

    /// Returns a handle to the global environment.
    pub fn globals(&self) -> Table {
        let state = self.state();
        unsafe {
            let _sg = StackGuard::new(state);
            assert_stack(state, 1);
            #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
            ffi::lua_rawgeti(state, ffi::LUA_REGISTRYINDEX, ffi::LUA_RIDX_GLOBALS);
            #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
            ffi::lua_pushvalue(state, ffi::LUA_GLOBALSINDEX);
            Table(self.pop_ref())
        }
    }

    /// Returns a handle to the active `Thread`. For calls to `Lua` this will be the main Lua thread,
    /// for parameters given to a callback, this will be whatever Lua thread called the callback.
    pub fn current_thread(&self) -> Thread {
        let state = self.state();
        unsafe {
            let _sg = StackGuard::new(state);
            assert_stack(state, 1);
            ffi::lua_pushthread(state);
            Thread::new(self.pop_ref())
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
    pub fn scope<'lua, 'scope, R>(
        &'lua self,
        f: impl FnOnce(&Scope<'lua, 'scope>) -> Result<R>,
    ) -> Result<R>
    where
        'lua: 'scope,
    {
        f(&Scope::new(self))
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
                let state = self.state();
                let _sg = StackGuard::new(state);
                check_stack(state, 4)?;

                self.push_value(v)?;
                let res = if self.unlikely_memory_error() {
                    ffi::lua_tolstring(state, -1, ptr::null_mut())
                } else {
                    protect_lua!(state, 1, 1, |state| {
                        ffi::lua_tolstring(state, -1, ptr::null_mut())
                    })?
                };
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
                let state = self.state();
                let _sg = StackGuard::new(state);
                check_stack(state, 2)?;

                self.push_value(v)?;
                let mut isint = 0;
                let i = ffi::lua_tointegerx(state, -1, &mut isint);
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
                let state = self.state();
                let _sg = StackGuard::new(state);
                check_stack(state, 2)?;

                self.push_value(v)?;
                let mut isnum = 0;
                let n = ffi::lua_tonumberx(state, -1, &mut isnum);
                if isnum == 0 {
                    None
                } else {
                    Some(n)
                }
            },
        })
    }

    /// Converts a value that implements `IntoLua` into a `Value` instance.
    pub fn pack<'lua, T: IntoLua<'lua>>(&'lua self, t: T) -> Result<Value<'lua>> {
        t.into_lua(self)
    }

    /// Converts a `Value` instance into a value that implements `FromLua`.
    pub fn unpack<'lua, T: FromLua<'lua>>(&'lua self, value: Value<'lua>) -> Result<T> {
        T::from_lua(value, self)
    }

    /// Converts a value that implements `IntoLuaMulti` into a `MultiValue` instance.
    pub fn pack_multi<'lua, T: IntoLuaMulti<'lua>>(&'lua self, t: T) -> Result<MultiValue<'lua>> {
        t.into_lua_multi(self)
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
    pub fn set_named_registry_value<'lua, T>(&'lua self, name: &str, t: T) -> Result<()>
    where
        T: IntoLua<'lua>,
    {
        let state = self.state();
        let t = t.into_lua(self)?;
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 5)?;

            self.push_value(t)?;
            rawset_field(state, ffi::LUA_REGISTRYINDEX, name)
        }
    }

    /// Get a value from the Lua registry based on a string name.
    ///
    /// Any Lua instance which shares the underlying main state may call this method to
    /// get a value previously set by [`set_named_registry_value`].
    ///
    /// [`set_named_registry_value`]: #method.set_named_registry_value
    pub fn named_registry_value<'lua, T>(&'lua self, name: &str) -> Result<T>
    where
        T: FromLua<'lua>,
    {
        let state = self.state();
        let value = unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 3)?;

            let protect = !self.unlikely_memory_error();
            push_string(state, name.as_bytes(), protect)?;
            ffi::lua_rawget(state, ffi::LUA_REGISTRYINDEX);

            self.pop_value()
        };
        T::from_lua(value, self)
    }

    /// Removes a named value in the Lua registry.
    ///
    /// Equivalent to calling [`set_named_registry_value`] with a value of Nil.
    ///
    /// [`set_named_registry_value`]: #method.set_named_registry_value
    pub fn unset_named_registry_value(&self, name: &str) -> Result<()> {
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
    pub fn create_registry_value<'lua, T: IntoLua<'lua>>(&'lua self, t: T) -> Result<RegistryKey> {
        let t = t.into_lua(self)?;
        if t == Value::Nil {
            // Special case to skip calling `luaL_ref` and use `LUA_REFNIL` instead
            let unref_list = unsafe { (*self.extra.get()).registry_unref_list.clone() };
            return Ok(RegistryKey::new(ffi::LUA_REFNIL, unref_list));
        }

        let state = self.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 4)?;

            self.push_value(t)?;

            // Try to reuse previously allocated slot
            let unref_list = (*self.extra.get()).registry_unref_list.clone();
            #[cfg(feature = "std")]
            let free_registry_id = mlua_expect!(unref_list.lock(), "unref list poisoned")
                .as_mut()
                .and_then(|x| x.pop());
            #[cfg(not(feature = "std"))]
            let free_registry_id = mlua_expect!(unref_list.try_borrow_mut(), "unref list poisoned")
                .as_mut()
                .and_then(|x| x.pop());
            if let Some(registry_id) = free_registry_id {
                // It must be safe to replace the value without triggering memory error
                ffi::lua_rawseti(state, ffi::LUA_REGISTRYINDEX, registry_id as Integer);
                return Ok(RegistryKey::new(registry_id, unref_list));
            }

            // Allocate a new RegistryKey
            let registry_id = if self.unlikely_memory_error() {
                ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX)
            } else {
                protect_lua!(state, 1, 0, |state| {
                    ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX)
                })?
            };
            Ok(RegistryKey::new(registry_id, unref_list))
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

        let state = self.state();
        let value = match key.is_nil() {
            true => Value::Nil,
            false => unsafe {
                let _sg = StackGuard::new(state);
                check_stack(state, 1)?;

                let id = key.registry_id as Integer;
                ffi::lua_rawgeti(state, ffi::LUA_REGISTRYINDEX, id);
                self.pop_value()
            },
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
            ffi::luaL_unref(self.state(), ffi::LUA_REGISTRYINDEX, key.take());
        }
        Ok(())
    }

    /// Replaces a value in the Lua registry by its `RegistryKey`.
    ///
    /// See [`create_registry_value`] for more details.
    ///
    /// [`create_registry_value`]: #method.create_registry_value
    pub fn replace_registry_value<'lua, T: IntoLua<'lua>>(
        &'lua self,
        key: &RegistryKey,
        t: T,
    ) -> Result<()> {
        if !self.owns_registry_value(key) {
            return Err(Error::MismatchedRegistryKey);
        }

        let t = t.into_lua(self)?;
        if t == Value::Nil && key.is_nil() {
            // Nothing to replace
            return Ok(());
        } else if t != Value::Nil && key.registry_id == ffi::LUA_REFNIL {
            // We cannot update `LUA_REFNIL` slot
            return Err(Error::runtime("cannot replace nil value with non-nil"));
        }

        let state = self.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 2)?;

            let id = key.registry_id as Integer;
            if t == Value::Nil {
                self.push_value(Value::Integer(id))?;
                key.set_nil(true);
            } else {
                self.push_value(t)?;
                key.set_nil(false);
            }
            // It must be safe to replace the value without triggering memory error
            ffi::lua_rawseti(state, ffi::LUA_REGISTRYINDEX, id);
        }
        Ok(())
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
        let state = self.state();
        unsafe {
            #[cfg(feature = "std")]
            let mut unref_list = mlua_expect!(
                (*self.extra.get()).registry_unref_list.lock(),
                "unref list poisoned"
            );
            #[cfg(not(feature = "std"))]
            let mut unref_list = mlua_expect!(
                (*self.extra.get()).registry_unref_list.try_borrow_mut(),
                "unref list poisoned"
            );
            let unref_list = mem::replace(&mut *unref_list, Some(Vec::new()));
            for id in mlua_expect!(unref_list, "unref list not set") {
                ffi::luaL_unref(state, ffi::LUA_REGISTRYINDEX, id);
            }
        }
    }

    /// Sets or replaces an application data object of type `T`.
    ///
    /// Application data could be accessed at any time by using [`Lua::app_data_ref()`] or [`Lua::app_data_mut()`]
    /// methods where `T` is the data type.
    ///
    /// # Panics
    ///
    /// Panics if the app data container is currently borrowed.
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
    #[track_caller]
    pub fn set_app_data<T: MaybeSend + 'static>(&self, data: T) -> Option<T> {
        let extra = unsafe { &*self.extra.get() };
        extra.app_data.insert(data)
    }

    /// Tries to set or replace an application data object of type `T`.
    ///
    /// Returns:
    /// - `Ok(Some(old_data))` if the data object of type `T` was successfully replaced.
    /// - `Ok(None)` if the data object of type `T` was successfully inserted.
    /// - `Err(data)` if the data object of type `T` was not inserted because the container is currently borrowed.
    ///
    /// See [`Lua::set_app_data()`] for examples.
    pub fn try_set_app_data<T: MaybeSend + 'static>(&self, data: T) -> StdResult<Option<T>, T> {
        let extra = unsafe { &*self.extra.get() };
        extra.app_data.try_insert(data)
    }

    /// Gets a reference to an application data object stored by [`Lua::set_app_data()`] of type `T`.
    ///
    /// # Panics
    ///
    /// Panics if the data object of type `T` is currently mutably borrowed. Multiple immutable reads
    /// can be taken out at the same time.
    #[track_caller]
    pub fn app_data_ref<T: 'static>(&self) -> Option<AppDataRef<T>> {
        let extra = unsafe { &*self.extra.get() };
        extra.app_data.borrow()
    }

    /// Gets a mutable reference to an application data object stored by [`Lua::set_app_data()`] of type `T`.
    ///
    /// # Panics
    ///
    /// Panics if the data object of type `T` is currently borrowed.
    #[track_caller]
    pub fn app_data_mut<T: 'static>(&self) -> Option<AppDataRefMut<T>> {
        let extra = unsafe { &*self.extra.get() };
        extra.app_data.borrow_mut()
    }

    /// Removes an application data of type `T`.
    ///
    /// # Panics
    ///
    /// Panics if the app data container is currently borrowed.
    #[track_caller]
    pub fn remove_app_data<T: 'static>(&self) -> Option<T> {
        let extra = unsafe { &*self.extra.get() };
        extra.app_data.remove()
    }

    /// Pushes a value onto the Lua stack.
    ///
    /// Uses 2 stack spaces, does not call checkstack.
    #[doc(hidden)]
    pub unsafe fn push_value(&self, value: Value) -> Result<()> {
        let state = self.state();
        match value {
            Value::Nil => {
                ffi::lua_pushnil(state);
            }

            Value::Boolean(b) => {
                ffi::lua_pushboolean(state, b as c_int);
            }

            Value::LightUserData(ud) => {
                ffi::lua_pushlightuserdata(state, ud.0);
            }

            Value::Integer(i) => {
                ffi::lua_pushinteger(state, i);
            }

            Value::Number(n) => {
                ffi::lua_pushnumber(state, n);
            }

            #[cfg(feature = "luau")]
            Value::Vector(v) => {
                #[cfg(not(feature = "luau-vector4"))]
                ffi::lua_pushvector(state, v.x(), v.y(), v.z());
                #[cfg(feature = "luau-vector4")]
                ffi::lua_pushvector(state, v.x(), v.y(), v.z(), v.w());
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
                let protect = !self.unlikely_memory_error();
                push_gc_userdata(state, WrappedFailure::Error(err), protect)?;
            }
        }

        Ok(())
    }

    /// Pops a value from the Lua stack.
    ///
    /// Uses 2 stack spaces, does not call checkstack.
    #[doc(hidden)]
    pub unsafe fn pop_value(&self) -> Value {
        let state = self.state();
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

            #[cfg(any(feature = "lua54", feature = "lua53"))]
            ffi::LUA_TNUMBER => {
                let v = if ffi::lua_isinteger(state, -1) != 0 {
                    Value::Integer(ffi::lua_tointeger(state, -1))
                } else {
                    Value::Number(ffi::lua_tonumber(state, -1))
                };
                ffi::lua_pop(state, 1);
                v
            }

            #[cfg(any(
                feature = "lua52",
                feature = "lua51",
                feature = "luajit",
                feature = "luau"
            ))]
            ffi::LUA_TNUMBER => {
                let n = ffi::lua_tonumber(state, -1);
                ffi::lua_pop(state, 1);
                match num_traits::cast(n) {
                    Some(i) if (n - (i as Number)).abs() < Number::EPSILON => Value::Integer(i),
                    _ => Value::Number(n),
                }
            }

            #[cfg(feature = "luau")]
            ffi::LUA_TVECTOR => {
                let v = ffi::lua_tovector(state, -1);
                mlua_debug_assert!(!v.is_null(), "vector is null");
                #[cfg(not(feature = "luau-vector4"))]
                let vec = Value::Vector(Vector([*v, *v.add(1), *v.add(2)]));
                #[cfg(feature = "luau-vector4")]
                let vec = Value::Vector(Vector([*v, *v.add(1), *v.add(2), *v.add(3)]));
                ffi::lua_pop(state, 1);
                vec
            }

            ffi::LUA_TSTRING => Value::String(String(self.pop_ref())),

            ffi::LUA_TTABLE => Value::Table(Table(self.pop_ref())),

            ffi::LUA_TFUNCTION => Value::Function(Function(self.pop_ref())),

            ffi::LUA_TUSERDATA => {
                let wrapped_failure_mt_ptr = (*self.extra.get()).wrapped_failure_mt_ptr;
                // We must prevent interaction with userdata types other than UserData OR a WrappedError.
                // WrappedPanics are automatically resumed.
                match get_gc_userdata::<WrappedFailure>(state, -1, wrapped_failure_mt_ptr).as_mut()
                {
                    Some(WrappedFailure::Error(err)) => {
                        let err = err.clone();
                        ffi::lua_pop(state, 1);
                        Value::Error(err)
                    }
                    #[cfg(feature = "panic-safety")]
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

            ffi::LUA_TTHREAD => Value::Thread(Thread::new(self.pop_ref())),

            #[cfg(feature = "luajit")]
            ffi::LUA_TCDATA => {
                ffi::lua_pop(state, 1);
                // TODO: Fix this in a next major release
                panic!("cdata objects cannot be handled by mlua yet");
            }

            _ => mlua_panic!("LUA_TNONE in pop_value"),
        }
    }

    /// Returns value at given stack index without popping it.
    ///
    /// Uses 2 stack spaces, does not call checkstack.
    pub(crate) unsafe fn stack_value(&self, idx: c_int) -> Value {
        let state = self.state();
        match ffi::lua_type(state, idx) {
            ffi::LUA_TNIL => Nil,

            ffi::LUA_TBOOLEAN => Value::Boolean(ffi::lua_toboolean(state, idx) != 0),

            ffi::LUA_TLIGHTUSERDATA => {
                Value::LightUserData(LightUserData(ffi::lua_touserdata(state, idx)))
            }

            #[cfg(any(feature = "lua54", feature = "lua53"))]
            ffi::LUA_TNUMBER => {
                if ffi::lua_isinteger(state, idx) != 0 {
                    Value::Integer(ffi::lua_tointeger(state, idx))
                } else {
                    Value::Number(ffi::lua_tonumber(state, idx))
                }
            }

            #[cfg(any(
                feature = "lua52",
                feature = "lua51",
                feature = "luajit",
                feature = "luau"
            ))]
            ffi::LUA_TNUMBER => {
                let n = ffi::lua_tonumber(state, idx);
                match num_traits::cast(n) {
                    Some(i) if (n - (i as Number)).abs() < Number::EPSILON => Value::Integer(i),
                    _ => Value::Number(n),
                }
            }

            #[cfg(feature = "luau")]
            ffi::LUA_TVECTOR => {
                let v = ffi::lua_tovector(state, idx);
                mlua_debug_assert!(!v.is_null(), "vector is null");
                #[cfg(not(feature = "luau-vector4"))]
                return Value::Vector(Vector([*v, *v.add(1), *v.add(2)]));
                #[cfg(feature = "luau-vector4")]
                return Value::Vector(Vector([*v, *v.add(1), *v.add(2), *v.add(3)]));
            }

            ffi::LUA_TSTRING => {
                ffi::lua_xpush(state, self.ref_thread(), idx);
                Value::String(String(self.pop_ref_thread()))
            }

            ffi::LUA_TTABLE => {
                ffi::lua_xpush(state, self.ref_thread(), idx);
                Value::Table(Table(self.pop_ref_thread()))
            }

            ffi::LUA_TFUNCTION => {
                ffi::lua_xpush(state, self.ref_thread(), idx);
                Value::Function(Function(self.pop_ref_thread()))
            }

            ffi::LUA_TUSERDATA => {
                let wrapped_failure_mt_ptr = (*self.extra.get()).wrapped_failure_mt_ptr;
                // We must prevent interaction with userdata types other than UserData OR a WrappedError.
                // WrappedPanics are automatically resumed.
                match get_gc_userdata::<WrappedFailure>(state, idx, wrapped_failure_mt_ptr).as_mut()
                {
                    Some(WrappedFailure::Error(err)) => Value::Error(err.clone()),
                    #[cfg(feature = "panic-safety")]
                    Some(WrappedFailure::Panic(panic)) => {
                        if let Some(panic) = panic.take() {
                            resume_unwind(panic);
                        }
                        // Previously resumed panic?
                        Value::Nil
                    }
                    _ => {
                        ffi::lua_xpush(state, self.ref_thread(), idx);
                        Value::UserData(AnyUserData(self.pop_ref_thread()))
                    }
                }
            }

            ffi::LUA_TTHREAD => {
                ffi::lua_xpush(state, self.ref_thread(), idx);
                Value::Thread(Thread::new(self.pop_ref_thread()))
            }

            #[cfg(feature = "luajit")]
            ffi::LUA_TCDATA => {
                // TODO: Fix this in a next major release
                panic!("cdata objects cannot be handled by mlua yet");
            }

            _ => mlua_panic!("LUA_TNONE in pop_value"),
        }
    }

    // Pushes a LuaRef value onto the stack, uses 1 stack space, does not call checkstack
    pub(crate) unsafe fn push_ref(&self, lref: &LuaRef) {
        assert!(
            Arc::ptr_eq(&lref.lua.0, &self.0),
            "Lua instance passed Value created from a different main Lua state"
        );
        ffi::lua_xpush(self.ref_thread(), self.state(), lref.index);
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
        ffi::lua_xmove(self.state(), self.ref_thread(), 1);
        let index = ref_stack_pop(self.extra.get());
        LuaRef::new(self, index)
    }

    // Same as `pop_ref` but assumes the value is already on the reference thread
    pub(crate) unsafe fn pop_ref_thread(&self) -> LuaRef {
        let index = ref_stack_pop(self.extra.get());
        LuaRef::new(self, index)
    }

    pub(crate) fn clone_ref(&self, lref: &LuaRef) -> LuaRef {
        unsafe {
            ffi::lua_pushvalue(self.ref_thread(), lref.index);
            let index = ref_stack_pop(self.extra.get());
            LuaRef::new(self, index)
        }
    }

    pub(crate) fn drop_ref_index(&self, index: c_int) {
        unsafe {
            let ref_thread = self.ref_thread();
            ffi::lua_pushnil(ref_thread);
            ffi::lua_replace(ref_thread, index);
            (*self.extra.get()).ref_free.push(index);
        }
    }

    #[cfg(all(feature = "unstable", not(feature = "send")))]
    pub(crate) fn adopt_owned_ref(&self, loref: crate::types::LuaOwnedRef) -> LuaRef {
        assert!(
            Arc::ptr_eq(&loref.inner, &self.0),
            "Lua instance passed Value created from a different main Lua state"
        );
        let index = loref.index;
        unsafe {
            ptr::read(&loref.inner);
            mem::forget(loref);
        }
        LuaRef::new(self, index)
    }

    unsafe fn register_userdata_metatable<'lua, T: 'static>(
        &'lua self,
        mut registry: UserDataRegistry<'lua, T>,
    ) -> Result<Integer> {
        let state = self.state();
        let _sg = StackGuard::new(state);
        check_stack(state, 13)?;

        // Prepare metatable, add meta methods first and then meta fields
        let metatable_nrec = registry.meta_methods.len() + registry.meta_fields.len();
        #[cfg(feature = "async")]
        let metatable_nrec = metatable_nrec + registry.async_meta_methods.len();
        push_table(state, 0, metatable_nrec, true)?;
        for (k, m) in registry.meta_methods {
            self.push_value(Value::Function(self.create_callback(m)?))?;
            rawset_field(state, -2, MetaMethod::validate(&k)?)?;
        }
        #[cfg(feature = "async")]
        for (k, m) in registry.async_meta_methods {
            self.push_value(Value::Function(self.create_async_callback(m)?))?;
            rawset_field(state, -2, MetaMethod::validate(&k)?)?;
        }
        let mut has_name = false;
        for (k, f) in registry.meta_fields {
            has_name = has_name || k == MetaMethod::Type;
            mlua_assert!(f(self, 0)? == 1, "field function must return one value");
            rawset_field(state, -2, MetaMethod::validate(&k)?)?;
        }
        // Set `__name/__type` if not provided
        if !has_name {
            let type_name = short_type_name::<T>();
            push_string(state, type_name.as_bytes(), !self.unlikely_memory_error())?;
            rawset_field(state, -2, MetaMethod::Type.name())?;
        }
        let metatable_index = ffi::lua_absindex(state, -1);

        let mut extra_tables_count = 0;

        let fields_nrec = registry.fields.len();
        if fields_nrec > 0 {
            // If __index is a table then update it inplace
            let index_type = ffi::lua_getfield(state, metatable_index, cstr!("__index"));
            match index_type {
                ffi::LUA_TNIL | ffi::LUA_TTABLE => {
                    if index_type == ffi::LUA_TNIL {
                        // Create a new table
                        ffi::lua_pop(state, 1);
                        push_table(state, 0, fields_nrec, true)?;
                    }
                    for (k, f) in registry.fields {
                        mlua_assert!(f(self, 0)? == 1, "field function must return one value");
                        rawset_field(state, -2, &k)?;
                    }
                    rawset_field(state, metatable_index, "__index")?;
                }
                _ => {
                    // Propagate fields to the field getters
                    for (k, f) in registry.fields {
                        registry.field_getters.push((k, f))
                    }
                }
            }
        }

        let mut field_getters_index = None;
        let field_getters_nrec = registry.field_getters.len();
        if field_getters_nrec > 0 {
            push_table(state, 0, field_getters_nrec, true)?;
            for (k, m) in registry.field_getters {
                self.push_value(Value::Function(self.create_callback(m)?))?;
                rawset_field(state, -2, &k)?;
            }
            field_getters_index = Some(ffi::lua_absindex(state, -1));
            extra_tables_count += 1;
        }

        let mut field_setters_index = None;
        let field_setters_nrec = registry.field_setters.len();
        if field_setters_nrec > 0 {
            push_table(state, 0, field_setters_nrec, true)?;
            for (k, m) in registry.field_setters {
                self.push_value(Value::Function(self.create_callback(m)?))?;
                rawset_field(state, -2, &k)?;
            }
            field_setters_index = Some(ffi::lua_absindex(state, -1));
            extra_tables_count += 1;
        }

        let mut methods_index = None;
        let methods_nrec = registry.methods.len();
        #[cfg(feature = "async")]
        let methods_nrec = methods_nrec + registry.async_methods.len();
        if methods_nrec > 0 {
            // If __index is a table then update it inplace
            let index_type = ffi::lua_getfield(state, metatable_index, cstr!("__index"));
            match index_type {
                ffi::LUA_TTABLE => {} // Update the existing table
                _ => {
                    // Create a new table
                    ffi::lua_pop(state, 1);
                    push_table(state, 0, methods_nrec, true)?;
                }
            }
            for (k, m) in registry.methods {
                self.push_value(Value::Function(self.create_callback(m)?))?;
                rawset_field(state, -2, &k)?;
            }
            #[cfg(feature = "async")]
            for (k, m) in registry.async_methods {
                self.push_value(Value::Function(self.create_async_callback(m)?))?;
                rawset_field(state, -2, &k)?;
            }
            match index_type {
                ffi::LUA_TTABLE => {
                    ffi::lua_pop(state, 1); // All done
                }
                ffi::LUA_TNIL => {
                    rawset_field(state, metatable_index, "__index")?; // Set the new table as __index
                }
                _ => {
                    methods_index = Some(ffi::lua_absindex(state, -1));
                    extra_tables_count += 1;
                }
            }
        }

        #[cfg(feature = "luau")]
        let extra_init = None;
        #[cfg(not(feature = "luau"))]
        let extra_init: Option<fn(*mut ffi::lua_State) -> Result<()>> = Some(|state| {
            ffi::lua_pushcfunction(state, util::userdata_destructor::<UserDataCell<T>>);
            rawset_field(state, -2, "__gc")
        });

        init_userdata_metatable(
            state,
            metatable_index,
            field_getters_index,
            field_setters_index,
            methods_index,
            extra_init,
        )?;

        // Pop extra tables to get metatable on top of the stack
        ffi::lua_pop(state, extra_tables_count);

        let mt_ptr = ffi::lua_topointer(state, -1);
        let id = protect_lua!(state, 1, 0, |state| {
            ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX)
        })?;

        let type_id = TypeId::of::<T>();
        (*self.extra.get()).registered_userdata.insert(type_id, id);
        (*self.extra.get())
            .registered_userdata_mt
            .insert(mt_ptr, Some(type_id));

        Ok(id as Integer)
    }

    #[inline]
    pub(crate) unsafe fn register_raw_userdata_metatable(
        &self,
        ptr: *const c_void,
        type_id: Option<TypeId>,
    ) {
        (*self.extra.get())
            .registered_userdata_mt
            .insert(ptr, type_id);
    }

    #[inline]
    pub(crate) unsafe fn deregister_raw_userdata_metatable(&self, ptr: *const c_void) {
        (*self.extra.get()).registered_userdata_mt.remove(&ptr);
        if (*self.extra.get()).last_checked_userdata_mt.0 == ptr {
            (*self.extra.get()).last_checked_userdata_mt = (ptr::null(), None);
        }
    }

    // Returns `TypeId` for the `lref` userdata, checking that it's registered and not destructed.
    //
    // Returns `None` if the userdata is registered but non-static.
    pub(crate) unsafe fn get_userdata_ref_type_id(&self, lref: &LuaRef) -> Result<Option<TypeId>> {
        self.get_userdata_type_id_inner(self.ref_thread(), lref.index)
    }

    // Same as `get_userdata_ref_type_id` but assumes the userdata is already on the stack.
    pub(crate) unsafe fn get_userdata_type_id(&self, idx: c_int) -> Result<Option<TypeId>> {
        self.get_userdata_type_id_inner(self.state(), idx)
    }

    unsafe fn get_userdata_type_id_inner(
        &self,
        state: *mut ffi::lua_State,
        idx: c_int,
    ) -> Result<Option<TypeId>> {
        if ffi::lua_getmetatable(state, idx) == 0 {
            return Err(Error::UserDataTypeMismatch);
        }
        let mt_ptr = ffi::lua_topointer(state, -1);
        ffi::lua_pop(state, 1);

        // Fast path to skip looking up the metatable in the map
        let (last_mt, last_type_id) = (*self.extra.get()).last_checked_userdata_mt;
        if last_mt == mt_ptr {
            return Ok(last_type_id);
        }

        match (*self.extra.get()).registered_userdata_mt.get(&mt_ptr) {
            Some(&type_id) if type_id == Some(TypeId::of::<DestructedUserdata>()) => {
                Err(Error::UserDataDestructed)
            }
            Some(&type_id) => {
                (*self.extra.get()).last_checked_userdata_mt = (mt_ptr, type_id);
                Ok(type_id)
            }
            None => Err(Error::UserDataTypeMismatch),
        }
    }

    // Pushes a LuaRef (userdata) value onto the stack, returning their `TypeId`.
    // Uses 1 stack space, does not call checkstack.
    pub(crate) unsafe fn push_userdata_ref(&self, lref: &LuaRef) -> Result<Option<TypeId>> {
        let type_id = self.get_userdata_type_id_inner(self.ref_thread(), lref.index)?;
        self.push_ref(lref);
        Ok(type_id)
    }

    // Creates a Function out of a Callback containing a 'static Fn. This is safe ONLY because the
    // Fn is 'static, otherwise it could capture 'lua arguments improperly. Without ATCs, we
    // cannot easily deal with the "correct" callback type of:
    //
    // Box<for<'lua> Fn(&'lua Lua, MultiValue<'lua>) -> Result<MultiValue<'lua>>)>
    //
    // So we instead use a caller provided lifetime, which without the 'static requirement would be
    // unsafe.
    pub(crate) fn create_callback<'lua>(
        &'lua self,
        func: Callback<'lua, 'static>,
    ) -> Result<Function<'lua>> {
        unsafe extern "C-unwind" fn call_callback(state: *mut ffi::lua_State) -> c_int {
            // Normal functions can be scoped and therefore destroyed,
            // so we need to check that the first upvalue is valid
            let (upvalue, extra) = match ffi::lua_type(state, ffi::lua_upvalueindex(1)) {
                ffi::LUA_TUSERDATA => {
                    let upvalue = get_userdata::<CallbackUpvalue>(state, ffi::lua_upvalueindex(1));
                    (upvalue, (*upvalue).extra.get())
                }
                _ => (ptr::null_mut(), ptr::null_mut()),
            };
            callback_error_ext(state, extra, |nargs| {
                // Lua ensures that `LUA_MINSTACK` stack spaces are available (after pushing arguments)
                if upvalue.is_null() {
                    return Err(Error::CallbackDestructed);
                }

                let lua: &Lua = mem::transmute((*extra).inner.assume_init_ref());
                let _guard = StateGuard::new(&lua.0, state);
                let func = &*(*upvalue).data;

                func(lua, nargs)
            })
        }

        let state = self.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 4)?;

            let func = mem::transmute(func);
            let extra = Arc::clone(&self.extra);
            let protect = !self.unlikely_memory_error();
            push_gc_userdata(state, CallbackUpvalue { data: func, extra }, protect)?;
            if protect {
                protect_lua!(state, 1, 1, fn(state) {
                    ffi::lua_pushcclosure(state, call_callback, 1);
                })?;
            } else {
                ffi::lua_pushcclosure(state, call_callback, 1);
            }

            Ok(Function(self.pop_ref()))
        }
    }

    #[cfg(feature = "async")]
    pub(crate) fn create_async_callback<'lua>(
        &'lua self,
        func: AsyncCallback<'lua, 'static>,
    ) -> Result<Function<'lua>> {
        #[cfg(any(
            feature = "lua54",
            feature = "lua53",
            feature = "lua52",
            feature = "luau"
        ))]
        unsafe {
            if !(*self.extra.get()).libs.contains(StdLib::COROUTINE) {
                load_from_std_lib(self.main_state, StdLib::COROUTINE)?;
                (*self.extra.get()).libs |= StdLib::COROUTINE;
            }
        }

        unsafe extern "C-unwind" fn call_callback(state: *mut ffi::lua_State) -> c_int {
            // Async functions cannot be scoped and therefore destroyed,
            // so the first upvalue is always valid
            let upvalue = get_userdata::<AsyncCallbackUpvalue>(state, ffi::lua_upvalueindex(1));
            let extra = (*upvalue).extra.get();
            callback_error_ext(state, extra, |nargs| {
                // Lua ensures that `LUA_MINSTACK` stack spaces are available (after pushing arguments)
                let lua: &Lua = mem::transmute((*extra).inner.assume_init_ref());
                let _guard = StateGuard::new(&lua.0, state);

                let args = MultiValue::from_stack_multi(nargs, lua)?;
                let func = &*(*upvalue).data;
                let fut = func(lua, args);
                let extra = Arc::clone(&(*upvalue).extra);
                let protect = !lua.unlikely_memory_error();
                push_gc_userdata(state, AsyncPollUpvalue { data: fut, extra }, protect)?;
                if protect {
                    protect_lua!(state, 1, 1, fn(state) {
                        ffi::lua_pushcclosure(state, poll_future, 1);
                    })?;
                } else {
                    ffi::lua_pushcclosure(state, poll_future, 1);
                }

                Ok(1)
            })
        }

        unsafe extern "C-unwind" fn poll_future(state: *mut ffi::lua_State) -> c_int {
            let upvalue = get_userdata::<AsyncPollUpvalue>(state, ffi::lua_upvalueindex(1));
            let extra = (*upvalue).extra.get();
            callback_error_ext(state, extra, |_| {
                // Lua ensures that `LUA_MINSTACK` stack spaces are available (after pushing arguments)
                let lua: &Lua = mem::transmute((*extra).inner.assume_init_ref());
                let _guard = StateGuard::new(&lua.0, state);

                let fut = &mut (*upvalue).data;
                let mut ctx = Context::from_waker(lua.waker());
                match fut.as_mut().poll(&mut ctx) {
                    Poll::Pending => Ok(0),
                    Poll::Ready(nresults) => {
                        let nresults = nresults?;
                        match nresults {
                            0..=2 => {
                                // Fast path for up to 2 results without creating a table
                                ffi::lua_pushinteger(state, nresults as _);
                                if nresults > 0 {
                                    ffi::lua_insert(state, -nresults - 1);
                                }
                                Ok(nresults + 1)
                            }
                            _ => {
                                let results = MultiValue::from_stack_multi(nresults, lua)?;
                                ffi::lua_pushinteger(state, nresults as _);
                                lua.push_value(Value::Table(lua.create_sequence_from(results)?))?;
                                Ok(2)
                            }
                        }
                    }
                }
            })
        }

        let state = self.state();
        let get_poll = unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 4)?;

            let func = mem::transmute(func);
            let extra = Arc::clone(&self.extra);
            let protect = !self.unlikely_memory_error();
            let upvalue = AsyncCallbackUpvalue { data: func, extra };
            push_gc_userdata(state, upvalue, protect)?;
            if protect {
                protect_lua!(state, 1, 1, fn(state) {
                    ffi::lua_pushcclosure(state, call_callback, 1);
                })?;
            } else {
                ffi::lua_pushcclosure(state, call_callback, 1);
            }

            Function(self.pop_ref())
        };

        unsafe extern "C-unwind" fn unpack(state: *mut ffi::lua_State) -> c_int {
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

        self.load(
            r#"
            local poll = get_poll(...)
            local pending, yield, unpack = pending, yield, unpack
            while true do
                local nres, res, res2 = poll()
                if nres ~= nil then
                    if nres == 0 then
                        return
                    elseif nres == 1 then
                        return res
                    elseif nres == 2 then
                        return res, res2
                    else
                        return unpack(res, nres)
                    end
                end
                yield(pending)
            end
            "#,
        )
        .try_cache()
        .set_name("__mlua_async_poll")
        .set_environment(env)
        .into_function()
    }

    #[cfg(feature = "async")]
    #[inline]
    pub(crate) unsafe fn waker(&self) -> &Waker {
        (*self.extra.get()).waker.as_ref()
    }

    #[cfg(feature = "async")]
    #[inline]
    pub(crate) unsafe fn set_waker(&self, waker: NonNull<Waker>) -> NonNull<Waker> {
        mem::replace(&mut (*self.extra.get()).waker, waker)
    }

    pub(crate) unsafe fn make_userdata<T>(&self, data: UserDataCell<T>) -> Result<AnyUserData>
    where
        T: UserData + 'static,
    {
        self.make_userdata_with_metatable(data, || {
            // Check if userdata/metatable is already registered
            let type_id = TypeId::of::<T>();
            if let Some(&table_id) = (*self.extra.get()).registered_userdata.get(&type_id) {
                return Ok(table_id as Integer);
            }

            // Create new metatable from UserData definition
            let mut registry = UserDataRegistry::new();
            T::add_fields(&mut registry);
            T::add_methods(&mut registry);

            self.register_userdata_metatable(registry)
        })
    }

    pub(crate) unsafe fn make_any_userdata<T>(&self, data: UserDataCell<T>) -> Result<AnyUserData>
    where
        T: 'static,
    {
        self.make_userdata_with_metatable(data, || {
            // Check if userdata/metatable is already registered
            let type_id = TypeId::of::<T>();
            if let Some(&table_id) = (*self.extra.get()).registered_userdata.get(&type_id) {
                return Ok(table_id as Integer);
            }

            // Create empty metatable
            let registry = UserDataRegistry::new();
            self.register_userdata_metatable::<T>(registry)
        })
    }

    unsafe fn make_userdata_with_metatable<T>(
        &self,
        data: UserDataCell<T>,
        get_metatable_id: impl FnOnce() -> Result<Integer>,
    ) -> Result<AnyUserData> {
        let state = self.state();
        let _sg = StackGuard::new(state);
        check_stack(state, 3)?;

        // We push metatable first to ensure having correct metatable with `__gc` method
        ffi::lua_pushnil(state);
        ffi::lua_rawgeti(state, ffi::LUA_REGISTRYINDEX, get_metatable_id()?);
        let protect = !self.unlikely_memory_error();
        #[cfg(not(feature = "lua54"))]
        push_userdata(state, data, protect)?;
        #[cfg(feature = "lua54")]
        push_userdata_uv(state, data, USER_VALUE_MAXSLOT as c_int, protect)?;
        ffi::lua_replace(state, -3);
        ffi::lua_setmetatable(state, -2);

        // Set empty environment for Lua 5.1
        #[cfg(any(feature = "lua51", feature = "luajit"))]
        if protect {
            protect_lua!(state, 1, 1, fn(state) {
                ffi::lua_newtable(state);
                ffi::lua_setuservalue(state, -2);
            })?;
        } else {
            ffi::lua_newtable(state);
            ffi::lua_setuservalue(state, -2);
        }

        Ok(AnyUserData(self.pop_ref()))
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

    pub(crate) unsafe fn try_from_ptr(state: *mut ffi::lua_State) -> Option<Self> {
        let extra = extra_data(state);
        if extra.is_null() {
            return None;
        }
        Some(Lua(Arc::clone((*extra).inner.assume_init_ref())))
    }

    #[inline]
    pub(crate) unsafe fn unlikely_memory_error(&self) -> bool {
        // MemoryInfo is empty in module mode so we cannot predict memory limits
        (*self.extra.get())
            .mem_state
            .map(|x| x.as_ref().memory_limit() == 0)
            .unwrap_or_else(|| {
                // Alternatively, check the special flag (only for module mode)
                #[cfg(feature = "module")]
                return (*self.extra.get()).skip_memory_check;
                #[cfg(not(feature = "module"))]
                return false;
            })
    }

    #[cfg(feature = "unstable")]
    #[inline]
    pub(crate) fn clone(&self) -> Arc<LuaInner> {
        Arc::clone(&self.0)
    }
}

impl LuaInner {
    #[inline(always)]
    pub(crate) fn state(&self) -> *mut ffi::lua_State {
        self.state.load(Ordering::Relaxed)
    }

    #[cfg(feature = "luau")]
    #[inline(always)]
    pub(crate) fn main_state(&self) -> *mut ffi::lua_State {
        self.main_state
    }

    #[inline(always)]
    pub(crate) fn ref_thread(&self) -> *mut ffi::lua_State {
        unsafe { (*self.extra.get()).ref_thread }
    }

    #[inline]
    pub(crate) fn pop_multivalue_from_pool(&self) -> Option<Vec<Value>> {
        let extra = unsafe { &mut *self.extra.get() };
        extra.multivalue_pool.pop()
    }

    #[inline]
    pub(crate) fn push_multivalue_to_pool(&self, mut multivalue: Vec<Value>) {
        let extra = unsafe { &mut *self.extra.get() };
        if extra.multivalue_pool.len() < MULTIVALUE_POOL_SIZE {
            multivalue.clear();
            extra
                .multivalue_pool
                .push(unsafe { mem::transmute(multivalue) });
        }
    }
}

impl ExtraData {
    #[cfg(feature = "luau")]
    #[inline]
    pub(crate) fn mem_state(&self) -> NonNull<MemoryState> {
        self.mem_state.unwrap()
    }
}

struct StateGuard<'a>(&'a LuaInner, *mut ffi::lua_State);

impl<'a> StateGuard<'a> {
    fn new(inner: &'a LuaInner, mut state: *mut ffi::lua_State) -> Self {
        #[cfg(any(feature = "std", target_has_atomic = "ptr"))]
        {
            state = inner.state.swap(state, Ordering::Relaxed);
        }
        #[cfg(not(any(feature = "std", target_has_atomic = "ptr")))]
        {
            // On platforms without atomic loads, just manually swap
            let inner_state = inner.state.load(Ordering::Relaxed);
            inner.state.store(state, Ordering::Relaxed);
            state = inner_state;
        }
        Self(inner, state)
    }
}

impl<'a> Drop for StateGuard<'a> {
    fn drop(&mut self) {
        self.0.state.store(self.1, Ordering::Relaxed);
    }
}

#[cfg(feature = "luau")]
unsafe fn extra_data(state: *mut ffi::lua_State) -> *mut ExtraData {
    (*ffi::lua_callbacks(state)).userdata as *mut ExtraData
}

#[cfg(not(feature = "luau"))]
unsafe fn extra_data(state: *mut ffi::lua_State) -> *mut ExtraData {
    let extra_key = &EXTRA_REGISTRY_KEY as *const u8 as *const c_void;
    if ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, extra_key) != ffi::LUA_TUSERDATA {
        // `ExtraData` can be null only when Lua state is foreign.
        // This case in used in `Lua::try_from_ptr()`.
        ffi::lua_pop(state, 1);
        return ptr::null_mut();
    }
    let extra_ptr = ffi::lua_touserdata(state, -1) as *mut Arc<UnsafeCell<ExtraData>>;
    ffi::lua_pop(state, 1);
    (*extra_ptr).get()
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
// and instead reuses unsed values from previous calls (or allocates new).
unsafe fn callback_error_ext<F, R>(state: *mut ffi::lua_State, mut extra: *mut ExtraData, f: F) -> R
where
    F: FnOnce(c_int) -> Result<R>,
{
    if extra.is_null() {
        extra = extra_data(state);
    }

    let nargs = ffi::lua_gettop(state);

    enum PreallocatedFailure {
        New(*mut WrappedFailure),
        Existing(i32),
    }

    impl PreallocatedFailure {
        unsafe fn reserve(state: *mut ffi::lua_State, extra: *mut ExtraData) -> Self {
            match (*extra).wrapped_failure_pool.pop() {
                Some(index) => PreallocatedFailure::Existing(index),
                None => {
                    // We need to check stack for Luau in case when callback is called from interrupt
                    // See https://github.com/Roblox/luau/issues/446 and mlua #142 and #153
                    #[cfg(feature = "luau")]
                    ffi::lua_rawcheckstack(state, 2);
                    // Place it to the beginning of the stack
                    let ud = WrappedFailure::new_userdata(state);
                    ffi::lua_insert(state, 1);
                    PreallocatedFailure::New(ud)
                }
            }
        }

        unsafe fn r#use(
            &self,
            state: *mut ffi::lua_State,
            extra: *mut ExtraData,
        ) -> *mut WrappedFailure {
            let ref_thread = (*extra).ref_thread;
            match *self {
                PreallocatedFailure::New(ud) => {
                    ffi::lua_settop(state, 1);
                    ud
                }
                PreallocatedFailure::Existing(index) => {
                    ffi::lua_settop(state, 0);
                    #[cfg(feature = "luau")]
                    ffi::lua_rawcheckstack(state, 2);
                    ffi::lua_pushvalue(ref_thread, index);
                    ffi::lua_xmove(ref_thread, state, 1);
                    ffi::lua_pushnil(ref_thread);
                    ffi::lua_replace(ref_thread, index);
                    (*extra).ref_free.push(index);
                    ffi::lua_touserdata(state, -1) as *mut WrappedFailure
                }
            }
        }

        unsafe fn release(self, state: *mut ffi::lua_State, extra: *mut ExtraData) {
            let ref_thread = (*extra).ref_thread;
            match self {
                PreallocatedFailure::New(_) => {
                    if (*extra).wrapped_failure_pool.len() < WRAPPED_FAILURE_POOL_SIZE {
                        ffi::lua_rotate(state, 1, -1);
                        ffi::lua_xmove(state, ref_thread, 1);
                        let index = ref_stack_pop(extra);
                        (*extra).wrapped_failure_pool.push(index);
                    } else {
                        ffi::lua_remove(state, 1);
                    }
                }
                PreallocatedFailure::Existing(index) => {
                    if (*extra).wrapped_failure_pool.len() < WRAPPED_FAILURE_POOL_SIZE {
                        (*extra).wrapped_failure_pool.push(index);
                    } else {
                        ffi::lua_pushnil(ref_thread);
                        ffi::lua_replace(ref_thread, index);
                        (*extra).ref_free.push(index);
                    }
                }
            }
        }
    }

    // We cannot shadow Rust errors with Lua ones, so we need to reserve pre-allocated memory
    // to store a wrapped failure (error or panic) *before* we proceed.
    let prealloc_failure = PreallocatedFailure::reserve(state, extra);

    #[cfg(not(feature = "panic-safety"))]
    fn catch_unwind<F: FnOnce() -> R, R>(f: F) -> Result<R> {
        Ok(f())
    }

    match catch_unwind(AssertUnwindSafe(|| f(nargs))) {
        Ok(Ok(r)) => {
            // Return unused `WrappedFailure` to the pool
            prealloc_failure.release(state, extra);
            r
        }
        Ok(Err(err)) => {
            let wrapped_error = prealloc_failure.r#use(state, extra);

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
        #[cfg(not(feature = "panic-safety"))]
        Err(p) => unreachable!("panic = abort, but encountered {:?}", p),
        #[cfg(feature = "panic-safety")]
        Err(p) => {
            let wrapped_panic = prealloc_failure.r#use(state, extra);
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
    pub unsafe fn requiref(
        state: *mut ffi::lua_State,
        modname: &str,
        openf: ffi::lua_CFunction,
        glb: c_int,
    ) -> Result<()> {
        let modname = mlua_expect!(CString::new(modname), "modname contains nil byte");
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

unsafe fn ref_stack_pop(extra: *mut ExtraData) -> c_int {
    let extra = &mut *extra;
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
                "cannot create a Lua reference, out of auxiliary stack space (used {top} slots)"
            );
        }
        extra.ref_stack_size += inc;
    }
    extra.ref_stack_top += 1;
    extra.ref_stack_top
}

#[cfg(test)]
mod assertions {
    use super::*;

    // Lua has lots of interior mutability, should not be RefUnwindSafe
    static_assertions::assert_not_impl_any!(Lua: std::panic::RefUnwindSafe);

    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_any!(Lua: Send);
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(Lua: Send);
}
