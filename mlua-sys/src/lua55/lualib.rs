//! Contains definitions from `lualib.h`.

use std::os::raw::{c_char, c_int};

use super::lua::lua_State;

pub const LUA_GLIBK: c_int = 1;

pub const LUA_LOADLIBNAME: *const c_char = cstr!("package");
pub const LUA_LOADLIBK: c_int = LUA_GLIBK << 1;

pub const LUA_COLIBNAME: *const c_char = cstr!("coroutine");
pub const LUA_COLIBK: c_int = LUA_GLIBK << 2;

pub const LUA_DBLIBNAME: *const c_char = cstr!("debug");
pub const LUA_DBLIBK: c_int = LUA_GLIBK << 3;

pub const LUA_IOLIBNAME: *const c_char = cstr!("io");
pub const LUA_IOLIBK: c_int = LUA_GLIBK << 4;

pub const LUA_MATHLIBNAME: *const c_char = cstr!("math");
pub const LUA_MATHLIBK: c_int = LUA_GLIBK << 5;

pub const LUA_OSLIBNAME: *const c_char = cstr!("os");
pub const LUA_OSLIBK: c_int = LUA_GLIBK << 6;

pub const LUA_STRLIBNAME: *const c_char = cstr!("string");
pub const LUA_STRLIBK: c_int = LUA_GLIBK << 7;

pub const LUA_TABLIBNAME: *const c_char = cstr!("table");
pub const LUA_TABLIBK: c_int = LUA_GLIBK << 8;

pub const LUA_UTF8LIBNAME: *const c_char = cstr!("utf8");
pub const LUA_UTF8LIBK: c_int = LUA_GLIBK << 9;

#[cfg_attr(all(windows, raw_dylib), link(name = "lua55", kind = "raw-dylib"))]
unsafe extern "C-unwind" {
    pub fn luaopen_base(L: *mut lua_State) -> c_int;
    pub fn luaopen_package(L: *mut lua_State) -> c_int;
    pub fn luaopen_coroutine(L: *mut lua_State) -> c_int;
    pub fn luaopen_debug(L: *mut lua_State) -> c_int;
    pub fn luaopen_io(L: *mut lua_State) -> c_int;
    pub fn luaopen_math(L: *mut lua_State) -> c_int;
    pub fn luaopen_os(L: *mut lua_State) -> c_int;
    pub fn luaopen_string(L: *mut lua_State) -> c_int;
    pub fn luaopen_table(L: *mut lua_State) -> c_int;
    pub fn luaopen_utf8(L: *mut lua_State) -> c_int;

    // open all builtin libraries
    pub fn luaL_openselectedlibs(L: *mut lua_State, load: c_int, preload: c_int);
}

pub unsafe fn luaL_openlibs(L: *mut lua_State) {
    luaL_openselectedlibs(L, !0, 0);
}
