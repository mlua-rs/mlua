//! Contains definitions from `lua.h`.

use std::os::raw::c_int;

#[cfg(feature = "lua54")]
pub use super::lua54::lua::*;

#[cfg(feature = "lua53")]
pub use super::lua53::lua::*;

#[cfg(feature = "lua52")]
pub use super::lua52::lua::*;

#[cfg(any(feature = "lua51", feature = "luajit"))]
pub use super::lua51::lua::*;

#[cfg(feature = "lua52")]
pub use super::compat53::{
    lua_dump, lua_getextraspace, lua_getfield, lua_getglobal, lua_geti, lua_gettable,
    lua_getuservalue, lua_isinteger, lua_pushlstring, lua_rawget, lua_rawgeti, lua_rawgetp,
    lua_rawseti, lua_rotate, lua_seti, lua_stringtonumber, lua_tointeger, lua_tointegerx,
    LUA_EXTRASPACE,
};

#[cfg(any(feature = "lua51", feature = "luajit"))]
pub use super::compat53::{
    lua_absindex, lua_arith, lua_compare, lua_copy, lua_dump, lua_getextraspace, lua_getfield,
    lua_getglobal, lua_geti, lua_gettable, lua_getuservalue, lua_isinteger, lua_len,
    lua_pushglobaltable, lua_pushlstring, lua_pushstring, lua_rawget, lua_rawgeti, lua_rawgetp,
    lua_rawlen, lua_rawseti, lua_rawsetp, lua_rotate, lua_seti, lua_setuservalue,
    lua_stringtonumber, lua_tointeger, lua_tointegerx, lua_tonumberx,
};

#[cfg(any(feature = "lua52", feature = "lua53", feature = "lua54",))]
pub const LUA_MAX_UPVALUES: c_int = 255;

#[cfg(any(feature = "lua51", all(feature = "luajit", not(feature = "vendored"))))]
pub const LUA_MAX_UPVALUES: c_int = 60;

#[cfg(all(feature = "luajit", feature = "vendored"))]
pub const LUA_MAX_UPVALUES: c_int = 120;

//
// Lua 5.4 compatibility layer
//

#[cfg(any(
    feature = "lua53",
    feature = "lua52",
    feature = "lua51",
    feature = "luajit"
))]
#[inline(always)]
pub unsafe fn lua_resume(
    L: *mut lua_State,
    from: *mut lua_State,
    narg: c_int,
    nres: *mut c_int,
) -> c_int {
    #[cfg(any(feature = "lua51", feature = "luajit"))]
    let _ = from;
    #[cfg(any(feature = "lua51", feature = "luajit"))]
    let ret = lua_resume_(L, narg);

    #[cfg(any(feature = "lua53", feature = "lua52"))]
    let ret = lua_resume_(L, from, narg);

    if ret == LUA_OK || ret == LUA_YIELD {
        *nres = lua_gettop(L);
    }
    ret
}

#[cfg(any(feature = "lua54", all(feature = "luajit", feature = "vendored")))]
pub unsafe fn lua_resetthreadx(L: *mut lua_State, th: *mut lua_State) -> c_int {
    #[cfg(all(feature = "luajit", feature = "vendored"))]
    {
        lua_resetthread(L, th);
        LUA_OK
    }
    #[cfg(feature = "lua54")]
    {
        let _ = L;
        lua_resetthread(th)
    }
}
