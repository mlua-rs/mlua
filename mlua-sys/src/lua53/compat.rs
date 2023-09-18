//! MLua compatibility layer for Lua 5.3

use std::ffi::c_int;

use super::lua::*;

#[inline(always)]
pub unsafe fn lua_resume(
    L: *mut lua_State,
    from: *mut lua_State,
    narg: c_int,
    nres: *mut c_int,
) -> c_int {
    let ret = lua_resume_(L, from, narg);
    if (ret == LUA_OK || ret == LUA_YIELD) && !(nres.is_null()) {
        *nres = lua_gettop(L);
    }
    ret
}
