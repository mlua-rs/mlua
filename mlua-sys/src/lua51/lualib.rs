//! Contains definitions from `lualib.h`.

use std::os::raw::{c_char, c_int};

use super::lua::lua_State;

pub const LUA_COLIBNAME: *const c_char = cstr!("coroutine");
pub const LUA_TABLIBNAME: *const c_char = cstr!("table");
pub const LUA_IOLIBNAME: *const c_char = cstr!("io");
pub const LUA_OSLIBNAME: *const c_char = cstr!("os");
pub const LUA_STRLIBNAME: *const c_char = cstr!("string");
pub const LUA_MATHLIBNAME: *const c_char = cstr!("math");
pub const LUA_DBLIBNAME: *const c_char = cstr!("debug");
pub const LUA_LOADLIBNAME: *const c_char = cstr!("package");

#[cfg(feature = "luajit")]
pub const LUA_BITLIBNAME: *const c_char = cstr!("bit");
#[cfg(feature = "luajit")]
pub const LUA_JITLIBNAME: *const c_char = cstr!("jit");
#[cfg(feature = "luajit")]
pub const LUA_FFILIBNAME: *const c_char = cstr!("ffi");

#[cfg_attr(all(windows, raw_dylib), link(name = "lua51", kind = "raw-dylib"))]
unsafe extern "C-unwind" {
    pub fn luaopen_base(L: *mut lua_State) -> c_int;
    pub fn luaopen_table(L: *mut lua_State) -> c_int;
    pub fn luaopen_io(L: *mut lua_State) -> c_int;
    pub fn luaopen_os(L: *mut lua_State) -> c_int;
    pub fn luaopen_string(L: *mut lua_State) -> c_int;
    pub fn luaopen_math(L: *mut lua_State) -> c_int;
    pub fn luaopen_debug(L: *mut lua_State) -> c_int;
    pub fn luaopen_package(L: *mut lua_State) -> c_int;

    #[cfg(feature = "luajit")]
    pub fn luaopen_bit(L: *mut lua_State) -> c_int;
    #[cfg(feature = "luajit")]
    pub fn luaopen_jit(L: *mut lua_State) -> c_int;
    #[cfg(feature = "luajit")]
    pub fn luaopen_ffi(L: *mut lua_State) -> c_int;

    // open all builtin libraries
    pub fn luaL_openlibs(L: *mut lua_State);
}
