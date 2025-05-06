//! Contains definitions from `lauxlib.h`.

use std::os::raw::{c_char, c_int, c_void};
use std::ptr;

use super::lua::{self, lua_CFunction, lua_Integer, lua_Number, lua_State};

// Extra error code for 'luaL_load'
pub const LUA_ERRFILE: c_int = lua::LUA_ERRERR + 1;

// Key, in the registry, for table of loaded modules
pub const LUA_LOADED_TABLE: *const c_char = cstr!("_LOADED");

#[repr(C)]
pub struct luaL_Reg {
    pub name: *const c_char,
    pub func: lua_CFunction,
}

#[cfg_attr(all(windows, raw_dylib), link(name = "lua51", kind = "raw-dylib"))]
unsafe extern "C-unwind" {
    pub fn luaL_register(L: *mut lua_State, libname: *const c_char, l: *const luaL_Reg);
    #[link_name = "luaL_getmetafield"]
    pub fn luaL_getmetafield_(L: *mut lua_State, obj: c_int, e: *const c_char) -> c_int;
    pub fn luaL_callmeta(L: *mut lua_State, obj: c_int, e: *const c_char) -> c_int;
    pub fn luaL_typerror(L: *mut lua_State, narg: c_int, tname: *const c_char) -> c_int;
    pub fn luaL_argerror(L: *mut lua_State, narg: c_int, extramsg: *const c_char) -> c_int;
    pub fn luaL_checklstring(L: *mut lua_State, narg: c_int, l: *mut usize) -> *const c_char;
    pub fn luaL_optlstring(
        L: *mut lua_State,
        narg: c_int,
        def: *const c_char,
        l: *mut usize,
    ) -> *const c_char;
    pub fn luaL_checknumber(L: *mut lua_State, narg: c_int) -> lua_Number;
    pub fn luaL_optnumber(L: *mut lua_State, narg: c_int, def: lua_Number) -> lua_Number;
    pub fn luaL_checkinteger(L: *mut lua_State, narg: c_int) -> lua_Integer;
    pub fn luaL_optinteger(L: *mut lua_State, narg: c_int, def: lua_Integer) -> lua_Integer;
    #[link_name = "luaL_checkstack"]
    pub fn luaL_checkstack_(L: *mut lua_State, sz: c_int, msg: *const c_char);
    pub fn luaL_checktype(L: *mut lua_State, narg: c_int, t: c_int);
    pub fn luaL_checkany(L: *mut lua_State, narg: c_int);

    #[link_name = "luaL_newmetatable"]
    pub fn luaL_newmetatable_(L: *mut lua_State, tname: *const c_char) -> c_int;
    pub fn luaL_checkudata(L: *mut lua_State, ud: c_int, tname: *const c_char) -> *mut c_void;

    pub fn luaL_where(L: *mut lua_State, lvl: c_int);
    pub fn luaL_error(L: *mut lua_State, fmt: *const c_char, ...) -> c_int;

    pub fn luaL_checkoption(
        L: *mut lua_State,
        narg: c_int,
        def: *const c_char,
        lst: *const *const c_char,
    ) -> c_int;
}

// Pre-defined references
pub const LUA_NOREF: c_int = -2;
pub const LUA_REFNIL: c_int = -1;

#[cfg_attr(all(windows, raw_dylib), link(name = "lua51", kind = "raw-dylib"))]
unsafe extern "C-unwind" {
    pub fn luaL_ref(L: *mut lua_State, t: c_int) -> c_int;
    pub fn luaL_unref(L: *mut lua_State, t: c_int, r#ref: c_int);

    pub fn luaL_loadfile(L: *mut lua_State, filename: *const c_char) -> c_int;
    pub fn luaL_loadbuffer(L: *mut lua_State, buff: *const c_char, sz: usize, name: *const c_char) -> c_int;
    pub fn luaL_loadstring(L: *mut lua_State, s: *const c_char) -> c_int;

    pub fn luaL_newstate() -> *mut lua_State;

    pub fn luaL_gsub(
        L: *mut lua_State,
        s: *const c_char,
        p: *const c_char,
        r: *const c_char,
    ) -> *const c_char;

    pub fn luaL_findtable(
        L: *mut lua_State,
        idx: c_int,
        fname: *const c_char,
        szhint: c_int,
    ) -> *const c_char;
}

//
// Some useful macros (implemented as Rust functions)
//

#[inline(always)]
pub unsafe fn luaL_argcheck(L: *mut lua_State, cond: c_int, narg: c_int, extramsg: *const c_char) {
    if cond == 0 {
        luaL_argerror(L, narg, extramsg);
    }
}

#[inline(always)]
pub unsafe fn luaL_checkstring(L: *mut lua_State, n: c_int) -> *const c_char {
    luaL_checklstring(L, n, ptr::null_mut())
}

#[inline(always)]
pub unsafe fn luaL_optstring(L: *mut lua_State, n: c_int, d: *const c_char) -> *const c_char {
    luaL_optlstring(L, n, d, ptr::null_mut())
}

// Deprecated from 5.3: luaL_checkint, luaL_optint, luaL_checklong, luaL_optlong

#[inline(always)]
pub unsafe fn luaL_typename(L: *mut lua_State, i: c_int) -> *const c_char {
    lua::lua_typename(L, lua::lua_type(L, i))
}

pub unsafe fn luaL_dofile(L: *mut lua_State, filename: *const c_char) -> c_int {
    let status = luaL_loadfile(L, filename);
    if status == 0 {
        lua::lua_pcall(L, 0, lua::LUA_MULTRET, 0)
    } else {
        status
    }
}

#[inline(always)]
pub unsafe fn luaL_dostring(L: *mut lua_State, s: *const c_char) -> c_int {
    let status = luaL_loadstring(L, s);
    if status == 0 {
        lua::lua_pcall(L, 0, lua::LUA_MULTRET, 0)
    } else {
        status
    }
}

#[inline(always)]
pub unsafe fn luaL_getmetatable(L: *mut lua_State, n: *const c_char) {
    lua::lua_getfield_(L, lua::LUA_REGISTRYINDEX, n);
}

// TODO: luaL_opt

//
// TODO: Generic Buffer Manipulation
//
