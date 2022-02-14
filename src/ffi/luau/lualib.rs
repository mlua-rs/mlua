//! Contains definitions from `lualib.h`.

use std::os::raw::{c_char, c_float, c_int, c_void};

use super::lua::{
    lua_CFunction, lua_Integer, lua_Number, lua_State, lua_Unsigned, lua_getfield,
    LUA_REGISTRYINDEX,
};

#[repr(C)]
pub struct luaL_Reg {
    pub name: *const c_char,
    pub func: lua_CFunction,
}

extern "C" {
    pub fn luaL_register(L: *mut lua_State, libname: *const c_char, l: *const luaL_Reg);
    pub fn luaL_getmetafield(L: *mut lua_State, obj: c_int, e: *const c_char) -> c_int;
    pub fn luaL_callmeta(L: *mut lua_State, obj: c_int, e: *const c_char) -> c_int;
    // TODO: luaL_typeerrorL, luaL_argerrorL
    pub fn luaL_checklstring(L: *mut lua_State, narg: c_int, l: *mut usize) -> *const c_char;
    pub fn luaL_optlstring(
        L: *mut lua_State,
        narg: c_int,
        def: *const c_char,
        l: *mut usize,
    ) -> *const c_char;
    pub fn luaL_checknumber(L: *mut lua_State, narg: c_int) -> lua_Number;
    pub fn luaL_optnumber(L: *mut lua_State, narg: c_int, def: lua_Number) -> lua_Number;

    pub fn luaL_checkboolean(L: *mut lua_State, narg: c_int) -> c_int;
    pub fn luaL_optboolean(L: *mut lua_State, narg: c_int, def: c_int) -> c_int;

    pub fn luaL_checkinteger(L: *mut lua_State, narg: c_int) -> lua_Integer;
    pub fn luaL_optinteger(L: *mut lua_State, narg: c_int, def: lua_Integer) -> lua_Integer;
    pub fn luaL_checkunsigned(L: *mut lua_State, narg: c_int) -> lua_Unsigned;
    pub fn luaL_optunsigned(L: *mut lua_State, narg: c_int, def: lua_Unsigned) -> lua_Unsigned;

    pub fn luaL_checkvector(L: *mut lua_State, narg: c_int) -> *const c_float;
    pub fn luaL_optvector(L: *mut lua_State, narg: c_int, def: *const c_float) -> *const c_float;

    pub fn luaL_checkstack(L: *mut lua_State, sz: c_int, msg: *const c_char);
    pub fn luaL_checktype(L: *mut lua_State, narg: c_int, t: c_int);
    pub fn luaL_checkany(L: *mut lua_State, narg: c_int);

    pub fn luaL_newmetatable(L: *mut lua_State, tname: *const c_char) -> c_int;
    pub fn luaL_checkudata(L: *mut lua_State, ud: c_int, tname: *const c_char) -> *mut c_void;

    pub fn luaL_where(L: *mut lua_State, lvl: c_int);

    #[link_name = "luaL_errorL"]
    pub fn luaL_error(L: *mut lua_State, fmt: *const c_char, ...) -> !;

    pub fn luaL_checkoption(
        L: *mut lua_State,
        narg: c_int,
        def: *const c_char,
        lst: *const *const c_char,
    ) -> c_int;

    pub fn luaL_tolstring(L: *mut lua_State, idx: c_int, len: *mut usize) -> *const c_char;

    pub fn luaL_newstate() -> *mut lua_State;

    // TODO: luaL_findtable
}

//
// Some useful macros (implemented as Rust functions)
//

// TODO: luaL_argcheck, luaL_argexpected, luaL_checkstring, luaL_optstring, luaL_typename, luaL_opt

#[inline(always)]
pub unsafe fn luaL_getmetatable(L: *mut lua_State, n: *const c_char) {
    lua_getfield(L, LUA_REGISTRYINDEX, n);
}

//
// TODO: Generic Buffer Manipulation
//

//
// Builtin libraries
//

pub const LUA_COLIBNAME: &str = "coroutine";
pub const LUA_TABLIBNAME: &str = "table";
pub const LUA_OSLIBNAME: &str = "os";
pub const LUA_STRLIBNAME: &str = "string";
pub const LUA_BITLIBNAME: &str = "bit32";
pub const LUA_UTF8LIBNAME: &str = "utf8";
pub const LUA_MATHLIBNAME: &str = "math";
pub const LUA_DBLIBNAME: &str = "debug";

extern "C" {
    pub fn luaopen_base(L: *mut lua_State) -> c_int;
    pub fn luaopen_coroutine(L: *mut lua_State) -> c_int;
    pub fn luaopen_table(L: *mut lua_State) -> c_int;
    pub fn luaopen_os(L: *mut lua_State) -> c_int;
    pub fn luaopen_string(L: *mut lua_State) -> c_int;
    pub fn luaopen_bit32(L: *mut lua_State) -> c_int;
    pub fn luaopen_utf8(L: *mut lua_State) -> c_int;
    pub fn luaopen_math(L: *mut lua_State) -> c_int;
    pub fn luaopen_debug(L: *mut lua_State) -> c_int;

    // open all builtin libraries
    pub fn luaL_openlibs(L: *mut lua_State);

    // sandbox libraries and globals
    pub fn luaL_sandbox(L: *mut lua_State);
    pub fn luaL_sandboxthread(L: *mut lua_State);
}
