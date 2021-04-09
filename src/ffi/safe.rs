use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};

use crate::error::Result;
use crate::util::protect_lua;

use super::lua::{lua_CFunction, lua_Debug, lua_Integer, lua_State};

extern "C" {
    #[link_name = "MLUA_WRAPPED_ERROR_SIZE"]
    pub static mut WRAPPED_ERROR_SIZE: usize;
    #[link_name = "MLUA_WRAPPED_PANIC_SIZE"]
    pub static mut WRAPPED_PANIC_SIZE: usize;
    #[link_name = "MLUA_WRAPPED_ERROR_KEY"]
    pub static mut WRAPPED_ERROR_KEY: *const c_void;
    #[link_name = "MLUA_WRAPPED_PANIC_KEY"]
    pub static mut WRAPPED_PANIC_KEY: *const c_void;

    pub fn lua_call_mlua_hook_proc(L: *mut lua_State, ar: *mut lua_Debug);

    pub fn meta_index_impl(state: *mut lua_State) -> c_int;
    pub fn meta_newindex_impl(state: *mut lua_State) -> c_int;
    pub fn bind_call_impl(state: *mut lua_State) -> c_int;
    pub fn error_traceback(state: *mut lua_State) -> c_int;

    fn lua_gc_s(L: *mut lua_State) -> c_int;
    fn luaL_ref_s(L: *mut lua_State) -> c_int;
    fn lua_pushlstring_s(L: *mut lua_State) -> c_int;
    fn lua_tolstring_s(L: *mut lua_State) -> c_int;
    fn lua_newthread_s(L: *mut lua_State) -> c_int;
    fn lua_newuserdata_s(L: *mut lua_State) -> c_int;
    fn lua_pushcclosure_s(L: *mut lua_State) -> c_int;
    fn lua_pushrclosure_s(L: *mut lua_State) -> c_int;
    fn luaL_requiref_s(L: *mut lua_State) -> c_int;
    fn error_traceback_s(L: *mut lua_State) -> c_int;

    fn lua_createtable_s(L: *mut lua_State) -> c_int;
    fn lua_gettable_s(L: *mut lua_State) -> c_int;
    fn lua_settable_s(L: *mut lua_State) -> c_int;
    fn lua_geti_s(L: *mut lua_State) -> c_int;
    fn lua_rawset_s(L: *mut lua_State) -> c_int;
    fn lua_rawseti_s(L: *mut lua_State) -> c_int;
    fn lua_rawsetp_s(L: *mut lua_State) -> c_int;
    fn lua_rawsetfield_s(L: *mut lua_State) -> c_int;
    fn lua_rawinsert_s(L: *mut lua_State) -> c_int;
    fn lua_rawremove_s(L: *mut lua_State) -> c_int;
    fn luaL_len_s(L: *mut lua_State) -> c_int;
    fn lua_next_s(L: *mut lua_State) -> c_int;
}

#[repr(C)]
struct StringArg {
    data: *const c_char,
    len: usize,
}

//
// Common functions
//

// Uses 4 stack spaces
pub unsafe fn lua_gc(state: *mut lua_State, what: c_int, data: c_int) -> Result<c_int> {
    super::lua_pushinteger(state, what as lua_Integer);
    super::lua_pushinteger(state, data as lua_Integer);
    protect_lua(state, 2, lua_gc_s)?;
    let ret = super::lua_tointeger(state, -1) as c_int;
    super::lua_pop(state, 1);
    Ok(ret)
}

// Uses 3 stack spaces
pub unsafe fn luaL_ref(state: *mut lua_State, table: c_int) -> Result<c_int> {
    super::lua_pushvalue(state, table);
    super::lua_rotate(state, -2, 1);
    protect_lua(state, 2, luaL_ref_s)?;
    let ret = super::lua_tointeger(state, -1) as c_int;
    super::lua_pop(state, 1);
    Ok(ret)
}

// Uses 3 stack spaces
pub unsafe fn lua_pushstring<S: AsRef<[u8]> + ?Sized>(state: *mut lua_State, s: &S) -> Result<()> {
    let s = s.as_ref();
    let s = StringArg {
        data: s.as_ptr() as *const c_char,
        len: s.len(),
    };
    super::lua_pushlightuserdata(state, &s as *const StringArg as *mut c_void);
    protect_lua(state, 1, lua_pushlstring_s)
}

// Uses 4 stack spaces
pub unsafe fn lua_tolstring(
    state: *mut lua_State,
    index: c_int,
    len: *mut usize,
) -> Result<*const c_char> {
    let index = super::lua_absindex(state, index);
    super::lua_pushvalue(state, index);
    super::lua_pushlightuserdata(state, len as *mut c_void);
    protect_lua(state, 2, lua_tolstring_s)?;
    let s = super::lua_touserdata(state, -1);
    super::lua_pop(state, 1);
    super::lua_replace(state, index);
    Ok(s as *const c_char)
}

// Uses 2 stack spaces
pub unsafe fn lua_newthread(state: *mut lua_State) -> Result<*mut lua_State> {
    protect_lua(state, 0, lua_newthread_s)?;
    Ok(super::lua_tothread(state, -1))
}

// Uses 3 stack spaces
pub unsafe fn lua_newuserdata(state: *mut lua_State, size: usize) -> Result<*mut c_void> {
    super::lua_pushinteger(state, size as lua_Integer);
    protect_lua(state, 1, lua_newuserdata_s)?;
    Ok(super::lua_touserdata(state, -1))
}

// Uses 4 stack spaces
pub unsafe fn lua_pushcclosure(state: *mut lua_State, f: lua_CFunction, n: c_int) -> Result<()> {
    super::lua_pushlightuserdata(state, f as *mut c_void);
    super::lua_pushinteger(state, n as lua_Integer);
    protect_lua(state, n + 2, lua_pushcclosure_s)
}

// Uses 4 stack spaces
pub unsafe fn lua_pushrclosure(state: *mut lua_State, f: lua_CFunction, n: c_int) -> Result<()> {
    super::lua_pushlightuserdata(state, f as *mut c_void);
    if n > 0 {
        super::lua_rotate(state, -n - 1, 1);
    }
    super::lua_pushinteger(state, n as lua_Integer + 1);
    protect_lua(state, n + 2, lua_pushrclosure_s)
}

// Uses 5 stack spaces
pub unsafe fn luaL_requiref<S: AsRef<[u8]> + ?Sized>(
    state: *mut lua_State,
    modname: &S,
    openf: lua_CFunction,
    glb: c_int,
) -> Result<()> {
    let modname = mlua_expect!(CString::new(modname.as_ref()), "modname contains nil bytes");
    super::lua_pushlightuserdata(state, modname.as_ptr() as *mut c_void);
    super::lua_pushlightuserdata(state, openf as *mut c_void);
    super::lua_pushinteger(state, glb as lua_Integer);
    protect_lua(state, 3, luaL_requiref_s)
}

// Uses 3 stack spaces
pub unsafe fn error_traceback2(state: *mut lua_State, state2: *mut lua_State) -> Result<()> {
    mlua_assert!(
        state != state2,
        "error_traceback2 must be used with two different states"
    );
    super::lua_pushlightuserdata(state, state2);
    protect_lua(state, 1, error_traceback_s)
}

//
// Table functions
//

// Uses 4 stack spaces
pub unsafe fn lua_createtable(state: *mut lua_State, narr: c_int, nrec: c_int) -> Result<()> {
    super::lua_pushinteger(state, narr as lua_Integer);
    super::lua_pushinteger(state, nrec as lua_Integer);
    protect_lua(state, 2, lua_createtable_s)
}

// Uses 3 stack spaces
pub unsafe fn lua_gettable(state: *mut lua_State, table: c_int) -> Result<()> {
    super::lua_pushvalue(state, table);
    super::lua_rotate(state, -2, 1);
    protect_lua(state, 2, lua_gettable_s)
}

// Uses 3 stack spaces
pub unsafe fn lua_settable(state: *mut lua_State, table: c_int) -> Result<()> {
    super::lua_pushvalue(state, table);
    super::lua_rotate(state, -3, 1);
    protect_lua(state, 3, lua_settable_s)
}

// Uses 4 stack spaces
pub unsafe fn lua_geti(state: *mut lua_State, table: c_int, i: lua_Integer) -> Result<c_int> {
    super::lua_pushvalue(state, table);
    super::lua_pushinteger(state, i);
    protect_lua(state, 2, lua_geti_s).map(|_| super::lua_type(state, -1))
}

// Uses 3 stack spaces
pub unsafe fn lua_rawset(state: *mut lua_State, table: c_int) -> Result<()> {
    super::lua_pushvalue(state, table);
    super::lua_rotate(state, -3, 1);
    protect_lua(state, 3, lua_rawset_s)
}

// Uses 4 stack spaces
pub unsafe fn lua_rawseti(state: *mut lua_State, table: c_int, i: lua_Integer) -> Result<()> {
    super::lua_pushvalue(state, table);
    super::lua_rotate(state, -2, 1);
    super::lua_pushinteger(state, i);
    protect_lua(state, 3, lua_rawseti_s)
}

// Uses 4 stack spaces
pub unsafe fn lua_rawsetp(state: *mut lua_State, table: c_int, ptr: *const c_void) -> Result<()> {
    super::lua_pushvalue(state, table);
    super::lua_rotate(state, -2, 1);
    super::lua_pushlightuserdata(state, ptr as *mut c_void);
    protect_lua(state, 3, lua_rawsetp_s)
}

// Uses 4 stack spaces
pub unsafe fn lua_rawsetfield<S>(state: *mut lua_State, table: c_int, field: &S) -> Result<()>
where
    S: AsRef<[u8]> + ?Sized,
{
    let field = field.as_ref();
    let s = StringArg {
        data: field.as_ptr() as *const c_char,
        len: field.len(),
    };
    super::lua_pushvalue(state, table);
    super::lua_pushlightuserdata(state, &s as *const StringArg as *mut c_void);
    super::lua_rotate(state, -3, 2);
    protect_lua(state, 3, lua_rawsetfield_s)
}

// Uses 4 stack spaces
pub unsafe fn lua_rawinsert(state: *mut lua_State, table: c_int, i: lua_Integer) -> Result<()> {
    super::lua_pushvalue(state, table);
    super::lua_rotate(state, -2, 1);
    super::lua_pushinteger(state, i);
    protect_lua(state, 3, lua_rawinsert_s)
}

// Uses 4 stack spaces
pub unsafe fn lua_rawremove(state: *mut lua_State, table: c_int, i: lua_Integer) -> Result<()> {
    super::lua_pushvalue(state, table);
    super::lua_pushinteger(state, i);
    protect_lua(state, 2, lua_rawremove_s)
}

// Uses 3 stack spaces
pub unsafe fn luaL_len(state: *mut lua_State, table: c_int) -> Result<lua_Integer> {
    super::lua_pushvalue(state, table);
    protect_lua(state, 1, luaL_len_s)?;
    let ret = super::lua_tointeger(state, -1);
    super::lua_pop(state, 1);
    Ok(ret)
}

// Uses 3 stack spaces
pub unsafe fn lua_next(state: *mut lua_State, table: c_int) -> Result<lua_Integer> {
    super::lua_pushvalue(state, table);
    super::lua_rotate(state, -2, 1);
    protect_lua(state, 2, lua_next_s)?;
    let ret = super::lua_tointeger(state, -1);
    super::lua_pop(state, 1);
    Ok(ret)
}
