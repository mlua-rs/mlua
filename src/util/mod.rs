use std::borrow::Cow;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_void};
use std::{ptr, slice, str};

use crate::error::{Error, Result};

pub(crate) use error::{
    error_traceback, error_traceback_thread, init_error_registry, pop_error, protect_lua_call,
    protect_lua_closure, WrappedFailure,
};
pub(crate) use short_names::short_type_name;
pub(crate) use types::TypeKey;
pub(crate) use userdata::{
    get_destructed_userdata_metatable, get_internal_metatable, get_internal_userdata, get_userdata,
    init_internal_metatable, push_internal_userdata, push_userdata, take_userdata,
    DESTRUCTED_USERDATA_METATABLE,
};

#[cfg(not(feature = "luau"))]
pub(crate) use userdata::push_uninit_userdata;

// Checks that Lua has enough free stack space for future stack operations. On failure, this will
// panic with an internal error message.
#[inline]
pub(crate) unsafe fn assert_stack(state: *mut ffi::lua_State, amount: c_int) {
    // TODO: This should only be triggered when there is a logic error in `mlua`. In the future,
    // when there is a way to be confident about stack safety and test it, this could be enabled
    // only when `cfg!(debug_assertions)` is true.
    mlua_assert!(ffi::lua_checkstack(state, amount) != 0, "out of stack space");
}

// Checks that Lua has enough free stack space and returns `Error::StackError` on failure.
#[inline]
pub(crate) unsafe fn check_stack(state: *mut ffi::lua_State, amount: c_int) -> Result<()> {
    if ffi::lua_checkstack(state, amount) == 0 {
        Err(Error::StackError)
    } else {
        Ok(())
    }
}

pub(crate) struct StackGuard {
    state: *mut ffi::lua_State,
    top: c_int,
}

impl StackGuard {
    // Creates a StackGuard instance with record of the stack size, and on Drop will check the
    // stack size and drop any extra elements. If the stack size at the end is *smaller* than at
    // the beginning, this is considered a fatal logic error and will result in a panic.
    #[inline]
    pub(crate) unsafe fn new(state: *mut ffi::lua_State) -> StackGuard {
        StackGuard {
            state,
            top: ffi::lua_gettop(state),
        }
    }

    // Same as `new()`, but allows specifying the expected stack size at the end of the scope.
    #[inline]
    pub(crate) fn with_top(state: *mut ffi::lua_State, top: c_int) -> StackGuard {
        StackGuard { state, top }
    }

    #[inline]
    pub(crate) fn keep(&mut self, n: c_int) {
        self.top += n;
    }
}

impl Drop for StackGuard {
    #[track_caller]
    fn drop(&mut self) {
        unsafe {
            let top = ffi::lua_gettop(self.state);
            if top < self.top {
                mlua_panic!("{} too many stack values popped", self.top - top)
            }
            if top > self.top {
                ffi::lua_settop(self.state, self.top);
            }
        }
    }
}

// Uses 3 (or 1 if unprotected) stack spaces, does not call checkstack.
#[inline(always)]
pub(crate) unsafe fn push_string(state: *mut ffi::lua_State, s: &[u8], protect: bool) -> Result<()> {
    // Always use protected mode if the string is too long
    if protect || s.len() >= const { 1 << 30 } {
        protect_lua!(state, 0, 1, |state| {
            ffi::lua_pushlstring(state, s.as_ptr() as *const c_char, s.len());
        })
    } else {
        ffi::lua_pushlstring(state, s.as_ptr() as *const c_char, s.len());
        Ok(())
    }
}

// Uses 3 stack spaces (when protect), does not call checkstack.
#[cfg(feature = "luau")]
#[inline(always)]
pub(crate) unsafe fn push_buffer(state: *mut ffi::lua_State, b: &[u8], protect: bool) -> Result<()> {
    let data = if protect {
        protect_lua!(state, 0, 1, |state| ffi::lua_newbuffer(state, b.len()))?
    } else {
        ffi::lua_newbuffer(state, b.len())
    };
    let buf = slice::from_raw_parts_mut(data as *mut u8, b.len());
    buf.copy_from_slice(b);
    Ok(())
}

// Uses 3 stack spaces, does not call checkstack.
#[inline]
pub(crate) unsafe fn push_table(
    state: *mut ffi::lua_State,
    narr: usize,
    nrec: usize,
    protect: bool,
) -> Result<()> {
    let narr: c_int = narr.try_into().unwrap_or(c_int::MAX);
    let nrec: c_int = nrec.try_into().unwrap_or(c_int::MAX);
    if protect || narr >= const { 1 << 30 } || nrec >= const { 1 << 27 } {
        protect_lua!(state, 0, 1, |state| ffi::lua_createtable(state, narr, nrec))
    } else {
        ffi::lua_createtable(state, narr, nrec);
        Ok(())
    }
}

// Uses 4 stack spaces, does not call checkstack.
pub(crate) unsafe fn rawget_field(state: *mut ffi::lua_State, table: c_int, field: &str) -> Result<c_int> {
    ffi::lua_pushvalue(state, table);
    protect_lua!(state, 1, 1, |state| {
        ffi::lua_pushlstring(state, field.as_ptr() as *const c_char, field.len());
        ffi::lua_rawget(state, -2)
    })
}

// Uses 4 stack spaces, does not call checkstack.
pub(crate) unsafe fn rawset_field(state: *mut ffi::lua_State, table: c_int, field: &str) -> Result<()> {
    ffi::lua_pushvalue(state, table);
    protect_lua!(state, 2, 0, |state| {
        ffi::lua_pushlstring(state, field.as_ptr() as *const c_char, field.len());
        ffi::lua_rotate(state, -3, 2);
        ffi::lua_rawset(state, -3);
    })
}

// A variant of `pcall` that does not allow Lua to catch Rust panics from `callback_error`.
pub(crate) unsafe extern "C-unwind" fn safe_pcall(state: *mut ffi::lua_State) -> c_int {
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
        let wf_ud = get_internal_userdata::<WrappedFailure>(state, -1, ptr::null());
        if let Some(WrappedFailure::Panic(_)) = wf_ud.as_ref() {
            ffi::lua_error(state);
        }
        ffi::lua_pushboolean(state, 0);
        ffi::lua_insert(state, -2);
        2
    }
}

// A variant of `xpcall` that does not allow Lua to catch Rust panics from `callback_error`.
pub(crate) unsafe extern "C-unwind" fn safe_xpcall(state: *mut ffi::lua_State) -> c_int {
    unsafe extern "C-unwind" fn xpcall_msgh(state: *mut ffi::lua_State) -> c_int {
        ffi::luaL_checkstack(state, 2, ptr::null());

        let wf_ud = get_internal_userdata::<WrappedFailure>(state, -1, ptr::null());
        if let Some(WrappedFailure::Panic(_)) = wf_ud.as_ref() {
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
        let wf_ud = get_internal_userdata::<WrappedFailure>(state, -1, ptr::null());
        if let Some(WrappedFailure::Panic(_)) = wf_ud.as_ref() {
            ffi::lua_error(state);
        }
        ffi::lua_pushboolean(state, 0);
        ffi::lua_insert(state, -2);
        2
    }
}

// Returns Lua main thread for Lua >= 5.2 or checks that the passed thread is main for Lua 5.1.
// Does not call lua_checkstack, uses 1 stack space.
pub(crate) unsafe fn get_main_state(state: *mut ffi::lua_State) -> Option<*mut ffi::lua_State> {
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
            #[cfg(not(feature = "luau-vector4"))]
            return format!("vector({x}, {y}, {z})");
            #[cfg(feature = "luau-vector4")]
            return format!("vector({x}, {y}, {z}, {w})", w = *v.add(3));
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
        #[cfg(feature = "luau")]
        ffi::LUA_TBUFFER => format!("<buffer {:?}>", ffi::lua_topointer(state, index)),
        type_id => {
            let type_name = CStr::from_ptr(ffi::lua_typename(state, type_id)).to_string_lossy();
            format!("<{type_name} {:?}>", ffi::lua_topointer(state, index))
        }
    }
}

#[inline(always)]
pub(crate) unsafe fn get_metatable_ptr(state: *mut ffi::lua_State, index: c_int) -> *const c_void {
    #[cfg(feature = "luau")]
    return ffi::lua_getmetatablepointer(state, index);

    #[cfg(not(feature = "luau"))]
    if ffi::lua_getmetatable(state, index) == 0 {
        ptr::null()
    } else {
        let p = ffi::lua_topointer(state, -1);
        ffi::lua_pop(state, 1);
        p
    }
}

pub(crate) unsafe fn ptr_to_str<'a>(input: *const c_char) -> Option<&'a str> {
    if input.is_null() {
        return None;
    }
    str::from_utf8(CStr::from_ptr(input).to_bytes()).ok()
}

pub(crate) unsafe fn ptr_to_lossy_str<'a>(input: *const c_char) -> Option<Cow<'a, str>> {
    if input.is_null() {
        return None;
    }
    Some(String::from_utf8_lossy(CStr::from_ptr(input).to_bytes()))
}

pub(crate) fn linenumber_to_usize(n: c_int) -> Option<usize> {
    match n {
        n if n < 0 => None,
        n => Some(n as usize),
    }
}

mod error;
mod short_names;
mod types;
mod userdata;
