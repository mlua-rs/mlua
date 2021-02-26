// The MIT License (MIT)
//
// Copyright (c) 2019-2021 A. Orlenko
// Copyright (c) 2014 J.C. Moyer
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

//! Contains definitions from `lualib.h`.

use std::os::raw::c_int;

use super::lua::lua_State;

pub use super::glue::{
    LUA_COLIBNAME, LUA_DBLIBNAME, LUA_IOLIBNAME, LUA_LOADLIBNAME, LUA_MATHLIBNAME, LUA_OSLIBNAME,
    LUA_STRLIBNAME, LUA_TABLIBNAME,
};

#[cfg(any(feature = "lua54", feature = "lua53"))]
pub use super::glue::LUA_UTF8LIBNAME;

#[cfg(any(feature = "lua52", feature = "luajit"))]
pub use super::glue::LUA_BITLIBNAME;

#[cfg(feature = "luajit")]
pub use super::glue::{LUA_FFILIBNAME, LUA_JITLIBNAME};

extern "C" {
    pub fn luaopen_base(L: *mut lua_State) -> c_int;
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    pub fn luaopen_coroutine(L: *mut lua_State) -> c_int;
    pub fn luaopen_table(L: *mut lua_State) -> c_int;
    pub fn luaopen_io(L: *mut lua_State) -> c_int;
    pub fn luaopen_os(L: *mut lua_State) -> c_int;
    pub fn luaopen_string(L: *mut lua_State) -> c_int;
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn luaopen_utf8(L: *mut lua_State) -> c_int;
    #[cfg(feature = "lua52")]
    pub fn luaopen_bit32(L: *mut lua_State) -> c_int;
    pub fn luaopen_math(L: *mut lua_State) -> c_int;
    pub fn luaopen_debug(L: *mut lua_State) -> c_int;
    pub fn luaopen_package(L: *mut lua_State) -> c_int;
    #[cfg(feature = "luajit")]
    pub fn luaopen_bit(L: *mut lua_State) -> c_int;
    #[cfg(feature = "luajit")]
    pub fn luaopen_jit(L: *mut lua_State) -> c_int;
    #[cfg(feature = "luajit")]
    pub fn luaopen_ffi(L: *mut lua_State) -> c_int;

    pub fn luaL_openlibs(L: *mut lua_State);
}
