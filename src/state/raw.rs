use std::any::TypeId;
use std::cell::{Cell, UnsafeCell};
use std::ffi::CStr;
use std::mem;
use std::os::raw::{c_char, c_int, c_void};
use std::panic::resume_unwind;
use std::ptr::{self, NonNull};
use std::sync::Arc;

use crate::chunk::ChunkMode;
use crate::error::{Error, Result};
use crate::function::Function;
use crate::memory::{MemoryState, ALLOCATOR};
use crate::state::util::callback_error_ext;
use crate::stdlib::StdLib;
use crate::string::String;
use crate::table::Table;
use crate::thread::Thread;
use crate::traits::IntoLua;
use crate::types::{
    AppDataRef, AppDataRefMut, Callback, CallbackUpvalue, DestructedUserdata, Integer, LightUserData,
    MaybeSend, ReentrantMutex, RegistryKey, ValueRef, XRc,
};
use crate::userdata::{
    init_userdata_metatable, AnyUserData, MetaMethod, RawUserDataRegistry, UserData, UserDataRegistry,
    UserDataStorage,
};
use crate::util::{
    assert_stack, check_stack, get_destructed_userdata_metatable, get_internal_userdata, get_main_state,
    get_metatable_ptr, get_userdata, init_error_registry, init_internal_metatable, pop_error,
    push_internal_userdata, push_string, push_table, rawset_field, safe_pcall, safe_xpcall, short_type_name,
    StackGuard, WrappedFailure,
};
use crate::value::{Nil, Value};

use super::extra::ExtraData;
use super::{Lua, LuaOptions, WeakLua};

#[cfg(not(feature = "luau"))]
use crate::{
    debug::Debug,
    types::{HookCallback, HookKind, VmState},
};

#[cfg(feature = "async")]
use {
    crate::multi::MultiValue,
    crate::traits::FromLuaMulti,
    crate::types::{AsyncCallback, AsyncCallbackUpvalue, AsyncPollUpvalue},
    std::task::{Context, Poll, Waker},
};

/// An inner Lua struct which holds a raw Lua state.
#[doc(hidden)]
pub struct RawLua {
    // The state is dynamic and depends on context
    pub(super) state: Cell<*mut ffi::lua_State>,
    pub(super) main_state: Option<NonNull<ffi::lua_State>>,
    pub(super) extra: XRc<UnsafeCell<ExtraData>>,
    owned: bool,
}

impl Drop for RawLua {
    fn drop(&mut self) {
        unsafe {
            if !self.owned {
                return;
            }

            let mem_state = MemoryState::get(self.main_state());

            #[cfg(feature = "luau")]
            {
                // Reset any callbacks
                (*ffi::lua_callbacks(self.main_state())).interrupt = None;
                (*ffi::lua_callbacks(self.main_state())).userthread = None;
            }

            ffi::lua_close(self.main_state());

            // Deallocate `MemoryState`
            if !mem_state.is_null() {
                drop(Box::from_raw(mem_state));
            }
        }
    }
}

#[cfg(feature = "send")]
unsafe impl Send for RawLua {}

impl RawLua {
    #[inline(always)]
    pub(crate) fn lua(&self) -> &Lua {
        unsafe { (*self.extra.get()).lua() }
    }

    #[inline(always)]
    pub(crate) fn weak(&self) -> &WeakLua {
        unsafe { (*self.extra.get()).weak() }
    }

    /// Returns a pointer to the current Lua state.
    ///
    /// The pointer refers to the active Lua coroutine and depends on the context.
    #[inline(always)]
    pub fn state(&self) -> *mut ffi::lua_State {
        self.state.get()
    }

    #[inline(always)]
    pub(crate) fn main_state(&self) -> *mut ffi::lua_State {
        self.main_state
            .map(|state| state.as_ptr())
            .unwrap_or_else(|| self.state())
    }

    #[inline(always)]
    pub(crate) fn ref_thread(&self) -> *mut ffi::lua_State {
        unsafe { (*self.extra.get()).ref_thread }
    }

    pub(super) unsafe fn new(libs: StdLib, options: &LuaOptions) -> XRc<ReentrantMutex<Self>> {
        let mem_state: *mut MemoryState = Box::into_raw(Box::default());
        let mut state = ffi::lua_newstate(ALLOCATOR, mem_state as *mut c_void);
        // If state is null then switch to Lua internal allocator
        if state.is_null() {
            drop(Box::from_raw(mem_state));
            state = ffi::luaL_newstate();
        }
        assert!(!state.is_null(), "Failed to create a Lua VM");

        ffi::luaL_requiref(state, cstr!("_G"), ffi::luaopen_base, 1);
        ffi::lua_pop(state, 1);

        // Init Luau code generator (jit)
        #[cfg(feature = "luau-jit")]
        if ffi::luau_codegen_supported() != 0 {
            ffi::luau_codegen_create(state);
        }

        let rawlua = Self::init_from_ptr(state, true);
        let extra = rawlua.lock().extra.get();

        mlua_expect!(
            load_std_libs(state, libs),
            "Error during loading standard libraries"
        );
        (*extra).libs |= libs;

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

        rawlua
    }

    pub(super) unsafe fn init_from_ptr(state: *mut ffi::lua_State, owned: bool) -> XRc<ReentrantMutex<Self>> {
        assert!(!state.is_null(), "Lua state is NULL");
        if let Some(lua) = Self::try_from_ptr(state) {
            return lua;
        }

        let main_state = get_main_state(state).unwrap_or(state);
        let main_state_top = ffi::lua_gettop(main_state);

        mlua_expect!(
            (|state| {
                init_error_registry(state)?;

                // Create the internal metatables and store them in the registry
                // to prevent from being garbage collected.

                init_internal_metatable::<XRc<UnsafeCell<ExtraData>>>(state, None)?;
                init_internal_metatable::<Callback>(state, None)?;
                init_internal_metatable::<CallbackUpvalue>(state, None)?;
                #[cfg(not(feature = "luau"))]
                init_internal_metatable::<HookCallback>(state, None)?;
                #[cfg(feature = "async")]
                {
                    init_internal_metatable::<AsyncCallback>(state, None)?;
                    init_internal_metatable::<AsyncCallbackUpvalue>(state, None)?;
                    init_internal_metatable::<AsyncPollUpvalue>(state, None)?;
                    init_internal_metatable::<Option<Waker>>(state, None)?;
                }

                // Init serde metatables
                #[cfg(feature = "serde")]
                crate::serde::init_metatables(state)?;

                Ok::<_, Error>(())
            })(main_state),
            "Error during Lua initialization",
        );

        // Init ExtraData
        let extra = ExtraData::init(main_state, owned);

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

        #[allow(clippy::arc_with_non_send_sync)]
        let rawlua = XRc::new(ReentrantMutex::new(RawLua {
            state: Cell::new(state),
            // Make sure that we don't store current state as main state (if it's not available)
            main_state: get_main_state(state).and_then(NonNull::new),
            extra: XRc::clone(&extra),
            owned,
        }));
        (*extra.get()).set_lua(&rawlua);
        if owned {
            // If Lua state is managed by us, then make internal `RawLua` reference "weak"
            XRc::decrement_strong_count(XRc::as_ptr(&rawlua));
        } else {
            // If Lua state is not managed by us, then keep internal `RawLua` reference "strong"
            // but `Extra` reference weak (it will be collected from registry at lua_close time)
            XRc::decrement_strong_count(XRc::as_ptr(&extra));
        }

        rawlua
    }

    unsafe fn try_from_ptr(state: *mut ffi::lua_State) -> Option<XRc<ReentrantMutex<Self>>> {
        match ExtraData::get(state) {
            extra if extra.is_null() => None,
            extra => Some(XRc::clone(&(*extra).lua().raw)),
        }
    }

    /// Marks the Lua state as safe.
    #[inline(always)]
    pub(super) fn mark_safe(&self) {
        unsafe { (*self.extra.get()).safe = true };
    }

    /// Loads the specified subset of the standard libraries into an existing Lua state.
    ///
    /// Use the [`StdLib`] flags to specify the libraries you want to load.
    ///
    /// [`StdLib`]: crate::StdLib
    pub(super) unsafe fn load_std_libs(&self, libs: StdLib) -> Result<()> {
        let is_safe = (*self.extra.get()).safe;

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

        let res = load_std_libs(self.main_state(), libs);

        // If `package` library loaded into a safe lua state then disable C modules
        #[cfg(not(feature = "luau"))]
        if is_safe {
            let curr_libs = (*self.extra.get()).libs;
            if (curr_libs ^ (curr_libs | libs)).contains(StdLib::PACKAGE) {
                mlua_expect!(self.lua().disable_c_modules(), "Error disabling C modules");
            }
        }
        #[cfg(feature = "luau")]
        let _ = is_safe;
        unsafe { (*self.extra.get()).libs |= libs };

        res
    }

    /// Private version of [`Lua::try_set_app_data`]
    #[inline]
    pub(crate) fn set_priv_app_data<T: MaybeSend + 'static>(&self, data: T) -> Option<T> {
        let extra = unsafe { &*self.extra.get() };
        extra.app_data_priv.insert(data)
    }

    /// Private version of [`Lua::app_data_ref`]
    #[track_caller]
    #[inline]
    pub(crate) fn priv_app_data_ref<T: 'static>(&self) -> Option<AppDataRef<'_, T>> {
        let extra = unsafe { &*self.extra.get() };
        extra.app_data_priv.borrow(None)
    }

    /// Private version of [`Lua::app_data_mut`]
    #[track_caller]
    #[inline]
    pub(crate) fn priv_app_data_mut<T: 'static>(&self) -> Option<AppDataRefMut<'_, T>> {
        let extra = unsafe { &*self.extra.get() };
        extra.app_data_priv.borrow_mut(None)
    }

    /// See [`Lua::create_registry_value`]
    #[inline]
    pub(crate) fn owns_registry_value(&self, key: &RegistryKey) -> bool {
        let registry_unref_list = unsafe { &(*self.extra.get()).registry_unref_list };
        Arc::ptr_eq(&key.unref_list, registry_unref_list)
    }

    pub(crate) fn load_chunk(
        &self,
        name: Option<&CStr>,
        env: Option<&Table>,
        mode: Option<ChunkMode>,
        source: &[u8],
    ) -> Result<Function> {
        let state = self.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 3)?;

            let name = name.map(CStr::as_ptr).unwrap_or(ptr::null());
            let mode = match mode {
                Some(ChunkMode::Binary) => cstr!("b"),
                Some(ChunkMode::Text) => cstr!("t"),
                None => cstr!("bt"),
            };
            let status = if self.unlikely_memory_error() {
                self.load_chunk_inner(state, name, env, mode, source)
            } else {
                // Luau and Lua 5.2 can trigger an exception during chunk loading
                protect_lua!(state, 0, 1, |state| {
                    self.load_chunk_inner(state, name, env, mode, source)
                })?
            };
            match status {
                ffi::LUA_OK => Ok(Function(self.pop_ref())),
                err => Err(pop_error(state, err)),
            }
        }
    }

    pub(crate) unsafe fn load_chunk_inner(
        &self,
        state: *mut ffi::lua_State,
        name: *const c_char,
        env: Option<&Table>,
        mode: *const c_char,
        source: &[u8],
    ) -> c_int {
        let status = ffi::luaL_loadbufferenv(
            state,
            source.as_ptr() as *const c_char,
            source.len(),
            name,
            mode,
            match env {
                Some(env) => {
                    self.push_ref(&env.0);
                    -1
                }
                _ => 0,
            },
        );
        #[cfg(feature = "luau-jit")]
        if status == ffi::LUA_OK {
            if (*self.extra.get()).enable_jit && ffi::luau_codegen_supported() != 0 {
                ffi::luau_codegen_compile(state, -1);
            }
        }
        status
    }

    /// Sets a hook for a thread (coroutine).
    #[cfg(not(feature = "luau"))]
    pub(crate) unsafe fn set_thread_hook(
        &self,
        thread_state: *mut ffi::lua_State,
        hook: HookKind,
    ) -> Result<()> {
        // Key to store hooks in the registry
        const HOOKS_KEY: *const c_char = cstr!("__mlua_hooks");

        unsafe fn process_status(state: *mut ffi::lua_State, event: c_int, status: VmState) {
            match status {
                VmState::Continue => {}
                VmState::Yield => {
                    // Only count and line events can yield
                    if event == ffi::LUA_HOOKCOUNT || event == ffi::LUA_HOOKLINE {
                        #[cfg(any(feature = "lua54", feature = "lua53"))]
                        if ffi::lua_isyieldable(state) != 0 {
                            ffi::lua_yield(state, 0);
                        }
                        #[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
                        {
                            ffi::lua_pushliteral(state, c"attempt to yield from a hook");
                            ffi::lua_error(state);
                        }
                    }
                }
            }
        }

        unsafe extern "C-unwind" fn global_hook_proc(state: *mut ffi::lua_State, ar: *mut ffi::lua_Debug) {
            let status = callback_error_ext(state, ptr::null_mut(), false, move |extra, _| {
                match (*extra).hook_callback.clone() {
                    Some(hook_callback) => {
                        let rawlua = (*extra).raw_lua();
                        let debug = Debug::new(rawlua, 0, ar);
                        hook_callback((*extra).lua(), &debug)
                    }
                    None => {
                        ffi::lua_sethook(state, None, 0, 0);
                        Ok(VmState::Continue)
                    }
                }
            });
            process_status(state, (*ar).event, status);
        }

        unsafe extern "C-unwind" fn hook_proc(state: *mut ffi::lua_State, ar: *mut ffi::lua_Debug) {
            let top = ffi::lua_gettop(state);
            let mut hook_callback_ptr = ptr::null();
            ffi::luaL_checkstack(state, 3, ptr::null());
            if ffi::lua_getfield(state, ffi::LUA_REGISTRYINDEX, HOOKS_KEY) == ffi::LUA_TTABLE {
                ffi::lua_pushthread(state);
                if ffi::lua_rawget(state, -2) == ffi::LUA_TUSERDATA {
                    hook_callback_ptr = get_internal_userdata::<HookCallback>(state, -1, ptr::null());
                }
            }
            ffi::lua_settop(state, top);
            if hook_callback_ptr.is_null() {
                ffi::lua_sethook(state, None, 0, 0);
                return;
            }

            let status = callback_error_ext(state, ptr::null_mut(), false, |extra, _| {
                let rawlua = (*extra).raw_lua();
                let debug = Debug::new(rawlua, 0, ar);
                let hook_callback = (*hook_callback_ptr).clone();
                hook_callback((*extra).lua(), &debug)
            });
            process_status(state, (*ar).event, status)
        }

        let (triggers, callback) = match hook {
            HookKind::Global if (*self.extra.get()).hook_callback.is_none() => {
                return Ok(());
            }
            HookKind::Global => {
                let triggers = (*self.extra.get()).hook_triggers;
                let (mask, count) = (triggers.mask(), triggers.count());
                ffi::lua_sethook(thread_state, Some(global_hook_proc), mask, count);
                return Ok(());
            }
            HookKind::Thread(triggers, callback) => (triggers, callback),
        };

        // Hooks for threads stored in the registry (in a weak table)
        let state = self.state();
        let _sg = StackGuard::new(state);
        check_stack(state, 3)?;
        protect_lua!(state, 0, 0, |state| {
            if ffi::luaL_getsubtable(state, ffi::LUA_REGISTRYINDEX, HOOKS_KEY) == 0 {
                // Table just created, initialize it
                ffi::lua_pushliteral(state, c"k");
                ffi::lua_setfield(state, -2, cstr!("__mode")); // hooktable.__mode = "k"
                ffi::lua_pushvalue(state, -1);
                ffi::lua_setmetatable(state, -2); // metatable(hooktable) = hooktable
            }

            ffi::lua_pushthread(thread_state);
            ffi::lua_xmove(thread_state, state, 1); // key (thread)
            let _ = push_internal_userdata(state, callback, false); // value (hook callback)
            ffi::lua_rawset(state, -3); // hooktable[thread] = hook callback
        })?;

        ffi::lua_sethook(thread_state, Some(hook_proc), triggers.mask(), triggers.count());

        Ok(())
    }

    /// See [`Lua::create_string`]
    pub(crate) unsafe fn create_string(&self, s: impl AsRef<[u8]>) -> Result<String> {
        let state = self.state();
        if self.unlikely_memory_error() {
            push_string(state, s.as_ref(), false)?;
            return Ok(String(self.pop_ref()));
        }

        let _sg = StackGuard::new(state);
        check_stack(state, 3)?;
        push_string(state, s.as_ref(), true)?;
        Ok(String(self.pop_ref()))
    }

    /// See [`Lua::create_table_with_capacity`]
    pub(crate) unsafe fn create_table_with_capacity(&self, narr: usize, nrec: usize) -> Result<Table> {
        let state = self.state();
        if self.unlikely_memory_error() {
            push_table(state, narr, nrec, false)?;
            return Ok(Table(self.pop_ref()));
        }

        let _sg = StackGuard::new(state);
        check_stack(state, 3)?;
        push_table(state, narr, nrec, true)?;
        Ok(Table(self.pop_ref()))
    }

    /// See [`Lua::create_table_from`]
    pub(crate) unsafe fn create_table_from<I, K, V>(&self, iter: I) -> Result<Table>
    where
        I: IntoIterator<Item = (K, V)>,
        K: IntoLua,
        V: IntoLua,
    {
        let state = self.state();
        let _sg = StackGuard::new(state);
        check_stack(state, 6)?;

        let iter = iter.into_iter();
        let lower_bound = iter.size_hint().0;
        let protect = !self.unlikely_memory_error();
        push_table(state, 0, lower_bound, protect)?;
        for (k, v) in iter {
            self.push(k)?;
            self.push(v)?;
            if protect {
                protect_lua!(state, 3, 1, fn(state) ffi::lua_rawset(state, -3))?;
            } else {
                ffi::lua_rawset(state, -3);
            }
        }

        Ok(Table(self.pop_ref()))
    }

    /// See [`Lua::create_sequence_from`]
    pub(crate) unsafe fn create_sequence_from<T, I>(&self, iter: I) -> Result<Table>
    where
        T: IntoLua,
        I: IntoIterator<Item = T>,
    {
        let state = self.state();
        let _sg = StackGuard::new(state);
        check_stack(state, 5)?;

        let iter = iter.into_iter();
        let lower_bound = iter.size_hint().0;
        let protect = !self.unlikely_memory_error();
        push_table(state, lower_bound, 0, protect)?;
        for (i, v) in iter.enumerate() {
            self.push(v)?;
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

    /// Wraps a Lua function into a new thread (or coroutine).
    ///
    /// Takes function by reference.
    pub(crate) unsafe fn create_thread(&self, func: &Function) -> Result<Thread> {
        let state = self.state();
        let _sg = StackGuard::new(state);
        check_stack(state, 3)?;

        let protect = !self.unlikely_memory_error();
        #[cfg(feature = "luau")]
        let protect = protect || (*self.extra.get()).thread_creation_callback.is_some();

        let thread_state = if !protect {
            ffi::lua_newthread(state)
        } else {
            protect_lua!(state, 0, 1, |state| ffi::lua_newthread(state))?
        };

        // Inherit global hook if set
        #[cfg(not(feature = "luau"))]
        self.set_thread_hook(thread_state, HookKind::Global)?;

        let thread = Thread(self.pop_ref(), thread_state);
        ffi::lua_xpush(self.ref_thread(), thread_state, func.0.index);
        Ok(thread)
    }

    /// Wraps a Lua function into a new or recycled thread (coroutine).
    #[cfg(feature = "async")]
    pub(crate) unsafe fn create_recycled_thread(&self, func: &Function) -> Result<Thread> {
        if let Some(index) = (*self.extra.get()).thread_pool.pop() {
            let thread_state = ffi::lua_tothread(self.ref_thread(), index);
            ffi::lua_xpush(self.ref_thread(), thread_state, func.0.index);

            #[cfg(feature = "luau")]
            {
                // Inherit `LUA_GLOBALSINDEX` from the caller
                ffi::lua_xpush(self.state(), thread_state, ffi::LUA_GLOBALSINDEX);
                ffi::lua_replace(thread_state, ffi::LUA_GLOBALSINDEX);
            }

            return Ok(Thread(ValueRef::new(self, index), thread_state));
        }

        self.create_thread(func)
    }

    /// Returns the thread to the pool for later use.
    #[cfg(feature = "async")]
    pub(crate) unsafe fn recycle_thread(&self, thread: &mut Thread) {
        let extra = &mut *self.extra.get();
        if extra.thread_pool.len() < extra.thread_pool.capacity() {
            extra.thread_pool.push(thread.0.index);
            thread.0.drop = false; // Prevent thread from being garbage collected
        }
    }

    /// Pushes a value that implements `IntoLua` onto the Lua stack.
    ///
    /// Uses up to 2 stack spaces to push a single value, does not call `checkstack`.
    #[inline(always)]
    pub(crate) unsafe fn push(&self, value: impl IntoLua) -> Result<()> {
        value.push_into_stack(self)
    }

    /// Pushes a `Value` (by reference) onto the Lua stack.
    ///
    /// Uses 2 stack spaces, does not call `checkstack`.
    pub(crate) unsafe fn push_value(&self, value: &Value) -> Result<()> {
        let state = self.state();
        match value {
            Value::Nil => ffi::lua_pushnil(state),
            Value::Boolean(b) => ffi::lua_pushboolean(state, *b as c_int),
            Value::LightUserData(ud) => ffi::lua_pushlightuserdata(state, ud.0),
            Value::Integer(i) => ffi::lua_pushinteger(state, *i),
            Value::Number(n) => ffi::lua_pushnumber(state, *n),
            #[cfg(feature = "luau")]
            Value::Vector(v) => {
                #[cfg(not(feature = "luau-vector4"))]
                ffi::lua_pushvector(state, v.x(), v.y(), v.z());
                #[cfg(feature = "luau-vector4")]
                ffi::lua_pushvector(state, v.x(), v.y(), v.z(), v.w());
            }
            Value::String(s) => self.push_ref(&s.0),
            Value::Table(t) => self.push_ref(&t.0),
            Value::Function(f) => self.push_ref(&f.0),
            Value::Thread(t) => self.push_ref(&t.0),
            Value::UserData(ud) => self.push_ref(&ud.0),
            #[cfg(feature = "luau")]
            Value::Buffer(buf) => self.push_ref(&buf.0),
            Value::Error(err) => {
                let protect = !self.unlikely_memory_error();
                push_internal_userdata(state, WrappedFailure::Error(*err.clone()), protect)?;
            }
            Value::Other(vref) => self.push_ref(vref),
        }
        Ok(())
    }

    /// Pops a value from the Lua stack.
    ///
    /// Uses up to 1 stack spaces, does not call `checkstack`.
    pub(crate) unsafe fn pop_value(&self) -> Value {
        let value = self.stack_value(-1, None);
        ffi::lua_pop(self.state(), 1);
        value
    }

    /// Returns value at given stack index without popping it.
    ///
    /// Uses up to 1 stack spaces, does not call `checkstack`.
    pub(crate) unsafe fn stack_value(&self, idx: c_int, type_hint: Option<c_int>) -> Value {
        let state = self.state();
        match type_hint.unwrap_or_else(|| ffi::lua_type(state, idx)) {
            ffi::LUA_TNIL => Nil,

            ffi::LUA_TBOOLEAN => Value::Boolean(ffi::lua_toboolean(state, idx) != 0),

            ffi::LUA_TLIGHTUSERDATA => Value::LightUserData(LightUserData(ffi::lua_touserdata(state, idx))),

            #[cfg(any(feature = "lua54", feature = "lua53"))]
            ffi::LUA_TNUMBER => {
                if ffi::lua_isinteger(state, idx) != 0 {
                    Value::Integer(ffi::lua_tointeger(state, idx))
                } else {
                    Value::Number(ffi::lua_tonumber(state, idx))
                }
            }

            #[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit", feature = "luau"))]
            ffi::LUA_TNUMBER => {
                use crate::types::Number;

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
                return Value::Vector(crate::Vector([*v, *v.add(1), *v.add(2)]));
                #[cfg(feature = "luau-vector4")]
                return Value::Vector(crate::Vector([*v, *v.add(1), *v.add(2), *v.add(3)]));
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
                // If the userdata is `WrappedFailure`, process it as an error or panic.
                let failure_mt_ptr = (*self.extra.get()).wrapped_failure_mt_ptr;
                match get_internal_userdata::<WrappedFailure>(state, idx, failure_mt_ptr).as_mut() {
                    Some(WrappedFailure::Error(err)) => Value::Error(Box::new(err.clone())),
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
                let thread_state = ffi::lua_tothread(self.ref_thread(), -1);
                Value::Thread(Thread(self.pop_ref_thread(), thread_state))
            }

            #[cfg(feature = "luau")]
            ffi::LUA_TBUFFER => {
                ffi::lua_xpush(state, self.ref_thread(), idx);
                Value::Buffer(crate::Buffer(self.pop_ref_thread()))
            }

            _ => {
                ffi::lua_xpush(state, self.ref_thread(), idx);
                Value::Other(self.pop_ref_thread())
            }
        }
    }

    // Pushes a ValueRef value onto the stack, uses 1 stack space, does not call checkstack
    #[inline]
    pub(crate) fn push_ref(&self, vref: &ValueRef) {
        assert!(
            self.weak() == &vref.lua,
            "Lua instance passed Value created from a different main Lua state"
        );
        unsafe { ffi::lua_xpush(self.ref_thread(), self.state(), vref.index) };
    }

    // Pops the topmost element of the stack and stores a reference to it. This pins the object,
    // preventing garbage collection until the returned `ValueRef` is dropped.
    //
    // References are stored on the stack of a specially created auxiliary thread that exists only
    // to store reference values. This is much faster than storing these in the registry, and also
    // much more flexible and requires less bookkeeping than storing them directly in the currently
    // used stack.
    #[inline]
    pub(crate) unsafe fn pop_ref(&self) -> ValueRef {
        ffi::lua_xmove(self.state(), self.ref_thread(), 1);
        let index = (*self.extra.get()).ref_stack_pop();
        ValueRef::new(self, index)
    }

    // Same as `pop_ref` but assumes the value is already on the reference thread
    #[inline]
    pub(crate) unsafe fn pop_ref_thread(&self) -> ValueRef {
        let index = (*self.extra.get()).ref_stack_pop();
        ValueRef::new(self, index)
    }

    #[inline]
    pub(crate) unsafe fn clone_ref(&self, vref: &ValueRef) -> ValueRef {
        ffi::lua_pushvalue(self.ref_thread(), vref.index);
        let index = (*self.extra.get()).ref_stack_pop();
        ValueRef::new(self, index)
    }

    pub(crate) unsafe fn drop_ref(&self, vref: &ValueRef) {
        let ref_thread = self.ref_thread();
        mlua_debug_assert!(
            ffi::lua_gettop(ref_thread) >= vref.index,
            "GC finalizer is not allowed in ref_thread"
        );
        ffi::lua_pushnil(ref_thread);
        ffi::lua_replace(ref_thread, vref.index);
        (*self.extra.get()).ref_free.push(vref.index);
    }

    #[inline]
    pub(crate) unsafe fn push_error_traceback(&self) {
        let state = self.state();
        #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
        ffi::lua_xpush(self.ref_thread(), state, ExtraData::ERROR_TRACEBACK_IDX);
        // Lua 5.2+ support light C functions that does not require extra allocations
        #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
        ffi::lua_pushcfunction(state, crate::util::error_traceback);
    }

    #[inline]
    pub(crate) unsafe fn unlikely_memory_error(&self) -> bool {
        #[cfg(debug_assertions)]
        if cfg!(force_memory_limit) {
            return false;
        }

        // MemoryInfo is empty in module mode so we cannot predict memory limits
        match MemoryState::get(self.state()) {
            mem_state if !mem_state.is_null() => (*mem_state).memory_limit() == 0,
            _ => (*self.extra.get()).skip_memory_check, // Check the special flag (only for module mode)
        }
    }

    pub(crate) unsafe fn make_userdata<T>(&self, data: UserDataStorage<T>) -> Result<AnyUserData>
    where
        T: UserData + 'static,
    {
        self.make_userdata_with_metatable(data, || {
            // Check if userdata/metatable is already registered
            let type_id = TypeId::of::<T>();
            if let Some(&table_id) = (*self.extra.get()).registered_userdata_t.get(&type_id) {
                return Ok(table_id as Integer);
            }

            // Create a new metatable from `UserData` definition
            let mut registry = UserDataRegistry::new(self.lua());
            T::register(&mut registry);

            self.create_userdata_metatable(registry.into_raw())
        })
    }

    pub(crate) unsafe fn make_any_userdata<T>(&self, data: UserDataStorage<T>) -> Result<AnyUserData>
    where
        T: 'static,
    {
        self.make_userdata_with_metatable(data, || {
            // Check if userdata/metatable is already registered
            let type_id = TypeId::of::<T>();
            if let Some(&table_id) = (*self.extra.get()).registered_userdata_t.get(&type_id) {
                return Ok(table_id as Integer);
            }

            // Check if metatable creation is pending or create an empty metatable otherwise
            let registry = match (*self.extra.get()).pending_userdata_reg.remove(&type_id) {
                Some(registry) => registry,
                None => UserDataRegistry::<T>::new(self.lua()).into_raw(),
            };
            self.create_userdata_metatable(registry)
        })
    }

    unsafe fn make_userdata_with_metatable<T>(
        &self,
        data: UserDataStorage<T>,
        get_metatable_id: impl FnOnce() -> Result<Integer>,
    ) -> Result<AnyUserData> {
        let state = self.state();
        let _sg = StackGuard::new(state);
        check_stack(state, 3)?;

        // We generate metatable first to make sure it *always* available when userdata pushed
        let mt_id = get_metatable_id()?;
        let protect = !self.unlikely_memory_error();
        crate::util::push_userdata(state, data, protect)?;
        ffi::lua_rawgeti(state, ffi::LUA_REGISTRYINDEX, mt_id);
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

    pub(crate) unsafe fn create_userdata_metatable(&self, registry: RawUserDataRegistry) -> Result<Integer> {
        let state = self.state();
        let type_id = registry.type_id;

        self.push_userdata_metatable(registry)?;

        let mt_ptr = ffi::lua_topointer(state, -1);
        let id = protect_lua!(state, 1, 0, |state| {
            ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX)
        })?;

        if let Some(type_id) = type_id {
            (*self.extra.get()).registered_userdata_t.insert(type_id, id);
        }
        self.register_userdata_metatable(mt_ptr, type_id);

        Ok(id as Integer)
    }

    pub(crate) unsafe fn push_userdata_metatable(&self, mut registry: RawUserDataRegistry) -> Result<()> {
        let state = self.state();
        let mut stack_guard = StackGuard::new(state);
        check_stack(state, 13)?;

        // Prepare metatable, add meta methods first and then meta fields
        let metatable_nrec = registry.meta_methods.len() + registry.meta_fields.len();
        #[cfg(feature = "async")]
        let metatable_nrec = metatable_nrec + registry.async_meta_methods.len();
        push_table(state, 0, metatable_nrec, true)?;
        for (k, m) in registry.meta_methods {
            self.push(self.create_callback(m)?)?;
            rawset_field(state, -2, MetaMethod::validate(&k)?)?;
        }
        #[cfg(feature = "async")]
        for (k, m) in registry.async_meta_methods {
            self.push(self.create_async_callback(m)?)?;
            rawset_field(state, -2, MetaMethod::validate(&k)?)?;
        }
        let mut has_name = false;
        for (k, v) in registry.meta_fields {
            has_name = has_name || k == MetaMethod::Type;
            v?.push_into_stack(self)?;
            rawset_field(state, -2, MetaMethod::validate(&k)?)?;
        }
        // Set `__name/__type` if not provided
        if !has_name {
            let type_name = registry.type_name;
            push_string(state, type_name.as_bytes(), !self.unlikely_memory_error())?;
            rawset_field(state, -2, MetaMethod::Type.name())?;
        }
        let metatable_index = ffi::lua_absindex(state, -1);

        let fields_nrec = registry.fields.len();
        if fields_nrec > 0 {
            // If `__index` is a table then update it in-place
            let index_type = ffi::lua_getfield(state, metatable_index, cstr!("__index"));
            match index_type {
                ffi::LUA_TNIL | ffi::LUA_TTABLE => {
                    if index_type == ffi::LUA_TNIL {
                        // Create a new table
                        ffi::lua_pop(state, 1);
                        push_table(state, 0, fields_nrec, true)?;
                    }
                    for (k, v) in mem::take(&mut registry.fields) {
                        v?.push_into_stack(self)?;
                        rawset_field(state, -2, &k)?;
                    }
                    rawset_field(state, metatable_index, "__index")?;
                }
                _ => {
                    ffi::lua_pop(state, 1);
                    // Fields will be converted to functions and added to field getters
                }
            }
        }

        let mut field_getters_index = None;
        let field_getters_nrec = registry.field_getters.len() + registry.fields.len();
        if field_getters_nrec > 0 {
            push_table(state, 0, field_getters_nrec, true)?;
            for (k, m) in registry.field_getters {
                self.push(self.create_callback(m)?)?;
                rawset_field(state, -2, &k)?;
            }
            for (k, v) in registry.fields {
                unsafe extern "C-unwind" fn return_field(state: *mut ffi::lua_State) -> c_int {
                    ffi::lua_pushvalue(state, ffi::lua_upvalueindex(1));
                    1
                }
                v?.push_into_stack(self)?;
                protect_lua!(state, 1, 1, fn(state) {
                    ffi::lua_pushcclosure(state, return_field, 1);
                })?;
                rawset_field(state, -2, &k)?;
            }
            field_getters_index = Some(ffi::lua_absindex(state, -1));
        }

        let mut field_setters_index = None;
        let field_setters_nrec = registry.field_setters.len();
        if field_setters_nrec > 0 {
            push_table(state, 0, field_setters_nrec, true)?;
            for (k, m) in registry.field_setters {
                self.push(self.create_callback(m)?)?;
                rawset_field(state, -2, &k)?;
            }
            field_setters_index = Some(ffi::lua_absindex(state, -1));
        }

        let mut methods_index = None;
        let methods_nrec = registry.methods.len();
        #[cfg(feature = "async")]
        let methods_nrec = methods_nrec + registry.async_methods.len();
        if methods_nrec > 0 {
            // If `__index` is a table then update it in-place
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
                self.push(self.create_callback(m)?)?;
                rawset_field(state, -2, &k)?;
            }
            #[cfg(feature = "async")]
            for (k, m) in registry.async_methods {
                self.push(self.create_async_callback(m)?)?;
                rawset_field(state, -2, &k)?;
            }
            match index_type {
                ffi::LUA_TTABLE => {
                    ffi::lua_pop(state, 1); // All done
                }
                ffi::LUA_TNIL => {
                    // Set the new table as `__index`
                    rawset_field(state, metatable_index, "__index")?;
                }
                _ => {
                    methods_index = Some(ffi::lua_absindex(state, -1));
                }
            }
        }

        ffi::lua_pushcfunction(state, registry.destructor);
        rawset_field(state, metatable_index, "__gc")?;

        init_userdata_metatable(
            state,
            metatable_index,
            field_getters_index,
            field_setters_index,
            methods_index,
        )?;

        // Update stack guard to keep metatable after return
        stack_guard.keep(1);

        Ok(())
    }

    #[inline(always)]
    pub(crate) unsafe fn register_userdata_metatable(&self, mt_ptr: *const c_void, type_id: Option<TypeId>) {
        (*self.extra.get()).registered_userdata_mt.insert(mt_ptr, type_id);
    }

    #[inline(always)]
    pub(crate) unsafe fn deregister_userdata_metatable(&self, mt_ptr: *const c_void) {
        (*self.extra.get()).registered_userdata_mt.remove(&mt_ptr);
        if (*self.extra.get()).last_checked_userdata_mt.0 == mt_ptr {
            (*self.extra.get()).last_checked_userdata_mt = (ptr::null(), None);
        }
    }

    // Returns `TypeId` for the userdata ref, checking that it's registered and not destructed.
    //
    // Returns `None` if the userdata is registered but non-static.
    #[inline(always)]
    pub(crate) fn get_userdata_ref_type_id(&self, vref: &ValueRef) -> Result<Option<TypeId>> {
        unsafe { self.get_userdata_type_id_inner(self.ref_thread(), vref.index) }
    }

    // Same as `get_userdata_ref_type_id` but assumes the userdata is already on the stack.
    pub(crate) unsafe fn get_userdata_type_id<T>(
        &self,
        state: *mut ffi::lua_State,
        idx: c_int,
    ) -> Result<Option<TypeId>> {
        match self.get_userdata_type_id_inner(state, idx) {
            Ok(type_id) => Ok(type_id),
            Err(Error::UserDataTypeMismatch) if ffi::lua_type(state, idx) != ffi::LUA_TUSERDATA => {
                // Report `FromLuaConversionError` instead
                let idx_type_name = CStr::from_ptr(ffi::luaL_typename(state, idx));
                let idx_type_name = idx_type_name.to_str().unwrap();
                let message = format!("expected userdata of type '{}'", short_type_name::<T>());
                Err(Error::from_lua_conversion(idx_type_name, "userdata", message))
            }
            Err(err) => Err(err),
        }
    }

    unsafe fn get_userdata_type_id_inner(
        &self,
        state: *mut ffi::lua_State,
        idx: c_int,
    ) -> Result<Option<TypeId>> {
        let mt_ptr = get_metatable_ptr(state, idx);
        if mt_ptr.is_null() {
            return Err(Error::UserDataTypeMismatch);
        }

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

    // Pushes a ValueRef (userdata) value onto the stack, returning their `TypeId`.
    // Uses 1 stack space, does not call checkstack.
    pub(crate) unsafe fn push_userdata_ref(&self, vref: &ValueRef) -> Result<Option<TypeId>> {
        let type_id = self.get_userdata_type_id_inner(self.ref_thread(), vref.index)?;
        self.push_ref(vref);
        Ok(type_id)
    }

    // Creates a Function out of a Callback containing a 'static Fn.
    pub(crate) fn create_callback(&self, func: Callback) -> Result<Function> {
        unsafe extern "C-unwind" fn call_callback(state: *mut ffi::lua_State) -> c_int {
            let upvalue = get_userdata::<CallbackUpvalue>(state, ffi::lua_upvalueindex(1));
            callback_error_ext(state, (*upvalue).extra.get(), true, |extra, nargs| {
                // Lua ensures that `LUA_MINSTACK` stack spaces are available (after pushing arguments)
                // The lock must be already held as the callback is executed
                let rawlua = (*extra).raw_lua();
                match (*upvalue).data {
                    Some(ref func) => func(rawlua, nargs),
                    None => Err(Error::CallbackDestructed),
                }
            })
        }

        let state = self.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 4)?;

            let func = Some(func);
            let extra = XRc::clone(&self.extra);
            let protect = !self.unlikely_memory_error();
            push_internal_userdata(state, CallbackUpvalue { data: func, extra }, protect)?;
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
    pub(crate) fn create_async_callback(&self, func: AsyncCallback) -> Result<Function> {
        // Ensure that the coroutine library is loaded
        #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luau"))]
        unsafe {
            if !(*self.extra.get()).libs.contains(StdLib::COROUTINE) {
                load_std_libs(self.main_state(), StdLib::COROUTINE)?;
                (*self.extra.get()).libs |= StdLib::COROUTINE;
            }
        }

        unsafe extern "C-unwind" fn call_callback(state: *mut ffi::lua_State) -> c_int {
            // Async functions cannot be scoped and therefore destroyed,
            // so the first upvalue is always valid
            let upvalue = get_userdata::<AsyncCallbackUpvalue>(state, ffi::lua_upvalueindex(1));
            callback_error_ext(state, (*upvalue).extra.get(), true, |extra, nargs| {
                // Lua ensures that `LUA_MINSTACK` stack spaces are available (after pushing arguments)
                // The lock must be already held as the callback is executed
                let rawlua = (*extra).raw_lua();

                let func = &*(*upvalue).data;
                let fut = Some(func(rawlua, nargs));
                let extra = XRc::clone(&(*upvalue).extra);
                let protect = !rawlua.unlikely_memory_error();
                push_internal_userdata(state, AsyncPollUpvalue { data: fut, extra }, protect)?;
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
            callback_error_ext(state, (*upvalue).extra.get(), true, |extra, nargs| {
                // Lua ensures that `LUA_MINSTACK` stack spaces are available (after pushing arguments)
                // The lock must be already held as the future is polled
                let rawlua = (*extra).raw_lua();

                if nargs == 1 && ffi::lua_tolightuserdata(state, -1) == Lua::poll_terminate().0 {
                    // Destroy the future and terminate the Lua thread
                    (*upvalue).data.take();
                    ffi::lua_pushinteger(state, -1);
                    return Ok(1);
                }

                let fut = &mut (*upvalue).data;
                let mut ctx = Context::from_waker(rawlua.waker());
                match fut.as_mut().map(|fut| fut.as_mut().poll(&mut ctx)) {
                    Some(Poll::Pending) => {
                        ffi::lua_pushnil(state);
                        ffi::lua_pushlightuserdata(state, Lua::poll_pending().0);
                        Ok(2)
                    }
                    Some(Poll::Ready(nresults)) => {
                        match nresults? {
                            nresults if nresults < 3 => {
                                // Fast path for up to 2 results without creating a table
                                ffi::lua_pushinteger(state, nresults as _);
                                if nresults > 0 {
                                    ffi::lua_insert(state, -nresults - 1);
                                }
                                Ok(nresults + 1)
                            }
                            nresults => {
                                let results = MultiValue::from_stack_multi(nresults, rawlua)?;
                                ffi::lua_pushinteger(state, nresults as _);
                                rawlua.push(rawlua.create_sequence_from(results)?)?;
                                Ok(2)
                            }
                        }
                    }
                    None => Err(Error::CallbackDestructed),
                }
            })
        }

        let state = self.state();
        let get_poll = unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 4)?;

            let extra = XRc::clone(&self.extra);
            let protect = !self.unlikely_memory_error();
            let upvalue = AsyncCallbackUpvalue { data: func, extra };
            push_internal_userdata(state, upvalue, protect)?;
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

        let lua = self.lua();
        let coroutine = lua.globals().get::<Table>("coroutine")?;

        // Prepare environment for the async poller
        let env = lua.create_table_with_capacity(0, 3)?;
        env.set("get_poll", get_poll)?;
        env.set("yield", coroutine.get::<Function>("yield")?)?;
        env.set("unpack", unsafe { lua.create_c_function(unpack)? })?;

        lua.load(
            r#"
            local poll = get_poll(...)
            local nres, res, res2 = poll()
            while true do
                if nres ~= nil then
                    if nres == 0 then
                        return
                    elseif nres == 1 then
                        return res
                    elseif nres == 2 then
                        return res, res2
                    elseif nres < 0 then
                        -- Negative `nres` means that the future is terminated
                        -- It must stay yielded and never be resumed again
                        yield()
                    else
                        return unpack(res, nres)
                    end
                end
                -- `res` is a "pending" value
                -- `yield` can return a signal to drop the future that we should propagate
                -- to the poller
                nres, res, res2 = poll(yield(res))
            end
            "#,
        )
        .try_cache()
        .set_name("=__mlua_async_poll")
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
}

// Uses 3 stack spaces
unsafe fn load_std_libs(state: *mut ffi::lua_State, libs: StdLib) -> Result<()> {
    unsafe fn requiref(
        state: *mut ffi::lua_State,
        modname: *const c_char,
        openf: ffi::lua_CFunction,
        glb: c_int,
    ) -> Result<()> {
        protect_lua!(state, 0, 0, |state| {
            ffi::luaL_requiref(state, modname, openf, glb)
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

    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luau"))]
    {
        if libs.contains(StdLib::COROUTINE) {
            requiref(state, ffi::LUA_COLIBNAME, ffi::luaopen_coroutine, 1)?;
        }
    }

    if libs.contains(StdLib::TABLE) {
        requiref(state, ffi::LUA_TABLIBNAME, ffi::luaopen_table, 1)?;
    }

    #[cfg(not(feature = "luau"))]
    if libs.contains(StdLib::IO) {
        requiref(state, ffi::LUA_IOLIBNAME, ffi::luaopen_io, 1)?;
    }

    if libs.contains(StdLib::OS) {
        requiref(state, ffi::LUA_OSLIBNAME, ffi::luaopen_os, 1)?;
    }

    if libs.contains(StdLib::STRING) {
        requiref(state, ffi::LUA_STRLIBNAME, ffi::luaopen_string, 1)?;
    }

    #[cfg(any(feature = "lua54", feature = "lua53", feature = "luau"))]
    {
        if libs.contains(StdLib::UTF8) {
            requiref(state, ffi::LUA_UTF8LIBNAME, ffi::luaopen_utf8, 1)?;
        }
    }

    #[cfg(any(feature = "lua52", feature = "luau"))]
    {
        if libs.contains(StdLib::BIT) {
            requiref(state, ffi::LUA_BITLIBNAME, ffi::luaopen_bit32, 1)?;
        }
    }

    #[cfg(feature = "luajit")]
    {
        if libs.contains(StdLib::BIT) {
            requiref(state, ffi::LUA_BITLIBNAME, ffi::luaopen_bit, 1)?;
        }
    }

    #[cfg(feature = "luau")]
    if libs.contains(StdLib::BUFFER) {
        requiref(state, ffi::LUA_BUFFERLIBNAME, ffi::luaopen_buffer, 1)?;
    }

    #[cfg(feature = "luau")]
    if libs.contains(StdLib::VECTOR) {
        requiref(state, ffi::LUA_VECLIBNAME, ffi::luaopen_vector, 1)?;
    }

    if libs.contains(StdLib::MATH) {
        requiref(state, ffi::LUA_MATHLIBNAME, ffi::luaopen_math, 1)?;
    }

    if libs.contains(StdLib::DEBUG) {
        requiref(state, ffi::LUA_DBLIBNAME, ffi::luaopen_debug, 1)?;
    }

    #[cfg(not(feature = "luau"))]
    if libs.contains(StdLib::PACKAGE) {
        requiref(state, ffi::LUA_LOADLIBNAME, ffi::luaopen_package, 1)?;
    }

    #[cfg(feature = "luajit")]
    if libs.contains(StdLib::JIT) {
        requiref(state, ffi::LUA_JITLIBNAME, ffi::luaopen_jit, 1)?;
    }

    #[cfg(feature = "luajit")]
    if libs.contains(StdLib::FFI) {
        requiref(state, ffi::LUA_FFILIBNAME, ffi::luaopen_ffi, 1)?;
    }

    Ok(())
}
