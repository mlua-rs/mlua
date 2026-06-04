use std::os::raw::c_int;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::Arc;

#[cfg(feature = "luau")]
use crate::debug::Debug;
use crate::error::{Error, Result};
use crate::state::{ExtraData, RawLua};
#[cfg(feature = "luau")]
use crate::types::{DebugCallback, VmState, XRc};
use crate::util::{self, WrappedFailure, get_internal_metatable};

struct StateGuard<'a>(&'a RawLua, *mut ffi::lua_State);

impl<'a> StateGuard<'a> {
    fn new(inner: &'a RawLua, mut state: *mut ffi::lua_State) -> Self {
        state = inner.state.replace(state);
        Self(inner, state)
    }
}

impl Drop for StateGuard<'_> {
    fn drop(&mut self) {
        self.0.state.set(self.1);
    }
}

/// Runs a Luau `debugbreak`/`debugstep` callback (selected by `select`) for the paused frame and
/// maps its [`VmState`] result the same way the `interrupt` callback does.
///
/// Unlike [`callback_error_ext`], the pre-allocated failure userdata is reserved at the *top* of
/// the stack rather than inserted at the running frame's base, so the paused function's registers
/// stay readable through [`Debug::get_local`]. The callback runs inline in the VM (no extra call
/// frame), hence it cannot reuse the regular callback path.
#[cfg(feature = "luau")]
pub(crate) unsafe fn debug_callback(
    state: *mut ffi::lua_State,
    ar: *mut ffi::lua_Debug,
    select: impl Fn(*mut ExtraData) -> Option<DebugCallback>,
) {
    let extra = ExtraData::get(state);

    // Reserve memory for a wrapped failure *before* running the callback (an error must not be
    // shadowed by an allocation failure), keeping it at the top so registers are not shifted.
    ffi::lua_rawcheckstack(state, 2);
    let wrapped_failure = WrappedFailure::new_userdata(state);
    let failure_idx = ffi::lua_gettop(state);

    let result = catch_unwind(AssertUnwindSafe(|| {
        let rawlua = (*extra).raw_lua();
        let _guard = StateGuard::new(rawlua, state);
        let callback = match select(extra) {
            // A strong count above 2 means the callback is already on the stack: avoid recursion.
            Some(callback) if XRc::strong_count(&callback) <= 2 => callback,
            _ => return Ok(VmState::Continue),
        };
        callback((*extra).lua(), &Debug::new(rawlua, 0, ar))
    }));

    let raise = |failure: WrappedFailure| {
        ptr::write(wrapped_failure, failure);
        get_internal_metatable::<WrappedFailure>(state);
        ffi::lua_setmetatable(state, -2);
        ffi::lua_error(state)
    };

    match result {
        Ok(Ok(state_result)) => {
            ffi::lua_settop(state, failure_idx - 1); // drop the unused failure
            if state_result == VmState::Yield && ffi::lua_isyieldable(state) != 0 {
                ffi::lua_yield(state, 0);
            }
        }
        Ok(Err(err)) => {
            let traceback = if ffi::lua_checkstack(state, ffi::LUA_TRACEBACK_STACK) != 0 {
                ffi::luaL_traceback(state, state, ptr::null(), 0);
                let traceback = util::to_string(state, -1);
                ffi::lua_pop(state, 1);
                traceback
            } else {
                "<not enough stack space for traceback>".to_string()
            };
            let cause = Arc::new(err);
            raise(WrappedFailure::Error(Error::CallbackError { traceback, cause }));
        }
        Err(panic) => raise(WrappedFailure::Panic(Some(panic))),
    }
}

// An optimized version of `callback_error` that does not allocate `WrappedFailure` userdata
// and instead reuses unused values from previous calls (or allocates new).
pub(crate) unsafe fn callback_error_ext<F, R>(
    state: *mut ffi::lua_State,
    mut extra: *mut ExtraData,
    wrap_error: bool,
    f: F,
) -> R
where
    F: FnOnce(*mut ExtraData, c_int) -> Result<R>,
{
    if extra.is_null() {
        extra = ExtraData::get(state);
    }

    let nargs = ffi::lua_gettop(state);

    enum PreallocatedFailure {
        New(*mut WrappedFailure),
        Reserved,
    }

    impl PreallocatedFailure {
        unsafe fn reserve(state: *mut ffi::lua_State, extra: *mut ExtraData) -> Self {
            if (*extra).wrapped_failure_top > 0 {
                (*extra).wrapped_failure_top -= 1;
                return PreallocatedFailure::Reserved;
            }

            // We need to check stack for Luau in case when callback is called from interrupt
            // See https://github.com/luau-lang/luau/issues/446 and mlua #142 and #153
            #[cfg(feature = "luau")]
            ffi::lua_rawcheckstack(state, 2);
            // Place it to the beginning of the stack
            let ud = WrappedFailure::new_userdata(state);
            ffi::lua_insert(state, 1);
            PreallocatedFailure::New(ud)
        }

        #[cold]
        unsafe fn r#use(&self, state: *mut ffi::lua_State, extra: *mut ExtraData) -> *mut WrappedFailure {
            let ref_thread = (*extra).ref_thread;
            match *self {
                PreallocatedFailure::New(ud) => {
                    ffi::lua_settop(state, 1);
                    ud
                }
                PreallocatedFailure::Reserved => {
                    let index = (*extra).wrapped_failure_pool.pop().unwrap();
                    ffi::lua_settop(state, 0);
                    #[cfg(feature = "luau")]
                    ffi::lua_rawcheckstack(state, 2);
                    ffi::lua_xpush(ref_thread, state, index);
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
                    ffi::lua_rotate(state, 1, -1);
                    ffi::lua_xmove(state, ref_thread, 1);
                    let index = (*extra).ref_stack_pop();
                    (*extra).wrapped_failure_pool.push(index);
                    (*extra).wrapped_failure_top += 1;
                }
                PreallocatedFailure::Reserved => (*extra).wrapped_failure_top += 1,
            }
        }
    }

    // We cannot shadow Rust errors with Lua ones, so we need to reserve pre-allocated memory
    // to store a wrapped failure (error or panic) *before* we proceed.
    let prealloc_failure = PreallocatedFailure::reserve(state, extra);

    match catch_unwind(AssertUnwindSafe(|| {
        let rawlua = (*extra).raw_lua();
        let _guard = StateGuard::new(rawlua, state);
        f(extra, nargs)
    })) {
        Ok(Ok(r)) => {
            // Return unused `WrappedFailure` to the pool
            prealloc_failure.release(state, extra);
            r
        }
        Ok(Err(err)) => {
            let wrapped_error = prealloc_failure.r#use(state, extra);

            if !wrap_error {
                ptr::write(wrapped_error, WrappedFailure::Error(err));
                get_internal_metatable::<WrappedFailure>(state);
                ffi::lua_setmetatable(state, -2);
                ffi::lua_error(state)
            }

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
            get_internal_metatable::<WrappedFailure>(state);
            ffi::lua_setmetatable(state, -2);

            ffi::lua_error(state)
        }
        Err(p) => {
            let wrapped_panic = prealloc_failure.r#use(state, extra);
            ptr::write(wrapped_panic, WrappedFailure::Panic(Some(p)));
            get_internal_metatable::<WrappedFailure>(state);
            ffi::lua_setmetatable(state, -2);
            ffi::lua_error(state)
        }
    }
}
