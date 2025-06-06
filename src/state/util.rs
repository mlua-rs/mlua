use crate::IntoLuaMulti;
use std::mem::take;
use std::os::raw::c_int;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::state::{ExtraData, RawLua};
use crate::util::{self, check_stack, get_internal_metatable, WrappedFailure};

#[cfg(all(not(feature = "lua51"), not(feature = "luajit"), not(feature = "luau")))]
use crate::{types::ContinuationUpvalue, util::get_userdata};

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

pub(crate) enum PreallocatedFailure {
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
                let index = ref_stack_pop(extra);
                (*extra).wrapped_failure_pool.push(index);
                (*extra).wrapped_failure_top += 1;
            }
            PreallocatedFailure::Reserved => (*extra).wrapped_failure_top += 1,
        }
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

    // We cannot shadow Rust errors with Lua ones, so we need to reserve pre-allocated memory
    // to store a wrapped failure (error or panic) *before* we proceed.
    let prealloc_failure = PreallocatedFailure::reserve(state, extra);

    match catch_unwind(AssertUnwindSafe(|| {
        let rawlua = (*extra).raw_lua();
        let _guard = StateGuard::new(rawlua, state);
        f(extra, nargs)
    })) {
        Ok(Ok(r)) => {
            // Ensure yielded values are cleared
            take(&mut extra.as_mut().unwrap_unchecked().yielded_values);
            #[cfg(all(not(feature = "luau"), not(feature = "lua51"), not(feature = "luajit")))]
            take(&mut extra.as_mut().unwrap_unchecked().yield_continuation);

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

/// An yieldable version of `callback_error_ext`
///
/// Unlike ``callback_error_ext``, this method requires a c_int return
/// and not a generic R
pub(crate) unsafe fn callback_error_ext_yieldable<F>(
    state: *mut ffi::lua_State,
    mut extra: *mut ExtraData,
    wrap_error: bool,
    f: F,
) -> c_int
where
    F: FnOnce(*mut ExtraData, c_int) -> Result<c_int>,
{
    if extra.is_null() {
        extra = ExtraData::get(state);
    }

    let nargs = ffi::lua_gettop(state);

    // We cannot shadow Rust errors with Lua ones, so we need to reserve pre-allocated memory
    // to store a wrapped failure (error or panic) *before* we proceed.
    let prealloc_failure = PreallocatedFailure::reserve(state, extra);

    match catch_unwind(AssertUnwindSafe(|| {
        let rawlua = (*extra).raw_lua();
        let _guard = StateGuard::new(rawlua, state);
        f(extra, nargs)
    })) {
        Ok(Ok(r)) => {
            let raw = extra.as_ref().unwrap_unchecked().raw_lua();
            let values = take(&mut extra.as_mut().unwrap_unchecked().yielded_values);

            #[cfg(all(not(feature = "luau"), not(feature = "lua51"), not(feature = "luajit")))]
            let yield_cont = take(&mut extra.as_mut().unwrap_unchecked().yield_continuation);

            if let Some(values) = values {
                if raw.state() == state {
                    // Edge case: main thread is being yielded
                    //
                    // We need to pop/clear stack early, then push args
                    ffi::lua_pop(state, -1);
                }

                match values.push_into_stack_multi(raw) {
                    Ok(nargs) => {
                        // If not main thread, then clear and xmove to target thread
                        if raw.state() != state {
                            // luau preserves the stack making yieldable continuations ugly and leaky
                            //
                            // Even outside of luau, clearing the stack is probably desirable
                            ffi::lua_pop(state, -1);
                            if let Err(err) = check_stack(state, nargs) {
                                let wrapped_error = prealloc_failure.r#use(state, extra);
                                ptr::write(
                                    wrapped_error,
                                    WrappedFailure::Error(Error::external(err.to_string())),
                                );
                                get_internal_metatable::<WrappedFailure>(state);
                                ffi::lua_setmetatable(state, -2);

                                ffi::lua_error(state)
                            }
                            ffi::lua_xmove(raw.state(), state, nargs);
                        }

                        #[cfg(all(not(feature = "luau"), not(feature = "lua51"), not(feature = "luajit")))]
                        {
                            // Yield to a continuation. Unlike luau, we need to do this manually and on the
                            // fly using a yieldk call
                            if yield_cont {
                                // On Lua 5.2, status and ctx are not present, so use 0 as status for
                                // compatibility
                                #[cfg(feature = "lua52")]
                                unsafe extern "C-unwind" fn cont_callback(
                                    state: *mut ffi::lua_State,
                                ) -> c_int {
                                    let upvalue =
                                        get_userdata::<ContinuationUpvalue>(state, ffi::lua_upvalueindex(1));
                                    callback_error_ext_yieldable(
                                        state,
                                        (*upvalue).extra.get(),
                                        true,
                                        |extra, nargs| {
                                            // Lua ensures that `LUA_MINSTACK` stack spaces are available
                                            // (after pushing arguments)
                                            // The lock must be already held as the callback is executed
                                            let rawlua = (*extra).raw_lua();
                                            match (*upvalue).data {
                                                Some(ref func) => (func.1)(rawlua, nargs, 0),
                                                None => Err(Error::CallbackDestructed),
                                            }
                                        },
                                    )
                                }

                                // Lua 5.3/5.4 case
                                #[cfg(not(feature = "lua52"))]
                                unsafe extern "C-unwind" fn cont_callback(
                                    state: *mut ffi::lua_State,
                                    status: c_int,
                                    _ctx: ffi::lua_KContext,
                                ) -> c_int {
                                    let upvalue =
                                        get_userdata::<ContinuationUpvalue>(state, ffi::lua_upvalueindex(1));
                                    callback_error_ext_yieldable(
                                        state,
                                        (*upvalue).extra.get(),
                                        true,
                                        |extra, nargs| {
                                            // Lua ensures that `LUA_MINSTACK` stack spaces are available
                                            // (after pushing arguments)
                                            // The lock must be already held as the callback is executed
                                            let rawlua = (*extra).raw_lua();
                                            match (*upvalue).data {
                                                Some(ref func) => (func.1)(rawlua, nargs, status),
                                                None => Err(Error::CallbackDestructed),
                                            }
                                        },
                                    )
                                }

                                return ffi::lua_yieldc(state, nargs, cont_callback);
                            }
                        }

                        return ffi::lua_yield(state, nargs);
                    }
                    Err(err) => {
                        let wrapped_error = prealloc_failure.r#use(state, extra);
                        ptr::write(
                            wrapped_error,
                            WrappedFailure::Error(Error::external(err.to_string())),
                        );
                        get_internal_metatable::<WrappedFailure>(state);
                        ffi::lua_setmetatable(state, -2);

                        ffi::lua_error(state)
                    }
                }
            }

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

pub(super) unsafe fn ref_stack_pop(extra: *mut ExtraData) -> c_int {
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
            panic!("cannot create a Lua reference, out of auxiliary stack space (used {top} slots)");
        }
        extra.ref_stack_size += inc;
    }
    extra.ref_stack_top += 1;
    extra.ref_stack_top
}
