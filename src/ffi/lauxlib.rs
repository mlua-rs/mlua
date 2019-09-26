// The MIT License (MIT)
//
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

//! Contains definitions from `lauxlib.h`.

use libc::{c_int, c_long, c_char, c_void, size_t};
use ffi::lua;
use ffi::lua::{lua_State, lua_CFunction, lua_Integer, lua_Number};
use ffi::luaconf::LUAL_BUFFERSIZE;
use std::ptr;

pub use super::glue::LUAL_NUMSIZES;
pub use super::glue::LUA_FILEHANDLE;

// extra error code for 'luaL_load'
pub const LUA_ERRFILE: c_int = lua::LUA_ERRERR + 1;

#[repr(C)]
pub struct luaL_Reg {
  pub name: *const c_char,
  pub func: lua_CFunction,
}


#[inline(always)]
pub unsafe fn luaL_checkversion(L: *mut lua_State) {
  luaL_checkversion_(L, lua::LUA_VERSION_NUM as lua_Number, LUAL_NUMSIZES as size_t)
}

extern {
  pub fn luaL_checkversion_(L: *mut lua_State, ver: lua_Number, sz: size_t);

  pub fn luaL_getmetafield(L: *mut lua_State, obj: c_int, e: *const c_char) -> c_int;
  pub fn luaL_callmeta(L: *mut lua_State, obj: c_int, e: *const c_char) -> c_int;
  pub fn luaL_tolstring(L: *mut lua_State, idx: c_int, len: *mut size_t) -> *const c_char;
  pub fn luaL_argerror(L: *mut lua_State, arg: c_int, l: *const c_char) -> c_int;
  pub fn luaL_checklstring(L: *mut lua_State, arg: c_int, l: *mut size_t) -> *const c_char;
  pub fn luaL_optlstring(L: *mut lua_State, arg: c_int, def: *const c_char, l: *mut size_t) -> *const c_char;
  pub fn luaL_checknumber(L: *mut lua_State, arg: c_int) -> lua_Number;
  pub fn luaL_optnumber(L: *mut lua_State, arg: c_int, def: lua_Number) -> lua_Number;
  pub fn luaL_checkinteger(L: *mut lua_State, arg: c_int) -> lua_Integer;
  pub fn luaL_optinteger(L: *mut lua_State, arg: c_int, def: lua_Integer) -> lua_Integer;

  pub fn luaL_checkstack(L: *mut lua_State, sz: c_int, msg: *const c_char);
  pub fn luaL_checktype(L: *mut lua_State, arg: c_int, t: c_int);
  pub fn luaL_checkany(L: *mut lua_State, arg: c_int);

  pub fn luaL_newmetatable(L: *mut lua_State, tname: *const c_char) -> c_int;
  pub fn luaL_setmetatable(L: *mut lua_State, tname: *const c_char);
  pub fn luaL_testudata(L: *mut lua_State, ud: c_int, tname: *const c_char) -> *mut c_void;
  pub fn luaL_checkudata(L: *mut lua_State, ud: c_int, tname: *const c_char) -> *mut c_void;

  pub fn luaL_where(L: *mut lua_State, lvl: c_int);
  pub fn luaL_error(L: *mut lua_State, fmt: *const c_char, ...) -> c_int;

  // TODO: test this
  pub fn luaL_checkoption(L: *mut lua_State, arg: c_int, def: *const c_char, lst: *const *const c_char) -> c_int;

  pub fn luaL_fileresult(L: *mut lua_State, stat: c_int, fname: *const c_char) -> c_int;
  pub fn luaL_execresult(L: *mut lua_State, stat: c_int) -> c_int;
}

// pre-defined references
pub const LUA_NOREF: c_int = -2;
pub const LUA_REFNIL: c_int = -1;

extern {
  pub fn luaL_ref(L: *mut lua_State, t: c_int) -> c_int;
  pub fn luaL_unref(L: *mut lua_State, t: c_int, r: c_int);

  pub fn luaL_loadfilex(L: *mut lua_State, filename: *const c_char, mode: *const c_char) -> c_int;
}

#[inline(always)]
pub unsafe fn luaL_loadfile(L: *mut lua_State, f: *const c_char) -> c_int {
  luaL_loadfilex(L, f, ptr::null())
}

extern {
  pub fn luaL_loadbufferx(L: *mut lua_State, buff: *const c_char, sz: size_t, name: *const c_char, mode: *const c_char) -> c_int;
  pub fn luaL_loadstring(L: *mut lua_State, s: *const c_char) -> c_int;

  pub fn luaL_newstate() -> *mut lua_State;

  pub fn luaL_len(L: *mut lua_State, idx: c_int) -> lua_Integer;

  pub fn luaL_gsub(L: *mut lua_State, s: *const c_char, p: *const c_char, r: *const c_char) -> *const c_char;

  pub fn luaL_setfuncs(L: *mut lua_State, l: *const luaL_Reg, nup: c_int);

  pub fn luaL_getsubtable(L: *mut lua_State, idx: c_int, fname: *const c_char) -> c_int;

  pub fn luaL_traceback(L: *mut lua_State, L1: *mut lua_State, msg: *const c_char, level: c_int);

  pub fn luaL_requiref(L: *mut lua_State, modname: *const c_char, openf: lua_CFunction, glb: c_int);
}

#[inline(always)]
#[allow(unused_variables)]
pub unsafe fn luaL_newlibtable(L: *mut lua_State, l: *const luaL_Reg) {
  // TODO: figure out how to pass an appropriate hint for the second param
  // this involves correcting the second parameter's type; in C this is
  // sizeof(l)/sizeof(l[0])
  lua::lua_createtable(L, 0, 0)
}

#[inline(always)]
pub unsafe fn luaL_newlib(L: *mut lua_State, l: *const luaL_Reg) {
  luaL_checkversion(L);
  luaL_newlibtable(L, l);
  luaL_setfuncs(L, l, 0)
}

#[inline(always)]
pub unsafe fn luaL_argcheck(L: *mut lua_State, cond: c_int, arg: c_int, extramsg: *const c_char) {
  if cond == 0 {
    luaL_argerror(L, arg, extramsg);
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

// From 5.3 user manual:
// Macros to project non-default integer types (luaL_checkint, luaL_optint,
// luaL_checklong, luaL_optlong) were deprecated. Use their equivalent over
// lua_Integer with a type cast (or, when possible, use lua_Integer in your
// code).
#[inline(always)]
//#[deprecated]
pub unsafe fn luaL_checkint(L: *mut lua_State, n: c_int) -> c_int {
  luaL_checkinteger(L, n) as c_int
}

#[inline(always)]
//#[deprecated]
pub unsafe fn luaL_optint(L: *mut lua_State, n: c_int, d: c_int) -> c_int {
  luaL_optinteger(L, n, d as lua_Integer) as c_int
}

#[inline(always)]
//#[deprecated]
pub unsafe fn luaL_checklong(L: *mut lua_State, n: c_int) -> c_long {
  luaL_checkinteger(L, n) as c_long
}

#[inline(always)]
//#[deprecated]
pub unsafe fn luaL_optlong(L: *mut lua_State, n: c_int, d: c_long) -> c_long {
  luaL_optinteger(L, n, d as lua_Integer) as c_long
}

#[inline(always)]
pub unsafe fn luaL_typename(L: *mut lua_State, i: c_int) -> *const c_char {
  lua::lua_typename(L, lua::lua_type(L, i))
}

#[inline(always)]
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
  lua::lua_getfield(L, lua::LUA_REGISTRYINDEX, n);
}

// luaL_opt would be implemented here but it is undocumented, so it's omitted

#[inline(always)]
pub unsafe fn luaL_loadbuffer(L: *mut lua_State, s: *const c_char, sz: size_t, n: *const c_char) -> c_int {
  luaL_loadbufferx(L, s, sz, n, ptr::null())
}

#[repr(C)]
pub struct luaL_Buffer {
  pub b: *mut c_char,
  pub size: size_t,
  pub n: size_t,
  pub L: *mut lua_State,
  pub initb: [c_char; LUAL_BUFFERSIZE as usize]
}

// TODO: Test this thoroughly
#[inline(always)]
pub unsafe fn luaL_addchar(B: *mut luaL_Buffer, c: c_char) {
  // (B)->n < (B) -> size || luaL_prepbuffsize((B), 1)
  if (*B).n < (*B).size {
    luaL_prepbuffsize(B, 1);
  }
  // (B)->b[(B)->n++] = (c)
  let offset = (*B).b.offset((*B).n as isize);
  ptr::write(offset, c);
  (*B).n += 1;
}

#[inline(always)]
pub unsafe fn luaL_addsize(B: *mut luaL_Buffer, s: size_t) {
  (*B).n += s;
}

extern {
  pub fn luaL_buffinit(L: *mut lua_State, B: *mut luaL_Buffer);
  pub fn luaL_prepbuffsize(B: *mut luaL_Buffer, sz: size_t) -> *mut c_char;
  pub fn luaL_addlstring(B: *mut luaL_Buffer, s: *const c_char, l: size_t);
  pub fn luaL_addstring(B: *mut luaL_Buffer, s: *const c_char);
  pub fn luaL_addvalue(B: *mut luaL_Buffer);
  pub fn luaL_pushresult(B: *mut luaL_Buffer);
  pub fn luaL_pushresultsize(B: *mut luaL_Buffer, sz: size_t);
  pub fn luaL_buffinitsize(L: *mut lua_State, B: *mut luaL_Buffer, sz: size_t) -> *mut c_char;
}

pub unsafe fn luaL_prepbuffer(B: *mut luaL_Buffer) -> *mut c_char {
  luaL_prepbuffsize(B, LUAL_BUFFERSIZE as size_t)
}

#[repr(C)]
pub struct luaL_Stream {
  pub f: *mut ::libc::FILE,
  pub closef: lua_CFunction
}

// omitted: old module system compatibility
