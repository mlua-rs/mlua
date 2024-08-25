//! MLua compatibility layer for Lua 5.3

use std::os::raw::{c_char, c_int};

use super::lauxlib::*;
use super::lua::*;

#[inline(always)]
pub unsafe fn lua_resume(L: *mut lua_State, from: *mut lua_State, narg: c_int, nres: *mut c_int) -> c_int {
    let ret = lua_resume_(L, from, narg);
    if (ret == LUA_OK || ret == LUA_YIELD) && !(nres.is_null()) {
        *nres = lua_gettop(L);
    }
    ret
}

pub unsafe fn luaL_loadbufferenv(
    L: *mut lua_State,
    data: *const c_char,
    size: usize,
    name: *const c_char,
    mode: *const c_char,
    mut env: c_int,
) -> c_int {
    if env != 0 {
        env = lua_absindex(L, env);
    }
    let status = luaL_loadbufferx(L, data, size, name, mode);
    if status == LUA_OK && env != 0 {
        lua_pushvalue(L, env);
        lua_setupvalue(L, -2, 1);
    }
    status
}
