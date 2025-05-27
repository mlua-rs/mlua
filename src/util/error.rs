use std::any::Any;
use std::fmt::Write as _;
use std::mem::MaybeUninit;
use std::os::raw::{c_int, c_void};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::memory::MemoryState;
use crate::util::{
    check_stack, get_internal_userdata, init_internal_metatable, push_internal_userdata, push_string,
    push_table, rawset_field, to_string, TypeKey, DESTRUCTED_USERDATA_METATABLE,
};

static WRAPPED_FAILURE_TYPE_KEY: u8 = 0;

pub(crate) enum WrappedFailure {
    None,
    Error(Error),
    Panic(Option<Box<dyn Any + Send + 'static>>),
}

impl TypeKey for WrappedFailure {
    #[inline(always)]
    fn type_key() -> *const c_void {
        &WRAPPED_FAILURE_TYPE_KEY as *const u8 as *const c_void
    }
}

impl WrappedFailure {
    pub(crate) unsafe fn new_userdata(state: *mut ffi::lua_State) -> *mut Self {
        // Unprotected calls always return `Ok`
        push_internal_userdata(state, WrappedFailure::None, false).unwrap()
    }
}

// In the context of a lua callback, this will call the given function and if the given function
// returns an error, *or if the given function panics*, this will result in a call to `lua_error` (a
// longjmp). The error or panic is wrapped in such a way that when calling `pop_error` back on
// the Rust side, it will resume the panic.
//
// This function assumes the structure of the stack at the beginning of a callback, that the only
// elements on the stack are the arguments to the callback.
//
// This function uses some of the bottom of the stack for error handling, the given callback will be
// given the number of arguments available as an argument, and should return the number of returns
// as normal, but cannot assume that the arguments available start at 0.
unsafe fn callback_error<F, R>(state: *mut ffi::lua_State, f: F) -> R
where
    F: FnOnce(c_int) -> Result<R>,
{
    let nargs = ffi::lua_gettop(state);

    // We need 2 extra stack spaces to store preallocated memory and error/panic metatable
    let extra_stack = if nargs < 2 { 2 - nargs } else { 1 };
    ffi::luaL_checkstack(
        state,
        extra_stack,
        cstr!("not enough stack space for callback error handling"),
    );

    // We cannot shadow Rust errors with Lua ones, we pre-allocate enough memory
    // to store a wrapped error or panic *before* we proceed.
    let ud = WrappedFailure::new_userdata(state);
    ffi::lua_rotate(state, 1, 1);

    match catch_unwind(AssertUnwindSafe(|| f(nargs))) {
        Ok(Ok(r)) => {
            ffi::lua_remove(state, 1);
            r
        }
        Ok(Err(err)) => {
            ffi::lua_settop(state, 1);

            // Build `CallbackError` with traceback
            let traceback = if ffi::lua_checkstack(state, ffi::LUA_TRACEBACK_STACK) != 0 {
                ffi::luaL_traceback(state, state, ptr::null(), 0);
                let traceback = to_string(state, -1);
                ffi::lua_pop(state, 1);
                traceback
            } else {
                "<not enough stack space for traceback>".to_string()
            };
            let cause = Arc::new(err);
            let wrapped_error = WrappedFailure::Error(Error::CallbackError { traceback, cause });
            ptr::write(ud, wrapped_error);
            ffi::lua_error(state)
        }
        Err(p) => {
            ffi::lua_settop(state, 1);
            ptr::write(ud, WrappedFailure::Panic(Some(p)));
            ffi::lua_error(state)
        }
    }
}

// Pops an error off of the stack and returns it. The specific behavior depends on the type of the
// error at the top of the stack:
//   1) If the error is actually a panic, this will continue the panic.
//   2) If the error on the top of the stack is actually an error, just returns it.
//   3) Otherwise, interprets the error as the appropriate lua error.
// Uses 2 stack spaces, does not call checkstack.
pub(crate) unsafe fn pop_error(state: *mut ffi::lua_State, err_code: c_int) -> Error {
    mlua_debug_assert!(
        err_code != ffi::LUA_OK && err_code != ffi::LUA_YIELD,
        "pop_error called with non-error return code"
    );

    match get_internal_userdata::<WrappedFailure>(state, -1, ptr::null()).as_mut() {
        Some(WrappedFailure::Error(err)) => {
            ffi::lua_pop(state, 1);
            err.clone()
        }
        Some(WrappedFailure::Panic(panic)) => {
            if let Some(p) = panic.take() {
                resume_unwind(p);
            } else {
                Error::PreviouslyResumedPanic
            }
        }
        _ => {
            let err_string = to_string(state, -1);
            ffi::lua_pop(state, 1);

            match err_code {
                ffi::LUA_ERRRUN => Error::RuntimeError(err_string),
                ffi::LUA_ERRSYNTAX => {
                    Error::SyntaxError {
                        // This seems terrible, but as far as I can tell, this is exactly what the
                        // stock Lua REPL does.
                        incomplete_input: err_string.ends_with("<eof>") || err_string.ends_with("'<eof>'"),
                        message: err_string,
                    }
                }
                ffi::LUA_ERRERR => {
                    // This error is raised when the error handler raises an error too many times
                    // recursively, and continuing to trigger the error handler would cause a stack
                    // overflow. It is not very useful to differentiate between this and "ordinary"
                    // runtime errors, so we handle them the same way.
                    Error::RuntimeError(err_string)
                }
                ffi::LUA_ERRMEM => Error::MemoryError(err_string),
                #[cfg(any(feature = "lua53", feature = "lua52"))]
                ffi::LUA_ERRGCMM => Error::GarbageCollectorError(err_string),
                _ => mlua_panic!("unrecognized lua error code"),
            }
        }
    }
}

// Call a function that calls into the Lua API and may trigger a Lua error (longjmp) in a safe way.
// Wraps the inner function in a call to `lua_pcall`, so the inner function only has access to a
// limited lua stack. `nargs` is the same as the the parameter to `lua_pcall`, and `nresults` is
// always `LUA_MULTRET`. Provided function must *not* panic, and since it will generally be
// longjmping, should not contain any values that implements Drop.
// Internally uses 2 extra stack spaces, and does not call checkstack.
pub(crate) unsafe fn protect_lua_call(
    state: *mut ffi::lua_State,
    nargs: c_int,
    f: unsafe extern "C-unwind" fn(*mut ffi::lua_State) -> c_int,
) -> Result<()> {
    let stack_start = ffi::lua_gettop(state) - nargs;

    MemoryState::relax_limit_with(state, || {
        ffi::lua_pushcfunction(state, error_traceback);
        ffi::lua_pushcfunction(state, f);
    });
    if nargs > 0 {
        ffi::lua_rotate(state, stack_start + 1, 2);
    }

    let ret = ffi::lua_pcall(state, nargs, ffi::LUA_MULTRET, stack_start + 1);
    ffi::lua_remove(state, stack_start + 1);

    if ret == ffi::LUA_OK {
        Ok(())
    } else {
        Err(pop_error(state, ret))
    }
}

// Call a function that calls into the Lua API and may trigger a Lua error (longjmp) in a safe way.
// Wraps the inner function in a call to `lua_pcall`, so the inner function only has access to a
// limited lua stack. `nargs` and `nresults` are similar to the parameters of `lua_pcall`, but the
// given function return type is not the return value count, instead the inner function return
// values are assumed to match the `nresults` param. Provided function must *not* panic, and since
// it will generally be longjmping, should not contain any values that implements Drop.
// Internally uses 3 extra stack spaces, and does not call checkstack.
pub(crate) unsafe fn protect_lua_closure<F, R>(
    state: *mut ffi::lua_State,
    nargs: c_int,
    nresults: c_int,
    f: F,
) -> Result<R>
where
    F: FnOnce(*mut ffi::lua_State) -> R,
    R: Copy,
{
    struct Params<F, R: Copy> {
        function: Option<F>,
        result: MaybeUninit<R>,
        nresults: c_int,
    }

    unsafe extern "C-unwind" fn do_call<F, R>(state: *mut ffi::lua_State) -> c_int
    where
        F: FnOnce(*mut ffi::lua_State) -> R,
        R: Copy,
    {
        let params = ffi::lua_touserdata(state, -1) as *mut Params<F, R>;
        ffi::lua_pop(state, 1);

        let f = (*params).function.take().unwrap();
        (*params).result.write(f(state));

        if (*params).nresults == ffi::LUA_MULTRET {
            ffi::lua_gettop(state)
        } else {
            (*params).nresults
        }
    }

    let stack_start = ffi::lua_gettop(state) - nargs;

    MemoryState::relax_limit_with(state, || {
        ffi::lua_pushcfunction(state, error_traceback);
        ffi::lua_pushcfunction(state, do_call::<F, R>);
    });
    if nargs > 0 {
        ffi::lua_rotate(state, stack_start + 1, 2);
    }

    let mut params = Params {
        function: Some(f),
        result: MaybeUninit::uninit(),
        nresults,
    };

    ffi::lua_pushlightuserdata(state, &mut params as *mut Params<F, R> as *mut c_void);
    let ret = ffi::lua_pcall(state, nargs + 1, nresults, stack_start + 1);
    ffi::lua_remove(state, stack_start + 1);

    if ret == ffi::LUA_OK {
        // `LUA_OK` is only returned when the `do_call` function has completed successfully, so
        // `params.result` is definitely initialized.
        Ok(params.result.assume_init())
    } else {
        Err(pop_error(state, ret))
    }
}

pub(crate) unsafe extern "C-unwind" fn error_traceback(state: *mut ffi::lua_State) -> c_int {
    // Luau calls error handler for memory allocation errors, skip it
    // See https://github.com/luau-lang/luau/issues/880
    #[cfg(feature = "luau")]
    if MemoryState::limit_reached(state) {
        return 0;
    }

    if ffi::lua_checkstack(state, 2) == 0 {
        // If we don't have enough stack space to even check the error type, do
        // nothing so we don't risk shadowing a rust panic.
        return 1;
    }

    if get_internal_userdata::<WrappedFailure>(state, -1, ptr::null()).is_null() {
        let s = ffi::luaL_tolstring(state, -1, ptr::null_mut());
        if ffi::lua_checkstack(state, ffi::LUA_TRACEBACK_STACK) != 0 {
            ffi::luaL_traceback(state, state, s, 0);
            ffi::lua_remove(state, -2);
        }
    }

    1
}

// A variant of `error_traceback` that can safely inspect another (yielded) thread stack
pub(crate) unsafe fn error_traceback_thread(state: *mut ffi::lua_State, thread: *mut ffi::lua_State) {
    // Move error object to the main thread to safely call `__tostring` metamethod if present
    ffi::lua_xmove(thread, state, 1);

    if get_internal_userdata::<WrappedFailure>(state, -1, ptr::null()).is_null() {
        let s = ffi::luaL_tolstring(state, -1, ptr::null_mut());
        if ffi::lua_checkstack(state, ffi::LUA_TRACEBACK_STACK) != 0 {
            ffi::luaL_traceback(state, thread, s, 0);
            ffi::lua_remove(state, -2);
        }
    }
}

// Initialize the error, panic, and destructed userdata metatables.
pub(crate) unsafe fn init_error_registry(state: *mut ffi::lua_State) -> Result<()> {
    check_stack(state, 7)?;

    // Create error and panic metatables

    static ERROR_PRINT_BUFFER_KEY: u8 = 0;

    unsafe extern "C-unwind" fn error_tostring(state: *mut ffi::lua_State) -> c_int {
        callback_error(state, |_| {
            check_stack(state, 3)?;

            let err_buf = match get_internal_userdata::<WrappedFailure>(state, -1, ptr::null()).as_ref() {
                Some(WrappedFailure::Error(error)) => {
                    let err_buf_key = &ERROR_PRINT_BUFFER_KEY as *const u8 as *const c_void;
                    ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, err_buf_key);
                    let err_buf = ffi::lua_touserdata(state, -1) as *mut String;
                    ffi::lua_pop(state, 2);

                    (*err_buf).clear();
                    // Depending on how the API is used and what error types scripts are given, it may
                    // be possible to make this consume arbitrary amounts of memory (for example, some
                    // kind of recursive error structure?)
                    let _ = write!(&mut (*err_buf), "{error}");
                    Ok(err_buf)
                }
                Some(WrappedFailure::Panic(Some(panic))) => {
                    let err_buf_key = &ERROR_PRINT_BUFFER_KEY as *const u8 as *const c_void;
                    ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, err_buf_key);
                    let err_buf = ffi::lua_touserdata(state, -1) as *mut String;
                    (*err_buf).clear();
                    ffi::lua_pop(state, 2);

                    if let Some(msg) = panic.downcast_ref::<&str>() {
                        let _ = write!(&mut (*err_buf), "{msg}");
                    } else if let Some(msg) = panic.downcast_ref::<String>() {
                        let _ = write!(&mut (*err_buf), "{msg}");
                    } else {
                        let _ = write!(&mut (*err_buf), "<panic>");
                    };
                    Ok(err_buf)
                }
                Some(WrappedFailure::Panic(None)) => Err(Error::PreviouslyResumedPanic),
                _ => {
                    // I'm not sure whether this is possible to trigger without bugs in mlua?
                    Err(Error::UserDataTypeMismatch)
                }
            }?;

            push_string(state, (*err_buf).as_bytes(), true)?;
            (*err_buf).clear();

            Ok(1)
        })
    }

    init_internal_metatable::<WrappedFailure>(
        state,
        Some(|state| {
            ffi::lua_pushcfunction(state, error_tostring);
            ffi::lua_setfield(state, -2, cstr!("__tostring"));

            // This is mostly for Luau typeof() function
            ffi::lua_pushstring(state, cstr!("error"));
            ffi::lua_setfield(state, -2, cstr!("__type"));
        }),
    )?;

    // Create destructed userdata metatable

    unsafe extern "C-unwind" fn destructed_error(state: *mut ffi::lua_State) -> c_int {
        callback_error(state, |_| Err(Error::UserDataDestructed))
    }

    push_table(state, 0, 26, true)?;
    ffi::lua_pushcfunction(state, destructed_error);
    for &method in &[
        "__add",
        "__sub",
        "__mul",
        "__div",
        "__mod",
        "__pow",
        "__unm",
        #[cfg(any(feature = "lua54", feature = "lua53", feature = "luau"))]
        "__idiv",
        #[cfg(any(feature = "lua54", feature = "lua53"))]
        "__band",
        #[cfg(any(feature = "lua54", feature = "lua53"))]
        "__bor",
        #[cfg(any(feature = "lua54", feature = "lua53"))]
        "__bxor",
        #[cfg(any(feature = "lua54", feature = "lua53"))]
        "__bnot",
        #[cfg(any(feature = "lua54", feature = "lua53"))]
        "__shl",
        #[cfg(any(feature = "lua54", feature = "lua53"))]
        "__shr",
        "__concat",
        "__len",
        "__eq",
        "__lt",
        "__le",
        "__index",
        "__newindex",
        "__call",
        "__tostring",
        #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luajit52"))]
        "__pairs",
        #[cfg(any(feature = "lua53", feature = "lua52", feature = "luajit52"))]
        "__ipairs",
        #[cfg(feature = "luau")]
        "__iter",
        #[cfg(feature = "lua54")]
        "__close",
    ] {
        ffi::lua_pushvalue(state, -1);
        rawset_field(state, -3, method)?;
    }
    ffi::lua_pop(state, 1);

    protect_lua!(state, 1, 0, fn(state) {
        let destructed_mt_key = &DESTRUCTED_USERDATA_METATABLE as *const u8 as *const c_void;
        ffi::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, destructed_mt_key);
    })?;

    // Create error print buffer
    init_internal_metatable::<String>(state, None)?;
    push_internal_userdata(state, String::new(), true)?;
    protect_lua!(state, 1, 0, fn(state) {
        let err_buf_key = &ERROR_PRINT_BUFFER_KEY as *const u8 as *const c_void;
        ffi::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, err_buf_key);
    })?;

    Ok(())
}
