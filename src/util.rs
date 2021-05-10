use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::error::Error as StdError;
use std::fmt::Write;
use std::os::raw::{c_int, c_void};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};
use std::{mem, ptr, slice};

use once_cell::sync::Lazy;

use crate::error::{Error, Result};
use crate::ffi;

static METATABLE_CACHE: Lazy<Mutex<HashMap<TypeId, u8>>> = Lazy::new(|| {
    // The capacity must(!) be greater than number of stored keys
    Mutex::new(HashMap::with_capacity(32))
});

// Checks that Lua has enough free stack space for future stack operations. On failure, this will
// panic with an internal error message.
pub unsafe fn assert_stack(state: *mut ffi::lua_State, amount: c_int) {
    // TODO: This should only be triggered when there is a logic error in `mlua`. In the future,
    // when there is a way to be confident about stack safety and test it, this could be enabled
    // only when `cfg!(debug_assertions)` is true.
    mlua_assert!(
        ffi::lua_checkstack(state, amount) != 0,
        "out of stack space"
    );
}

// Checks that Lua has enough free stack space and returns `Error::StackError` on failure.
pub unsafe fn check_stack(state: *mut ffi::lua_State, amount: c_int) -> Result<()> {
    if ffi::lua_checkstack(state, amount) == 0 {
        Err(Error::StackError)
    } else {
        Ok(())
    }
}

pub struct StackGuard {
    state: *mut ffi::lua_State,
    top: c_int,
    extra: c_int,
}

impl StackGuard {
    // Creates a StackGuard instance with record of the stack size, and on Drop will check the
    // stack size and drop any extra elements. If the stack size at the end is *smaller* than at
    // the beginning, this is considered a fatal logic error and will result in a panic.
    pub unsafe fn new(state: *mut ffi::lua_State) -> StackGuard {
        StackGuard {
            state,
            top: ffi::lua_gettop(state),
            extra: 0,
        }
    }

    // Similar to `new`, but checks and keeps `extra` elements from top of the stack on Drop.
    pub unsafe fn new_extra(state: *mut ffi::lua_State, extra: c_int) -> StackGuard {
        StackGuard {
            state,
            top: ffi::lua_gettop(state),
            extra,
        }
    }
}

impl Drop for StackGuard {
    fn drop(&mut self) {
        unsafe {
            let top = ffi::lua_gettop(self.state);
            if top < self.top + self.extra {
                mlua_panic!("{} too many stack values popped", self.top - top)
            }
            if top > self.top + self.extra {
                if self.extra > 0 {
                    ffi::lua_rotate(self.state, self.top + 1, self.extra);
                }
                ffi::lua_settop(self.state, self.top + self.extra);
            }
        }
    }
}

// Call a function that calls into the Lua API and may trigger a Lua error (longjmp) in a safe way.
// Wraps the inner function in a call to `lua_pcall`, so the inner function only has access to a
// limited lua stack. `nargs` is the same as the the parameter to `lua_pcall`, and `nresults` is
// always LUA_MULTRET. Internally uses 2 extra stack spaces, and does not call checkstack.
// Provided function must *never* panic.
pub unsafe fn protect_lua(
    state: *mut ffi::lua_State,
    nargs: c_int,
    f: unsafe extern "C" fn(*mut ffi::lua_State) -> c_int, // Must be "C-unwind" after stabilizing
) -> Result<()> {
    let stack_start = ffi::lua_gettop(state) - nargs;

    ffi::lua_pushcfunction(state, ffi::safe::error_traceback);
    ffi::lua_pushcfunction(state, f);
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

// Pops an error off of the stack and returns it. The specific behavior depends on the type of the
// error at the top of the stack:
//   1) If the error is actually a WrappedPanic, this will continue the panic.
//   2) If the error on the top of the stack is actually a WrappedError, just returns it.
//   3) Otherwise, interprets the error as the appropriate lua error.
// Uses 2 stack spaces, does not call checkstack.
pub unsafe fn pop_error(state: *mut ffi::lua_State, err_code: c_int) -> Error {
    mlua_debug_assert!(
        err_code != ffi::LUA_OK && err_code != ffi::LUA_YIELD,
        "pop_error called with non-error return code"
    );

    if let Some(err) = get_wrapped_error(state, -1).as_ref() {
        ffi::lua_pop(state, 1);
        err.clone()
    } else if let Some(panic) = get_gc_userdata::<WrappedPanic>(state, -1).as_mut() {
        if let Some(p) = (*panic).0.take() {
            resume_unwind(p);
        } else {
            Error::PreviouslyResumedPanic
        }
    } else {
        let err_string = to_string(state, -1);
        ffi::lua_pop(state, 1);

        match err_code {
            ffi::LUA_ERRRUN => Error::RuntimeError(err_string),
            ffi::LUA_ERRSYNTAX => {
                Error::SyntaxError {
                    // This seems terrible, but as far as I can tell, this is exactly what the
                    // stock Lua REPL does.
                    incomplete_input: err_string.ends_with("<eof>")
                        || err_string.ends_with("'<eof>'"),
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

// Internally uses 3 stack spaces, does not call checkstack.
pub unsafe fn push_userdata<T>(state: *mut ffi::lua_State, t: T) -> Result<()> {
    let ud = ffi::safe::lua_newuserdata(state, mem::size_of::<T>())? as *mut T;
    ptr::write(ud, t);
    Ok(())
}

pub unsafe fn get_userdata<T>(state: *mut ffi::lua_State, index: c_int) -> *mut T {
    let ud = ffi::lua_touserdata(state, index) as *mut T;
    mlua_debug_assert!(!ud.is_null(), "userdata pointer is null");
    ud
}

// Pops the userdata off of the top of the stack and returns it to rust, invalidating the lua
// userdata and gives it the special "destructed" userdata metatable. Userdata must not have been
// previously invalidated, and this method does not check for this.
// Uses 1 extra stack space and does not call checkstack.
pub unsafe fn take_userdata<T>(state: *mut ffi::lua_State) -> T {
    // We set the metatable of userdata on __gc to a special table with no __gc method and with
    // metamethods that trigger an error on access. We do this so that it will not be double
    // dropped, and also so that it cannot be used or identified as any particular userdata type
    // after the first call to __gc.
    get_destructed_userdata_metatable(state);
    ffi::lua_setmetatable(state, -2);
    let ud = get_userdata(state, -1);
    ffi::lua_pop(state, 1);
    ptr::read(ud)
}

// Pushes the userdata and attaches a metatable with __gc method.
// Internally uses 3 stack spaces, does not call checkstack.
pub unsafe fn push_gc_userdata<T: Any>(state: *mut ffi::lua_State, t: T) -> Result<()> {
    push_userdata(state, t)?;
    get_gc_metatable_for::<T>(state);
    ffi::lua_setmetatable(state, -2);
    Ok(())
}

// Uses 2 stack spaces, does not call checkstack
pub unsafe fn get_gc_userdata<T: Any>(state: *mut ffi::lua_State, index: c_int) -> *mut T {
    let ud = ffi::lua_touserdata(state, index) as *mut T;
    if ud.is_null() || ffi::lua_getmetatable(state, index) == 0 {
        return ptr::null_mut();
    }
    get_gc_metatable_for::<T>(state);
    let res = ffi::lua_rawequal(state, -1, -2);
    ffi::lua_pop(state, 2);
    if res == 0 {
        return ptr::null_mut();
    }
    ud
}

// Populates the given table with the appropriate members to be a userdata metatable for the given type.
// This function takes the given table at the `metatable` index, and adds an appropriate `__gc` member
// to it for the given type and a `__metatable` entry to protect the table from script access.
// The function also, if given a `field_getters` or `methods` tables, will create an `__index` metamethod
// (capturing previous one) to lookup in `field_getters` first, then `methods` and falling back to the
// captured `__index` if no matches found.
// The same is also applicable for `__newindex` metamethod and `field_setters` table.
// Internally uses 9 stack spaces and does not call checkstack.
pub unsafe fn init_userdata_metatable<T>(
    state: *mut ffi::lua_State,
    metatable: c_int,
    field_getters: Option<c_int>,
    field_setters: Option<c_int>,
    methods: Option<c_int>,
) -> Result<()> {
    ffi::lua_pushvalue(state, metatable);

    if field_getters.is_some() || methods.is_some() {
        ffi::safe::lua_pushstring(state, "__index")?;

        ffi::lua_pushvalue(state, -1);
        let index_type = ffi::lua_rawget(state, -3);
        match index_type {
            ffi::LUA_TNIL | ffi::LUA_TTABLE | ffi::LUA_TFUNCTION => {
                for &idx in &[field_getters, methods] {
                    if let Some(idx) = idx {
                        ffi::lua_pushvalue(state, idx);
                    } else {
                        ffi::lua_pushnil(state);
                    }
                }
                ffi::safe::lua_pushcclosure(state, ffi::safe::meta_index_impl, 3)?;
            }
            _ => mlua_panic!("improper __index type {}", index_type),
        }

        ffi::safe::lua_rawset(state, -3)?;
    }

    if let Some(field_setters) = field_setters {
        ffi::safe::lua_pushstring(state, "__newindex")?;

        ffi::lua_pushvalue(state, -1);
        let newindex_type = ffi::lua_rawget(state, -3);
        match newindex_type {
            ffi::LUA_TNIL | ffi::LUA_TTABLE | ffi::LUA_TFUNCTION => {
                ffi::lua_pushvalue(state, field_setters);
                ffi::safe::lua_pushcclosure(state, ffi::safe::meta_newindex_impl, 2)?;
            }
            _ => mlua_panic!("improper __newindex type {}", newindex_type),
        }

        ffi::safe::lua_rawset(state, -3)?;
    }

    ffi::safe::lua_pushrclosure(state, userdata_destructor::<T>, 0)?;
    ffi::safe::lua_rawsetfield(state, -2, "__gc")?;

    ffi::lua_pushboolean(state, 0);
    ffi::safe::lua_rawsetfield(state, -2, "__metatable")?;

    ffi::lua_pop(state, 1);

    Ok(())
}

pub unsafe extern "C" fn userdata_destructor<T>(state: *mut ffi::lua_State) -> c_int {
    callback_error(state, |_| {
        check_stack(state, 1)?;
        take_userdata::<T>(state);
        Ok(0)
    })
}

// In the context of a lua callback, this will call the given function and if the given function
// returns an error, *or if the given function panics*, this will result in a call to lua_error (a
// longjmp) by a C shim. The error or panic is wrapped in such a way that when calling pop_error back
// on the rust side, it will resume the panic (or when popping a panic value from the stack).
//
// This function assumes the structure of the stack at the beginning of a callback, that the only
// elements on the stack are the arguments to the callback.
//
// This function uses some of the bottom of the stack for error handling, the given callback will be
// given the number of arguments available as an argument, and should return the number of returns
// as normal, but cannot assume that the arguments available start at 1.
pub unsafe fn callback_error<F>(state: *mut ffi::lua_State, f: F) -> c_int
where
    F: FnOnce(c_int) -> Result<c_int>,
{
    let nargs = ffi::lua_gettop(state) - 1;
    match catch_unwind(AssertUnwindSafe(|| f(nargs))) {
        Ok(Ok(r)) => {
            ffi::lua_remove(state, 1);
            r
        }
        Ok(Err(err)) => {
            ffi::lua_settop(state, 1);
            let error_ud = ffi::lua_touserdata(state, 1);
            ptr::write(error_ud as *mut WrappedError, WrappedError(err, None));
            get_gc_metatable_for::<WrappedError>(state);
            ffi::lua_setmetatable(state, -2);
            -1
        }
        Err(p) => {
            ffi::lua_settop(state, 1);
            let error_ud = ffi::lua_touserdata(state, 1);
            ptr::write(error_ud as *mut WrappedPanic, WrappedPanic(Some(p)));
            get_gc_metatable_for::<WrappedPanic>(state);
            ffi::lua_setmetatable(state, -2);
            -2
        }
    }
}

// A part of the C shim for errors handling.
// Receives indexes of error and traceback (optional) in the stack.
// Depending on `convert_to_callback_error` flag attaches traceback to the error or
// converts error into a `CallbackError` (using previously attached traceback).
#[no_mangle]
pub unsafe extern "C" fn wrapped_error_traceback(
    state: *mut ffi::lua_State,
    error_idx: c_int,
    traceback_idx: c_int,
    convert_to_callback_error: c_int,
) {
    let wrapped_error = mlua_expect!(
        get_gc_userdata::<WrappedError>(state, error_idx).as_mut(),
        "cannot get <WrappedError>"
    );

    if convert_to_callback_error != 0 {
        let traceback = wrapped_error
            .1
            .take()
            .unwrap_or_else(|| "<not enough stack space for traceback>".into());
        let cause = Arc::new(wrapped_error.0.clone());
        wrapped_error.0 = Error::CallbackError { traceback, cause };
    } else {
        wrapped_error.1 = Some(to_string(state, traceback_idx));
    }
}

// Returns Lua main thread for Lua >= 5.2 or checks that the passed thread is main for Lua 5.1.
// Does not call lua_checkstack, uses 1 stack space.
pub unsafe fn get_main_state(state: *mut ffi::lua_State) -> Option<*mut ffi::lua_State> {
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    {
        ffi::lua_rawgeti(state, ffi::LUA_REGISTRYINDEX, ffi::LUA_RIDX_MAINTHREAD);
        let main_state = ffi::lua_tothread(state, -1);
        ffi::lua_pop(state, 1);
        Some(main_state)
    }
    #[cfg(any(feature = "lua51", feature = "luajit"))]
    {
        // Check the current state first
        let is_main_state = ffi::lua_pushthread(state) == 1;
        ffi::lua_pop(state, 1);
        if is_main_state {
            Some(state)
        } else {
            None
        }
    }
}

// Pushes a WrappedError to the top of the stack.
// Uses 2 stack spaces and does not call checkstack.
pub unsafe fn push_wrapped_error(state: *mut ffi::lua_State, err: Error) -> Result<()> {
    let error_ud = ffi::safe::lua_newwrappederror(state)? as *mut WrappedError;
    ptr::write(error_ud, WrappedError(err, None));
    get_gc_metatable_for::<WrappedError>(state);
    ffi::lua_setmetatable(state, -2);
    Ok(())
}

// Checks if the value at the given index is a WrappedError, and if it is returns a pointer to it,
// otherwise returns null.
// Uses 2 stack spaces and does not call checkstack.
pub unsafe fn get_wrapped_error(state: *mut ffi::lua_State, index: c_int) -> *const Error {
    let ud = get_gc_userdata::<WrappedError>(state, index);
    if ud.is_null() {
        return ptr::null();
    }
    &(*ud).0
}

// Initialize the internal (with __gc method) metatable for a type T.
// Uses 6 stack spaces and calls checkstack.
pub unsafe fn init_gc_metatable_for<T: Any>(
    state: *mut ffi::lua_State,
    customize_fn: Option<fn(*mut ffi::lua_State) -> Result<()>>,
) -> Result<*const u8> {
    check_stack(state, 6)?;

    let type_id = TypeId::of::<T>();
    let ref_addr = {
        let mut mt_cache = mlua_expect!(METATABLE_CACHE.lock(), "cannot lock metatable cache");
        mlua_assert!(
            mt_cache.capacity() - mt_cache.len() > 0,
            "out of metatable cache capacity"
        );
        mt_cache.insert(type_id, 0);
        &mt_cache[&type_id] as *const u8
    };

    ffi::safe::lua_createtable(state, 0, 3)?;

    ffi::safe::lua_pushrclosure(state, userdata_destructor::<T>, 0)?;
    ffi::safe::lua_rawsetfield(state, -2, "__gc")?;

    ffi::lua_pushboolean(state, 0);
    ffi::safe::lua_rawsetfield(state, -2, "__metatable")?;

    if let Some(f) = customize_fn {
        f(state)?;
    }

    ffi::safe::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, ref_addr as *mut c_void)?;

    Ok(ref_addr)
}

pub unsafe fn get_gc_metatable_for<T: Any>(state: *mut ffi::lua_State) {
    let type_id = TypeId::of::<T>();
    let ref_addr = {
        let mt_cache = mlua_expect!(METATABLE_CACHE.lock(), "cannot lock metatable cache");
        mlua_expect!(mt_cache.get(&type_id), "gc metatable does not exist") as *const u8
    };
    ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, ref_addr as *const c_void);
}

// Initialize the error, panic, and destructed userdata metatables.
// Returns address of WrappedError and WrappedPanic metatables in Lua registry.
pub unsafe fn init_error_registry(state: *mut ffi::lua_State) -> Result<(*const u8, *const u8)> {
    check_stack(state, 7)?;

    // Create error and panic metatables

    unsafe extern "C" fn error_tostring(state: *mut ffi::lua_State) -> c_int {
        callback_error(state, |_| {
            check_stack(state, 3)?;

            let wrapped_error = get_gc_userdata::<WrappedError>(state, -1).as_ref();
            let err_buf = if let Some(error) = wrapped_error {
                let err_buf_key = &ERROR_PRINT_BUFFER_KEY as *const u8 as *const c_void;
                ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, err_buf_key);
                let err_buf = ffi::lua_touserdata(state, -1) as *mut String;
                ffi::lua_pop(state, 2);

                (*err_buf).clear();
                // Depending on how the API is used and what error types scripts are given, it may
                // be possible to make this consume arbitrary amounts of memory (for example, some
                // kind of recursive error structure?)
                let _ = write!(&mut (*err_buf), "{}", error.0);
                match error {
                    WrappedError(Error::CallbackError { .. }, _) => {}
                    // This is a workaround to avoid having duplicate traceback for string errors
                    // with attached traceback at the `error_traceback` (message handler) stage.
                    WrappedError(Error::RuntimeError(ref err), _)
                        if err.contains("\nstack traceback:\n") => {}
                    WrappedError(_, Some(ref traceback)) => {
                        let _ = write!(&mut (*err_buf), "\n{}", traceback);
                    }
                    _ => {}
                }
                // Find first two sources that caused the error
                let mut source1 = error.0.source();
                let mut source0 = source1.and_then(|s| s.source());
                while let Some(source) = source0.and_then(|s| s.source()) {
                    source1 = source0;
                    source0 = Some(source);
                }
                match (source1, source0) {
                    (_, Some(error0)) if error0.to_string().contains("\nstack traceback:\n") => {
                        let _ = write!(&mut (*err_buf), "\ncaused by: {}", error0);
                    }
                    (Some(error1), Some(error0)) => {
                        let _ = write!(&mut (*err_buf), "\ncaused by: {}", error0);
                        let s = error1.to_string();
                        if let Some(traceback) = s.splitn(2, "\nstack traceback:\n").nth(1) {
                            let _ = write!(&mut (*err_buf), "\nstack traceback:\n{}", traceback);
                        }
                    }
                    (Some(error1), None) => {
                        let _ = write!(&mut (*err_buf), "\ncaused by: {}", error1);
                    }
                    _ => {}
                }
                Ok(err_buf)
            } else if let Some(panic) = get_gc_userdata::<WrappedPanic>(state, -1).as_ref() {
                if let Some(ref p) = (*panic).0 {
                    let err_buf_key = &ERROR_PRINT_BUFFER_KEY as *const u8 as *const c_void;
                    ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, err_buf_key);
                    let err_buf = ffi::lua_touserdata(state, -1) as *mut String;
                    (*err_buf).clear();
                    ffi::lua_pop(state, 2);

                    if let Some(msg) = p.downcast_ref::<&str>() {
                        let _ = write!(&mut (*err_buf), "{}", msg);
                    } else if let Some(msg) = p.downcast_ref::<String>() {
                        let _ = write!(&mut (*err_buf), "{}", msg);
                    } else {
                        let _ = write!(&mut (*err_buf), "<panic>");
                    };
                    Ok(err_buf)
                } else {
                    Err(Error::PreviouslyResumedPanic)
                }
            } else {
                // I'm not sure whether this is possible to trigger without bugs in mlua?
                Err(Error::UserDataTypeMismatch)
            }?;

            ffi::safe::lua_pushstring(state, &*err_buf)?;
            (*err_buf).clear();

            Ok(1)
        })
    }

    let wrapped_error_key = init_gc_metatable_for::<WrappedError>(
        state,
        Some(|state| {
            ffi::safe::lua_pushrclosure(state, error_tostring, 0)?;
            ffi::safe::lua_rawsetfield(state, -2, "__tostring")
        }),
    )?;

    let wrapped_panic_key = init_gc_metatable_for::<WrappedPanic>(
        state,
        Some(|state| {
            ffi::safe::lua_pushrclosure(state, error_tostring, 0)?;
            ffi::safe::lua_rawsetfield(state, -2, "__tostring")
        }),
    )?;

    // Create destructed userdata metatable

    unsafe extern "C" fn destructed_error(state: *mut ffi::lua_State) -> c_int {
        callback_error(state, |_| {
            check_stack(state, 2)?;
            let error_ud = ffi::safe::lua_newwrappederror(state)? as *mut WrappedError;
            ptr::write(error_ud, WrappedError(Error::CallbackDestructed, None));
            get_gc_metatable_for::<WrappedError>(state);
            ffi::lua_setmetatable(state, -2);
            Ok(-1) // to trigger lua_error
        })
    }

    ffi::safe::lua_createtable(state, 0, 26)?;
    ffi::safe::lua_pushrclosure(state, destructed_error, 0)?;
    for &method in &[
        "__add",
        "__sub",
        "__mul",
        "__div",
        "__mod",
        "__pow",
        "__unm",
        #[cfg(any(feature = "lua54", feature = "lua53"))]
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
        #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
        "__pairs",
        #[cfg(any(feature = "lua53", feature = "lua52"))]
        "__ipairs",
        #[cfg(feature = "lua54")]
        "__close",
    ] {
        ffi::lua_pushvalue(state, -1);
        ffi::safe::lua_rawsetfield(state, -3, method)?;
    }
    ffi::lua_pop(state, 1);

    let destructed_metatable_key = &DESTRUCTED_USERDATA_METATABLE as *const u8 as *const c_void;
    ffi::safe::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, destructed_metatable_key)?;

    // Create error print buffer
    init_gc_metatable_for::<String>(state, None)?;
    push_gc_userdata(state, String::new())?;
    let err_buf_key = &ERROR_PRINT_BUFFER_KEY as *const u8 as *const c_void;
    ffi::safe::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, err_buf_key)?;

    Ok((wrapped_error_key, wrapped_panic_key))
}

pub(crate) struct WrappedError(pub Error, pub Option<String>);
pub(crate) struct WrappedPanic(pub Option<Box<dyn Any + Send + 'static>>);

// Converts the given lua value to a string in a reasonable format without causing a Lua error or
// panicking.
unsafe fn to_string(state: *mut ffi::lua_State, index: c_int) -> String {
    match ffi::lua_type(state, index) {
        ffi::LUA_TNONE => "<none>".to_string(),
        ffi::LUA_TNIL => "<nil>".to_string(),
        ffi::LUA_TBOOLEAN => (ffi::lua_toboolean(state, index) != 1).to_string(),
        ffi::LUA_TLIGHTUSERDATA => {
            format!("<lightuserdata {:?}>", ffi::lua_topointer(state, index))
        }
        ffi::LUA_TNUMBER => {
            let mut isint = 0;
            let i = ffi::lua_tointegerx(state, -1, &mut isint);
            if isint == 0 {
                ffi::lua_tonumber(state, index).to_string()
            } else {
                i.to_string()
            }
        }
        ffi::LUA_TSTRING => {
            let mut size = 0;
            // This will not trigger a 'm' error, because the reference is guaranteed to be of
            // string type
            let data = ffi::lua_tolstring(state, index, &mut size);
            String::from_utf8_lossy(slice::from_raw_parts(data as *const u8, size)).into_owned()
        }
        ffi::LUA_TTABLE => format!("<table {:?}>", ffi::lua_topointer(state, index)),
        ffi::LUA_TFUNCTION => format!("<function {:?}>", ffi::lua_topointer(state, index)),
        ffi::LUA_TUSERDATA => format!("<userdata {:?}>", ffi::lua_topointer(state, index)),
        ffi::LUA_TTHREAD => format!("<thread {:?}>", ffi::lua_topointer(state, index)),
        _ => "<unknown>".to_string(),
    }
}

pub(crate) unsafe fn get_destructed_userdata_metatable(state: *mut ffi::lua_State) {
    let key = &DESTRUCTED_USERDATA_METATABLE as *const u8 as *const c_void;
    ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, key);
}

static DESTRUCTED_USERDATA_METATABLE: u8 = 0;
static ERROR_PRINT_BUFFER_KEY: u8 = 0;
