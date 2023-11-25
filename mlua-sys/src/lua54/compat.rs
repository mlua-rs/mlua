//! MLua compatibility layer for Lua 5.4

use super::lua::*;
use std::os::raw::c_int;

#[inline(always)]
pub unsafe fn lua_error(L: *mut lua_State) -> ! {
    lua_error_(L);
    unreachable!()
}

#[inline(always)]
pub unsafe fn lua_rawlen(L: *mut lua_State, idx: c_int) -> usize {
    lua_rawlen_(L, idx) as usize
}
