use std::any::TypeId;
use std::cell::{BorrowError, BorrowMutError, RefCell};
use std::marker::PhantomData;
use std::ops::Deref;
use std::os::raw::{c_char, c_int};
use std::panic::Location;
use std::result::Result as StdResult;
use std::{fmt, mem, ptr};

use crate::chunk::{AsChunk, Chunk};
use crate::debug::Debug;
use crate::error::{Error, Result};
use crate::function::Function;
use crate::memory::MemoryState;
use crate::multi::MultiValue;
use crate::scope::Scope;
use crate::stdlib::StdLib;
use crate::string::String;
use crate::table::Table;
use crate::thread::Thread;
use crate::traits::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti};
use crate::types::{
    AppDataRef, AppDataRefMut, ArcReentrantMutexGuard, Integer, LuaType, MaybeSend, Number, ReentrantMutex,
    ReentrantMutexGuard, RegistryKey, VmState, XRc, XWeak,
};
use crate::userdata::{AnyUserData, UserData, UserDataProxy, UserDataRegistry, UserDataStorage};
use crate::util::{assert_stack, check_stack, protect_lua_closure, push_string, rawset_field, StackGuard};
use crate::value::{Nil, Value};

#[cfg(not(feature = "luau"))]
use crate::{debug::HookTriggers, types::HookKind};

#[cfg(any(feature = "luau", doc))]
use crate::{buffer::Buffer, chunk::Compiler};

#[cfg(feature = "async")]
use {
    crate::types::LightUserData,
    std::future::{self, Future},
};

#[cfg(feature = "serde")]
use serde::Serialize;

pub(crate) use extra::ExtraData;
pub use raw::RawLua;
pub(crate) use util::callback_error_ext;

/// Top level Lua struct which represents an instance of Lua VM.
pub struct Lua {
    pub(self) raw: XRc<ReentrantMutex<RawLua>>,
    // Controls whether garbage collection should be run on drop
    pub(self) collect_garbage: bool,
}

/// Weak reference to Lua instance.
///
/// This can used to prevent circular references between Lua and Rust objects.
#[derive(Clone)]
pub struct WeakLua(XWeak<ReentrantMutex<RawLua>>);

pub(crate) struct LuaGuard(ArcReentrantMutexGuard<RawLua>);

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
    pub catch_rust_panics: bool,

    /// Max size of thread (coroutine) object pool used to execute asynchronous functions.
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
        const { LuaOptions::new() }
    }
}

impl LuaOptions {
    /// Returns a new instance of `LuaOptions` with default parameters.
    pub const fn new() -> Self {
        LuaOptions {
            catch_rust_panics: true,
            #[cfg(feature = "async")]
            thread_pool_size: 0,
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

impl Drop for Lua {
    fn drop(&mut self) {
        if self.collect_garbage {
            let _ = self.gc_collect();
        }
    }
}

impl Clone for Lua {
    #[inline]
    fn clone(&self) -> Self {
        Lua {
            raw: XRc::clone(&self.raw),
            collect_garbage: false,
        }
    }
}

impl fmt::Debug for Lua {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Lua({:p})", self.lock().state())
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
    /// The created Lua state will have _some_ safety guarantees and will not allow to load unsafe
    /// standard libraries or C modules.
    ///
    /// See [`StdLib`] documentation for a list of unsafe modules that cannot be loaded.
    pub fn new() -> Lua {
        mlua_expect!(
            Self::new_with(StdLib::ALL_SAFE, LuaOptions::default()),
            "Cannot create a Lua state"
        )
    }

    /// Creates a new Lua state and loads all the standard libraries.
    ///
    /// # Safety
    /// The created Lua state will not have safety guarantees and will allow to load C modules.
    pub unsafe fn unsafe_new() -> Lua {
        Self::unsafe_new_with(StdLib::ALL, LuaOptions::default())
    }

    /// Creates a new Lua state and loads the specified safe subset of the standard libraries.
    ///
    /// Use the [`StdLib`] flags to specify the libraries you want to load.
    ///
    /// # Safety
    /// The created Lua state will have _some_ safety guarantees and will not allow to load unsafe
    /// standard libraries or C modules.
    ///
    /// See [`StdLib`] documentation for a list of unsafe modules that cannot be loaded.
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
            mlua_expect!(lua.disable_c_modules(), "Error disabling C modules");
        }
        lua.lock().mark_safe();

        Ok(lua)
    }

    /// Creates a new Lua state and loads the specified subset of the standard libraries.
    ///
    /// Use the [`StdLib`] flags to specify the libraries you want to load.
    ///
    /// # Safety
    /// The created Lua state will not have safety guarantees and allow to load C modules.
    pub unsafe fn unsafe_new_with(libs: StdLib, options: LuaOptions) -> Lua {
        // Workaround to avoid stripping a few unused Lua symbols that could be imported
        // by C modules in unsafe mode
        let mut _symbols: Vec<*const extern "C-unwind" fn()> =
            vec![ffi::lua_isuserdata as _, ffi::lua_tocfunction as _];

        #[cfg(not(feature = "luau"))]
        _symbols.extend_from_slice(&[
            ffi::lua_atpanic as _,
            ffi::luaL_loadstring as _,
            ffi::luaL_openlibs as _,
        ]);
        #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
        {
            _symbols.push(ffi::lua_getglobal as _);
            _symbols.push(ffi::lua_setglobal as _);
            _symbols.push(ffi::luaL_setfuncs as _);
        }

        Self::inner_new(libs, options)
    }

    /// Creates a new Lua state with required `libs` and `options`
    unsafe fn inner_new(libs: StdLib, options: LuaOptions) -> Lua {
        let lua = Lua {
            raw: RawLua::new(libs, &options),
            collect_garbage: true,
        };

        #[cfg(feature = "luau")]
        mlua_expect!(lua.configure_luau(), "Error configuring Luau");

        lua
    }

    /// Returns or constructs Lua instance from a raw state.
    ///
    /// Once initialized, the returned Lua instance is cached in the registry and can be retrieved
    /// by calling this function again.
    ///
    /// # Safety
    /// The `Lua` must outlive the chosen lifetime `'a`.
    #[inline]
    pub unsafe fn get_or_init_from_ptr<'a>(state: *mut ffi::lua_State) -> &'a Lua {
        debug_assert!(!state.is_null(), "Lua state is null");
        match ExtraData::get(state) {
            extra if !extra.is_null() => (*extra).lua(),
            _ => {
                // The `owned` flag is set to `false` as we don't own the Lua state.
                RawLua::init_from_ptr(state, false);
                (*ExtraData::get(state)).lua()
            }
        }
    }

    /// Calls provided function passing a raw lua state.
    ///
    /// The arguments will be pushed onto the stack before calling the function.
    ///
    /// This method ensures that the Lua instance is locked while the function is called
    /// and restores Lua stack after the function returns.
    ///
    /// # Example
    /// ```
    /// # use mlua::{Lua, Result};
    /// # fn main() -> Result<()> {
    /// let lua = Lua::new();
    /// let n: i32 = unsafe {
    ///     let nums = (3, 4, 5);
    ///     lua.exec_raw(nums, |state| {
    ///         let n = ffi::lua_gettop(state);
    ///         let mut sum = 0;
    ///         for i in 1..=n {
    ///             sum += ffi::lua_tointeger(state, i);
    ///         }
    ///         ffi::lua_pop(state, n);
    ///         ffi::lua_pushinteger(state, sum);
    ///     })
    /// }?;
    /// assert_eq!(n, 12);
    /// # Ok(())
    /// # }
    /// ```
    #[allow(clippy::missing_safety_doc)]
    pub unsafe fn exec_raw<R: FromLuaMulti>(
        &self,
        args: impl IntoLuaMulti,
        f: impl FnOnce(*mut ffi::lua_State),
    ) -> Result<R> {
        let lua = self.lock();
        let state = lua.state();
        let _sg = StackGuard::new(state);
        let stack_start = ffi::lua_gettop(state);
        let nargs = args.push_into_stack_multi(&lua)?;
        check_stack(state, 3)?;
        protect_lua_closure::<_, ()>(state, nargs, ffi::LUA_MULTRET, f)?;
        let nresults = ffi::lua_gettop(state) - stack_start;
        R::from_stack_multi(nresults, &lua)
    }

    /// Loads the specified subset of the standard libraries into an existing Lua state.
    ///
    /// Use the [`StdLib`] flags to specify the libraries you want to load.
    pub fn load_std_libs(&self, libs: StdLib) -> Result<()> {
        unsafe { self.lock().load_std_libs(libs) }
    }

    /// Registers module into an existing Lua state using the specified value.
    ///
    /// After registration, the given value will always be immediately returned when the
    /// given module is [required].
    ///
    /// [required]: https://www.lua.org/manual/5.4/manual.html#pdf-require
    pub fn register_module(&self, modname: &str, value: impl IntoLua) -> Result<()> {
        #[cfg(not(feature = "luau"))]
        const LOADED_MODULES_KEY: *const c_char = ffi::LUA_LOADED_TABLE;
        #[cfg(feature = "luau")]
        const LOADED_MODULES_KEY: *const c_char = ffi::LUA_REGISTERED_MODULES_TABLE;

        if cfg!(feature = "luau") && !modname.starts_with('@') {
            return Err(Error::runtime("module name must begin with '@'"));
        }
        unsafe {
            self.exec_raw::<()>(value, |state| {
                ffi::luaL_getsubtable(state, ffi::LUA_REGISTRYINDEX, LOADED_MODULES_KEY);
                ffi::lua_pushlstring(state, modname.as_ptr() as *const c_char, modname.len() as _);
                ffi::lua_pushvalue(state, -3);
                ffi::lua_rawset(state, -3);
            })
        }
    }

    /// Preloads module into an existing Lua state using the specified loader function.
    ///
    /// When the module is required, the loader function will be called with module name as the
    /// first argument.
    ///
    /// This is similar to setting the [`package.preload[modname]`] field.
    ///
    /// [`package.preload[modname]`]: <https://www.lua.org/manual/5.4/manual.html#pdf-package.preload>
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub fn preload_module(&self, modname: &str, func: Function) -> Result<()> {
        #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
        let preload = unsafe {
            self.exec_raw::<Option<Table>>((), |state| {
                ffi::lua_getfield(state, ffi::LUA_REGISTRYINDEX, ffi::LUA_PRELOAD_TABLE);
            })?
        };
        #[cfg(any(feature = "lua51", feature = "luajit"))]
        let preload = unsafe {
            self.exec_raw::<Option<Table>>((), |state| {
                if ffi::lua_getfield(state, ffi::LUA_REGISTRYINDEX, ffi::LUA_LOADED_TABLE) != ffi::LUA_TNIL {
                    ffi::luaL_getsubtable(state, -1, ffi::LUA_LOADLIBNAME);
                    ffi::luaL_getsubtable(state, -1, cstr!("preload"));
                    ffi::lua_rotate(state, 1, 1);
                }
            })?
        };
        if let Some(preload) = preload {
            preload.raw_set(modname, func)?;
        }
        Ok(())
    }

    #[doc(hidden)]
    #[deprecated(since = "0.11.0", note = "Use `register_module` instead")]
    #[cfg(not(feature = "luau"))]
    #[cfg(not(tarpaulin_include))]
    pub fn load_from_function<T: FromLua>(&self, modname: &str, func: Function) -> Result<T> {
        let loaded = unsafe {
            self.exec_raw::<Table>((), |state| {
                ffi::luaL_getsubtable(state, ffi::LUA_REGISTRYINDEX, ffi::LUA_LOADED_TABLE);
            })?
        };

        let value = match loaded.raw_get(modname)? {
            Value::Nil => {
                let result = match func.call(modname)? {
                    Value::Nil => Value::Boolean(true),
                    res => res,
                };
                loaded.raw_set(modname, &result)?;
                result
            }
            res => res,
        };
        T::from_lua(value, self)
    }

    /// Unloads module `modname`.
    ///
    /// This method does not support unloading binary Lua modules since they are internally cached
    /// and can be unloaded only by closing Lua state.
    ///
    /// This is similar to calling [`Lua::register_module`] with `Nil` value.
    ///
    /// [`package.loaded`]: https://www.lua.org/manual/5.4/manual.html#pdf-package.loaded
    pub fn unload_module(&self, modname: &str) -> Result<()> {
        self.register_module(modname, Nil)
    }

    // Executes module entrypoint function, which returns only one Value.
    // The returned value then pushed onto the stack.
    #[doc(hidden)]
    #[cfg(not(tarpaulin_include))]
    pub unsafe fn entrypoint<F, A, R>(state: *mut ffi::lua_State, func: F) -> c_int
    where
        F: FnOnce(&Lua, A) -> Result<R>,
        A: FromLuaMulti,
        R: IntoLua,
    {
        // Make sure that Lua is initialized
        let _ = Self::get_or_init_from_ptr(state);

        callback_error_ext(state, ptr::null_mut(), true, move |extra, nargs| {
            let rawlua = (*extra).raw_lua();
            let args = A::from_stack_args(nargs, 1, None, rawlua)?;
            func(rawlua.lua(), args)?.push_into_stack(rawlua)?;
            Ok(1)
        })
    }

    // A simple module entrypoint without arguments
    #[doc(hidden)]
    #[cfg(not(tarpaulin_include))]
    pub unsafe fn entrypoint1<F, R>(state: *mut ffi::lua_State, func: F) -> c_int
    where
        F: FnOnce(&Lua) -> Result<R>,
        R: IntoLua,
    {
        Self::entrypoint(state, move |lua, _: ()| func(lua))
    }

    /// Skips memory checks for some operations.
    #[doc(hidden)]
    #[cfg(feature = "module")]
    pub fn skip_memory_check(&self, skip: bool) {
        let lua = self.lock();
        unsafe { (*lua.extra.get()).skip_memory_check = skip };
    }

    /// Enables (or disables) sandbox mode on this Lua instance.
    ///
    /// This method, in particular:
    /// - Set all libraries to read-only
    /// - Set all builtin metatables to read-only
    /// - Set globals to read-only (and activates safeenv)
    /// - Setup local environment table that performs writes locally and proxies reads to the global
    ///   environment.
    /// - Allow only `count` mode in `collectgarbage` function.
    ///
    /// # Examples
    ///
    /// ```
    /// # use mlua::{Lua, Result};
    /// # #[cfg(feature = "luau")]
    /// # fn main() -> Result<()> {
    /// let lua = Lua::new();
    ///
    /// lua.sandbox(true)?;
    /// lua.load("var = 123").exec()?;
    /// assert_eq!(lua.globals().get::<u32>("var")?, 123);
    ///
    /// // Restore the global environment (clear changes made in sandbox)
    /// lua.sandbox(false)?;
    /// assert_eq!(lua.globals().get::<Option<u32>>("var")?, None);
    /// # Ok(())
    /// # }
    ///
    /// # #[cfg(not(feature = "luau"))]
    /// # fn main() {}
    /// ```
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn sandbox(&self, enabled: bool) -> Result<()> {
        let lua = self.lock();
        unsafe {
            if (*lua.extra.get()).sandboxed != enabled {
                let state = lua.main_state();
                check_stack(state, 3)?;
                protect_lua!(state, 0, 0, |state| {
                    if enabled {
                        ffi::luaL_sandbox(state, 1);
                        ffi::luaL_sandboxthread(state);
                    } else {
                        // Restore original `LUA_GLOBALSINDEX`
                        ffi::lua_xpush(lua.ref_thread(), state, ffi::LUA_GLOBALSINDEX);
                        ffi::lua_replace(state, ffi::LUA_GLOBALSINDEX);
                        ffi::luaL_sandbox(state, 0);
                    }
                })?;
                (*lua.extra.get()).sandboxed = enabled;
            }
            Ok(())
        }
    }

    /// Sets or replaces a global hook function that will periodically be called as Lua code
    /// executes.
    ///
    /// All new threads created (by mlua) after this call will use the global hook function.
    ///
    /// For more information see [`Lua::set_hook`].
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub fn set_global_hook<F>(&self, triggers: HookTriggers, callback: F) -> Result<()>
    where
        F: Fn(&Lua, &Debug) -> Result<VmState> + MaybeSend + 'static,
    {
        let lua = self.lock();
        unsafe {
            (*lua.extra.get()).hook_triggers = triggers;
            (*lua.extra.get()).hook_callback = Some(XRc::new(callback));
            lua.set_thread_hook(lua.state(), HookKind::Global)
        }
    }

    /// Sets a hook function that will periodically be called as Lua code executes.
    ///
    /// When exactly the hook function is called depends on the contents of the `triggers`
    /// parameter, see [`HookTriggers`] for more details.
    ///
    /// The provided hook function can error, and this error will be propagated through the Lua code
    /// that was executing at the time the hook was triggered. This can be used to implement a
    /// limited form of execution limits by setting [`HookTriggers.every_nth_instruction`] and
    /// erroring once an instruction limit has been reached.
    ///
    /// This method sets a hook function for the *current* thread of this Lua instance.
    /// If you want to set a hook function for another thread (coroutine), use
    /// [`Thread::set_hook`] instead.
    ///
    /// # Example
    ///
    /// Shows each line number of code being executed by the Lua interpreter.
    ///
    /// ```
    /// # use mlua::{Lua, HookTriggers, Result, VmState};
    /// # fn main() -> Result<()> {
    /// let lua = Lua::new();
    /// lua.set_hook(HookTriggers::EVERY_LINE, |_lua, debug| {
    ///     println!("line {:?}", debug.current_line());
    ///     Ok(VmState::Continue)
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
    /// [`HookTriggers.every_nth_instruction`]: crate::HookTriggers::every_nth_instruction
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub fn set_hook<F>(&self, triggers: HookTriggers, callback: F) -> Result<()>
    where
        F: Fn(&Lua, &Debug) -> Result<VmState> + MaybeSend + 'static,
    {
        let lua = self.lock();
        unsafe { lua.set_thread_hook(lua.state(), HookKind::Thread(triggers, XRc::new(callback))) }
    }

    /// Removes a global hook previously set by [`Lua::set_global_hook`].
    ///
    /// This function has no effect if a hook was not previously set.
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub fn remove_global_hook(&self) {
        let lua = self.lock();
        unsafe {
            (*lua.extra.get()).hook_callback = None;
            (*lua.extra.get()).hook_triggers = HookTriggers::default();
        }
    }

    /// Removes any hook from the current thread.
    ///
    /// This function has no effect if a hook was not previously set.
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub fn remove_hook(&self) {
        let lua = self.lock();
        unsafe {
            ffi::lua_sethook(lua.state(), None, 0, 0);
        }
    }

    /// Sets an interrupt function that will periodically be called by Luau VM.
    ///
    /// Any Luau code is guaranteed to call this handler "eventually"
    /// (in practice this can happen at any function call or at any loop iteration).
    ///
    /// The provided interrupt function can error, and this error will be propagated through
    /// the Luau code that was executing at the time the interrupt was triggered.
    /// Also this can be used to implement continuous execution limits by instructing Luau VM to
    /// yield by returning [`VmState::Yield`].
    ///
    /// This is similar to `Lua::set_hook` but in more simplified form.
    ///
    /// # Example
    ///
    /// Periodically yield Luau VM to suspend execution.
    ///
    /// ```
    /// # use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
    /// # use mlua::{Lua, Result, ThreadStatus, VmState};
    /// # #[cfg(feature = "luau")]
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
    ///     co.resume::<()>(())?;
    /// }
    /// # Ok(())
    /// # }
    ///
    /// # #[cfg(not(feature = "luau"))]
    /// # fn main() {}
    /// ```
    #[cfg(any(feature = "luau", doc))]
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
            let result = callback_error_ext(state, ptr::null_mut(), false, move |extra, _| {
                let interrupt_cb = (*extra).interrupt_callback.clone();
                let interrupt_cb = mlua_expect!(interrupt_cb, "no interrupt callback set in interrupt_proc");
                if XRc::strong_count(&interrupt_cb) > 2 {
                    return Ok(VmState::Continue); // Don't allow recursion
                }
                interrupt_cb((*extra).lua())
            });
            match result {
                VmState::Continue => {}
                VmState::Yield => {
                    ffi::lua_yield(state, 0);
                }
            }
        }

        // Set interrupt callback
        let lua = self.lock();
        unsafe {
            (*lua.extra.get()).interrupt_callback = Some(XRc::new(callback));
            (*ffi::lua_callbacks(lua.main_state())).interrupt = Some(interrupt_proc);
        }
    }

    /// Removes any interrupt function previously set by `set_interrupt`.
    ///
    /// This function has no effect if an 'interrupt' was not previously set.
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn remove_interrupt(&self) {
        let lua = self.lock();
        unsafe {
            (*lua.extra.get()).interrupt_callback = None;
            (*ffi::lua_callbacks(lua.main_state())).interrupt = None;
        }
    }

    /// Sets a thread creation callback that will be called when a thread is created.
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn set_thread_creation_callback<F>(&self, callback: F)
    where
        F: Fn(&Lua, Thread) -> Result<()> + MaybeSend + 'static,
    {
        let lua = self.lock();
        unsafe {
            (*lua.extra.get()).thread_creation_callback = Some(XRc::new(callback));
            (*ffi::lua_callbacks(lua.main_state())).userthread = Some(Self::userthread_proc);
        }
    }

    /// Sets a thread collection callback that will be called when a thread is destroyed.
    ///
    /// Luau GC does not support exceptions during collection, so the callback must be
    /// non-panicking. If the callback panics, the program will be aborted.
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn set_thread_collection_callback<F>(&self, callback: F)
    where
        F: Fn(crate::LightUserData) + MaybeSend + 'static,
    {
        let lua = self.lock();
        unsafe {
            (*lua.extra.get()).thread_collection_callback = Some(XRc::new(callback));
            (*ffi::lua_callbacks(lua.main_state())).userthread = Some(Self::userthread_proc);
        }
    }

    #[cfg(feature = "luau")]
    unsafe extern "C-unwind" fn userthread_proc(parent: *mut ffi::lua_State, child: *mut ffi::lua_State) {
        let extra = ExtraData::get(child);
        if !parent.is_null() {
            // Thread is created
            let callback = match (*extra).thread_creation_callback {
                Some(ref cb) => cb.clone(),
                None => return,
            };
            if XRc::strong_count(&callback) > 2 {
                return; // Don't allow recursion
            }
            ffi::lua_pushthread(child);
            ffi::lua_xmove(child, (*extra).ref_thread, 1);
            let value = Thread((*extra).raw_lua().pop_ref_thread(), child);
            callback_error_ext(parent, extra, false, move |extra, _| {
                callback((*extra).lua(), value)
            })
        } else {
            // Thread is about to be collected
            let callback = match (*extra).thread_collection_callback {
                Some(ref cb) => cb.clone(),
                None => return,
            };

            // We need to wrap the callback call in non-unwind function as it's not safe to unwind when
            // Luau GC is running.
            // This will trigger `abort()` if the callback panics.
            unsafe extern "C" fn run_callback(
                callback: *const crate::types::ThreadCollectionCallback,
                value: *mut ffi::lua_State,
            ) {
                (*callback)(crate::LightUserData(value as _));
            }

            (*extra).running_gc = true;
            run_callback(&callback, child);
            (*extra).running_gc = false;
        }
    }

    /// Removes any thread creation or collection callbacks previously set by
    /// [`Lua::set_thread_creation_callback`] or [`Lua::set_thread_collection_callback`].
    ///
    /// This function has no effect if a thread callbacks were not previously set.
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn remove_thread_callbacks(&self) {
        let lua = self.lock();
        unsafe {
            let extra = lua.extra.get();
            (*extra).thread_creation_callback = None;
            (*extra).thread_collection_callback = None;
            (*ffi::lua_callbacks(lua.main_state())).userthread = None;
        }
    }

    /// Sets the warning function to be used by Lua to emit warnings.
    #[cfg(feature = "lua54")]
    #[cfg_attr(docsrs, doc(cfg(feature = "lua54")))]
    pub fn set_warning_function<F>(&self, callback: F)
    where
        F: Fn(&Lua, &str, bool) -> Result<()> + MaybeSend + 'static,
    {
        use std::ffi::CStr;
        use std::os::raw::{c_char, c_void};
        use std::string::String as StdString;

        unsafe extern "C-unwind" fn warn_proc(ud: *mut c_void, msg: *const c_char, tocont: c_int) {
            let extra = ud as *mut ExtraData;
            callback_error_ext((*extra).raw_lua().state(), extra, false, |extra, _| {
                let warn_callback = (*extra).warn_callback.clone();
                let warn_callback = mlua_expect!(warn_callback, "no warning callback set in warn_proc");
                if XRc::strong_count(&warn_callback) > 2 {
                    return Ok(());
                }
                let msg = StdString::from_utf8_lossy(CStr::from_ptr(msg).to_bytes());
                warn_callback((*extra).lua(), &msg, tocont != 0)
            });
        }

        let lua = self.lock();
        unsafe {
            (*lua.extra.get()).warn_callback = Some(XRc::new(callback));
            ffi::lua_setwarnf(lua.state(), Some(warn_proc), lua.extra.get() as *mut c_void);
        }
    }

    /// Removes warning function previously set by `set_warning_function`.
    ///
    /// This function has no effect if a warning function was not previously set.
    #[cfg(feature = "lua54")]
    #[cfg_attr(docsrs, doc(cfg(feature = "lua54")))]
    pub fn remove_warning_function(&self) {
        let lua = self.lock();
        unsafe {
            (*lua.extra.get()).warn_callback = None;
            ffi::lua_setwarnf(lua.state(), None, ptr::null_mut());
        }
    }

    /// Emits a warning with the given message.
    ///
    /// A message in a call with `incomplete` set to `true` should be continued in
    /// another call to this function.
    #[cfg(feature = "lua54")]
    #[cfg_attr(docsrs, doc(cfg(feature = "lua54")))]
    pub fn warning(&self, msg: impl AsRef<str>, incomplete: bool) {
        let msg = msg.as_ref();
        let mut bytes = vec![0; msg.len() + 1];
        bytes[..msg.len()].copy_from_slice(msg.as_bytes());
        let real_len = bytes.iter().position(|&c| c == 0).unwrap();
        bytes.truncate(real_len);
        let lua = self.lock();
        unsafe {
            ffi::lua_warning(lua.state(), bytes.as_ptr() as *const _, incomplete as c_int);
        }
    }

    /// Gets information about the interpreter runtime stack at a given level.
    ///
    /// This function calls callback `f`, passing the [`Debug`] structure that can be used to get
    /// information about the function executing at a given level.
    /// Level `0` is the current running function, whereas level `n+1` is the function that has
    /// called level `n` (except for tail calls, which do not count in the stack).
    pub fn inspect_stack<R>(&self, level: usize, f: impl FnOnce(&Debug) -> R) -> Option<R> {
        let lua = self.lock();
        unsafe {
            let mut ar = mem::zeroed::<ffi::lua_Debug>();
            let level = level as c_int;
            #[cfg(not(feature = "luau"))]
            if ffi::lua_getstack(lua.state(), level, &mut ar) == 0 {
                return None;
            }
            #[cfg(feature = "luau")]
            if ffi::lua_getinfo(lua.state(), level, cstr!(""), &mut ar) == 0 {
                return None;
            }

            Some(f(&Debug::new(&lua, level, &mut ar)))
        }
    }

    /// Returns the amount of memory (in bytes) currently used inside this Lua state.
    pub fn used_memory(&self) -> usize {
        let lua = self.lock();
        let state = lua.main_state();
        unsafe {
            match MemoryState::get(state) {
                mem_state if !mem_state.is_null() => (*mem_state).used_memory(),
                _ => {
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
    /// Once an allocation occurs that would pass this memory limit, a `Error::MemoryError` is
    /// generated instead.
    /// Returns previous limit (zero means no limit).
    ///
    /// Does not work in module mode where Lua state is managed externally.
    pub fn set_memory_limit(&self, limit: usize) -> Result<usize> {
        let lua = self.lock();
        unsafe {
            match MemoryState::get(lua.state()) {
                mem_state if !mem_state.is_null() => Ok((*mem_state).set_memory_limit(limit)),
                _ => Err(Error::MemoryControlNotAvailable),
            }
        }
    }

    /// Returns `true` if the garbage collector is currently running automatically.
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luau"))]
    pub fn gc_is_running(&self) -> bool {
        let lua = self.lock();
        unsafe { ffi::lua_gc(lua.main_state(), ffi::LUA_GCISRUNNING, 0) != 0 }
    }

    /// Stop the Lua GC from running
    pub fn gc_stop(&self) {
        let lua = self.lock();
        unsafe { ffi::lua_gc(lua.main_state(), ffi::LUA_GCSTOP, 0) };
    }

    /// Restarts the Lua GC if it is not running
    pub fn gc_restart(&self) {
        let lua = self.lock();
        unsafe { ffi::lua_gc(lua.main_state(), ffi::LUA_GCRESTART, 0) };
    }

    /// Perform a full garbage-collection cycle.
    ///
    /// It may be necessary to call this function twice to collect all currently unreachable
    /// objects. Once to finish the current gc cycle, and once to start and finish the next cycle.
    pub fn gc_collect(&self) -> Result<()> {
        let lua = self.lock();
        let state = lua.main_state();
        unsafe {
            check_stack(state, 2)?;
            protect_lua!(state, 0, 0, fn(state) ffi::lua_gc(state, ffi::LUA_GCCOLLECT, 0))
        }
    }

    /// Steps the garbage collector one indivisible step.
    ///
    /// Returns `true` if this has finished a collection cycle.
    pub fn gc_step(&self) -> Result<bool> {
        self.gc_step_kbytes(0)
    }

    /// Steps the garbage collector as though memory had been allocated.
    ///
    /// if `kbytes` is 0, then this is the same as calling `gc_step`. Returns true if this step has
    /// finished a collection cycle.
    pub fn gc_step_kbytes(&self, kbytes: c_int) -> Result<bool> {
        let lua = self.lock();
        let state = lua.main_state();
        unsafe {
            check_stack(state, 3)?;
            protect_lua!(state, 0, 0, |state| {
                ffi::lua_gc(state, ffi::LUA_GCSTEP, kbytes) != 0
            })
        }
    }

    /// Sets the `pause` value of the collector.
    ///
    /// Returns the previous value of `pause`. More information can be found in the Lua
    /// [documentation].
    ///
    /// For Luau this parameter sets GC goal
    ///
    /// [documentation]: https://www.lua.org/manual/5.4/manual.html#2.5
    pub fn gc_set_pause(&self, pause: c_int) -> c_int {
        let lua = self.lock();
        let state = lua.main_state();
        unsafe {
            #[cfg(not(feature = "luau"))]
            return ffi::lua_gc(state, ffi::LUA_GCSETPAUSE, pause);
            #[cfg(feature = "luau")]
            return ffi::lua_gc(state, ffi::LUA_GCSETGOAL, pause);
        }
    }

    /// Sets the `step multiplier` value of the collector.
    ///
    /// Returns the previous value of the `step multiplier`. More information can be found in the
    /// Lua [documentation].
    ///
    /// [documentation]: https://www.lua.org/manual/5.4/manual.html#2.5
    pub fn gc_set_step_multiplier(&self, step_multiplier: c_int) -> c_int {
        let lua = self.lock();
        unsafe { ffi::lua_gc(lua.main_state(), ffi::LUA_GCSETSTEPMUL, step_multiplier) }
    }

    /// Changes the collector to incremental mode with the given parameters.
    ///
    /// Returns the previous mode (always `GCMode::Incremental` in Lua < 5.4).
    /// More information can be found in the Lua [documentation].
    ///
    /// [documentation]: https://www.lua.org/manual/5.4/manual.html#2.5.1
    pub fn gc_inc(&self, pause: c_int, step_multiplier: c_int, step_size: c_int) -> GCMode {
        let lua = self.lock();
        let state = lua.main_state();

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
        let prev_mode = unsafe { ffi::lua_gc(state, ffi::LUA_GCINC, pause, step_multiplier, step_size) };
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
    /// [lua_doc]: https://www.lua.org/manual/5.4/manual.html#2.5.2
    #[cfg(feature = "lua54")]
    #[cfg_attr(docsrs, doc(cfg(feature = "lua54")))]
    pub fn gc_gen(&self, minor_multiplier: c_int, major_multiplier: c_int) -> GCMode {
        let lua = self.lock();
        let state = lua.main_state();
        let prev_mode = unsafe { ffi::lua_gc(state, ffi::LUA_GCGEN, minor_multiplier, major_multiplier) };
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
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn set_compiler(&self, compiler: Compiler) {
        let lua = self.lock();
        unsafe { (*lua.extra.get()).compiler = Some(compiler) };
    }

    /// Toggles JIT compilation mode for new chunks of code.
    ///
    /// By default JIT is enabled. Changing this option does not have any effect on
    /// already loaded functions.
    #[cfg(any(feature = "luau-jit", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau-jit")))]
    pub fn enable_jit(&self, enable: bool) {
        let lua = self.lock();
        unsafe { (*lua.extra.get()).enable_jit = enable };
    }

    /// Sets Luau feature flag (global setting).
    ///
    /// See https://github.com/luau-lang/luau/blob/master/CONTRIBUTING.md#feature-flags for details.
    #[cfg(feature = "luau")]
    #[doc(hidden)]
    #[allow(clippy::result_unit_err)]
    pub fn set_fflag(name: &str, enabled: bool) -> StdResult<(), ()> {
        if let Ok(name) = std::ffi::CString::new(name) {
            if unsafe { ffi::luau_setfflag(name.as_ptr(), enabled as c_int) != 0 } {
                return Ok(());
            }
        }
        Err(())
    }

    /// Returns Lua source code as a `Chunk` builder type.
    ///
    /// In order to actually compile or run the resulting code, you must call [`Chunk::exec`] or
    /// similar on the returned builder. Code is not even parsed until one of these methods is
    /// called.
    ///
    /// [`Chunk::exec`]: crate::Chunk::exec
    #[track_caller]
    pub fn load<'a>(&self, chunk: impl AsChunk + 'a) -> Chunk<'a> {
        self.load_with_location(chunk, Location::caller())
    }

    pub(crate) fn load_with_location<'a>(
        &self,
        chunk: impl AsChunk + 'a,
        location: &'static Location<'static>,
    ) -> Chunk<'a> {
        Chunk {
            lua: self.weak(),
            name: chunk
                .name()
                .unwrap_or_else(|| format!("@{}:{}", location.file(), location.line())),
            env: chunk.environment(self),
            mode: chunk.mode(),
            source: chunk.source(),
            #[cfg(feature = "luau")]
            compiler: unsafe { (*self.lock().extra.get()).compiler.clone() },
        }
    }

    /// Create and return an interned Lua string.
    ///
    /// Lua strings can be arbitrary `[u8]` data including embedded nulls, so in addition to `&str`
    /// and `&String`, you can also pass plain `&[u8]` here.
    #[inline]
    pub fn create_string(&self, s: impl AsRef<[u8]>) -> Result<String> {
        unsafe { self.lock().create_string(s) }
    }

    /// Create and return a Luau [buffer] object from a byte slice of data.
    ///
    /// [buffer]: https://luau.org/library#buffer-library
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn create_buffer(&self, buf: impl AsRef<[u8]>) -> Result<Buffer> {
        let lua = self.lock();
        let state = lua.state();
        unsafe {
            if lua.unlikely_memory_error() {
                crate::util::push_buffer(state, buf.as_ref(), false)?;
                return Ok(Buffer(lua.pop_ref()));
            }

            let _sg = StackGuard::new(state);
            check_stack(state, 3)?;
            crate::util::push_buffer(state, buf.as_ref(), true)?;
            Ok(Buffer(lua.pop_ref()))
        }
    }

    /// Creates and returns a new empty table.
    #[inline]
    pub fn create_table(&self) -> Result<Table> {
        self.create_table_with_capacity(0, 0)
    }

    /// Creates and returns a new empty table, with the specified capacity.
    ///
    /// - `narr` is a hint for how many elements the table will have as a sequence.
    /// - `nrec` is a hint for how many other elements the table will have.
    ///
    /// Lua may use these hints to preallocate memory for the new table.
    pub fn create_table_with_capacity(&self, narr: usize, nrec: usize) -> Result<Table> {
        unsafe { self.lock().create_table_with_capacity(narr, nrec) }
    }

    /// Creates a table and fills it with values from an iterator.
    pub fn create_table_from<K, V>(&self, iter: impl IntoIterator<Item = (K, V)>) -> Result<Table>
    where
        K: IntoLua,
        V: IntoLua,
    {
        unsafe { self.lock().create_table_from(iter) }
    }

    /// Creates a table from an iterator of values, using `1..` as the keys.
    pub fn create_sequence_from<T>(&self, iter: impl IntoIterator<Item = T>) -> Result<Table>
    where
        T: IntoLua,
    {
        unsafe { self.lock().create_sequence_from(iter) }
    }

    /// Wraps a Rust function or closure, creating a callable Lua function handle to it.
    ///
    /// The function's return value is always a `Result`: If the function returns `Err`, the error
    /// is raised as a Lua error, which can be caught using `(x)pcall` or bubble up to the Rust code
    /// that invoked the Lua code. This allows using the `?` operator to propagate errors through
    /// intermediate Lua code.
    ///
    /// If the function returns `Ok`, the contained value will be converted to one or more Lua
    /// values. For details on Rust-to-Lua conversions, refer to the [`IntoLua`] and
    /// [`IntoLuaMulti`] traits.
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
    pub fn create_function<F, A, R>(&self, func: F) -> Result<Function>
    where
        F: Fn(&Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        (self.lock()).create_callback(Box::new(move |rawlua, nargs| unsafe {
            let args = A::from_stack_args(nargs, 1, None, rawlua)?;
            func(rawlua.lua(), args)?.push_into_stack_multi(rawlua)
        }))
    }

    /// Wraps a Rust mutable closure, creating a callable Lua function handle to it.
    ///
    /// This is a version of [`Lua::create_function`] that accepts a `FnMut` argument.
    pub fn create_function_mut<F, A, R>(&self, func: F) -> Result<Function>
    where
        F: FnMut(&Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let func = RefCell::new(func);
        self.create_function(move |lua, args| {
            (*func.try_borrow_mut().map_err(|_| Error::RecursiveMutCallback)?)(lua, args)
        })
    }

    /// Wraps a C function, creating a callable Lua function handle to it.
    ///
    /// # Safety
    /// This function is unsafe because provides a way to execute unsafe C function.
    pub unsafe fn create_c_function(&self, func: ffi::lua_CFunction) -> Result<Function> {
        let lua = self.lock();
        if cfg!(any(feature = "lua54", feature = "lua53", feature = "lua52")) {
            ffi::lua_pushcfunction(lua.ref_thread(), func);
            return Ok(Function(lua.pop_ref_thread()));
        }

        // Lua <5.2 requires memory allocation to push a C function
        let state = lua.state();
        {
            let _sg = StackGuard::new(state);
            check_stack(state, 3)?;

            if lua.unlikely_memory_error() {
                ffi::lua_pushcfunction(state, func);
            } else {
                protect_lua!(state, 0, 1, |state| ffi::lua_pushcfunction(state, func))?;
            }
            Ok(Function(lua.pop_ref()))
        }
    }

    /// Wraps a Rust async function or closure, creating a callable Lua function handle to it.
    ///
    /// While executing the function Rust will poll the Future and if the result is not ready,
    /// call `yield()` passing internal representation of a `Poll::Pending` value.
    ///
    /// The function must be called inside Lua coroutine ([`Thread`]) to be able to suspend its
    /// execution. An executor should be used to poll [`AsyncThread`] and mlua will take a provided
    /// Waker in that case. Otherwise noop waker will be used if try to call the function outside of
    /// Rust executors.
    ///
    /// The family of `call_async()` functions takes care about creating [`Thread`].
    ///
    /// # Examples
    ///
    /// Non blocking sleep:
    ///
    /// ```
    /// use std::time::Duration;
    /// use mlua::{Lua, Result};
    ///
    /// async fn sleep(_lua: Lua, n: u64) -> Result<&'static str> {
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
    /// [`AsyncThread`]: crate::AsyncThread
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn create_async_function<F, A, FR, R>(&self, func: F) -> Result<Function>
    where
        F: Fn(Lua, A) -> FR + MaybeSend + 'static,
        A: FromLuaMulti,
        FR: Future<Output = Result<R>> + MaybeSend + 'static,
        R: IntoLuaMulti,
    {
        // In future we should switch to async closures when they are stable to capture `&Lua`
        // See https://rust-lang.github.io/rfcs/3668-async-closures.html
        (self.lock()).create_async_callback(Box::new(move |rawlua, nargs| unsafe {
            let args = match A::from_stack_args(nargs, 1, None, rawlua) {
                Ok(args) => args,
                Err(e) => return Box::pin(future::ready(Err(e))),
            };
            let lua = rawlua.lua();
            let fut = func(lua.clone(), args);
            Box::pin(async move { fut.await?.push_into_stack_multi(lua.raw_lua()) })
        }))
    }

    /// Wraps a Lua function into a new thread (or coroutine).
    ///
    /// Equivalent to `coroutine.create`.
    pub fn create_thread(&self, func: Function) -> Result<Thread> {
        unsafe { self.lock().create_thread(&func) }
    }

    /// Creates a Lua userdata object from a custom userdata type.
    ///
    /// All userdata instances of the same type `T` shares the same metatable.
    #[inline]
    pub fn create_userdata<T>(&self, data: T) -> Result<AnyUserData>
    where
        T: UserData + MaybeSend + 'static,
    {
        unsafe { self.lock().make_userdata(UserDataStorage::new(data)) }
    }

    /// Creates a Lua userdata object from a custom serializable userdata type.
    #[cfg(feature = "serde")]
    #[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
    #[inline]
    pub fn create_ser_userdata<T>(&self, data: T) -> Result<AnyUserData>
    where
        T: UserData + Serialize + MaybeSend + 'static,
    {
        unsafe { self.lock().make_userdata(UserDataStorage::new_ser(data)) }
    }

    /// Creates a Lua userdata object from a custom Rust type.
    ///
    /// You can register the type using [`Lua::register_userdata_type`] to add fields or methods
    /// _before_ calling this method.
    /// Otherwise, the userdata object will have an empty metatable.
    ///
    /// All userdata instances of the same type `T` shares the same metatable.
    #[inline]
    pub fn create_any_userdata<T>(&self, data: T) -> Result<AnyUserData>
    where
        T: MaybeSend + 'static,
    {
        unsafe { self.lock().make_any_userdata(UserDataStorage::new(data)) }
    }

    /// Creates a Lua userdata object from a custom serializable Rust type.
    ///
    /// See [`Lua::create_any_userdata`] for more details.
    #[cfg(feature = "serde")]
    #[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
    #[inline]
    pub fn create_ser_any_userdata<T>(&self, data: T) -> Result<AnyUserData>
    where
        T: Serialize + MaybeSend + 'static,
    {
        unsafe { (self.lock()).make_any_userdata(UserDataStorage::new_ser(data)) }
    }

    /// Registers a custom Rust type in Lua to use in userdata objects.
    ///
    /// This methods provides a way to add fields or methods to userdata objects of a type `T`.
    pub fn register_userdata_type<T: 'static>(&self, f: impl FnOnce(&mut UserDataRegistry<T>)) -> Result<()> {
        let type_id = TypeId::of::<T>();
        let mut registry = UserDataRegistry::new(self);
        f(&mut registry);

        let lua = self.lock();
        unsafe {
            // Deregister the type if it already registered
            if let Some(table_id) = (*lua.extra.get()).registered_userdata_t.remove(&type_id) {
                ffi::luaL_unref(lua.state(), ffi::LUA_REGISTRYINDEX, table_id);
            }

            // Add to "pending" registration map
            ((*lua.extra.get()).pending_userdata_reg).insert(type_id, registry.into_raw());
        }
        Ok(())
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
    ///     fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
    ///         fields.add_field_method_get("val", |_, this| Ok(this.0));
    ///     }
    ///
    ///     fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
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
        let ud = UserDataProxy::<T>(PhantomData);
        unsafe { self.lock().make_userdata(UserDataStorage::new(ud)) }
    }

    /// Sets the metatable for a Lua builtin type.
    ///
    /// The metatable will be shared by all values of the given type.
    ///
    /// # Examples
    ///
    /// Change metatable for Lua boolean type:
    ///
    /// ```
    /// # use mlua::{Lua, Result, Function};
    /// # fn main() -> Result<()> {
    /// # let lua = Lua::new();
    /// let mt = lua.create_table()?;
    /// mt.set("__tostring", lua.create_function(|_, b: bool| Ok(if b { "2" } else { "0" }))?)?;
    /// lua.set_type_metatable::<bool>(Some(mt));
    /// lua.load("assert(tostring(true) == '2')").exec()?;
    /// # Ok(())
    /// # }
    /// ```
    #[allow(private_bounds)]
    pub fn set_type_metatable<T: LuaType>(&self, metatable: Option<Table>) {
        let lua = self.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            assert_stack(state, 2);

            match T::TYPE_ID {
                ffi::LUA_TBOOLEAN => {
                    ffi::lua_pushboolean(state, 0);
                }
                ffi::LUA_TLIGHTUSERDATA => {
                    ffi::lua_pushlightuserdata(state, ptr::null_mut());
                }
                ffi::LUA_TNUMBER => {
                    ffi::lua_pushnumber(state, 0.);
                }
                #[cfg(feature = "luau")]
                ffi::LUA_TVECTOR => {
                    #[cfg(not(feature = "luau-vector4"))]
                    ffi::lua_pushvector(state, 0., 0., 0.);
                    #[cfg(feature = "luau-vector4")]
                    ffi::lua_pushvector(state, 0., 0., 0., 0.);
                }
                ffi::LUA_TSTRING => {
                    ffi::lua_pushstring(state, b"\0" as *const u8 as *const _);
                }
                ffi::LUA_TFUNCTION => match self.load("function() end").eval::<Function>() {
                    Ok(func) => lua.push_ref(&func.0),
                    Err(_) => return,
                },
                ffi::LUA_TTHREAD => {
                    ffi::lua_pushthread(state);
                }
                #[cfg(feature = "luau")]
                ffi::LUA_TBUFFER => {
                    ffi::lua_newbuffer(state, 0);
                }
                _ => return,
            }
            match metatable {
                Some(metatable) => lua.push_ref(&metatable.0),
                None => ffi::lua_pushnil(state),
            }
            ffi::lua_setmetatable(state, -2);
        }
    }

    /// Returns a handle to the global environment.
    pub fn globals(&self) -> Table {
        let lua = self.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            assert_stack(state, 1);
            #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
            ffi::lua_rawgeti(state, ffi::LUA_REGISTRYINDEX, ffi::LUA_RIDX_GLOBALS);
            #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
            ffi::lua_pushvalue(state, ffi::LUA_GLOBALSINDEX);
            Table(lua.pop_ref())
        }
    }

    /// Sets the global environment.
    ///
    /// This will replace the current global environment with the provided `globals` table.
    ///
    /// For Lua 5.2+ the globals table is stored in the registry and shared between all threads.
    /// For Lua 5.1 and Luau the globals table is stored in each thread.
    ///
    /// Please note that any existing Lua functions have cached global environment and will not
    /// see the changes made by this method.
    /// To update the environment for existing Lua functions, use [`Function::set_environment`].
    pub fn set_globals(&self, globals: Table) -> Result<()> {
        let lua = self.lock();
        let state = lua.state();
        unsafe {
            #[cfg(feature = "luau")]
            if (*lua.extra.get()).sandboxed {
                return Err(Error::runtime("cannot change globals in a sandboxed Lua state"));
            }

            let _sg = StackGuard::new(state);
            check_stack(state, 1)?;

            lua.push_ref(&globals.0);

            #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
            ffi::lua_rawseti(state, ffi::LUA_REGISTRYINDEX, ffi::LUA_RIDX_GLOBALS);
            #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
            ffi::lua_replace(state, ffi::LUA_GLOBALSINDEX);
        }

        Ok(())
    }

    /// Returns a handle to the active `Thread`.
    ///
    /// For calls to `Lua` this will be the main Lua thread, for parameters given to a callback,
    /// this will be whatever Lua thread called the callback.
    pub fn current_thread(&self) -> Thread {
        let lua = self.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            assert_stack(state, 1);
            ffi::lua_pushthread(state);
            Thread(lua.pop_ref(), state)
        }
    }

    /// Calls the given function with a [`Scope`] parameter, giving the function the ability to
    /// create userdata and callbacks from Rust types that are `!Send` or non-`'static`.
    ///
    /// The lifetime of any function or userdata created through [`Scope`] lasts only until the
    /// completion of this method call, on completion all such created values are automatically
    /// dropped and Lua references to them are invalidated. If a script accesses a value created
    /// through [`Scope`] outside of this method, a Lua error will result. Since we can ensure the
    /// lifetime of values created through [`Scope`], and we know that [`Lua`] cannot be sent to
    /// another thread while [`Scope`] is live, it is safe to allow `!Send` data types and whose
    /// lifetimes only outlive the scope lifetime.
    pub fn scope<'env, R>(
        &self,
        f: impl for<'scope> FnOnce(&'scope Scope<'scope, 'env>) -> Result<R>,
    ) -> Result<R> {
        f(&Scope::new(self.lock_arc()))
    }

    /// Attempts to coerce a Lua value into a String in a manner consistent with Lua's internal
    /// behavior.
    ///
    /// To succeed, the value must be a string (in which case this is a no-op), an integer, or a
    /// number.
    pub fn coerce_string(&self, v: Value) -> Result<Option<String>> {
        Ok(match v {
            Value::String(s) => Some(s),
            v => unsafe {
                let lua = self.lock();
                let state = lua.state();
                let _sg = StackGuard::new(state);
                check_stack(state, 4)?;

                lua.push_value(&v)?;
                let res = if lua.unlikely_memory_error() {
                    ffi::lua_tolstring(state, -1, ptr::null_mut())
                } else {
                    protect_lua!(state, 1, 1, |state| {
                        ffi::lua_tolstring(state, -1, ptr::null_mut())
                    })?
                };
                if !res.is_null() {
                    Some(String(lua.pop_ref()))
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
                let lua = self.lock();
                let state = lua.state();
                let _sg = StackGuard::new(state);
                check_stack(state, 2)?;

                lua.push_value(&v)?;
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
                let lua = self.lock();
                let state = lua.state();
                let _sg = StackGuard::new(state);
                check_stack(state, 2)?;

                lua.push_value(&v)?;
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

    /// Converts a value that implements [`IntoLua`] into a [`Value`] instance.
    #[inline]
    pub fn pack(&self, t: impl IntoLua) -> Result<Value> {
        t.into_lua(self)
    }

    /// Converts a [`Value`] instance into a value that implements [`FromLua`].
    #[inline]
    pub fn unpack<T: FromLua>(&self, value: Value) -> Result<T> {
        T::from_lua(value, self)
    }

    /// Converts a value that implements [`IntoLua`] into a [`FromLua`] variant.
    #[inline]
    pub fn convert<U: FromLua>(&self, value: impl IntoLua) -> Result<U> {
        U::from_lua(value.into_lua(self)?, self)
    }

    /// Converts a value that implements [`IntoLuaMulti`] into a [`MultiValue`] instance.
    #[inline]
    pub fn pack_multi(&self, t: impl IntoLuaMulti) -> Result<MultiValue> {
        t.into_lua_multi(self)
    }

    /// Converts a [`MultiValue`] instance into a value that implements [`FromLuaMulti`].
    #[inline]
    pub fn unpack_multi<T: FromLuaMulti>(&self, value: MultiValue) -> Result<T> {
        T::from_lua_multi(value, self)
    }

    /// Set a value in the Lua registry based on a string key.
    ///
    /// This value will be available to Rust from all Lua instances which share the same main
    /// state.
    pub fn set_named_registry_value(&self, key: &str, t: impl IntoLua) -> Result<()> {
        let lua = self.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 5)?;

            lua.push(t)?;
            rawset_field(state, ffi::LUA_REGISTRYINDEX, key)
        }
    }

    /// Get a value from the Lua registry based on a string key.
    ///
    /// Any Lua instance which shares the underlying main state may call this method to
    /// get a value previously set by [`Lua::set_named_registry_value`].
    pub fn named_registry_value<T>(&self, key: &str) -> Result<T>
    where
        T: FromLua,
    {
        let lua = self.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 3)?;

            let protect = !lua.unlikely_memory_error();
            push_string(state, key.as_bytes(), protect)?;
            ffi::lua_rawget(state, ffi::LUA_REGISTRYINDEX);

            T::from_stack(-1, &lua)
        }
    }

    /// Removes a named value in the Lua registry.
    ///
    /// Equivalent to calling [`Lua::set_named_registry_value`] with a value of [`Nil`].
    #[inline]
    pub fn unset_named_registry_value(&self, key: &str) -> Result<()> {
        self.set_named_registry_value(key, Nil)
    }

    /// Place a value in the Lua registry with an auto-generated key.
    ///
    /// This value will be available to Rust from all Lua instances which share the same main
    /// state.
    ///
    /// Be warned, garbage collection of values held inside the registry is not automatic, see
    /// [`RegistryKey`] for more details.
    /// However, dropped [`RegistryKey`]s automatically reused to store new values.
    pub fn create_registry_value(&self, t: impl IntoLua) -> Result<RegistryKey> {
        let lua = self.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 4)?;

            lua.push(t)?;

            let unref_list = (*lua.extra.get()).registry_unref_list.clone();

            // Check if the value is nil (no need to store it in the registry)
            if ffi::lua_isnil(state, -1) != 0 {
                return Ok(RegistryKey::new(ffi::LUA_REFNIL, unref_list));
            }

            // Try to reuse previously allocated slot
            let free_registry_id = unref_list.lock().as_mut().and_then(|x| x.pop());
            if let Some(registry_id) = free_registry_id {
                // It must be safe to replace the value without triggering memory error
                ffi::lua_rawseti(state, ffi::LUA_REGISTRYINDEX, registry_id as Integer);
                return Ok(RegistryKey::new(registry_id, unref_list));
            }

            // Allocate a new RegistryKey slot
            let registry_id = if lua.unlikely_memory_error() {
                ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX)
            } else {
                protect_lua!(state, 1, 0, |state| {
                    ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX)
                })?
            };
            Ok(RegistryKey::new(registry_id, unref_list))
        }
    }

    /// Get a value from the Lua registry by its [`RegistryKey`]
    ///
    /// Any Lua instance which shares the underlying main state may call this method to get a value
    /// previously placed by [`Lua::create_registry_value`].
    pub fn registry_value<T: FromLua>(&self, key: &RegistryKey) -> Result<T> {
        let lua = self.lock();
        if !lua.owns_registry_value(key) {
            return Err(Error::MismatchedRegistryKey);
        }

        let state = lua.state();
        match key.id() {
            ffi::LUA_REFNIL => T::from_lua(Value::Nil, self),
            registry_id => unsafe {
                let _sg = StackGuard::new(state);
                check_stack(state, 1)?;

                ffi::lua_rawgeti(state, ffi::LUA_REGISTRYINDEX, registry_id as Integer);
                T::from_stack(-1, &lua)
            },
        }
    }

    /// Removes a value from the Lua registry.
    ///
    /// You may call this function to manually remove a value placed in the registry with
    /// [`Lua::create_registry_value`]. In addition to manual [`RegistryKey`] removal, you can also
    /// call [`Lua::expire_registry_values`] to automatically remove values from the registry
    /// whose [`RegistryKey`]s have been dropped.
    pub fn remove_registry_value(&self, key: RegistryKey) -> Result<()> {
        let lua = self.lock();
        if !lua.owns_registry_value(&key) {
            return Err(Error::MismatchedRegistryKey);
        }

        unsafe { ffi::luaL_unref(lua.state(), ffi::LUA_REGISTRYINDEX, key.take()) };
        Ok(())
    }

    /// Replaces a value in the Lua registry by its [`RegistryKey`].
    ///
    /// An identifier used in [`RegistryKey`] may possibly be changed to a new value.
    ///
    /// See [`Lua::create_registry_value`] for more details.
    pub fn replace_registry_value(&self, key: &mut RegistryKey, t: impl IntoLua) -> Result<()> {
        let lua = self.lock();
        if !lua.owns_registry_value(key) {
            return Err(Error::MismatchedRegistryKey);
        }

        let t = t.into_lua(self)?;

        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 2)?;

            match (t, key.id()) {
                (Value::Nil, ffi::LUA_REFNIL) => {
                    // Do nothing, no need to replace nil with nil
                }
                (Value::Nil, registry_id) => {
                    // Remove the value
                    ffi::luaL_unref(state, ffi::LUA_REGISTRYINDEX, registry_id);
                    key.set_id(ffi::LUA_REFNIL);
                }
                (value, ffi::LUA_REFNIL) => {
                    // Allocate a new `RegistryKey`
                    let new_key = self.create_registry_value(value)?;
                    key.set_id(new_key.take());
                }
                (value, registry_id) => {
                    // It must be safe to replace the value without triggering memory error
                    lua.push_value(&value)?;
                    ffi::lua_rawseti(state, ffi::LUA_REGISTRYINDEX, registry_id as Integer);
                }
            }
        }
        Ok(())
    }

    /// Returns true if the given [`RegistryKey`] was created by a Lua which shares the
    /// underlying main state with this Lua instance.
    ///
    /// Other than this, methods that accept a [`RegistryKey`] will return
    /// [`Error::MismatchedRegistryKey`] if passed a [`RegistryKey`] that was not created with a
    /// matching [`Lua`] state.
    #[inline]
    pub fn owns_registry_value(&self, key: &RegistryKey) -> bool {
        self.lock().owns_registry_value(key)
    }

    /// Remove any registry values whose [`RegistryKey`]s have all been dropped.
    ///
    /// Unlike normal handle values, [`RegistryKey`]s do not automatically remove themselves on
    /// Drop, but you can call this method to remove any unreachable registry values not
    /// manually removed by [`Lua::remove_registry_value`].
    pub fn expire_registry_values(&self) {
        let lua = self.lock();
        let state = lua.state();
        unsafe {
            let mut unref_list = (*lua.extra.get()).registry_unref_list.lock();
            let unref_list = unref_list.replace(Vec::new());
            for id in mlua_expect!(unref_list, "unref list is not set") {
                ffi::luaL_unref(state, ffi::LUA_REGISTRYINDEX, id);
            }
        }
    }

    /// Sets or replaces an application data object of type `T`.
    ///
    /// Application data could be accessed at any time by using [`Lua::app_data_ref`] or
    /// [`Lua::app_data_mut`] methods where `T` is the data type.
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
    ///     lua.create_function(hello)?.call::<()>(())?;
    ///     let s = lua.app_data_ref::<&str>().unwrap();
    ///     assert_eq!(*s, "world");
    ///     Ok(())
    /// }
    /// ```
    #[track_caller]
    pub fn set_app_data<T: MaybeSend + 'static>(&self, data: T) -> Option<T> {
        let lua = self.lock();
        let extra = unsafe { &*lua.extra.get() };
        extra.app_data.insert(data)
    }

    /// Tries to set or replace an application data object of type `T`.
    ///
    /// Returns:
    /// - `Ok(Some(old_data))` if the data object of type `T` was successfully replaced.
    /// - `Ok(None)` if the data object of type `T` was successfully inserted.
    /// - `Err(data)` if the data object of type `T` was not inserted because the container is
    ///   currently borrowed.
    ///
    /// See [`Lua::set_app_data`] for examples.
    pub fn try_set_app_data<T: MaybeSend + 'static>(&self, data: T) -> StdResult<Option<T>, T> {
        let lua = self.lock();
        let extra = unsafe { &*lua.extra.get() };
        extra.app_data.try_insert(data)
    }

    /// Gets a reference to an application data object stored by [`Lua::set_app_data`] of type
    /// `T`.
    ///
    /// # Panics
    ///
    /// Panics if the data object of type `T` is currently mutably borrowed. Multiple immutable
    /// reads can be taken out at the same time.
    #[track_caller]
    pub fn app_data_ref<T: 'static>(&self) -> Option<AppDataRef<'_, T>> {
        let guard = self.lock_arc();
        let extra = unsafe { &*guard.extra.get() };
        extra.app_data.borrow(Some(guard))
    }

    /// Tries to get a reference to an application data object stored by [`Lua::set_app_data`] of
    /// type `T`.
    pub fn try_app_data_ref<T: 'static>(&self) -> StdResult<Option<AppDataRef<'_, T>>, BorrowError> {
        let guard = self.lock_arc();
        let extra = unsafe { &*guard.extra.get() };
        extra.app_data.try_borrow(Some(guard))
    }

    /// Gets a mutable reference to an application data object stored by [`Lua::set_app_data`] of
    /// type `T`.
    ///
    /// # Panics
    ///
    /// Panics if the data object of type `T` is currently borrowed.
    #[track_caller]
    pub fn app_data_mut<T: 'static>(&self) -> Option<AppDataRefMut<'_, T>> {
        let guard = self.lock_arc();
        let extra = unsafe { &*guard.extra.get() };
        extra.app_data.borrow_mut(Some(guard))
    }

    /// Tries to get a mutable reference to an application data object stored by
    /// [`Lua::set_app_data`] of type `T`.
    pub fn try_app_data_mut<T: 'static>(&self) -> StdResult<Option<AppDataRefMut<'_, T>>, BorrowMutError> {
        let guard = self.lock_arc();
        let extra = unsafe { &*guard.extra.get() };
        extra.app_data.try_borrow_mut(Some(guard))
    }

    /// Removes an application data of type `T`.
    ///
    /// # Panics
    ///
    /// Panics if the app data container is currently borrowed.
    #[track_caller]
    pub fn remove_app_data<T: 'static>(&self) -> Option<T> {
        let lua = self.lock();
        let extra = unsafe { &*lua.extra.get() };
        extra.app_data.remove()
    }

    /// Returns an internal `Poll::Pending` constant used for executing async callbacks.
    ///
    /// Every time when [`Future`] is Pending, Lua corotine is suspended with this constant.
    #[cfg(feature = "async")]
    #[doc(hidden)]
    #[inline(always)]
    pub fn poll_pending() -> LightUserData {
        static ASYNC_POLL_PENDING: u8 = 0;
        LightUserData(&ASYNC_POLL_PENDING as *const u8 as *mut std::os::raw::c_void)
    }

    #[cfg(feature = "async")]
    #[inline(always)]
    pub(crate) fn poll_terminate() -> LightUserData {
        static ASYNC_POLL_TERMINATE: u8 = 0;
        LightUserData(&ASYNC_POLL_TERMINATE as *const u8 as *mut std::os::raw::c_void)
    }

    /// Returns a weak reference to the Lua instance.
    ///
    /// This is useful for creating a reference to the Lua instance that does not prevent it from
    /// being deallocated.
    #[inline(always)]
    pub fn weak(&self) -> WeakLua {
        WeakLua(XRc::downgrade(&self.raw))
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
        searchers.raw_set(3, loader)?;
        if searchers.raw_len() >= 4 {
            searchers.raw_remove(4)?;
        }

        Ok(())
    }

    #[inline(always)]
    pub(crate) fn lock(&self) -> ReentrantMutexGuard<'_, RawLua> {
        let rawlua = self.raw.lock();
        #[cfg(feature = "luau")]
        if unsafe { (*rawlua.extra.get()).running_gc } {
            panic!("Luau VM is suspended while GC is running");
        }
        rawlua
    }

    #[inline(always)]
    pub(crate) fn lock_arc(&self) -> LuaGuard {
        LuaGuard(self.raw.lock_arc())
    }

    /// Returns a handle to the unprotected Lua state without any synchronization.
    ///
    /// This is useful where we know that the lock is already held by the caller.
    #[cfg(feature = "async")]
    #[inline(always)]
    pub(crate) unsafe fn raw_lua(&self) -> &RawLua {
        &*self.raw.data_ptr()
    }
}

impl WeakLua {
    #[track_caller]
    #[inline(always)]
    pub(crate) fn lock(&self) -> LuaGuard {
        let guard = LuaGuard::new(self.0.upgrade().expect("Lua instance is destroyed"));
        #[cfg(feature = "luau")]
        if unsafe { (*guard.extra.get()).running_gc } {
            panic!("Luau VM is suspended while GC is running");
        }
        guard
    }

    #[inline(always)]
    pub(crate) fn try_lock(&self) -> Option<LuaGuard> {
        Some(LuaGuard::new(self.0.upgrade()?))
    }

    /// Upgrades the weak Lua reference to a strong reference.
    ///
    /// # Panics
    ///
    /// Panics if the Lua instance is destroyed.
    #[track_caller]
    #[inline(always)]
    pub fn upgrade(&self) -> Lua {
        Lua {
            raw: self.0.upgrade().expect("Lua instance is destroyed"),
            collect_garbage: false,
        }
    }

    /// Tries to upgrade the weak Lua reference to a strong reference.
    ///
    /// Returns `None` if the Lua instance is destroyed.
    #[inline(always)]
    pub fn try_upgrade(&self) -> Option<Lua> {
        Some(Lua {
            raw: self.0.upgrade()?,
            collect_garbage: false,
        })
    }
}

impl PartialEq for WeakLua {
    fn eq(&self, other: &Self) -> bool {
        XWeak::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for WeakLua {}

impl LuaGuard {
    #[cfg(feature = "send")]
    pub(crate) fn new(handle: XRc<ReentrantMutex<RawLua>>) -> Self {
        LuaGuard(handle.lock_arc())
    }

    #[cfg(not(feature = "send"))]
    pub(crate) fn new(handle: XRc<ReentrantMutex<RawLua>>) -> Self {
        LuaGuard(handle.into_lock_arc())
    }
}

impl Deref for LuaGuard {
    type Target = RawLua;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub(crate) mod extra;
mod raw;
pub(crate) mod util;

#[cfg(test)]
mod assertions {
    use super::*;

    // Lua has lots of interior mutability, should not be RefUnwindSafe
    static_assertions::assert_not_impl_any!(Lua: std::panic::RefUnwindSafe);

    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_any!(Lua: Send);
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(Lua: Send, Sync);
}
