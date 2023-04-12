//! Contains definitions from `lualib.h`.

use std::os::raw::c_int;

use super::lua::lua_State;

pub const LUA_COLIBNAME: &str = "coroutine";
pub const LUA_TABLIBNAME: &str = "table";
pub const LUA_IOLIBNAME: &str = "io";
pub const LUA_OSLIBNAME: &str = "os";
pub const LUA_STRLIBNAME: &str = "string";
pub const LUA_UTF8LIBNAME: &str = "utf8";
pub const LUA_MATHLIBNAME: &str = "math";
pub const LUA_DBLIBNAME: &str = "debug";
pub const LUA_LOADLIBNAME: &str = "package";

extern "C" {
    pub fn luaopen_base(L: *mut lua_State) -> c_int;
    pub fn luaopen_coroutine(L: *mut lua_State) -> c_int;
    pub fn luaopen_table(L: *mut lua_State) -> c_int;
    pub fn luaopen_io(L: *mut lua_State) -> c_int;
    pub fn luaopen_os(L: *mut lua_State) -> c_int;
    pub fn luaopen_string(L: *mut lua_State) -> c_int;
    pub fn luaopen_utf8(L: *mut lua_State) -> c_int;
    pub fn luaopen_math(L: *mut lua_State) -> c_int;
    pub fn luaopen_debug(L: *mut lua_State) -> c_int;
    pub fn luaopen_package(L: *mut lua_State) -> c_int;

    // open all builtin libraries
    pub fn luaL_openlibs(L: *mut lua_State);
}
