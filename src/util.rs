use std::any::{Any, TypeId};
use std::ffi::CStr;
use std::fmt::Write;
use std::mem::MaybeUninit;
use std::os::raw::{c_char, c_int, c_void};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::{mem, ptr, slice};

use once_cell::sync::Lazy;
use rustc_hash::FxHashMap;

use crate::error::{Error, Result};
use crate::ffi;

static METATABLE_CACHE: Lazy<FxHashMap<TypeId, u8>> = Lazy::new(|| {
    let mut map = FxHashMap::with_capacity_and_hasher(32, Default::default());
    crate::lua::init_metatable_cache(&mut map);
    map.insert(TypeId::of::<WrappedFailure>(), 0);
    map.insert(TypeId::of::<String>(), 0);
    map
});

// Checks that Lua has enough free stack space for future stack operations. On failure, this will
// panic with an internal error message.
#[inline]
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
#[inline]
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
    #[inline]
    pub unsafe fn new(state: *mut ffi::lua_State) -> StackGuard {
        StackGuard {
            state,
            top: ffi::lua_gettop(state),
            extra: 0,
        }
    }

    // Similar to `new`, but checks and keeps `extra` elements from top of the stack on Drop.
    #[inline]
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
// always `LUA_MULTRET`. Provided function must *not* panic, and since it will generally be lonjmping,
// should not contain any values that implements Drop.
// Internally uses 2 extra stack spaces, and does not call checkstack.
pub unsafe fn protect_lua_call(
    state: *mut ffi::lua_State,
    nargs: c_int,
    f: unsafe extern "C" fn(*mut ffi::lua_State) -> c_int,
) -> Result<()> {
    let stack_start = ffi::lua_gettop(state) - nargs;

    ffi::lua_pushcfunction(state, error_traceback);
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

// Call a function that calls into the Lua API and may trigger a Lua error (longjmp) in a safe way.
// Wraps the inner function in a call to `lua_pcall`, so the inner function only has access to a
// limited lua stack. `nargs` and `nresults` are similar to the parameters of `lua_pcall`, but the
// given function return type is not the return value count, instead the inner function return
// values are assumed to match the `nresults` param. Provided function must *not* panic, and since it
// will generally be lonjmping, should not contain any values that implements Drop.
// Internally uses 3 extra stack spaces, and does not call checkstack.
pub unsafe fn protect_lua_closure<F, R>(
    state: *mut ffi::lua_State,
    nargs: c_int,
    nresults: c_int,
    f: F,
) -> Result<R>
where
    F: Fn(*mut ffi::lua_State) -> R,
    R: Copy,
{
    struct Params<F, R: Copy> {
        function: F,
        result: MaybeUninit<R>,
        nresults: c_int,
    }

    unsafe extern "C" fn do_call<F, R>(state: *mut ffi::lua_State) -> c_int
    where
        F: Fn(*mut ffi::lua_State) -> R,
        R: Copy,
    {
        let params = ffi::lua_touserdata(state, -1) as *mut Params<F, R>;
        ffi::lua_pop(state, 1);

        (*params).result.write(((*params).function)(state));

        if (*params).nresults == ffi::LUA_MULTRET {
            ffi::lua_gettop(state)
        } else {
            (*params).nresults
        }
    }

    let stack_start = ffi::lua_gettop(state) - nargs;

    ffi::lua_pushcfunction(state, error_traceback);
    ffi::lua_pushcfunction(state, do_call::<F, R>);
    if nargs > 0 {
        ffi::lua_rotate(state, stack_start + 1, 2);
    }

    let mut params = Params {
        function: f,
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

    match get_gc_userdata::<WrappedFailure>(state, -1, ptr::null()).as_mut() {
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
}

// Uses 3 (or 1 if unprotected) stack spaces, does not call checkstack.
#[inline(always)]
pub unsafe fn push_string(state: *mut ffi::lua_State, s: &[u8], protect: bool) -> Result<()> {
    if protect {
        protect_lua!(state, 0, 1, |state| {
            ffi::lua_pushlstring(state, s.as_ptr() as *const c_char, s.len());
        })
    } else {
        ffi::lua_pushlstring(state, s.as_ptr() as *const c_char, s.len());
        Ok(())
    }
}

// Uses 3 stack spaces, does not call checkstack.
#[inline]
pub unsafe fn push_table(
    state: *mut ffi::lua_State,
    narr: c_int,
    nrec: c_int,
    protect: bool,
) -> Result<()> {
    if protect {
        protect_lua!(state, 0, 1, |state| ffi::lua_createtable(state, narr, nrec))
    } else {
        ffi::lua_createtable(state, narr, nrec);
        Ok(())
    }
}

// Uses 4 stack spaces, does not call checkstack.
pub unsafe fn rawset_field<S>(state: *mut ffi::lua_State, table: c_int, field: &S) -> Result<()>
where
    S: AsRef<[u8]> + ?Sized,
{
    let field = field.as_ref();
    ffi::lua_pushvalue(state, table);
    protect_lua!(state, 2, 0, |state| {
        ffi::lua_pushlstring(state, field.as_ptr() as *const c_char, field.len());
        ffi::lua_rotate(state, -3, 2);
        ffi::lua_rawset(state, -3);
    })
}

// Internally uses 3 stack spaces, does not call checkstack.
#[cfg(not(feature = "luau"))]
#[inline]
pub unsafe fn push_userdata<T>(state: *mut ffi::lua_State, t: T, protect: bool) -> Result<()> {
    let ud = if protect {
        protect_lua!(state, 0, 1, |state| {
            ffi::lua_newuserdata(state, mem::size_of::<T>()) as *mut T
        })?
    } else {
        ffi::lua_newuserdata(state, mem::size_of::<T>()) as *mut T
    };
    ptr::write(ud, t);
    Ok(())
}

// Internally uses 3 stack spaces, does not call checkstack.
#[cfg(feature = "luau")]
#[inline]
pub unsafe fn push_userdata<T>(state: *mut ffi::lua_State, t: T, protect: bool) -> Result<()> {
    unsafe extern "C" fn destructor<T>(ud: *mut c_void) {
        ptr::drop_in_place(ud as *mut T);
    }

    let size = mem::size_of::<T>();
    let ud = if protect {
        protect_lua!(state, 0, 1, |state| {
            ffi::lua_newuserdatadtor(state, size, destructor::<T>) as *mut T
        })?
    } else {
        ffi::lua_newuserdatadtor(state, size, destructor::<T>) as *mut T
    };
    ptr::write(ud, t);

    Ok(())
}

// Internally uses 3 stack spaces, does not call checkstack.
#[cfg(feature = "lua54")]
#[inline]
pub unsafe fn push_userdata_uv<T>(
    state: *mut ffi::lua_State,
    t: T,
    nuvalue: c_int,
    protect: bool,
) -> Result<()> {
    let ud = if protect {
        protect_lua!(state, 0, 1, |state| {
            ffi::lua_newuserdatauv(state, mem::size_of::<T>(), nuvalue) as *mut T
        })?
    } else {
        ffi::lua_newuserdatauv(state, mem::size_of::<T>(), nuvalue) as *mut T
    };
    ptr::write(ud, t);
    Ok(())
}

#[inline]
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
    let ud = get_userdata::<T>(state, -1);

    // Update userdata tag to disable destructor and mark as destructed
    #[cfg(feature = "luau")]
    ffi::lua_setuserdatatag(state, -1, 1);

    ffi::lua_pop(state, 1);
    ptr::read(ud)
}

// Pushes the userdata and attaches a metatable with __gc method.
// Internally uses 3 stack spaces, does not call checkstack.
pub unsafe fn push_gc_userdata<T: Any>(
    state: *mut ffi::lua_State,
    t: T,
    protect: bool,
) -> Result<()> {
    push_userdata(state, t, protect)?;
    get_gc_metatable::<T>(state);
    ffi::lua_setmetatable(state, -2);
    Ok(())
}

// Uses 2 stack spaces, does not call checkstack
pub unsafe fn get_gc_userdata<T: Any>(
    state: *mut ffi::lua_State,
    index: c_int,
    mt_ptr: *const c_void,
) -> *mut T {
    let ud = ffi::lua_touserdata(state, index) as *mut T;
    if ud.is_null() || ffi::lua_getmetatable(state, index) == 0 {
        return ptr::null_mut();
    }
    if !mt_ptr.is_null() {
        let ud_mt_ptr = ffi::lua_topointer(state, -1);
        ffi::lua_pop(state, 1);
        if !ptr::eq(ud_mt_ptr, mt_ptr) {
            return ptr::null_mut();
        }
    } else {
        get_gc_metatable::<T>(state);
        let res = ffi::lua_rawequal(state, -1, -2);
        ffi::lua_pop(state, 2);
        if res == 0 {
            return ptr::null_mut();
        }
    }
    ud
}

unsafe extern "C" fn lua_error_impl(state: *mut ffi::lua_State) -> c_int {
    ffi::lua_error(state);
}

unsafe extern "C" fn lua_isfunction_impl(state: *mut ffi::lua_State) -> c_int {
    let t = ffi::lua_type(state, -1);
    ffi::lua_pop(state, 1);
    ffi::lua_pushboolean(state, (t == ffi::LUA_TFUNCTION) as c_int);
    1
}

unsafe fn init_userdata_metatable_index(state: *mut ffi::lua_State) -> Result<()> {
    let index_key = &USERDATA_METATABLE_INDEX as *const u8 as *const _;
    if ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, index_key) == ffi::LUA_TFUNCTION {
        return Ok(());
    }
    ffi::lua_pop(state, 1);

    // Create and cache `__index` helper
    let code = cstr!(
        r#"
            local error, isfunction = ...
            return function (__index, field_getters, methods)
                return function (self, key)
                    if field_getters ~= nil then
                        local field_getter = field_getters[key]
                        if field_getter ~= nil then
                            return field_getter(self)
                        end
                    end

                    if methods ~= nil then
                        local method = methods[key]
                        if method ~= nil then
                            return method
                        end
                    end

                    if isfunction(__index) then
                        return __index(self, key)
                    elseif __index == nil then
                        error("attempt to get an unknown field '"..key.."'")
                    else
                        return __index[key]
                    end
                end
            end
    "#
    );
    let code_len = CStr::from_ptr(code).to_bytes().len();
    protect_lua!(state, 0, 1, |state| {
        let ret = ffi::luaL_loadbuffer(state, code, code_len, cstr!("__mlua_index"));
        if ret != ffi::LUA_OK {
            ffi::lua_error(state);
        }
        ffi::lua_pushcfunction(state, lua_error_impl);
        ffi::lua_pushcfunction(state, lua_isfunction_impl);
        ffi::lua_call(state, 2, 1);

        // Store in the registry
        ffi::lua_pushvalue(state, -1);
        ffi::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, index_key);
    })
}

pub unsafe fn init_userdata_metatable_newindex(state: *mut ffi::lua_State) -> Result<()> {
    let newindex_key = &USERDATA_METATABLE_NEWINDEX as *const u8 as *const _;
    if ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, newindex_key) == ffi::LUA_TFUNCTION {
        return Ok(());
    }
    ffi::lua_pop(state, 1);

    // Create and cache `__newindex` helper
    let code = cstr!(
        r#"
            local error, isfunction = ...
            return function (__newindex, field_setters)
                return function (self, key, value)
                    if field_setters ~= nil then
                        local field_setter = field_setters[key]
                        if field_setter ~= nil then
                            field_setter(self, value)
                            return
                        end
                    end

                    if isfunction(__newindex) then
                        __newindex(self, key, value)
                    elseif __newindex == nil then
                        error("attempt to set an unknown field '"..key.."'")
                    else
                        __newindex[key] = value
                    end
                end
            end
    "#
    );
    let code_len = CStr::from_ptr(code).to_bytes().len();
    protect_lua!(state, 0, 1, |state| {
        let ret = ffi::luaL_loadbuffer(state, code, code_len, cstr!("__mlua_newindex"));
        if ret != ffi::LUA_OK {
            ffi::lua_error(state);
        }
        ffi::lua_pushcfunction(state, lua_error_impl);
        ffi::lua_pushcfunction(state, lua_isfunction_impl);
        ffi::lua_call(state, 2, 1);

        // Store in the registry
        ffi::lua_pushvalue(state, -1);
        ffi::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, newindex_key);
    })
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
        // Push `__index` generator function
        init_userdata_metatable_index(state)?;

        push_string(state, b"__index", true)?;
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

                // Generate `__index`
                protect_lua!(state, 4, 1, fn(state) ffi::lua_call(state, 3, 1))?;
            }
            _ => mlua_panic!("improper __index type {}", index_type),
        }

        rawset_field(state, -2, "__index")?;
    }

    if let Some(field_setters) = field_setters {
        // Push `__newindex` generator function
        init_userdata_metatable_newindex(state)?;

        push_string(state, b"__newindex", true)?;
        let newindex_type = ffi::lua_rawget(state, -3);
        match newindex_type {
            ffi::LUA_TNIL | ffi::LUA_TTABLE | ffi::LUA_TFUNCTION => {
                ffi::lua_pushvalue(state, field_setters);
                // Generate `__newindex`
                protect_lua!(state, 3, 1, fn(state) ffi::lua_call(state, 2, 1))?;
            }
            _ => mlua_panic!("improper __newindex type {}", newindex_type),
        }

        rawset_field(state, -2, "__newindex")?;
    }

    #[cfg(not(feature = "luau"))]
    {
        ffi::lua_pushcfunction(state, userdata_destructor::<T>);
        rawset_field(state, -2, "__gc")?;
    }

    ffi::lua_pushboolean(state, 0);
    rawset_field(state, -2, "__metatable")?;

    ffi::lua_pop(state, 1);

    Ok(())
}

#[cfg(not(feature = "luau"))]
pub unsafe extern "C" fn userdata_destructor<T>(state: *mut ffi::lua_State) -> c_int {
    // It's probably NOT a good idea to catch Rust panics in finalizer
    // Lua 5.4 ignores it, other versions generates `LUA_ERRGCMM` without calling message handler
    take_userdata::<T>(state);
    0
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
pub unsafe fn callback_error<F, R>(state: *mut ffi::lua_State, f: F) -> R
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

            let wrapped_error = ud as *mut WrappedFailure;

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
            ptr::write(
                wrapped_error,
                WrappedFailure::Error(Error::CallbackError { traceback, cause }),
            );
            get_gc_metatable::<WrappedFailure>(state);
            ffi::lua_setmetatable(state, -2);

            ffi::lua_error(state)
        }
        Err(p) => {
            ffi::lua_settop(state, 1);
            ptr::write(ud as *mut WrappedFailure, WrappedFailure::Panic(Some(p)));
            get_gc_metatable::<WrappedFailure>(state);
            ffi::lua_setmetatable(state, -2);
            ffi::lua_error(state)
        }
    }
}

pub unsafe extern "C" fn error_traceback(state: *mut ffi::lua_State) -> c_int {
    if ffi::lua_checkstack(state, 2) == 0 {
        // If we don't have enough stack space to even check the error type, do
        // nothing so we don't risk shadowing a rust panic.
        return 1;
    }

    if get_gc_userdata::<WrappedFailure>(state, -1, ptr::null()).is_null() {
        let s = ffi::luaL_tolstring(state, -1, ptr::null_mut());
        if ffi::lua_checkstack(state, ffi::LUA_TRACEBACK_STACK) != 0 {
            ffi::luaL_traceback(state, state, s, 0);
            ffi::lua_remove(state, -2);
        }
    }

    1
}

// A variant of `error_traceback` that can safely inspect another (yielded) thread stack
pub unsafe fn error_traceback_thread(state: *mut ffi::lua_State, thread: *mut ffi::lua_State) {
    // Move error object to the main thread to safely call `__tostring` metamethod if present
    ffi::lua_xmove(thread, state, 1);

    if get_gc_userdata::<WrappedFailure>(state, -1, ptr::null()).is_null() {
        let s = ffi::luaL_tolstring(state, -1, ptr::null_mut());
        if ffi::lua_checkstack(state, ffi::LUA_TRACEBACK_STACK) != 0 {
            ffi::luaL_traceback(state, thread, s, 0);
            ffi::lua_remove(state, -2);
        }
    }
}

// A variant of `pcall` that does not allow Lua to catch Rust panics from `callback_error`.
pub unsafe extern "C" fn safe_pcall(state: *mut ffi::lua_State) -> c_int {
    ffi::luaL_checkstack(state, 2, ptr::null());

    let top = ffi::lua_gettop(state);
    if top == 0 {
        ffi::lua_pushstring(state, cstr!("not enough arguments to pcall"));
        ffi::lua_error(state);
    }

    if ffi::lua_pcall(state, top - 1, ffi::LUA_MULTRET, 0) == ffi::LUA_OK {
        ffi::lua_pushboolean(state, 1);
        ffi::lua_insert(state, 1);
        ffi::lua_gettop(state)
    } else {
        if let Some(WrappedFailure::Panic(_)) =
            get_gc_userdata::<WrappedFailure>(state, -1, ptr::null()).as_ref()
        {
            ffi::lua_error(state);
        }
        ffi::lua_pushboolean(state, 0);
        ffi::lua_insert(state, -2);
        2
    }
}

// A variant of `xpcall` that does not allow Lua to catch Rust panics from `callback_error`.
pub unsafe extern "C" fn safe_xpcall(state: *mut ffi::lua_State) -> c_int {
    unsafe extern "C" fn xpcall_msgh(state: *mut ffi::lua_State) -> c_int {
        ffi::luaL_checkstack(state, 2, ptr::null());

        if let Some(WrappedFailure::Panic(_)) =
            get_gc_userdata::<WrappedFailure>(state, -1, ptr::null()).as_ref()
        {
            1
        } else {
            ffi::lua_pushvalue(state, ffi::lua_upvalueindex(1));
            ffi::lua_insert(state, 1);
            ffi::lua_call(state, ffi::lua_gettop(state) - 1, ffi::LUA_MULTRET);
            ffi::lua_gettop(state)
        }
    }

    ffi::luaL_checkstack(state, 2, ptr::null());

    let top = ffi::lua_gettop(state);
    if top < 2 {
        ffi::lua_pushstring(state, cstr!("not enough arguments to xpcall"));
        ffi::lua_error(state);
    }

    ffi::lua_pushvalue(state, 2);
    ffi::lua_pushcclosure(state, xpcall_msgh, 1);
    ffi::lua_copy(state, 1, 2);
    ffi::lua_replace(state, 1);

    if ffi::lua_pcall(state, ffi::lua_gettop(state) - 2, ffi::LUA_MULTRET, 1) == ffi::LUA_OK {
        ffi::lua_pushboolean(state, 1);
        ffi::lua_insert(state, 2);
        ffi::lua_gettop(state) - 1
    } else {
        if let Some(WrappedFailure::Panic(_)) =
            get_gc_userdata::<WrappedFailure>(state, -1, ptr::null()).as_ref()
        {
            ffi::lua_error(state);
        }
        ffi::lua_pushboolean(state, 0);
        ffi::lua_insert(state, -2);
        2
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
    #[cfg(feature = "luau")]
    Some(ffi::lua_mainthread(state))
}

// Initialize the internal (with __gc method) metatable for a type T.
// Uses 6 stack spaces and calls checkstack.
pub unsafe fn init_gc_metatable<T: Any>(
    state: *mut ffi::lua_State,
    customize_fn: Option<fn(*mut ffi::lua_State) -> Result<()>>,
) -> Result<()> {
    check_stack(state, 6)?;

    push_table(state, 0, 3, true)?;

    #[cfg(not(feature = "luau"))]
    {
        ffi::lua_pushcfunction(state, userdata_destructor::<T>);
        rawset_field(state, -2, "__gc")?;
    }

    ffi::lua_pushboolean(state, 0);
    rawset_field(state, -2, "__metatable")?;

    if let Some(f) = customize_fn {
        f(state)?;
    }

    let type_id = TypeId::of::<T>();
    let ref_addr = &METATABLE_CACHE[&type_id] as *const u8;
    protect_lua!(state, 1, 0, |state| {
        ffi::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, ref_addr as *const c_void);
    })?;

    Ok(())
}

pub unsafe fn get_gc_metatable<T: Any>(state: *mut ffi::lua_State) {
    let type_id = TypeId::of::<T>();
    let ref_addr =
        mlua_expect!(METATABLE_CACHE.get(&type_id), "gc metatable does not exist") as *const u8;
    ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, ref_addr as *const c_void);
}

// Initialize the error, panic, and destructed userdata metatables.
pub unsafe fn init_error_registry(state: *mut ffi::lua_State) -> Result<()> {
    check_stack(state, 7)?;

    // Create error and panic metatables

    unsafe extern "C" fn error_tostring(state: *mut ffi::lua_State) -> c_int {
        callback_error(state, |_| {
            check_stack(state, 3)?;

            let err_buf = match get_gc_userdata::<WrappedFailure>(state, -1, ptr::null()).as_ref() {
                Some(WrappedFailure::Error(error)) => {
                    let err_buf_key = &ERROR_PRINT_BUFFER_KEY as *const u8 as *const c_void;
                    ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, err_buf_key);
                    let err_buf = ffi::lua_touserdata(state, -1) as *mut String;
                    ffi::lua_pop(state, 2);

                    (*err_buf).clear();
                    // Depending on how the API is used and what error types scripts are given, it may
                    // be possible to make this consume arbitrary amounts of memory (for example, some
                    // kind of recursive error structure?)
                    let _ = write!(&mut (*err_buf), "{}", error);
                    Ok(err_buf)
                }
                Some(WrappedFailure::Panic(Some(ref panic))) => {
                    let err_buf_key = &ERROR_PRINT_BUFFER_KEY as *const u8 as *const c_void;
                    ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, err_buf_key);
                    let err_buf = ffi::lua_touserdata(state, -1) as *mut String;
                    (*err_buf).clear();
                    ffi::lua_pop(state, 2);

                    if let Some(msg) = panic.downcast_ref::<&str>() {
                        let _ = write!(&mut (*err_buf), "{}", msg);
                    } else if let Some(msg) = panic.downcast_ref::<String>() {
                        let _ = write!(&mut (*err_buf), "{}", msg);
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

    init_gc_metatable::<WrappedFailure>(
        state,
        Some(|state| {
            ffi::lua_pushcfunction(state, error_tostring);
            rawset_field(state, -2, "__tostring")
        }),
    )?;

    // Create destructed userdata metatable

    unsafe extern "C" fn destructed_error(state: *mut ffi::lua_State) -> c_int {
        callback_error(state, |_| Err(Error::CallbackDestructed))
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
        #[cfg(any(
            feature = "lua54",
            feature = "lua53",
            feature = "lua52",
            feature = "luajit52"
        ))]
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
    init_gc_metatable::<String>(state, None)?;
    push_gc_userdata(state, String::new(), true)?;
    protect_lua!(state, 1, 0, fn(state) {
        let err_buf_key = &ERROR_PRINT_BUFFER_KEY as *const u8 as *const c_void;
        ffi::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, err_buf_key);
    })?;

    Ok(())
}

pub(crate) enum WrappedFailure {
    None,
    Error(Error),
    Panic(Option<Box<dyn Any + Send + 'static>>),
}

impl WrappedFailure {
    pub(crate) unsafe fn new_userdata(state: *mut ffi::lua_State) -> *mut Self {
        let size = mem::size_of::<WrappedFailure>();
        #[cfg(feature = "luau")]
        let ud = {
            unsafe extern "C" fn destructor(p: *mut c_void) {
                ptr::drop_in_place(p as *mut WrappedFailure);
            }
            ffi::lua_newuserdatadtor(state, size, destructor) as *mut Self
        };
        #[cfg(not(feature = "luau"))]
        let ud = ffi::lua_newuserdata(state, size) as *mut Self;
        ptr::write(ud, WrappedFailure::None);
        ud
    }
}

// Converts the given lua value to a string in a reasonable format without causing a Lua error or
// panicking.
pub(crate) unsafe fn to_string(state: *mut ffi::lua_State, index: c_int) -> String {
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
        #[cfg(feature = "luau")]
        ffi::LUA_TVECTOR => {
            let v = ffi::lua_tovector(state, index);
            mlua_debug_assert!(!v.is_null(), "vector is null");
            let (x, y, z) = (*v, *v.add(1), *v.add(2));
            format!("vector({},{},{})", x, y, z)
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

pub(crate) unsafe fn ptr_to_cstr_bytes<'a>(input: *const c_char) -> Option<&'a [u8]> {
    if input.is_null() {
        return None;
    }
    Some(CStr::from_ptr(input).to_bytes())
}

static DESTRUCTED_USERDATA_METATABLE: u8 = 0;
static ERROR_PRINT_BUFFER_KEY: u8 = 0;
static USERDATA_METATABLE_INDEX: u8 = 0;
static USERDATA_METATABLE_NEWINDEX: u8 = 0;
