use std::any::TypeId;
use std::cell::{RefCell, UnsafeCell};
use std::collections::HashMap;
use std::ffi::CString;
use std::marker::PhantomData;
use std::os::raw::{c_char, c_int, c_void};
use std::sync::{Arc, Mutex, Weak};
use std::{mem, ptr, str};

use crate::error::{Error, Result};
use crate::ffi;
use crate::function::Function;
use crate::hook::{hook_proc, Debug, HookTriggers};
use crate::scope::Scope;
use crate::stdlib::StdLib;
use crate::string::String;
use crate::table::Table;
use crate::thread::Thread;
use crate::types::{
    Callback, HookCallback, Integer, LightUserData, LuaRef, MaybeSend, Number, RegistryKey,
    UserDataCell,
};
use crate::userdata::{AnyUserData, MetaMethod, UserData, UserDataMethods, UserDataWrapped};
use crate::util::{
    assert_stack, callback_error, check_stack, get_gc_userdata, get_main_state, get_userdata,
    get_wrapped_error, init_error_registry, init_gc_metatable_for, init_userdata_metatable,
    pop_error, protect_lua, protect_lua_closure, push_gc_userdata, push_meta_gc_userdata,
    push_string, push_userdata, push_wrapped_error, StackGuard,
};
use crate::value::{FromLua, FromLuaMulti, MultiValue, Nil, ToLua, ToLuaMulti, Value};

#[cfg(feature = "async")]
use {
    crate::types::AsyncCallback,
    futures_core::{
        future::{Future, LocalBoxFuture},
        task::{Context, Poll, Waker},
    },
    futures_task::noop_waker,
    futures_util::future::{self, TryFutureExt},
};

#[cfg(feature = "serialize")]
use {crate::util::get_destructed_userdata_metatable, serde::Serialize};

/// Top level Lua struct which holds the Lua state itself.
pub struct Lua {
    pub(crate) state: *mut ffi::lua_State,
    main_state: Option<*mut ffi::lua_State>,
    extra: Arc<Mutex<ExtraData>>,
    ephemeral: bool,
    safe: bool,
    // Lua has lots of interior mutability, should not be RefUnwindSafe
    _no_ref_unwind_safe: PhantomData<UnsafeCell<()>>,
}

// Data associated with the lua_State.
struct ExtraData {
    registered_userdata: HashMap<TypeId, c_int>,
    registry_unref_list: Arc<Mutex<Option<Vec<c_int>>>>,

    libs: StdLib,
    mem_info: *mut MemoryInfo,

    ref_thread: *mut ffi::lua_State,
    ref_stack_size: c_int,
    ref_stack_max: c_int,
    ref_free: Vec<c_int>,

    hook_callback: Option<HookCallback>,
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
/// More information can be found in the Lua 5.x [documentation][lua_doc].
///
/// [lua_doc]: https://www.lua.org/manual/5.4/manual.html#2.5
pub enum GCMode {
    Incremental,
    /// Requires `feature = "lua54"`
    #[cfg(any(feature = "lua54", doc))]
    Generational,
}

#[cfg(feature = "async")]
pub(crate) struct AsyncPollPending;
#[cfg(feature = "async")]
pub(crate) static WAKER_REGISTRY_KEY: u8 = 0;
pub(crate) static EXTRA_REGISTRY_KEY: u8 = 0;

/// Requires `feature = "send"`
#[cfg(feature = "send")]
#[cfg_attr(docsrs, doc(cfg(feature = "send")))]
unsafe impl Send for Lua {}

impl Drop for Lua {
    fn drop(&mut self) {
        unsafe {
            if !self.ephemeral {
                let extra = mlua_expect!(self.extra.lock(), "extra is poisoned");
                mlua_debug_assert!(
                    ffi::lua_gettop(extra.ref_thread) == extra.ref_stack_max
                        && extra.ref_stack_max as usize == extra.ref_free.len(),
                    "reference leak detected"
                );
                let mut unref_list =
                    mlua_expect!(extra.registry_unref_list.lock(), "unref list poisoned");
                *unref_list = None;
                ffi::lua_close(self.main_state.expect("main_state is null"));
                if !extra.mem_info.is_null() {
                    Box::from_raw(extra.mem_info);
                }
            }
        }
    }
}

impl Lua {
    /// Creates a new Lua state and loads the safe subset of the standard libraries.
    ///
    /// # Safety
    /// The created Lua state would have _some_ safety guarantees and would not allow to load unsafe
    /// standard libraries or C modules.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Lua {
        mlua_expect!(
            Self::new_with(StdLib::ALL_SAFE),
            "can't create new safe Lua state"
        )
    }

    /// Creates a new Lua state and loads all the standard libraries.
    ///
    /// # Safety
    /// The created Lua state would not have safety guarantees and would allow to load C modules.
    pub unsafe fn unsafe_new() -> Lua {
        Self::unsafe_new_with(StdLib::ALL)
    }

    /// Creates a new Lua state and loads the specified safe subset of the standard libraries.
    ///
    /// Use the [`StdLib`] flags to specifiy the libraries you want to load.
    ///
    /// # Safety
    /// The created Lua state would have _some_ safety guarantees and would not allow to load unsafe
    /// standard libraries or C modules.
    ///
    /// [`StdLib`]: struct.StdLib.html
    pub fn new_with(libs: StdLib) -> Result<Lua> {
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

        let mut lua = unsafe { Self::unsafe_new_with(libs) };

        if libs.contains(StdLib::PACKAGE) {
            mlua_expect!(lua.disable_c_modules(), "Error during disabling C modules");
        }
        lua.safe = true;

        Ok(lua)
    }

    /// Creates a new Lua state and loads the specified subset of the standard libraries.
    ///
    /// Use the [`StdLib`] flags to specifiy the libraries you want to load.
    ///
    /// # Safety
    /// The created Lua state would not have safety guarantees and would allow to load C modules.
    ///
    /// [`StdLib`]: struct.StdLib.html
    pub unsafe fn unsafe_new_with(libs: StdLib) -> Lua {
        #[cfg_attr(any(feature = "lua51", feature = "luajit"), allow(dead_code))]
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
                // Should not happend
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
        #[cfg(any(feature = "lua51", feature = "luajit"))]
        let state = ffi::luaL_newstate();

        ffi::luaL_requiref(state, cstr!("_G"), ffi::luaopen_base, 1);
        ffi::lua_pop(state, 1);

        let mut lua = Lua::init_from_ptr(state);
        lua.ephemeral = false;
        #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
        {
            mlua_expect!(lua.extra.lock(), "extra is poisoned").mem_info = mem_info;
        }

        mlua_expect!(
            protect_lua_closure(lua.main_state.expect("main_state is null"), 0, 0, |state| {
                load_from_std_lib(state, libs);
            }),
            "Error during loading standard libraries"
        );
        mlua_expect!(lua.extra.lock(), "extra is poisoned").libs |= libs;

        lua
    }

    /// Constructs a new Lua instance from an existing raw state.
    #[allow(clippy::missing_safety_doc)]
    pub unsafe fn init_from_ptr(state: *mut ffi::lua_State) -> Lua {
        let maybe_main_state = get_main_state(state);
        let main_state = maybe_main_state.unwrap_or(state);
        let main_state_top = ffi::lua_gettop(main_state);

        let ref_thread = mlua_expect!(
            protect_lua_closure(main_state, 0, 0, |state| {
                init_error_registry(state);

                // Create the internal metatables and place them in the registry
                // to prevent them from being garbage collected.

                init_gc_metatable_for::<Callback>(state, None);
                init_gc_metatable_for::<Lua>(state, None);
                init_gc_metatable_for::<Weak<Mutex<ExtraData>>>(state, None);
                #[cfg(feature = "async")]
                {
                    init_gc_metatable_for::<AsyncCallback>(state, None);
                    init_gc_metatable_for::<LocalBoxFuture<Result<MultiValue>>>(state, None);
                    init_gc_metatable_for::<AsyncPollPending>(state, None);
                    init_gc_metatable_for::<Waker>(state, None);
                }

                // Init serde metatables
                #[cfg(feature = "serialize")]
                crate::serde::init_metatables(state);

                // Create ref stack thread and place it in the registry to prevent it from being garbage
                // collected.

                let _ref_thread = ffi::lua_newthread(state);
                ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX);
                _ref_thread
            }),
            "Error during Lua construction",
        );

        // Create ExtraData

        let extra = Arc::new(Mutex::new(ExtraData {
            registered_userdata: HashMap::new(),
            registry_unref_list: Arc::new(Mutex::new(Some(Vec::new()))),
            ref_thread,
            libs: StdLib::NONE,
            mem_info: ptr::null_mut(),
            // We need 1 extra stack space to move values in and out of the ref stack.
            ref_stack_size: ffi::LUA_MINSTACK - 1,
            ref_stack_max: 0,
            ref_free: Vec::new(),
            hook_callback: None,
        }));

        mlua_expect!(
            push_gc_userdata(main_state, Arc::downgrade(&extra)),
            "Error while storing extra data",
        );
        mlua_expect!(
            protect_lua_closure(main_state, 1, 0, |state| {
                ffi::lua_rawsetp(
                    state,
                    ffi::LUA_REGISTRYINDEX,
                    &EXTRA_REGISTRY_KEY as *const u8 as *mut c_void,
                );
            }),
            "Error while storing extra data"
        );

        mlua_debug_assert!(
            ffi::lua_gettop(main_state) == main_state_top,
            "stack leak during creation"
        );
        assert_stack(main_state, ffi::LUA_MINSTACK);

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
    /// Use the [`StdLib`] flags to specifiy the libraries you want to load.
    ///
    /// [`StdLib`]: struct.StdLib.html
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
        let res = unsafe {
            protect_lua_closure(state, 0, 0, |state| {
                load_from_std_lib(state, libs);
            })
        };

        // If `package` library loaded into a safe lua state then disable C modules
        let curr_libs = mlua_expect!(self.extra.lock(), "extra is poisoned").libs;
        if self.safe && (curr_libs ^ (curr_libs | libs)).contains(StdLib::PACKAGE) {
            mlua_expect!(self.disable_c_modules(), "Error during disabling C modules");
        }
        mlua_expect!(self.extra.lock(), "extra is poisoned").libs |= libs;

        res
    }

    /// Consumes and leaks `Lua` object, returning a static reference `&'static Lua`.
    ///
    /// This function is useful when the `Lua` object is supposed to live for the remainder
    /// of the program's life.
    /// In particular in asynchronous context this will allow to spawn Lua tasks to execute
    /// in background.
    ///
    /// Dropping the returned reference will cause a memory leak. If this is not acceptable,
    /// the reference should first be wrapped with the [`Lua::from_static`] function producing a `Lua`.
    /// This `Lua` object can then be dropped which will properly release the allocated memory.
    ///
    /// [`Lua::from_static`]: #method.from_static
    pub fn into_static(self) -> &'static Self {
        Box::leak(Box::new(self))
    }

    /// Constructs a `Lua` from a static reference to it.
    ///
    /// # Safety
    /// This function is unsafe because improper use may lead to memory problems or undefined behavior.
    pub unsafe fn from_static(lua: &'static Lua) -> Self {
        *Box::from_raw(lua as *const Lua as *mut Lua)
    }

    // Executes module entrypoint function, which returns only one Value.
    // The returned value then pushed to the Lua stack.
    #[doc(hidden)]
    pub fn entrypoint1<'lua, 'callback, R, F>(&'lua self, func: F) -> Result<c_int>
    where
        'lua: 'callback,
        R: ToLua<'callback>,
        F: 'static + MaybeSend + Fn(&'callback Lua) -> Result<R>,
    {
        let cb = self.create_callback(Box::new(move |lua, _| func(lua)?.to_lua_multi(lua)))?;
        unsafe { self.push_value(cb.call(())?).map(|_| 1) }
    }

    /// Sets a 'hook' function that will periodically be called as Lua code executes.
    ///
    /// When exactly the hook function is called depends on the contents of the `triggers`
    /// parameter, see [`HookTriggers`] for more details.
    ///
    /// The provided hook function can error, and this error will be propagated through the Lua code
    /// that was executing at the time the hook was triggered.  This can be used to implement a
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
    /// lua.set_hook(HookTriggers {
    ///     every_line: true, ..Default::default()
    /// }, |_lua, debug| {
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
    /// [`HookTriggers`]: struct.HookTriggers.html
    /// [`HookTriggers.every_nth_instruction`]: struct.HookTriggers.html#field.every_nth_instruction
    pub fn set_hook<F>(&self, triggers: HookTriggers, callback: F) -> Result<()>
    where
        F: 'static + MaybeSend + FnMut(&Lua, Debug) -> Result<()>,
    {
        let state = self.main_state.ok_or(Error::MainThreadNotAvailable)?;
        unsafe {
            let mut extra = mlua_expect!(self.extra.lock(), "extra is poisoned");
            extra.hook_callback = Some(Arc::new(RefCell::new(callback)));
            ffi::lua_sethook(state, Some(hook_proc), triggers.mask(), triggers.count());
        }
        Ok(())
    }

    /// Remove any hook previously set by `set_hook`. This function has no effect if a hook was not
    /// previously set.
    pub fn remove_hook(&self) {
        // If main_state is not available, then sethook wasn't called.
        let state = match self.main_state {
            Some(state) => state,
            None => return,
        };
        let mut extra = mlua_expect!(self.extra.lock(), "extra is poisoned");
        unsafe {
            extra.hook_callback = None;
            ffi::lua_sethook(state, None, 0, 0);
        }
    }

    /// Returns the amount of memory (in bytes) currently used inside this Lua state.
    pub fn used_memory(&self) -> usize {
        let extra = mlua_expect!(self.extra.lock(), "extra is poisoned");
        let state = self.main_state.unwrap_or(self.state);
        if extra.mem_info.is_null() {
            // Get data from the Lua GC
            unsafe {
                let used_kbytes = ffi::lua_gc(state, ffi::LUA_GCCOUNT, 0);
                let used_kbytes_rem = ffi::lua_gc(state, ffi::LUA_GCCOUNTB, 0);
                return (used_kbytes as usize) * 1024 + (used_kbytes_rem as usize);
            }
        }
        unsafe { (*extra.mem_info).used_memory as usize }
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
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", doc))]
    pub fn set_memory_limit(&self, memory_limit: usize) -> Result<usize> {
        let mut extra = mlua_expect!(self.extra.lock(), "extra is poisoned");
        if extra.mem_info.is_null() {
            return Err(Error::MemoryLimitNotAvailable);
        }
        unsafe {
            let prev_limit = (*extra.mem_info).memory_limit as usize;
            (*extra.mem_info).memory_limit = memory_limit as isize;
            Ok(prev_limit)
        }
    }

    /// Returns true if the garbage collector is currently running automatically.
    ///
    /// Requires `feature = "lua54/lua53/lua52"`
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", doc))]
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
    /// objects.  Once to finish the current gc cycle, and once to start and finish the next cycle.
    pub fn gc_collect(&self) -> Result<()> {
        let state = self.main_state.unwrap_or(self.state);
        unsafe {
            protect_lua_closure(state, 0, 0, |state| {
                ffi::lua_gc(state, ffi::LUA_GCCOLLECT, 0);
            })
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
    /// if `kbytes` is 0, then this is the same as calling `gc_step`.  Returns true if this step has
    /// finished a collection cycle.
    pub fn gc_step_kbytes(&self, kbytes: c_int) -> Result<bool> {
        let state = self.main_state.unwrap_or(self.state);
        unsafe {
            protect_lua_closure(state, 0, 0, |state| {
                ffi::lua_gc(state, ffi::LUA_GCSTEP, kbytes) != 0
            })
        }
    }

    /// Sets the 'pause' value of the collector.
    ///
    /// Returns the previous value of 'pause'.  More information can be found in the [Lua 5.3
    /// documentation][lua_doc].
    ///
    /// [lua_doc]: https://www.lua.org/manual/5.3/manual.html#2.5
    pub fn gc_set_pause(&self, pause: c_int) -> c_int {
        let state = self.main_state.unwrap_or(self.state);
        unsafe { ffi::lua_gc(state, ffi::LUA_GCSETPAUSE, pause) }
    }

    /// Sets the 'step multiplier' value of the collector.
    ///
    /// Returns the previous value of the 'step multiplier'.  More information can be found in the
    /// Lua 5.x [documentation][lua_doc].
    ///
    /// [lua_doc]: https://www.lua.org/manual/5.3/manual.html#2.5
    pub fn gc_set_step_multiplier(&self, step_multiplier: c_int) -> c_int {
        let state = self.main_state.unwrap_or(self.state);
        unsafe { ffi::lua_gc(state, ffi::LUA_GCSETSTEPMUL, step_multiplier) }
    }

    /// Changes the collector to incremental mode with the given parameters.
    ///
    /// Returns the previous mode (always `GCMode::Incremental` in Lua < 5.4).
    /// More information can be found in the Lua 5.x [documentation][lua_doc].
    ///
    /// [lua_doc]: https://www.lua.org/manual/5.4/manual.html#2.5.1
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
        let prev_mode = unsafe {
            ffi::lua_gc(
                state,
                ffi::LUA_GCSETPAUSE,
                pause,
                step_multiplier,
                step_size,
            )
        };
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
    #[cfg(any(feature = "lua54", doc))]
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
    /// similar on the returned builder.  Code is not even parsed until one of these methods is
    /// called.
    ///
    /// If this `Lua` was created with `unsafe_new`, `load` will automatically detect and load
    /// chunks of either text or binary type, as if passing `bt` mode to `luaL_loadbufferx`.
    ///
    /// [`Chunk::exec`]: struct.Chunk.html#method.exec
    pub fn load<'lua, 'a, S>(&'lua self, source: &'a S) -> Chunk<'lua, 'a>
    where
        S: ?Sized + AsRef<[u8]>,
    {
        Chunk {
            lua: self,
            source: source.as_ref(),
            name: None,
            env: None,
            mode: None,
        }
    }

    fn load_chunk<'lua>(
        &'lua self,
        source: &[u8],
        name: Option<&CString>,
        env: Option<Value<'lua>>,
        mode: Option<ChunkMode>,
    ) -> Result<Function<'lua>> {
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 1);

            let mode_str = match mode {
                Some(ChunkMode::Binary) if self.safe => {
                    return Err(Error::SafetyError(
                        "binary chunks are disabled in safe mode".to_string(),
                    ))
                }
                Some(ChunkMode::Binary) => cstr!("b"),
                Some(ChunkMode::Text) => cstr!("t"),
                None if source.starts_with(ffi::LUA_SIGNATURE) && self.safe => {
                    return Err(Error::SafetyError(
                        "binary chunks are disabled in safe mode".to_string(),
                    ))
                }
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
                        #[cfg(any(feature = "lua51", feature = "luajit"))]
                        ffi::lua_setfenv(self.state, -2);
                    }
                    Ok(Function(self.pop_ref()))
                }
                err => Err(pop_error(self.state, err)),
            }
        }
    }

    /// Create and return an interned Lua string.  Lua strings can be arbitrary [u8] data including
    /// embedded nulls, so in addition to `&str` and `&String`, you can also pass plain `&[u8]`
    /// here.
    pub fn create_string<S>(&self, s: &S) -> Result<String>
    where
        S: ?Sized + AsRef<[u8]>,
    {
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 4);
            push_string(self.state, s)?;
            Ok(String(self.pop_ref()))
        }
    }

    /// Creates and returns a new empty table.
    pub fn create_table(&self) -> Result<Table> {
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 3);
            unsafe extern "C" fn new_table(state: *mut ffi::lua_State) -> c_int {
                ffi::lua_newtable(state);
                1
            }
            protect_lua(self.state, 0, new_table)?;
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
            assert_stack(self.state, 4);
            protect_lua_closure(self.state, 0, 1, |state| {
                ffi::lua_createtable(state, narr, nrec)
            })?;
            Ok(Table(self.pop_ref()))
        }
    }

    /// Creates a table and fills it with values from an iterator.
    pub fn create_table_from<'lua, K, V, I>(&'lua self, cont: I) -> Result<Table<'lua>>
    where
        K: ToLua<'lua>,
        V: ToLua<'lua>,
        I: IntoIterator<Item = (K, V)>,
    {
        unsafe {
            let _sg = StackGuard::new(self.state);
            // `Lua` instance assumes that on any callback, the Lua stack has at least LUA_MINSTACK
            // slots available to avoid panics.
            check_stack(self.state, 5 + ffi::LUA_MINSTACK)?;

            unsafe extern "C" fn new_table(state: *mut ffi::lua_State) -> c_int {
                ffi::lua_newtable(state);
                1
            }
            protect_lua(self.state, 0, new_table)?;

            for (k, v) in cont {
                self.push_value(k.to_lua(self)?)?;
                self.push_value(v.to_lua(self)?)?;
                unsafe extern "C" fn raw_set(state: *mut ffi::lua_State) -> c_int {
                    ffi::lua_rawset(state, -3);
                    1
                }
                protect_lua(self.state, 3, raw_set)?;
            }
            Ok(Table(self.pop_ref()))
        }
    }

    /// Creates a table from an iterator of values, using `1..` as the keys.
    pub fn create_sequence_from<'lua, T, I>(&'lua self, cont: I) -> Result<Table<'lua>>
    where
        T: ToLua<'lua>,
        I: IntoIterator<Item = T>,
    {
        self.create_table_from(cont.into_iter().enumerate().map(|(k, v)| (k + 1, v)))
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
    /// [`ToLua`]: trait.ToLua.html
    /// [`ToLuaMulti`]: trait.ToLuaMulti.html
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
    /// This is a version of [`create_function`] that accepts a FnMut argument.  Refer to
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
            (&mut *func
                .try_borrow_mut()
                .map_err(|_| Error::RecursiveMutCallback)?)(lua, args)
        })
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
    /// [`Thread`]: struct.Thread.html
    /// [`AsyncThread`]: struct.AsyncThread.html
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
            assert_stack(self.state, 2);

            let thread_state =
                protect_lua_closure(self.state, 0, 1, |state| ffi::lua_newthread(state))?;
            self.push_ref(&func.0);
            ffi::lua_xmove(self.state, thread_state, 1);

            Ok(Thread(self.pop_ref()))
        }
    }

    /// Create a Lua userdata object from a custom userdata type.
    pub fn create_userdata<T>(&self, data: T) -> Result<AnyUserData>
    where
        T: 'static + MaybeSend + UserData,
    {
        unsafe { self.make_userdata(UserDataWrapped::new(data)) }
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
        unsafe { self.make_userdata(UserDataWrapped::new_ser(data)) }
    }

    /// Returns a handle to the global environment.
    pub fn globals(&self) -> Table {
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 2);
            #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
            ffi::lua_rawgeti(self.state, ffi::LUA_REGISTRYINDEX, ffi::LUA_RIDX_GLOBALS);
            #[cfg(any(feature = "lua51", feature = "luajit"))]
            ffi::lua_pushvalue(self.state, ffi::LUA_GLOBALSINDEX);
            Table(self.pop_ref())
        }
    }

    /// Returns a handle to the active `Thread`.  For calls to `Lua` this will be the main Lua thread,
    /// for parameters given to a callback, this will be whatever Lua thread called the callback.
    pub fn current_thread(&self) -> Thread {
        unsafe {
            ffi::lua_pushthread(self.state);
            Thread(self.pop_ref())
        }
    }

    /// Calls the given function with a `Scope` parameter, giving the function the ability to create
    /// userdata and callbacks from rust types that are !Send or non-'static.
    ///
    /// The lifetime of any function or userdata created through `Scope` lasts only until the
    /// completion of this method call, on completion all such created values are automatically
    /// dropped and Lua references to them are invalidated.  If a script accesses a value created
    /// through `Scope` outside of this method, a Lua error will result.  Since we can ensure the
    /// lifetime of values created through `Scope`, and we know that `Lua` cannot be sent to another
    /// thread while `Scope` is live, it is safe to allow !Send datatypes and whose lifetimes only
    /// outlive the scope lifetime.
    ///
    /// Inside the scope callback, all handles created through Scope will share the same unique 'lua
    /// lifetime of the parent `Lua`.  This allows scoped and non-scoped values to be mixed in
    /// API calls, which is very useful (e.g. passing a scoped userdata to a non-scoped function).
    /// However, this also enables handles to scoped values to be trivially leaked from the given
    /// callback. This is not dangerous, though!  After the callback returns, all scoped values are
    /// invalidated, which means that though references may exist, the Rust types backing them have
    /// dropped.  `Function` types will error when called, and `AnyUserData` will be typeless.  It
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
                assert_stack(self.state, 4);

                self.push_value(v)?;
                let ok = protect_lua_closure(self.state, 1, 1, |state| {
                    !ffi::lua_tostring(state, -1).is_null()
                })?;
                if ok {
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
                assert_stack(self.state, 2);

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
                assert_stack(self.state, 2);

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
        S: ?Sized + AsRef<[u8]>,
        T: ToLua<'lua>,
    {
        let t = t.to_lua(self)?;
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 5);

            push_string(self.state, name)?;
            self.push_value(t)?;

            unsafe extern "C" fn set_registry(state: *mut ffi::lua_State) -> c_int {
                ffi::lua_rawset(state, ffi::LUA_REGISTRYINDEX);
                0
            }
            protect_lua(self.state, 2, set_registry)
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
        S: ?Sized + AsRef<[u8]>,
        T: FromLua<'lua>,
    {
        let value = unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 4);

            push_string(self.state, name)?;
            unsafe extern "C" fn get_registry(state: *mut ffi::lua_State) -> c_int {
                ffi::lua_rawget(state, ffi::LUA_REGISTRYINDEX);
                1
            }
            protect_lua(self.state, 1, get_registry)?;

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
        S: ?Sized + AsRef<[u8]>,
    {
        self.set_named_registry_value(name, Nil)
    }

    /// Place a value in the Lua registry with an auto-generated key.
    ///
    /// This value will be available to rust from all `Lua` instances which share the same main
    /// state.
    ///
    /// Be warned, garbage collection of values held inside the registry is not automatic, see
    /// [`RegistryKey`] for more details.
    ///
    /// [`RegistryKey`]: struct.RegistryKey.html
    pub fn create_registry_value<'lua, T: ToLua<'lua>>(&'lua self, t: T) -> Result<RegistryKey> {
        let t = t.to_lua(self)?;
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 2);

            self.push_value(t)?;
            let registry_id = protect_lua_closure(self.state, 1, 0, |state| {
                ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX)
            })?;

            let extra = mlua_expect!(self.extra.lock(), "extra is poisoned");

            Ok(RegistryKey {
                registry_id,
                unref_list: extra.registry_unref_list.clone(),
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
        let value = unsafe {
            if !self.owns_registry_value(key) {
                return Err(Error::MismatchedRegistryKey);
            }

            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 2);

            ffi::lua_rawgeti(
                self.state,
                ffi::LUA_REGISTRYINDEX,
                key.registry_id as ffi::lua_Integer,
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
        unsafe {
            if !self.owns_registry_value(&key) {
                return Err(Error::MismatchedRegistryKey);
            }

            ffi::luaL_unref(self.state, ffi::LUA_REGISTRYINDEX, key.take());
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
        let extra = mlua_expect!(self.extra.lock(), "extra is poisoned");
        Arc::ptr_eq(&key.unref_list, &extra.registry_unref_list)
    }

    /// Remove any registry values whose `RegistryKey`s have all been dropped.
    ///
    /// Unlike normal handle values, `RegistryKey`s do not automatically remove themselves on Drop,
    /// but you can call this method to remove any unreachable registry values not manually removed
    /// by `Lua::remove_registry_value`.
    pub fn expire_registry_values(&self) {
        unsafe {
            let extra = mlua_expect!(self.extra.lock(), "extra is poisoned");
            let mut unref_list =
                mlua_expect!(extra.registry_unref_list.lock(), "unref list poisoned");
            let unref_list = mem::replace(&mut *unref_list, Some(Vec::new()));
            for id in mlua_expect!(unref_list, "unref list not set") {
                ffi::luaL_unref(self.state, ffi::LUA_REGISTRYINDEX, id);
            }
        }
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

            Value::Error(e) => {
                push_wrapped_error(self.state, e)?;
            }
        }

        Ok(())
    }

    // Uses 2 stack spaces, does not call checkstack
    pub(crate) unsafe fn pop_value(&self) -> Value {
        match ffi::lua_type(self.state, -1) {
            ffi::LUA_TNIL => {
                ffi::lua_pop(self.state, 1);
                Nil
            }

            ffi::LUA_TBOOLEAN => {
                let b = Value::Boolean(ffi::lua_toboolean(self.state, -1) != 0);
                ffi::lua_pop(self.state, 1);
                b
            }

            ffi::LUA_TLIGHTUSERDATA => {
                let ud = Value::LightUserData(LightUserData(ffi::lua_touserdata(self.state, -1)));
                ffi::lua_pop(self.state, 1);
                ud
            }

            ffi::LUA_TNUMBER => {
                if ffi::lua_isinteger(self.state, -1) != 0 {
                    let i = Value::Integer(ffi::lua_tointeger(self.state, -1));
                    ffi::lua_pop(self.state, 1);
                    i
                } else {
                    let n = Value::Number(ffi::lua_tonumber(self.state, -1));
                    ffi::lua_pop(self.state, 1);
                    n
                }
            }

            ffi::LUA_TSTRING => Value::String(String(self.pop_ref())),

            ffi::LUA_TTABLE => Value::Table(Table(self.pop_ref())),

            ffi::LUA_TFUNCTION => Value::Function(Function(self.pop_ref())),

            ffi::LUA_TUSERDATA => {
                // It should not be possible to interact with userdata types other than custom
                // UserData types OR a WrappedError.  WrappedPanic should not be here.
                if let Some(err) = get_wrapped_error(self.state, -1).as_ref() {
                    let err = err.clone();
                    ffi::lua_pop(self.state, 1);
                    Value::Error(err)
                } else {
                    Value::UserData(AnyUserData(self.pop_ref()))
                }
            }

            ffi::LUA_TTHREAD => Value::Thread(Thread(self.pop_ref())),

            _ => mlua_panic!("LUA_TNONE in pop_value"),
        }
    }

    // Pushes a LuaRef value onto the stack, uses 1 stack space, does not call checkstack
    pub(crate) unsafe fn push_ref<'lua>(&'lua self, lref: &LuaRef<'lua>) {
        assert!(
            Arc::ptr_eq(&lref.lua.extra, &self.extra),
            "Lua instance passed Value created from a different main Lua state"
        );
        let extra = mlua_expect!(self.extra.lock(), "extra is poisoned");
        ffi::lua_pushvalue(extra.ref_thread, lref.index);
        ffi::lua_xmove(extra.ref_thread, self.state, 1);
    }

    // Pops the topmost element of the stack and stores a reference to it.  This pins the object,
    // preventing garbage collection until the returned `LuaRef` is dropped.
    //
    // References are stored in the stack of a specially created auxiliary thread that exists only
    // to store reference values.  This is much faster than storing these in the registry, and also
    // much more flexible and requires less bookkeeping than storing them directly in the currently
    // used stack.  The implementation is somewhat biased towards the use case of a relatively small
    // number of short term references being created, and `RegistryKey` being used for long term
    // references.
    pub(crate) unsafe fn pop_ref(&self) -> LuaRef {
        let mut extra = mlua_expect!(self.extra.lock(), "extra is poisoned");
        ffi::lua_xmove(self.state, extra.ref_thread, 1);
        let index = ref_stack_pop(&mut extra);
        LuaRef { lua: self, index }
    }

    pub(crate) fn clone_ref<'lua>(&'lua self, lref: &LuaRef<'lua>) -> LuaRef<'lua> {
        unsafe {
            let mut extra = mlua_expect!(self.extra.lock(), "extra is poisoned");
            ffi::lua_pushvalue(extra.ref_thread, lref.index);
            let index = ref_stack_pop(&mut extra);
            LuaRef { lua: self, index }
        }
    }

    pub(crate) fn drop_ref<'lua>(&'lua self, lref: &mut LuaRef<'lua>) {
        unsafe {
            let mut extra = mlua_expect!(self.extra.lock(), "extra is poisoned");
            ffi::lua_pushnil(extra.ref_thread);
            ffi::lua_replace(extra.ref_thread, lref.index);
            extra.ref_free.push(lref.index);
        }
    }

    pub(crate) unsafe fn userdata_metatable<T: 'static + UserData>(&self) -> Result<c_int> {
        let type_id = TypeId::of::<T>();
        if let Some(table_id) = mlua_expect!(self.extra.lock(), "extra is poisoned")
            .registered_userdata
            .get(&type_id)
        {
            return Ok(*table_id);
        }

        let _sg = StackGuard::new(self.state);
        assert_stack(self.state, 8);

        let mut methods = StaticUserDataMethods::default();
        T::add_methods(&mut methods);

        protect_lua_closure(self.state, 0, 1, |state| {
            ffi::lua_newtable(state);
        })?;
        for (k, m) in methods.meta_methods {
            push_string(self.state, k.name())?;
            self.push_value(Value::Function(self.create_callback(m)?))?;

            protect_lua_closure(self.state, 3, 1, |state| {
                ffi::lua_rawset(state, -3);
            })?;
        }

        #[cfg(feature = "async")]
        let no_methods = methods.methods.is_empty() && methods.async_methods.is_empty();
        #[cfg(not(feature = "async"))]
        let no_methods = methods.methods.is_empty();

        if no_methods {
            init_userdata_metatable::<UserDataCell<T>>(self.state, -1, None)?;
        } else {
            protect_lua_closure(self.state, 0, 1, |state| {
                ffi::lua_newtable(state);
            })?;
            for (k, m) in methods.methods {
                push_string(self.state, &k)?;
                self.push_value(Value::Function(self.create_callback(m)?))?;
                protect_lua_closure(self.state, 3, 1, |state| {
                    ffi::lua_rawset(state, -3);
                })?;
            }
            #[cfg(feature = "async")]
            for (k, m) in methods.async_methods {
                push_string(self.state, &k)?;
                self.push_value(Value::Function(self.create_async_callback(m)?))?;
                protect_lua_closure(self.state, 3, 1, |state| {
                    ffi::lua_rawset(state, -3);
                })?;
            }

            init_userdata_metatable::<UserDataCell<T>>(self.state, -2, Some(-1))?;
            ffi::lua_pop(self.state, 1);
        }

        let id = protect_lua_closure(self.state, 1, 0, |state| {
            ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX)
        })?;

        let mut extra = mlua_expect!(self.extra.lock(), "extra is poisoned");
        extra.registered_userdata.insert(type_id, id);

        Ok(id)
    }

    // Pushes a LuaRef value onto the stack, checking that it's not destructed
    // Uses 2 stack spaces, does not call checkstack
    #[cfg(feature = "serialize")]
    pub(crate) unsafe fn push_userdata_ref(&self, lref: &LuaRef) -> Result<()> {
        self.push_ref(lref);
        if ffi::lua_getmetatable(self.state, -1) == 0 {
            Err(Error::UserDataTypeMismatch)
        } else {
            // Check that userdata is not destructed
            get_destructed_userdata_metatable(self.state);
            let eq = ffi::lua_rawequal(self.state, -1, -2) == 1;
            ffi::lua_pop(self.state, 2);
            if eq {
                Err(Error::UserDataDestructed)
            } else {
                Ok(())
            }
        }
    }

    // Creates a Function out of a Callback containing a 'static Fn.  This is safe ONLY because the
    // Fn is 'static, otherwise it could capture 'callback arguments improperly.  Without ATCs, we
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
            callback_error(state, |nargs| {
                if ffi::lua_type(state, ffi::lua_upvalueindex(1)) == ffi::LUA_TNIL
                    || ffi::lua_type(state, ffi::lua_upvalueindex(2)) == ffi::LUA_TNIL
                {
                    return Err(Error::CallbackDestructed);
                }
                let func = get_userdata::<Callback>(state, ffi::lua_upvalueindex(1));
                let lua = get_userdata::<Lua>(state, ffi::lua_upvalueindex(2));

                if nargs < ffi::LUA_MINSTACK {
                    check_stack(state, ffi::LUA_MINSTACK - nargs)?;
                }

                let lua = &mut *lua;
                lua.state = state;

                let mut args = MultiValue::new();
                args.reserve(nargs as usize);
                for _ in 0..nargs {
                    args.push_front(lua.pop_value());
                }

                let results = (*func)(lua, args)?;
                let nresults = results.len() as c_int;

                check_stack(state, nresults)?;
                for r in results {
                    lua.push_value(r)?;
                }

                Ok(nresults)
            })
        }

        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 6);

            push_meta_gc_userdata::<Callback, _>(self.state, func)?;
            push_gc_userdata(self.state, self.clone())?;

            protect_lua_closure(self.state, 2, 1, |state| {
                ffi::lua_pushcclosure(state, call_callback, 2);
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
        #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
        {
            let libs = mlua_expect!(self.extra.lock(), "extra is poisoned").libs;
            if !libs.contains(StdLib::COROUTINE) {
                self.load_from_std_lib(StdLib::COROUTINE)?;
            }
        }

        unsafe extern "C" fn call_callback(state: *mut ffi::lua_State) -> c_int {
            callback_error(state, |nargs| {
                if ffi::lua_type(state, ffi::lua_upvalueindex(1)) == ffi::LUA_TNIL
                    || ffi::lua_type(state, ffi::lua_upvalueindex(2)) == ffi::LUA_TNIL
                {
                    return Err(Error::CallbackDestructed);
                }
                let func = get_userdata::<AsyncCallback>(state, ffi::lua_upvalueindex(1));
                let lua = get_userdata::<Lua>(state, ffi::lua_upvalueindex(2));

                if nargs < ffi::LUA_MINSTACK {
                    check_stack(state, ffi::LUA_MINSTACK - nargs)?;
                }

                let lua = &mut *lua;
                lua.state = state;

                let mut args = MultiValue::new();
                args.reserve(nargs as usize);
                for _ in 0..nargs {
                    args.push_front(lua.pop_value());
                }

                let fut = (*func)(lua, args);
                push_gc_userdata(state, fut)?;
                push_gc_userdata(state, lua.clone())?;

                ffi::lua_pushcclosure(state, poll_future, 2);

                Ok(1)
            })
        }

        unsafe extern "C" fn poll_future(state: *mut ffi::lua_State) -> c_int {
            callback_error(state, |nargs| {
                if ffi::lua_type(state, ffi::lua_upvalueindex(1)) == ffi::LUA_TNIL
                    || ffi::lua_type(state, ffi::lua_upvalueindex(2)) == ffi::LUA_TNIL
                {
                    return Err(Error::CallbackDestructed);
                }
                let fut = get_userdata::<LocalBoxFuture<Result<MultiValue>>>(
                    state,
                    ffi::lua_upvalueindex(1),
                );
                let lua = get_userdata::<Lua>(state, ffi::lua_upvalueindex(2));

                if nargs < ffi::LUA_MINSTACK {
                    check_stack(state, ffi::LUA_MINSTACK - nargs)?;
                }

                let lua = &mut *lua;
                let mut waker = noop_waker();

                // Try to get an outer poll waker
                ffi::lua_pushlightuserdata(state, &WAKER_REGISTRY_KEY as *const u8 as *mut c_void);
                ffi::lua_rawget(state, ffi::LUA_REGISTRYINDEX);
                if let Some(w) = get_gc_userdata::<Waker>(state, -1).as_ref() {
                    waker = (*w).clone();
                }
                ffi::lua_pop(state, 1);

                let mut ctx = Context::from_waker(&waker);

                match (*fut).as_mut().poll(&mut ctx) {
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
            assert_stack(self.state, 6);

            push_meta_gc_userdata::<AsyncCallback, _>(self.state, func)?;
            push_gc_userdata(self.state, self.clone())?;

            protect_lua_closure(self.state, 2, 1, |state| {
                ffi::lua_pushcclosure(state, call_callback, 2);
            })?;

            Function(self.pop_ref())
        };

        let coroutine = self.globals().get::<_, Table>("coroutine")?;

        let env = self.create_table()?;
        env.set("get_poll", get_poll)?;
        env.set("yield", coroutine.get::<_, Function>("yield")?)?;
        env.set(
            "unpack",
            self.create_function(|_, (tbl, len): (Table, Integer)| {
                Ok(MultiValue::from_vec(
                    tbl.raw_sequence_values_by_len(Some(len))
                        .collect::<Result<Vec<Value>>>()?,
                ))
            })?,
        )?;
        env.set("pending", unsafe {
            let _sg = StackGuard::new(self.state);
            check_stack(self.state, 5)?;
            push_gc_userdata(self.state, AsyncPollPending)?;
            self.pop_value()
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

    pub(crate) unsafe fn make_userdata<T>(&self, data: UserDataWrapped<T>) -> Result<AnyUserData>
    where
        T: 'static + UserData,
    {
        let _sg = StackGuard::new(self.state);
        assert_stack(self.state, 4);

        let ud_index = self.userdata_metatable::<T>()?;
        push_userdata::<UserDataCell<T>>(self.state, RefCell::new(data))?;

        ffi::lua_rawgeti(
            self.state,
            ffi::LUA_REGISTRYINDEX,
            ud_index as ffi::lua_Integer,
        );
        ffi::lua_setmetatable(self.state, -2);

        Ok(AnyUserData(self.pop_ref()))
    }

    pub(crate) fn clone(&self) -> Self {
        Lua {
            state: self.state,
            main_state: self.main_state,
            extra: self.extra.clone(),
            ephemeral: true,
            safe: self.safe,
            _no_ref_unwind_safe: PhantomData,
        }
    }

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

    pub(crate) unsafe fn make_from_ptr(state: *mut ffi::lua_State) -> Self {
        let _sg = StackGuard::new(state);
        assert_stack(state, 3);

        ffi::lua_rawgetp(
            state,
            ffi::LUA_REGISTRYINDEX,
            &EXTRA_REGISTRY_KEY as *const u8 as *mut c_void,
        );
        let extra = mlua_expect!(
            (*get_gc_userdata::<Weak<Mutex<ExtraData>>>(state, -1)).upgrade(),
            "extra is destroyed"
        );
        ffi::lua_pop(state, 1);

        Lua {
            state,
            main_state: get_main_state(state),
            extra,
            ephemeral: true,
            safe: true, // TODO: Inherit the attribute
            _no_ref_unwind_safe: PhantomData,
        }
    }

    pub(crate) unsafe fn hook_callback(&self) -> Option<HookCallback> {
        let extra = mlua_expect!(self.extra.lock(), "extra is poisoned");
        extra.hook_callback.clone()
    }
}

/// Returned from [`Lua::load`] and is used to finalize loading and executing Lua main chunks.
///
/// [`Lua::load`]: struct.Lua.html#method.load
#[must_use = "`Chunk`s do nothing unless one of `exec`, `eval`, `call`, or `into_function` are called on them"]
pub struct Chunk<'lua, 'a> {
    lua: &'lua Lua,
    source: &'a [u8],
    name: Option<CString>,
    env: Option<Value<'lua>>,
    mode: Option<ChunkMode>,
}

/// Represents chunk mode (text or binary).
#[derive(Clone, Copy)]
pub enum ChunkMode {
    Text,
    Binary,
}

impl<'lua, 'a> Chunk<'lua, 'a> {
    /// Sets the name of this chunk, which results in more informative error traces.
    pub fn set_name<S: ?Sized + AsRef<[u8]>>(mut self, name: &S) -> Result<Chunk<'lua, 'a>> {
        let name =
            CString::new(name.as_ref().to_vec()).map_err(|e| Error::ToLuaConversionError {
                from: "&str",
                to: "string",
                message: Some(e.to_string()),
            })?;
        self.name = Some(name);
        Ok(self)
    }

    /// Sets the first upvalue (`_ENV`) of the loaded chunk to the given value.
    ///
    /// Lua main chunks always have exactly one upvalue, and this upvalue is used as the `_ENV`
    /// variable inside the chunk.  By default this value is set to the global environment.
    ///
    /// Calling this method changes the `_ENV` upvalue to the value provided, and variables inside
    /// the chunk will refer to the given environment rather than the global one.
    ///
    /// All global variables (including the standard library!) are looked up in `_ENV`, so it may be
    /// necessary to populate the environment in order for scripts using custom environments to be
    /// useful.
    pub fn set_environment<V: ToLua<'lua>>(mut self, env: V) -> Result<Chunk<'lua, 'a>> {
        self.env = Some(env.to_lua(self.lua)?);
        Ok(self)
    }

    /// Sets whether the chunk is text or binary (autodetected by default).
    ///
    /// Lua does not check the consistency of binary chunks, therefore this mode is allowed only
    /// for instances created with [`Lua::unsafe_new`].
    ///
    /// [`Lua::unsafe_new`]: struct.Lua.html#method.unsafe_new
    pub fn set_mode(mut self, mode: ChunkMode) -> Chunk<'lua, 'a> {
        self.mode = Some(mode);
        self
    }

    /// Execute this chunk of code.
    ///
    /// This is equivalent to calling the chunk function with no arguments and no return values.
    pub fn exec(self) -> Result<()> {
        self.call(())?;
        Ok(())
    }

    /// Asynchronously execute this chunk of code.
    ///
    /// See [`Chunk::exec`] for more details.
    ///
    /// Requires `feature = "async"`
    ///
    /// [`Chunk::exec`]: struct.Chunk.html#method.exec
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn exec_async<'fut>(self) -> LocalBoxFuture<'fut, Result<()>>
    where
        'lua: 'fut,
    {
        self.call_async(())
    }

    /// Evaluate the chunk as either an expression or block.
    ///
    /// If the chunk can be parsed as an expression, this loads and executes the chunk and returns
    /// the value that it evaluates to.  Otherwise, the chunk is interpreted as a block as normal,
    /// and this is equivalent to calling `exec`.
    pub fn eval<R: FromLuaMulti<'lua>>(self) -> Result<R> {
        // Bytecode is always interpreted as a statement.
        // For source code, first try interpreting the lua as an expression by adding
        // "return", then as a statement.  This is the same thing the
        // actual lua repl does.
        if self.source.starts_with(ffi::LUA_SIGNATURE) {
            self.call(())
        } else if let Ok(function) = self.lua.load_chunk(
            &self.expression_source(),
            self.name.as_ref(),
            self.env.clone(),
            self.mode,
        ) {
            function.call(())
        } else {
            self.call(())
        }
    }

    /// Asynchronously evaluate the chunk as either an expression or block.
    ///
    /// See [`Chunk::eval`] for more details.
    ///
    /// Requires `feature = "async"`
    ///
    /// [`Chunk::eval`]: struct.Chunk.html#method.eval
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn eval_async<'fut, R>(self) -> LocalBoxFuture<'fut, Result<R>>
    where
        'lua: 'fut,
        R: FromLuaMulti<'lua> + 'fut,
    {
        if self.source.starts_with(ffi::LUA_SIGNATURE) {
            self.call_async(())
        } else if let Ok(function) = self.lua.load_chunk(
            &self.expression_source(),
            self.name.as_ref(),
            self.env.clone(),
            self.mode,
        ) {
            function.call_async(())
        } else {
            self.call_async(())
        }
    }

    /// Load the chunk function and call it with the given arguemnts.
    ///
    /// This is equivalent to `into_function` and calling the resulting function.
    pub fn call<A: ToLuaMulti<'lua>, R: FromLuaMulti<'lua>>(self, args: A) -> Result<R> {
        self.into_function()?.call(args)
    }

    /// Load the chunk function and asynchronously call it with the given arguemnts.
    ///
    /// See [`Chunk::call`] for more details.
    ///
    /// Requires `feature = "async"`
    ///
    /// [`Chunk::call`]: struct.Chunk.html#method.call
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn call_async<'fut, A, R>(self, args: A) -> LocalBoxFuture<'fut, Result<R>>
    where
        'lua: 'fut,
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'fut,
    {
        match self.into_function() {
            Ok(func) => func.call_async(args),
            Err(e) => Box::pin(future::err(e)),
        }
    }

    /// Load this chunk into a regular `Function`.
    ///
    /// This simply compiles the chunk without actually executing it.
    pub fn into_function(self) -> Result<Function<'lua>> {
        self.lua
            .load_chunk(self.source, self.name.as_ref(), self.env, self.mode)
    }

    fn expression_source(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(b"return ".len() + self.source.len());
        buf.extend(b"return ");
        buf.extend(self.source);
        buf
    }
}

unsafe fn load_from_std_lib(state: *mut ffi::lua_State, libs: StdLib) {
    #[cfg(feature = "luajit")]
    // Stop collector during library initialization
    ffi::lua_gc(state, ffi::LUA_GCSTOP, 0);

    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    {
        if libs.contains(StdLib::COROUTINE) {
            let colib_name = CString::new(ffi::LUA_COLIBNAME).unwrap();
            ffi::luaL_requiref(state, colib_name.as_ptr(), ffi::luaopen_coroutine, 1);
            ffi::lua_pop(state, 1);
        }
    }

    if libs.contains(StdLib::TABLE) {
        let tablib_name = CString::new(ffi::LUA_TABLIBNAME).unwrap();
        ffi::luaL_requiref(state, tablib_name.as_ptr(), ffi::luaopen_table, 1);
        ffi::lua_pop(state, 1);
    }

    if libs.contains(StdLib::IO) {
        let iolib_name = CString::new(ffi::LUA_IOLIBNAME).unwrap();
        ffi::luaL_requiref(state, iolib_name.as_ptr(), ffi::luaopen_io, 1);
        ffi::lua_pop(state, 1);
    }

    if libs.contains(StdLib::OS) {
        let oslib_name = CString::new(ffi::LUA_OSLIBNAME).unwrap();
        ffi::luaL_requiref(state, oslib_name.as_ptr(), ffi::luaopen_os, 1);
        ffi::lua_pop(state, 1);
    }

    if libs.contains(StdLib::STRING) {
        let strlib_name = CString::new(ffi::LUA_STRLIBNAME).unwrap();
        ffi::luaL_requiref(state, strlib_name.as_ptr(), ffi::luaopen_string, 1);
        ffi::lua_pop(state, 1);
    }

    #[cfg(any(feature = "lua54", feature = "lua53"))]
    {
        if libs.contains(StdLib::UTF8) {
            let utf8lib_name = CString::new(ffi::LUA_UTF8LIBNAME).unwrap();
            ffi::luaL_requiref(state, utf8lib_name.as_ptr(), ffi::luaopen_utf8, 1);
            ffi::lua_pop(state, 1);
        }
    }

    #[cfg(feature = "lua52")]
    {
        if libs.contains(StdLib::BIT) {
            let bitlib_name = CString::new(ffi::LUA_BITLIBNAME).unwrap();
            ffi::luaL_requiref(state, bitlib_name.as_ptr(), ffi::luaopen_bit32, 1);
            ffi::lua_pop(state, 1);
        }
    }

    #[cfg(feature = "luajit")]
    {
        if libs.contains(StdLib::BIT) {
            let bitlib_name = CString::new(ffi::LUA_BITLIBNAME).unwrap();
            ffi::luaL_requiref(state, bitlib_name.as_ptr(), ffi::luaopen_bit, 1);
            ffi::lua_pop(state, 1);
        }
    }

    if libs.contains(StdLib::MATH) {
        let mathlib_name = CString::new(ffi::LUA_MATHLIBNAME).unwrap();
        ffi::luaL_requiref(state, mathlib_name.as_ptr(), ffi::luaopen_math, 1);
        ffi::lua_pop(state, 1);
    }

    if libs.contains(StdLib::DEBUG) {
        let dblib_name = CString::new(ffi::LUA_DBLIBNAME).unwrap();
        ffi::luaL_requiref(state, dblib_name.as_ptr(), ffi::luaopen_debug, 1);
        ffi::lua_pop(state, 1);
    }

    if libs.contains(StdLib::PACKAGE) {
        let loadlib_name = CString::new(ffi::LUA_LOADLIBNAME).unwrap();
        ffi::luaL_requiref(state, loadlib_name.as_ptr(), ffi::luaopen_package, 1);
        ffi::lua_pop(state, 1);
    }

    #[cfg(feature = "luajit")]
    {
        if libs.contains(StdLib::JIT) {
            let jitlib_name = CString::new(ffi::LUA_JITLIBNAME).unwrap();
            ffi::luaL_requiref(state, jitlib_name.as_ptr(), ffi::luaopen_jit, 1);
            ffi::lua_pop(state, 1);
        }

        if libs.contains(StdLib::FFI) {
            let ffilib_name = CString::new(ffi::LUA_FFILIBNAME).unwrap();
            ffi::luaL_requiref(state, ffilib_name.as_ptr(), ffi::luaopen_ffi, 1);
            ffi::lua_pop(state, 1);
        }
    }

    #[cfg(feature = "luajit")]
    ffi::lua_gc(state, ffi::LUA_GCRESTART, -1);
}

unsafe fn ref_stack_pop(extra: &mut ExtraData) -> c_int {
    if let Some(free) = extra.ref_free.pop() {
        ffi::lua_replace(extra.ref_thread, free);
        free
    } else {
        if extra.ref_stack_max >= extra.ref_stack_size {
            // It is a user error to create enough references to exhaust the Lua max stack size for
            // the ref thread.
            if ffi::lua_checkstack(extra.ref_thread, extra.ref_stack_size) == 0 {
                mlua_panic!("cannot create a Lua reference, out of auxiliary stack space");
            }
            extra.ref_stack_size *= 2;
        }
        extra.ref_stack_max += 1;
        extra.ref_stack_max
    }
}

struct StaticUserDataMethods<'lua, T: 'static + UserData> {
    methods: Vec<(Vec<u8>, Callback<'lua, 'static>)>,
    #[cfg(feature = "async")]
    async_methods: Vec<(Vec<u8>, AsyncCallback<'lua, 'static>)>,
    meta_methods: Vec<(MetaMethod, Callback<'lua, 'static>)>,
    _type: PhantomData<T>,
}

impl<'lua, T: 'static + UserData> Default for StaticUserDataMethods<'lua, T> {
    fn default() -> StaticUserDataMethods<'lua, T> {
        StaticUserDataMethods {
            methods: Vec::new(),
            #[cfg(feature = "async")]
            async_methods: Vec::new(),
            meta_methods: Vec::new(),
            _type: PhantomData,
        }
    }
}

impl<'lua, T: 'static + UserData> UserDataMethods<'lua, T> for StaticUserDataMethods<'lua, T> {
    fn add_method<S, A, R, M>(&mut self, name: &S, method: M)
    where
        S: AsRef<[u8]> + ?Sized,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + Fn(&'lua Lua, &T, A) -> Result<R>,
    {
        self.methods
            .push((name.as_ref().to_vec(), Self::box_method(method)));
    }

    fn add_method_mut<S, A, R, M>(&mut self, name: &S, method: M)
    where
        S: AsRef<[u8]> + ?Sized,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + FnMut(&'lua Lua, &mut T, A) -> Result<R>,
    {
        self.methods
            .push((name.as_ref().to_vec(), Self::box_method_mut(method)));
    }

    #[cfg(feature = "async")]
    fn add_async_method<S, A, R, M, MR>(&mut self, name: &S, method: M)
    where
        T: Clone,
        S: AsRef<[u8]> + ?Sized,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + Fn(&'lua Lua, T, A) -> MR,
        MR: 'lua + Future<Output = Result<R>>,
    {
        self.async_methods
            .push((name.as_ref().to_vec(), Self::box_async_method(method)));
    }

    fn add_function<S, A, R, F>(&mut self, name: &S, function: F)
    where
        S: AsRef<[u8]> + ?Sized,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua, A) -> Result<R>,
    {
        self.methods
            .push((name.as_ref().to_vec(), Self::box_function(function)));
    }

    fn add_function_mut<S, A, R, F>(&mut self, name: &S, function: F)
    where
        S: AsRef<[u8]> + ?Sized,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + FnMut(&'lua Lua, A) -> Result<R>,
    {
        self.methods
            .push((name.as_ref().to_vec(), Self::box_function_mut(function)));
    }

    #[cfg(feature = "async")]
    fn add_async_function<S, A, R, F, FR>(&mut self, name: &S, function: F)
    where
        T: Clone,
        S: AsRef<[u8]> + ?Sized,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua, A) -> FR,
        FR: 'lua + Future<Output = Result<R>>,
    {
        self.async_methods
            .push((name.as_ref().to_vec(), Self::box_async_function(function)));
    }

    fn add_meta_method<A, R, M>(&mut self, meta: MetaMethod, method: M)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + Fn(&'lua Lua, &T, A) -> Result<R>,
    {
        self.meta_methods.push((meta, Self::box_method(method)));
    }

    fn add_meta_method_mut<A, R, M>(&mut self, meta: MetaMethod, method: M)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + FnMut(&'lua Lua, &mut T, A) -> Result<R>,
    {
        self.meta_methods.push((meta, Self::box_method_mut(method)));
    }

    fn add_meta_function<A, R, F>(&mut self, meta: MetaMethod, function: F)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua, A) -> Result<R>,
    {
        self.meta_methods.push((meta, Self::box_function(function)));
    }

    fn add_meta_function_mut<A, R, F>(&mut self, meta: MetaMethod, function: F)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + FnMut(&'lua Lua, A) -> Result<R>,
    {
        self.meta_methods
            .push((meta, Self::box_function_mut(function)));
    }
}

impl<'lua, T: 'static + UserData> StaticUserDataMethods<'lua, T> {
    fn box_method<A, R, M>(method: M) -> Callback<'lua, 'static>
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + Fn(&'lua Lua, &T, A) -> Result<R>,
    {
        Box::new(move |lua, mut args| {
            if let Some(front) = args.pop_front() {
                let userdata = AnyUserData::from_lua(front, lua)?;
                let userdata = userdata.borrow::<T>()?;
                method(lua, &userdata, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
            } else {
                Err(Error::FromLuaConversionError {
                    from: "missing argument",
                    to: "userdata",
                    message: None,
                })
            }
        })
    }

    fn box_method_mut<A, R, M>(method: M) -> Callback<'lua, 'static>
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + FnMut(&'lua Lua, &mut T, A) -> Result<R>,
    {
        let method = RefCell::new(method);
        Box::new(move |lua, mut args| {
            if let Some(front) = args.pop_front() {
                let userdata = AnyUserData::from_lua(front, lua)?;
                let mut userdata = userdata.borrow_mut::<T>()?;
                let mut method = method
                    .try_borrow_mut()
                    .map_err(|_| Error::RecursiveMutCallback)?;
                (&mut *method)(lua, &mut userdata, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
            } else {
                Err(Error::FromLuaConversionError {
                    from: "missing argument",
                    to: "userdata",
                    message: None,
                })
            }
        })
    }

    #[cfg(feature = "async")]
    fn box_async_method<A, R, M, MR>(method: M) -> AsyncCallback<'lua, 'static>
    where
        T: Clone,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + Fn(&'lua Lua, T, A) -> MR,
        MR: 'lua + Future<Output = Result<R>>,
    {
        Box::new(move |lua, mut args| {
            let fut_res = || {
                if let Some(front) = args.pop_front() {
                    let userdata = AnyUserData::from_lua(front, lua)?;
                    let userdata = userdata.borrow::<T>()?.clone();
                    Ok(method(lua, userdata, A::from_lua_multi(args, lua)?))
                } else {
                    Err(Error::FromLuaConversionError {
                        from: "missing argument",
                        to: "userdata",
                        message: None,
                    })
                }
            };
            match fut_res() {
                Ok(fut) => Box::pin(fut.and_then(move |ret| future::ready(ret.to_lua_multi(lua)))),
                Err(e) => Box::pin(future::err(e)),
            }
        })
    }

    fn box_function<A, R, F>(function: F) -> Callback<'lua, 'static>
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua, A) -> Result<R>,
    {
        Box::new(move |lua, args| function(lua, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua))
    }

    fn box_function_mut<A, R, F>(function: F) -> Callback<'lua, 'static>
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + FnMut(&'lua Lua, A) -> Result<R>,
    {
        let function = RefCell::new(function);
        Box::new(move |lua, args| {
            let function = &mut *function
                .try_borrow_mut()
                .map_err(|_| Error::RecursiveMutCallback)?;
            function(lua, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
        })
    }

    #[cfg(feature = "async")]
    fn box_async_function<A, R, F, FR>(function: F) -> AsyncCallback<'lua, 'static>
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua, A) -> FR,
        FR: 'lua + Future<Output = Result<R>>,
    {
        Box::new(move |lua, args| {
            let args = match A::from_lua_multi(args, lua) {
                Ok(args) => args,
                Err(e) => return Box::pin(future::err(e)),
            };
            Box::pin(function(lua, args).and_then(move |ret| future::ready(ret.to_lua_multi(lua))))
        })
    }
}
