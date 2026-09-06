use std::any::TypeId;
use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::os::raw::{c_int, c_void};
use std::ptr;
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::error::{Error, Result};
use crate::memory::MemoryState;
use crate::state::RawLua;
use crate::stdlib::StdLib;
use crate::thread::ThreadTriggers;
use crate::types::{AppData, ReentrantMutex, ThreadEventCallback, XRc};
use crate::userdata::RawUserDataRegistry;
use crate::util::{TypeKey, WrappedFailure, get_internal_metatable, push_internal_userdata};

#[cfg(any(feature = "luau", doc))]
use crate::chunk::Compiler;

#[cfg(feature = "async")]
use {futures_util::task::noop_waker_ref, std::ptr::NonNull, std::task::Waker};

use super::{Lua, WeakLua};

// Unique key to store `ExtraData` in the registry
static EXTRA_REGISTRY_KEY: u8 = 0;

const WRAPPED_FAILURE_POOL_DEFAULT_CAPACITY: usize = 64;
const REF_STACK_RESERVE: c_int = 3;

/// Data associated with the Lua state.
pub(crate) struct ExtraData {
    pub(super) lua: MaybeUninit<Lua>,
    pub(super) weak: MaybeUninit<WeakLua>,
    pub(super) owned: bool,

    pub(super) pending_userdata_reg: FxHashMap<TypeId, RawUserDataRegistry>,
    pub(super) registered_userdata_t: FxHashMap<TypeId, c_int>,
    pub(super) registered_userdata_mt: FxHashMap<*const c_void, Option<TypeId>>,
    pub(super) last_checked_userdata_mt: (*const c_void, Option<TypeId>),

    // When Lua instance dropped, setting `None` would prevent collecting `RegistryKey`s
    pub(super) registry_unref_list: Arc<Mutex<Option<Vec<c_int>>>>,

    // Containers to store arbitrary data (extensions)
    pub(super) app_data: AppData,
    pub(super) app_data_priv: AppData,

    pub(super) safe: bool,
    pub(super) libs: StdLib,
    // Cached allocation protection decision for owned states
    pub(super) unlikely_memory_error: bool,
    // Used in module mode
    pub(super) skip_memory_check: bool,

    // Auxiliary thread to store references
    pub(super) ref_thread: *mut ffi::lua_State,
    pub(super) ref_stack_size: c_int,
    pub(super) ref_stack_top: c_int,
    pub(super) ref_free: Vec<c_int>,

    // Pool of `WrappedFailure` enums in the ref thread (as userdata)
    pub(super) wrapped_failure_pool: Vec<c_int>,
    pub(super) wrapped_failure_top: usize,
    // Pool of `Thread`s (coroutines) for async execution
    #[cfg(feature = "async")]
    pub(super) thread_pool: Vec<c_int>,
    // Map for implicit threads to root user-owned Thread
    #[cfg(feature = "async")]
    pub(super) thread_ownership_map: FxHashMap<*mut ffi::lua_State, *mut ffi::lua_State>,

    // Address of `WrappedFailure` metatable
    pub(super) wrapped_failure_mt_ptr: *const c_void,

    // Waker for polling futures
    #[cfg(feature = "async")]
    pub(super) waker: NonNull<Waker>,

    #[cfg(not(feature = "luau"))]
    pub(super) hook_callback: Option<crate::types::HookCallback>,
    #[cfg(not(feature = "luau"))]
    pub(super) hook_triggers: crate::debug::HookTriggers,
    #[cfg(any(feature = "lua55", feature = "lua54", feature = "lua53"))]
    pub(super) hook_removed_while_yielded: bool,
    #[cfg(any(feature = "lua55", feature = "lua54"))]
    pub(super) warn_callback: Option<crate::types::WarnCallback>,
    #[cfg(feature = "luau")]
    pub(super) interrupt_callback: Option<crate::types::InterruptCallback>,
    pub(super) thread_triggers: ThreadTriggers,
    pub(super) thread_event_callback: Option<ThreadEventCallback>,
    pub(super) thread_event_state: *mut ffi::lua_State,

    #[cfg(feature = "luau")]
    pub(crate) running_gc: bool,
    #[cfg(feature = "luau")]
    pub(crate) sandboxed: bool,
    #[cfg(feature = "luau")]
    pub(super) compiler: Option<Compiler>,
    #[cfg(feature = "luau-jit")]
    pub(super) enable_jit: bool,
    #[cfg(feature = "luau")]
    pub(crate) mem_categories: Vec<std::ffi::CString>,
}

impl Drop for ExtraData {
    fn drop(&mut self) {
        unsafe {
            if !self.owned {
                self.lua.assume_init_drop();
            }

            self.weak.assume_init_drop();
        }
        *self.registry_unref_list.lock() = None;
    }
}

static EXTRA_TYPE_KEY: u8 = 0;

impl TypeKey for XRc<UnsafeCell<ExtraData>> {
    #[inline(always)]
    fn type_key() -> *const c_void {
        &EXTRA_TYPE_KEY as *const u8 as *const c_void
    }
}

impl ExtraData {
    // Index of `error_traceback` function in auxiliary thread stack
    #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
    pub(super) const ERROR_TRACEBACK_IDX: c_int = 1;

    pub(super) unsafe fn init(state: *mut ffi::lua_State, owned: bool) -> XRc<UnsafeCell<Self>> {
        // Create ref stack thread and place it in the registry to prevent it
        // from being garbage collected.
        let ref_thread = mlua_expect!(
            protect_lua!(state, 0, 0, |state| {
                let thread = ffi::lua_newthread(state);
                ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX);
                thread
            }),
            "Error while creating ref thread",
        );

        // Do not run inherited hooks during protected ref stack growth (matters in module mode)
        #[cfg(all(feature = "lua51", not(feature = "vendored")))]
        ffi::lua_sethook(ref_thread, None, 0, 0);

        let wrapped_failure_mt_ptr = {
            get_internal_metatable::<WrappedFailure>(state);
            let ptr = ffi::lua_topointer(state, -1);
            ffi::lua_pop(state, 1);
            ptr
        };

        // Store `error_traceback` function on the ref stack
        #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
        {
            ffi::lua_pushcfunction(ref_thread, crate::util::error_traceback);
            assert_eq!(ffi::lua_gettop(ref_thread), Self::ERROR_TRACEBACK_IDX);
        }

        #[allow(clippy::arc_with_non_send_sync)]
        let extra = XRc::new(UnsafeCell::new(ExtraData {
            lua: MaybeUninit::uninit(),
            weak: MaybeUninit::uninit(),
            owned,
            pending_userdata_reg: FxHashMap::default(),
            registered_userdata_t: FxHashMap::default(),
            registered_userdata_mt: FxHashMap::default(),
            last_checked_userdata_mt: (ptr::null(), None),
            registry_unref_list: Arc::new(Mutex::new(Some(Vec::new()))),
            app_data: AppData::default(),
            app_data_priv: AppData::default(),
            safe: false,
            libs: StdLib::NONE,
            unlikely_memory_error: owned && {
                let mem_state = MemoryState::get(state);
                !mem_state.is_null() && (*mem_state).memory_limit() == 0
            },
            skip_memory_check: false,
            ref_thread,
            // We need some reserved stack space to move values in and out of the ref stack.
            ref_stack_size: ffi::LUA_MINSTACK - REF_STACK_RESERVE,
            ref_stack_top: ffi::lua_gettop(ref_thread),
            ref_free: Vec::new(),
            wrapped_failure_pool: Vec::with_capacity(WRAPPED_FAILURE_POOL_DEFAULT_CAPACITY),
            wrapped_failure_top: 0,
            #[cfg(feature = "async")]
            thread_pool: Vec::new(),
            #[cfg(feature = "async")]
            thread_ownership_map: FxHashMap::default(),
            wrapped_failure_mt_ptr,
            #[cfg(feature = "async")]
            waker: NonNull::from(noop_waker_ref()),
            #[cfg(not(feature = "luau"))]
            hook_callback: None,
            #[cfg(not(feature = "luau"))]
            hook_triggers: Default::default(),
            #[cfg(any(feature = "lua55", feature = "lua54", feature = "lua53"))]
            hook_removed_while_yielded: false,
            #[cfg(any(feature = "lua55", feature = "lua54"))]
            warn_callback: None,
            #[cfg(feature = "luau")]
            interrupt_callback: None,
            thread_triggers: ThreadTriggers::default(),
            thread_event_callback: None,
            thread_event_state: ptr::null_mut(),
            #[cfg(feature = "luau")]
            sandboxed: false,
            #[cfg(feature = "luau")]
            compiler: None,
            #[cfg(feature = "luau-jit")]
            enable_jit: true,
            #[cfg(feature = "luau")]
            running_gc: false,
            #[cfg(feature = "luau")]
            mem_categories: vec![std::ffi::CString::new("main").unwrap()],
        }));

        // Store it in the registry
        mlua_expect!(Self::store(&extra, state), "Error while storing extra data");

        extra
    }

    pub(super) unsafe fn set_lua(&mut self, raw: &XRc<ReentrantMutex<RawLua>>) {
        self.lua.write(Lua {
            raw: XRc::clone(raw),
            collect_garbage: false,
        });
        self.weak.write(WeakLua(XRc::downgrade(raw)));
    }

    pub(crate) unsafe fn get(state: *mut ffi::lua_State) -> *mut Self {
        #[cfg(feature = "luau")]
        if cfg!(not(feature = "module")) {
            // In the main app we can use `lua_callbacks` to access ExtraData
            return (*ffi::lua_callbacks(state)).userdata as *mut _;
        }

        let extra_key = &EXTRA_REGISTRY_KEY as *const u8 as *const c_void;
        if ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, extra_key) != ffi::LUA_TUSERDATA {
            // `ExtraData` can be null only when Lua state is foreign.
            // This case in used in `Lua::try_from_ptr()`.
            ffi::lua_pop(state, 1);
            return ptr::null_mut();
        }
        let extra_ptr = ffi::lua_touserdata(state, -1) as *mut Rc<UnsafeCell<ExtraData>>;
        ffi::lua_pop(state, 1);
        (*extra_ptr).get()
    }

    unsafe fn store(extra: &XRc<UnsafeCell<Self>>, state: *mut ffi::lua_State) -> Result<()> {
        #[cfg(feature = "luau")]
        if cfg!(not(feature = "module")) {
            (*ffi::lua_callbacks(state)).userdata = extra.get() as *mut _;
            return Ok(());
        }

        push_internal_userdata(state, XRc::clone(extra), true)?;
        protect_lua!(state, 1, 0, fn(state) {
            let extra_key = &EXTRA_REGISTRY_KEY as *const u8 as *const c_void;
            ffi::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, extra_key);
        })
    }

    #[inline(always)]
    pub(super) unsafe fn lua(&self) -> &Lua {
        self.lua.assume_init_ref()
    }

    #[inline(always)]
    pub(crate) unsafe fn raw_lua(&self) -> &RawLua {
        &*self.lua.assume_init_ref().raw.data_ptr()
    }

    #[inline(always)]
    pub(super) unsafe fn weak(&self) -> &WeakLua {
        self.weak.assume_init_ref()
    }

    /// Like `try_ref_stack_pop`, but panics if the auxiliary stack cannot grow.
    pub(super) unsafe fn ref_stack_pop(&mut self) -> c_int {
        self.try_ref_stack_pop().unwrap_or_else(|_| {
            let top = self.ref_stack_top;
            panic!("cannot create a Lua reference, out of auxiliary stack space (used {top} slots)");
        })
    }

    /// Pops a reference from the top of the auxiliary stack and moves it to the first free slot.
    ///
    /// Returns an error if the auxiliary stack cannot grow. The value is popped even on failure.
    pub(super) unsafe fn try_ref_stack_pop(&mut self) -> Result<c_int> {
        if let Some(free) = self.ref_free.pop() {
            ffi::lua_replace(self.ref_thread, free);
            return Ok(free);
        }

        // Try to grow max stack size
        if self.ref_stack_top >= self.ref_stack_size {
            let mut inc = self.ref_stack_size; // Try to double stack size
            while inc > 0 && !self.check_ref_stack(inc + REF_STACK_RESERVE) {
                inc /= 2;
            }
            if inc == 0 {
                // Pop the pending value to keep the stack balanced on failure.
                ffi::lua_pop(self.ref_thread, 1);
                return Err(Error::StackError);
            }
            self.ref_stack_size += inc;
        }
        self.ref_stack_top += 1;
        Ok(self.ref_stack_top)
    }

    #[cfg(any(not(feature = "lua51"), feature = "vendored"))]
    #[inline(always)]
    unsafe fn check_ref_stack(&self, amount: c_int) -> bool {
        ffi::lua_checkstack(self.ref_thread, amount) != 0
    }

    #[cfg(all(feature = "lua51", not(feature = "vendored")))]
    unsafe fn check_ref_stack(&self, amount: c_int) -> bool {
        if self.raw_lua().unlikely_memory_error() {
            return ffi::lua_checkstack(self.ref_thread, amount) != 0;
        }

        unsafe extern "C-unwind" fn grow_stack(state: *mut ffi::lua_State) -> c_int {
            let args = ffi::lua_tolightuserdata(state, 1) as *mut (c_int, bool);
            (*args).1 = ffi::lua_checkstack(state, (*args).0) != 0;
            // we always return an error to skip GC
            ffi::lua_error(state)
        }

        // Lua 5.1 can throw on allocation failure, protect growth on the ref thread
        let state = self.ref_thread;
        let mut args = (amount, false);
        ffi::lua_cpcall(state, grow_stack, &mut args as *mut _ as *mut c_void);
        ffi::lua_pop(state, 1);
        args.1 && ffi::lua_checkstack(state, amount) != 0
    }
}
