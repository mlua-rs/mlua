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

//! Contains definitions from `lua.h`.

#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
use std::os::raw::c_uchar;
use std::os::raw::{c_char, c_int, c_void};
#[cfg(feature = "lua54")]
use std::os::raw::{c_uint, c_ushort};
use std::ptr;

use super::luaconf;

#[cfg(any(feature = "lua51", feature = "luajit"))]
pub use super::glue::{LUA_ENVIRONINDEX, LUA_GLOBALSINDEX};
pub use super::glue::{LUA_REGISTRYINDEX, LUA_VERSION_NUM};

#[cfg(not(feature = "luajit"))]
pub const LUA_SIGNATURE: &[u8] = b"\x1bLua";
#[cfg(feature = "luajit")]
pub const LUA_SIGNATURE: &[u8] = b"\x1bLJ";

// option for multiple returns in 'lua_pcall' and 'lua_call'
pub const LUA_MULTRET: c_int = -1;

#[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
pub use super::compat53::{
    lua_dump, lua_getextraspace, lua_getfield, lua_geti, lua_gettable, lua_getuservalue,
    lua_isinteger, lua_pushlstring, lua_rawget, lua_rawgeti, lua_rawgetp, lua_rotate, lua_seti,
    lua_stringtonumber, lua_tointegerx,
};

#[cfg(any(feature = "lua51", feature = "luajit"))]
pub use super::compat53::{
    lua_absindex, lua_arith, lua_compare, lua_copy, lua_len, lua_pushglobaltable, lua_pushstring,
    lua_rawlen, lua_rawsetp, lua_resume as lua_resume_53, lua_setuservalue, lua_tonumberx,
    lua_upvalueindex,
};

#[cfg(feature = "lua52")]
pub use super::compat53::lua_getglobal;

#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
#[inline(always)]
pub fn lua_upvalueindex(i: c_int) -> c_int {
    LUA_REGISTRYINDEX - i
}

// thread status
pub const LUA_OK: c_int = 0;
pub const LUA_YIELD: c_int = 1;
pub const LUA_ERRRUN: c_int = 2;
pub const LUA_ERRSYNTAX: c_int = 3;
pub const LUA_ERRMEM: c_int = 4;
#[cfg(any(feature = "lua53", feature = "lua52"))]
pub const LUA_ERRGCMM: c_int = 5;
#[cfg(any(feature = "lua54", feature = "lua51", feature = "luajit"))]
pub const LUA_ERRERR: c_int = 5;
#[cfg(any(feature = "lua53", feature = "lua52"))]
pub const LUA_ERRERR: c_int = 6;

pub type lua_State = c_void;

// basic types
pub const LUA_TNONE: c_int = -1;

pub const LUA_TNIL: c_int = 0;
pub const LUA_TBOOLEAN: c_int = 1;
pub const LUA_TLIGHTUSERDATA: c_int = 2;
pub const LUA_TNUMBER: c_int = 3;
pub const LUA_TSTRING: c_int = 4;
pub const LUA_TTABLE: c_int = 5;
pub const LUA_TFUNCTION: c_int = 6;
pub const LUA_TUSERDATA: c_int = 7;
pub const LUA_TTHREAD: c_int = 8;

#[cfg(feature = "lua54")]
pub const LUA_NUMTYPES: c_int = 9;
#[cfg(any(feature = "lua53", feature = "lua52"))]
pub const LUA_NUMTAGS: c_int = 9;

// minimum stack available to a C function
pub const LUA_MINSTACK: c_int = 20;

// predefined values in the registry
#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
pub const LUA_RIDX_MAINTHREAD: lua_Integer = 1;
#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
pub const LUA_RIDX_GLOBALS: lua_Integer = 2;
#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
pub const LUA_RIDX_LAST: lua_Integer = LUA_RIDX_GLOBALS;

/// A Lua number, usually equivalent to `f64`.
pub type lua_Number = luaconf::LUA_NUMBER;

/// A Lua integer, usually equivalent to `i64`.
pub type lua_Integer = luaconf::LUA_INTEGER;

// unsigned integer type
pub type lua_Unsigned = luaconf::LUA_UNSIGNED;

// type for continuation-function contexts
#[cfg(any(feature = "lua54", feature = "lua53"))]
pub type lua_KContext = luaconf::LUA_KCONTEXT;

/// Type for native functions that can be passed to Lua.
pub type lua_CFunction = unsafe extern "C" fn(L: *mut lua_State) -> c_int;

// Type for continuation functions
#[cfg(any(feature = "lua54", feature = "lua53"))]
pub type lua_KFunction =
    unsafe extern "C" fn(L: *mut lua_State, status: c_int, ctx: lua_KContext) -> c_int;

// Type for functions that read/write blocks when loading/dumping Lua chunks
pub type lua_Reader =
    unsafe extern "C" fn(L: *mut lua_State, ud: *mut c_void, sz: *mut usize) -> *const c_char;
pub type lua_Writer =
    unsafe extern "C" fn(L: *mut lua_State, p: *const c_void, sz: usize, ud: *mut c_void) -> c_int;

// Type for memory-allocation functions.
pub type lua_Alloc = unsafe extern "C" fn(
    ud: *mut c_void,
    ptr: *mut c_void,
    osize: usize,
    nsize: usize,
) -> *mut c_void;

// Type for warning functions
#[cfg(feature = "lua54")]
pub type lua_WarnFunction =
    unsafe extern "C" fn(ud: *mut c_void, msg: *const c_char, tocont: c_int);

extern "C" {
    // state manipulation
    pub fn lua_newstate(f: lua_Alloc, ud: *mut c_void) -> *mut lua_State;
    pub fn lua_close(L: *mut lua_State);
    pub fn lua_newthread(L: *mut lua_State) -> *mut lua_State;
    #[cfg(feature = "lua54")]
    pub fn lua_resetthread(L: *mut lua_State) -> c_int;

    pub fn lua_atpanic(L: *mut lua_State, panicf: lua_CFunction) -> lua_CFunction;

    #[cfg(feature = "lua54")]
    pub fn lua_version(L: *mut lua_State) -> lua_Number;
    #[cfg(feature = "lua53")]
    pub fn lua_version(L: *mut lua_State) -> *const lua_Number;

    // basic stack manipulation
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    pub fn lua_absindex(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_gettop(L: *mut lua_State) -> c_int;
    pub fn lua_settop(L: *mut lua_State, idx: c_int);
    pub fn lua_pushvalue(L: *mut lua_State, idx: c_int);
    #[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
    pub fn lua_remove(L: *mut lua_State, idx: c_int);
    #[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
    pub fn lua_insert(L: *mut lua_State, idx: c_int);
    #[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
    pub fn lua_replace(L: *mut lua_State, idx: c_int);
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_rotate(L: *mut lua_State, idx: c_int, n: c_int);
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    pub fn lua_copy(L: *mut lua_State, fromidx: c_int, toidx: c_int);
    pub fn lua_checkstack(L: *mut lua_State, sz: c_int) -> c_int;

    pub fn lua_xmove(from: *mut lua_State, to: *mut lua_State, n: c_int);

    // access functions (stack -> C)
    pub fn lua_isnumber(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_isstring(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_iscfunction(L: *mut lua_State, idx: c_int) -> c_int;
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_isinteger(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_isuserdata(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_type(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_typename(L: *mut lua_State, tp: c_int) -> *const c_char;

    #[cfg(any(feature = "lua51", feature = "luajit"))]
    pub fn lua_tonumber(L: *mut lua_State, idx: c_int) -> lua_Number;
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    pub fn lua_tonumberx(L: *mut lua_State, idx: c_int, isnum: *mut c_int) -> lua_Number;
    #[cfg(any(feature = "lua51", feature = "luajit"))]
    pub fn lua_tointeger(L: *mut lua_State, idx: c_int) -> lua_Integer;
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_tointegerx(L: *mut lua_State, idx: c_int, isnum: *mut c_int) -> lua_Integer;
    pub fn lua_toboolean(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_tolstring(L: *mut lua_State, idx: c_int, len: *mut usize) -> *const c_char;
    #[cfg(any(feature = "lua51", feature = "luajit"))]
    pub fn lua_objlen(L: *mut lua_State, idx: c_int) -> usize;
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    pub fn lua_rawlen(L: *mut lua_State, idx: c_int) -> usize;
    pub fn lua_tocfunction(L: *mut lua_State, idx: c_int) -> lua_CFunction;
    pub fn lua_touserdata(L: *mut lua_State, idx: c_int) -> *mut c_void;
    pub fn lua_tothread(L: *mut lua_State, idx: c_int) -> *mut lua_State;
    pub fn lua_topointer(L: *mut lua_State, idx: c_int) -> *const c_void;
}

// Comparison and arithmetic functions
pub const LUA_OPADD: c_int = 0;
pub const LUA_OPSUB: c_int = 1;
pub const LUA_OPMUL: c_int = 2;

#[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
pub const LUA_OPDIV: c_int = 3;
#[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
pub const LUA_OPMOD: c_int = 4;
#[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
pub const LUA_OPPOW: c_int = 5;
#[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
pub const LUA_OPUNM: c_int = 6;

#[cfg(any(feature = "lua54", feature = "lua53"))]
pub const LUA_OPMOD: c_int = 3;
#[cfg(any(feature = "lua54", feature = "lua53"))]
pub const LUA_OPPOW: c_int = 4;
#[cfg(any(feature = "lua54", feature = "lua53"))]
pub const LUA_OPDIV: c_int = 5;
#[cfg(any(feature = "lua54", feature = "lua53"))]
pub const LUA_OPIDIV: c_int = 6;
#[cfg(any(feature = "lua54", feature = "lua53"))]
pub const LUA_OPBAND: c_int = 7;
#[cfg(any(feature = "lua54", feature = "lua53"))]
pub const LUA_OPBOR: c_int = 8;
#[cfg(any(feature = "lua54", feature = "lua53"))]
pub const LUA_OPBXOR: c_int = 9;
#[cfg(any(feature = "lua54", feature = "lua53"))]
pub const LUA_OPSHL: c_int = 10;
#[cfg(any(feature = "lua54", feature = "lua53"))]
pub const LUA_OPSHR: c_int = 11;
#[cfg(any(feature = "lua54", feature = "lua53"))]
pub const LUA_OPUNM: c_int = 12;
#[cfg(any(feature = "lua54", feature = "lua53"))]
pub const LUA_OPBNOT: c_int = 13;

extern "C" {
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    pub fn lua_arith(L: *mut lua_State, op: c_int);
}

pub const LUA_OPEQ: c_int = 0;
pub const LUA_OPLT: c_int = 1;
pub const LUA_OPLE: c_int = 2;

extern "C" {
    #[cfg(any(feature = "lua51", feature = "luajit"))]
    pub fn lua_equal(L: *mut lua_State, idx1: c_int, idx2: c_int) -> c_int;
    pub fn lua_rawequal(L: *mut lua_State, idx1: c_int, idx2: c_int) -> c_int;
    #[cfg(any(feature = "lua51", feature = "luajit"))]
    pub fn lua_lessthan(L: *mut lua_State, idx1: c_int, idx2: c_int) -> c_int;
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    pub fn lua_compare(L: *mut lua_State, idx1: c_int, idx2: c_int, op: c_int) -> c_int;
}

// push functions (C -> stack)
extern "C" {
    pub fn lua_pushnil(L: *mut lua_State);
    pub fn lua_pushnumber(L: *mut lua_State, n: lua_Number);
    pub fn lua_pushinteger(L: *mut lua_State, n: lua_Integer);

    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_pushlstring(L: *mut lua_State, s: *const c_char, l: usize) -> *const c_char;
    #[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
    #[link_name = "lua_pushlstring"]
    pub fn lua_pushlstring_old(L: *mut lua_State, s: *const c_char, l: usize) -> *const c_char;

    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    pub fn lua_pushstring(L: *mut lua_State, s: *const c_char) -> *const c_char;
    #[cfg(any(feature = "lua51", feature = "luajit"))]
    #[link_name = "lua_pushstring"]
    pub fn lua_pushstring_old(L: *mut lua_State, s: *const c_char) -> *const c_char;

    // TODO: omitted:
    // lua_pushvfstring
    pub fn lua_pushfstring(L: *mut lua_State, fmt: *const c_char, ...) -> *const c_char;
    pub fn lua_pushcclosure(L: *mut lua_State, f: lua_CFunction, n: c_int);
    pub fn lua_pushboolean(L: *mut lua_State, b: c_int);
    pub fn lua_pushlightuserdata(L: *mut lua_State, p: *mut c_void);
    pub fn lua_pushthread(L: *mut lua_State) -> c_int;
}

// get functions (Lua -> stack)
extern "C" {
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_getglobal(L: *mut lua_State, var: *const c_char) -> c_int;
    #[cfg(feature = "lua52")]
    #[link_name = "lua_getglobal"]
    pub fn lua_getglobal_old(L: *mut lua_State, var: *const c_char);

    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_gettable(L: *mut lua_State, idx: c_int) -> c_int;
    #[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
    #[link_name = "lua_gettable"]
    pub fn lua_gettable_old(L: *mut lua_State, idx: c_int);

    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_getfield(L: *mut lua_State, idx: c_int, k: *const c_char) -> c_int;
    #[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
    #[link_name = "lua_getfield"]
    pub fn lua_getfield_old(L: *mut lua_State, idx: c_int, k: *const c_char);

    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_geti(L: *mut lua_State, idx: c_int, n: lua_Integer) -> c_int;

    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_rawget(L: *mut lua_State, idx: c_int) -> c_int;
    #[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
    #[link_name = "lua_rawget"]
    pub fn lua_rawget_old(L: *mut lua_State, idx: c_int);

    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_rawgeti(L: *mut lua_State, idx: c_int, n: lua_Integer) -> c_int;
    #[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
    #[link_name = "lua_rawgeti"]
    pub fn lua_rawgeti_old(L: *mut lua_State, idx: c_int, n: lua_Integer);

    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_rawgetp(L: *mut lua_State, idx: c_int, p: *const c_void) -> c_int;
    #[cfg(feature = "lua52")]
    #[link_name = "lua_rawgetp"]
    pub fn lua_rawgetp_old(L: *mut lua_State, idx: c_int, p: *const c_void);

    pub fn lua_createtable(L: *mut lua_State, narr: c_int, nrec: c_int);
    #[cfg(feature = "lua54")]
    pub fn lua_newuserdatauv(L: *mut lua_State, sz: usize, nuvalue: c_int) -> *mut c_void;
    #[cfg(any(
        feature = "lua53",
        feature = "lua52",
        feature = "lua51",
        feature = "luajit"
    ))]
    pub fn lua_newuserdata(L: *mut lua_State, sz: usize) -> *mut c_void;
    pub fn lua_getmetatable(L: *mut lua_State, objindex: c_int) -> c_int;

    #[cfg(feature = "lua54")]
    pub fn lua_getiuservalue(L: *mut lua_State, idx: c_int, n: c_int) -> c_int;
    #[cfg(feature = "lua53")]
    pub fn lua_getuservalue(L: *mut lua_State, idx: c_int) -> c_int;
    #[cfg(feature = "lua52")]
    #[link_name = "lua_getuservalue"]
    pub fn lua_getuservalue_old(L: *mut lua_State, idx: c_int);
    #[cfg(any(feature = "lua51", feature = "luajit"))]
    pub fn lua_getfenv(L: *mut lua_State, idx: c_int);
}

#[cfg(feature = "lua54")]
#[inline(always)]
pub unsafe fn lua_newuserdata(L: *mut lua_State, sz: usize) -> *mut c_void {
    lua_newuserdatauv(L, sz, 1)
}

#[cfg(feature = "lua54")]
#[inline(always)]
pub unsafe fn lua_getuservalue(L: *mut lua_State, idx: c_int) -> c_int {
    lua_getiuservalue(L, idx, 1)
}

// set functions (stack -> Lua)
extern "C" {
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    pub fn lua_setglobal(L: *mut lua_State, var: *const c_char);
    pub fn lua_settable(L: *mut lua_State, idx: c_int);
    pub fn lua_setfield(L: *mut lua_State, idx: c_int, k: *const c_char);
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_seti(L: *mut lua_State, idx: c_int, n: lua_Integer);
    pub fn lua_rawset(L: *mut lua_State, idx: c_int);
    pub fn lua_rawseti(L: *mut lua_State, idx: c_int, n: lua_Integer);
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    pub fn lua_rawsetp(L: *mut lua_State, idx: c_int, p: *const c_void);
    pub fn lua_setmetatable(L: *mut lua_State, objindex: c_int) -> c_int;
    #[cfg(feature = "lua54")]
    pub fn lua_setiuservalue(L: *mut lua_State, idx: c_int, n: c_int) -> c_int;
    #[cfg(any(feature = "lua53", feature = "lua52"))]
    pub fn lua_setuservalue(L: *mut lua_State, idx: c_int);
    #[cfg(any(feature = "lua51", feature = "luajit"))]
    pub fn lua_setfenv(L: *mut lua_State, idx: c_int) -> c_int;
}

#[cfg(feature = "lua54")]
#[inline(always)]
pub unsafe fn lua_setuservalue(L: *mut lua_State, idx: c_int) {
    lua_setiuservalue(L, idx, 1);
}

// 'load' and 'call' functions (load and run Lua code)
extern "C" {
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_callk(
        L: *mut lua_State,
        nargs: c_int,
        nresults: c_int,
        ctx: lua_KContext,
        k: Option<lua_KFunction>,
    );
    #[cfg(feature = "lua52")]
    pub fn lua_callk(
        L: *mut lua_State,
        nargs: c_int,
        nresults: c_int,
        ctx: c_int,
        k: Option<lua_CFunction>,
    );

    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_pcallk(
        L: *mut lua_State,
        nargs: c_int,
        nresults: c_int,
        errfunc: c_int,
        ctx: lua_KContext,
        k: Option<lua_KFunction>,
    ) -> c_int;
    #[cfg(feature = "lua52")]
    pub fn lua_pcallk(
        L: *mut lua_State,
        nargs: c_int,
        nresults: c_int,
        errfunc: c_int,
        ctx: c_int,
        k: Option<lua_CFunction>,
    ) -> c_int;

    #[cfg(feature = "lua52")]
    pub fn lua_getctx(L: *mut lua_State, ctx: *mut c_int) -> c_int;

    #[cfg(any(feature = "lua51", feature = "luajit"))]
    pub fn lua_call(L: *mut lua_State, nargs: c_int, nresults: c_int);
    #[cfg(any(feature = "lua51", feature = "luajit"))]
    pub fn lua_pcall(L: *mut lua_State, nargs: c_int, nresults: c_int, errfunc: c_int) -> c_int;

    // TODO
    pub fn lua_load(
        L: *mut lua_State,
        reader: lua_Reader,
        dt: *mut c_void,
        chunkname: *const c_char,
        mode: *const c_char,
    ) -> c_int;

    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_dump(
        L: *mut lua_State,
        writer: lua_Writer,
        data: *mut c_void,
        strip: c_int,
    ) -> c_int;
    #[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
    #[link_name = "lua_dump"]
    pub fn lua_dump_old(L: *mut lua_State, writer: lua_Writer, data: *mut c_void) -> c_int;
}

#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
#[inline(always)]
pub unsafe fn lua_call(L: *mut lua_State, n: c_int, r: c_int) {
    lua_callk(L, n, r, 0, None)
}

#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
#[inline(always)]
pub unsafe fn lua_pcall(L: *mut lua_State, n: c_int, r: c_int, f: c_int) -> c_int {
    lua_pcallk(L, n, r, f, 0, None)
}

// coroutine functions
extern "C" {
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_yieldk(
        L: *mut lua_State,
        nresults: c_int,
        ctx: lua_KContext,
        k: Option<lua_KFunction>,
    ) -> c_int;
    #[cfg(feature = "lua52")]
    pub fn lua_yieldk(
        L: *mut lua_State,
        nresults: c_int,
        ctx: c_int,
        k: Option<lua_CFunction>,
    ) -> c_int;
    #[cfg(any(feature = "lua51", feature = "luajit"))]
    pub fn lua_yield(L: *mut lua_State, nresults: c_int) -> c_int;

    #[cfg(feature = "lua54")]
    pub fn lua_resume(
        L: *mut lua_State,
        from: *mut lua_State,
        narg: c_int,
        nres: *mut c_int,
    ) -> c_int;
    #[cfg(any(feature = "lua53", feature = "lua52"))]
    #[link_name = "lua_resume"]
    pub fn lua_resume_53(L: *mut lua_State, from: *mut lua_State, narg: c_int) -> c_int;
    #[cfg(any(feature = "lua51", feature = "luajit"))]
    #[link_name = "lua_resume"]
    pub fn lua_resume_old(L: *mut lua_State, narg: c_int) -> c_int;

    pub fn lua_status(L: *mut lua_State) -> c_int;
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_isyieldable(L: *mut lua_State) -> c_int;
}

#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
#[inline(always)]
pub unsafe fn lua_yield(L: *mut lua_State, n: c_int) -> c_int {
    lua_yieldk(L, n, 0, None)
}

#[cfg(any(
    feature = "lua53",
    feature = "lua52",
    feature = "lua51",
    feature = "luajit"
))]
pub unsafe fn lua_resume(
    L: *mut lua_State,
    from: *mut lua_State,
    narg: c_int,
    nres: *mut c_int,
) -> c_int {
    let ret = lua_resume_53(L, from, narg);
    if ret == LUA_OK || ret == LUA_YIELD {
        *nres = lua_gettop(L);
    }
    ret
}

// warning-related functions
#[cfg(feature = "lua54")]
extern "C" {
    pub fn lua_setwarnf(L: *mut lua_State, f: lua_WarnFunction, ud: *mut c_void);
    pub fn lua_warning(L: *mut lua_State, msg: *const c_char, tocont: c_int);
}

// garbage-collection function and options
pub const LUA_GCSTOP: c_int = 0;
pub const LUA_GCRESTART: c_int = 1;
pub const LUA_GCCOLLECT: c_int = 2;
pub const LUA_GCCOUNT: c_int = 3;
pub const LUA_GCCOUNTB: c_int = 4;
pub const LUA_GCSTEP: c_int = 5;
pub const LUA_GCSETPAUSE: c_int = 6;
pub const LUA_GCSETSTEPMUL: c_int = 7;
#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
pub const LUA_GCISRUNNING: c_int = 9;
#[cfg(feature = "lua54")]
pub const LUA_GCGEN: c_int = 10;
#[cfg(feature = "lua54")]
pub const LUA_GCINC: c_int = 11;

extern "C" {
    #[cfg(feature = "lua54")]
    pub fn lua_gc(L: *mut lua_State, what: c_int, ...) -> c_int;
    #[cfg(any(
        feature = "lua53",
        feature = "lua52",
        feature = "lua51",
        feature = "luajit"
    ))]
    pub fn lua_gc(L: *mut lua_State, what: c_int, data: c_int) -> c_int;
}

// miscellaneous functions
extern "C" {
    pub fn lua_error(L: *mut lua_State) -> !;
    pub fn lua_next(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_concat(L: *mut lua_State, n: c_int);
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    pub fn lua_len(L: *mut lua_State, idx: c_int);
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub fn lua_stringtonumber(L: *mut lua_State, s: *const c_char) -> usize;
    pub fn lua_getallocf(L: *mut lua_State, ud: *mut *mut c_void) -> lua_Alloc;
    pub fn lua_setallocf(L: *mut lua_State, f: lua_Alloc, ud: *mut c_void);
    #[cfg(feature = "lua54")]
    pub fn lua_toclose(L: *mut lua_State, idx: c_int);
}

// some useful macros
// here, implemented as Rust functions
#[cfg(any(feature = "lua54", feature = "lua53"))]
#[inline(always)]
pub unsafe fn lua_getextraspace(L: *mut lua_State) -> *mut c_void {
    L.offset(-super::glue::LUA_EXTRASPACE as isize) as *mut c_void
}

#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
#[inline(always)]
pub unsafe fn lua_tonumber(L: *mut lua_State, i: c_int) -> lua_Number {
    lua_tonumberx(L, i, ptr::null_mut())
}

#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
#[inline(always)]
pub unsafe fn lua_tointeger(L: *mut lua_State, i: c_int) -> lua_Integer {
    lua_tointegerx(L, i, ptr::null_mut())
}

#[inline(always)]
pub unsafe fn lua_pop(L: *mut lua_State, n: c_int) {
    lua_settop(L, -n - 1)
}

#[inline(always)]
pub unsafe fn lua_newtable(L: *mut lua_State) {
    lua_createtable(L, 0, 0)
}

#[inline(always)]
pub unsafe fn lua_register(L: *mut lua_State, n: *const c_char, f: lua_CFunction) {
    lua_pushcfunction(L, f);
    lua_setglobal(L, n)
}

#[inline(always)]
pub unsafe fn lua_pushcfunction(L: *mut lua_State, f: lua_CFunction) {
    lua_pushcclosure(L, f, 0)
}

#[inline(always)]
pub unsafe fn lua_isfunction(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) == LUA_TFUNCTION) as c_int
}

#[inline(always)]
pub unsafe fn lua_istable(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) == LUA_TTABLE) as c_int
}

#[inline(always)]
pub unsafe fn lua_islightuserdata(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) == LUA_TLIGHTUSERDATA) as c_int
}

#[inline(always)]
pub unsafe fn lua_isnil(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) == LUA_TNIL) as c_int
}

#[inline(always)]
pub unsafe fn lua_isboolean(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) == LUA_TBOOLEAN) as c_int
}

#[inline(always)]
pub unsafe fn lua_isthread(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) == LUA_TTHREAD) as c_int
}

#[inline(always)]
pub unsafe fn lua_isnone(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) == LUA_TNONE) as c_int
}

#[inline(always)]
pub unsafe fn lua_isnoneornil(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) <= 0) as c_int
}

#[inline(always)]
pub unsafe fn lua_pushliteral(L: *mut lua_State, s: &'static str) -> *const c_char {
    use std::ffi::CString;
    let c_str = CString::new(s).unwrap();
    lua_pushlstring(L, c_str.as_ptr(), c_str.as_bytes().len())
}

#[cfg(any(feature = "lua51", feature = "luajit"))]
#[inline(always)]
pub unsafe fn lua_setglobal(L: *mut lua_State, var: *const c_char) {
    lua_setfield(L, LUA_GLOBALSINDEX, var)
}

#[cfg(any(feature = "lua51", feature = "luajit"))]
#[inline(always)]
pub unsafe fn lua_getglobal(L: *mut lua_State, var: *const c_char) -> c_int {
    lua_getfield(L, LUA_GLOBALSINDEX, var)
}

#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
#[inline(always)]
pub unsafe fn lua_pushglobaltable(L: *mut lua_State) -> c_int {
    lua_rawgeti(L, LUA_REGISTRYINDEX, LUA_RIDX_GLOBALS)
}

#[inline(always)]
pub unsafe fn lua_tostring(L: *mut lua_State, i: c_int) -> *const c_char {
    lua_tolstring(L, i, ptr::null_mut())
}

#[cfg(any(feature = "lua54", feature = "lua53"))]
#[inline(always)]
pub unsafe fn lua_insert(L: *mut lua_State, idx: c_int) {
    lua_rotate(L, idx, 1)
}

#[cfg(any(feature = "lua54", feature = "lua53"))]
#[inline(always)]
pub unsafe fn lua_remove(L: *mut lua_State, idx: c_int) {
    lua_rotate(L, idx, -1);
    lua_pop(L, 1)
}

#[cfg(any(feature = "lua54", feature = "lua53"))]
#[inline(always)]
pub unsafe fn lua_replace(L: *mut lua_State, idx: c_int) {
    lua_copy(L, -1, idx);
    lua_pop(L, 1)
}

// Debug API
// Event codes
pub const LUA_HOOKCALL: c_int = 0;
pub const LUA_HOOKRET: c_int = 1;
pub const LUA_HOOKLINE: c_int = 2;
pub const LUA_HOOKCOUNT: c_int = 3;
pub const LUA_HOOKTAILCALL: c_int = 4;

// Event masks
pub const LUA_MASKCALL: c_int = 1 << (LUA_HOOKCALL as usize);
pub const LUA_MASKRET: c_int = 1 << (LUA_HOOKRET as usize);
pub const LUA_MASKLINE: c_int = 1 << (LUA_HOOKLINE as usize);
pub const LUA_MASKCOUNT: c_int = 1 << (LUA_HOOKCOUNT as usize);

/// Type for functions to be called on debug events.
pub type lua_Hook = unsafe extern "C" fn(L: *mut lua_State, ar: *mut lua_Debug);

extern "C" {
    pub fn lua_getstack(L: *mut lua_State, level: c_int, ar: *mut lua_Debug) -> c_int;
    pub fn lua_getinfo(L: *mut lua_State, what: *const c_char, ar: *mut lua_Debug) -> c_int;
    pub fn lua_getlocal(L: *mut lua_State, ar: *const lua_Debug, n: c_int) -> *const c_char;
    pub fn lua_setlocal(L: *mut lua_State, ar: *const lua_Debug, n: c_int) -> *const c_char;
    pub fn lua_getupvalue(L: *mut lua_State, funcindex: c_int, n: c_int) -> *const c_char;
    pub fn lua_setupvalue(L: *mut lua_State, funcindex: c_int, n: c_int) -> *const c_char;

    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    pub fn lua_upvalueid(L: *mut lua_State, fidx: c_int, n: c_int) -> *mut c_void;
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    pub fn lua_upvaluejoin(L: *mut lua_State, fidx1: c_int, n1: c_int, fidx2: c_int, n2: c_int);

    pub fn lua_sethook(L: *mut lua_State, func: Option<lua_Hook>, mask: c_int, count: c_int);
    pub fn lua_gethook(L: *mut lua_State) -> Option<lua_Hook>;
    pub fn lua_gethookmask(L: *mut lua_State) -> c_int;
    pub fn lua_gethookcount(L: *mut lua_State) -> c_int;

    #[cfg(feature = "lua54")]
    pub fn lua_setcstacklimit(L: *mut lua_State, limit: c_uint) -> c_int;
}

#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
#[repr(C)]
pub struct lua_Debug {
    pub event: c_int,
    pub name: *const c_char,
    pub namewhat: *const c_char,
    pub what: *const c_char,
    pub source: *const c_char,
    #[cfg(feature = "lua54")]
    pub srclen: usize,
    pub currentline: c_int,
    pub linedefined: c_int,
    pub lastlinedefined: c_int,
    pub nups: c_uchar,
    pub nparams: c_uchar,
    pub isvararg: c_char,
    pub istailcall: c_char,
    #[cfg(feature = "lua54")]
    pub ftransfer: c_ushort,
    #[cfg(feature = "lua54")]
    pub ntransfer: c_ushort,
    pub short_src: [c_char; luaconf::LUA_IDSIZE as usize],
    // lua.h mentions this is for private use
    i_ci: *mut c_void,
}

#[cfg(any(feature = "lua51", feature = "luajit"))]
#[repr(C)]
pub struct lua_Debug {
    pub event: c_int,
    pub name: *const c_char,
    pub namewhat: *const c_char,
    pub what: *const c_char,
    pub source: *const c_char,
    pub currentline: c_int,
    pub nups: c_int,
    pub linedefined: c_int,
    pub lastlinedefined: c_int,
    pub short_src: [c_char; luaconf::LUA_IDSIZE as usize],
    // lua.h mentions this is for private use
    i_ci: c_int,
}
