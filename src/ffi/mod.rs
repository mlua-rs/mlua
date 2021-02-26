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

//! Low level bindings to Lua.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use std::os::raw::c_int;

// This is more or less in the order it appears in the Lua manual, with the
// exception of constants, which appear scattered throughout the manual text.

// C API types
pub use self::lua::{
    lua_Alloc, lua_CFunction, lua_Debug, lua_Hook, lua_Integer, lua_Number, lua_Reader, lua_State,
    lua_Unsigned, lua_Writer,
};

#[cfg(feature = "lua54")]
pub use self::lua::lua_WarnFunction;

#[cfg(any(feature = "lua54", feature = "lua53"))]
pub use self::lua::{lua_KContext, lua_KFunction};

#[cfg(any(feature = "lua51", feature = "luajit"))]
pub use self::lua::{lua_getfenv, lua_setfenv};

// C API functions
pub use self::lua::{
    lua_absindex,
    lua_arith,
    lua_atpanic,
    lua_call,
    lua_checkstack,
    lua_close,
    lua_compare,
    lua_concat,
    lua_copy,
    lua_createtable,
    lua_dump,
    lua_error,
    lua_gc,
    lua_getallocf,
    lua_getextraspace,
    lua_getfield,
    lua_getglobal,
    lua_gethook,
    lua_gethookcount,
    lua_gethookmask,
    lua_geti,
    lua_getinfo,
    lua_getlocal,
    lua_getmetatable,
    lua_getstack,
    lua_gettable,
    lua_gettop,
    lua_getupvalue,
    lua_getuservalue,
    lua_insert,
    lua_isboolean,
    lua_iscfunction,
    lua_isfunction,
    lua_isinteger,
    lua_islightuserdata,
    lua_isnil,
    lua_isnone,
    lua_isnoneornil,
    lua_isnumber,
    lua_isstring,
    lua_istable,
    lua_isthread,
    lua_isuserdata,
    lua_len,
    lua_load,
    lua_newstate,
    lua_newtable,
    lua_newthread,
    lua_newuserdata,
    lua_next,
    lua_pcall,
    lua_pop,
    lua_pushboolean,
    lua_pushcclosure,
    lua_pushcfunction,
    lua_pushfstring,
    lua_pushglobaltable,
    lua_pushinteger,
    lua_pushlightuserdata,
    lua_pushliteral,
    lua_pushlstring,
    lua_pushnil,
    lua_pushnumber,
    lua_pushstring,
    lua_pushthread,
    lua_pushvalue,
    // omitted: lua_pushvfstring
    lua_rawequal,
    lua_rawget,
    lua_rawgeti,
    lua_rawgetp,
    lua_rawlen,
    lua_rawset,
    lua_rawseti,
    lua_rawsetp,
    lua_register,
    lua_remove,
    lua_replace,
    lua_resume,
    lua_rotate,
    lua_setallocf,
    lua_setfield,
    lua_setglobal,
    lua_sethook,
    lua_seti,
    lua_setlocal,
    lua_setmetatable,
    lua_settable,
    lua_settop,
    lua_setupvalue,
    lua_setuservalue,
    lua_status,
    lua_stringtonumber,
    lua_toboolean,
    lua_tocfunction,
    lua_tointeger,
    lua_tointegerx,
    lua_tolstring,
    lua_tonumber,
    lua_tonumberx,
    lua_topointer,
    lua_tostring,
    lua_tothread,
    lua_touserdata,
    lua_type,
    lua_typename,
    lua_upvalueindex,
    lua_xmove,
    lua_yield,
};

#[cfg(feature = "lua54")]
pub use self::lua::{
    lua_getiuservalue, lua_newuserdatauv, lua_resetthread, lua_setcstacklimit, lua_setiuservalue,
    lua_setwarnf, lua_toclose, lua_warning,
};

#[cfg(any(feature = "lua54", feature = "lua53"))]
pub use self::lua::{lua_isyieldable, lua_version};

#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
pub use self::lua::{lua_callk, lua_pcallk, lua_upvalueid, lua_upvaluejoin, lua_yieldk};

// auxiliary library types
pub use self::lauxlib::luaL_Reg;

// auxiliary library functions
pub use self::lauxlib::{
    luaL_argcheck, luaL_argerror, luaL_callmeta, luaL_checkany, luaL_checkint, luaL_checkinteger,
    luaL_checklong, luaL_checklstring, luaL_checknumber, luaL_checkoption, luaL_checkstack,
    luaL_checkstring, luaL_checktype, luaL_checkudata, luaL_checkversion, luaL_dofile,
    luaL_dostring, luaL_error, luaL_getmetafield, luaL_getmetatable, luaL_getsubtable, luaL_gsub,
    luaL_len, luaL_loadbuffer, luaL_loadbufferx, luaL_loadfile, luaL_loadstring, luaL_newlib,
    luaL_newlibtable, luaL_newmetatable, luaL_newstate, luaL_optint, luaL_optinteger, luaL_optlong,
    luaL_optlstring, luaL_optnumber, luaL_optstring, luaL_ref, luaL_requiref, luaL_setfuncs,
    luaL_setmetatable, luaL_testudata, luaL_tolstring, luaL_traceback, luaL_typename, luaL_unref,
    luaL_where,
};

#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
pub use self::lauxlib::{luaL_execresult, luaL_fileresult, luaL_loadfilex};

// lualib.h functions
pub use self::lualib::{
    luaL_openlibs, luaopen_base, luaopen_debug, luaopen_io, luaopen_math, luaopen_os,
    luaopen_package, luaopen_string, luaopen_table,
};

#[cfg(any(feature = "lua54", feature = "lua53"))]
pub use self::lualib::{luaopen_coroutine, luaopen_utf8};

#[cfg(feature = "lua52")]
pub use self::lualib::{luaopen_bit32, luaopen_coroutine};

#[cfg(feature = "luajit")]
pub use self::lualib::{luaopen_bit, luaopen_ffi, luaopen_jit};

// constants from lua.h
pub use self::lua::{
    LUA_ERRERR, LUA_ERRMEM, LUA_ERRRUN, LUA_ERRSYNTAX, LUA_GCCOLLECT, LUA_GCCOUNT, LUA_GCCOUNTB,
    LUA_GCRESTART, LUA_GCSETPAUSE, LUA_GCSETSTEPMUL, LUA_GCSTEP, LUA_GCSTOP, LUA_HOOKCALL,
    LUA_HOOKCOUNT, LUA_HOOKLINE, LUA_HOOKRET, LUA_HOOKTAILCALL, LUA_MASKCALL, LUA_MASKCOUNT,
    LUA_MASKLINE, LUA_MASKRET, LUA_MINSTACK, LUA_MULTRET, LUA_OK, LUA_OPADD, LUA_OPDIV, LUA_OPEQ,
    LUA_OPLE, LUA_OPLT, LUA_OPMOD, LUA_OPMUL, LUA_OPPOW, LUA_OPSUB, LUA_OPUNM, LUA_REGISTRYINDEX,
    LUA_SIGNATURE, LUA_TBOOLEAN, LUA_TFUNCTION, LUA_TLIGHTUSERDATA, LUA_TNIL, LUA_TNONE,
    LUA_TNUMBER, LUA_TSTRING, LUA_TTABLE, LUA_TTHREAD, LUA_TUSERDATA, LUA_YIELD,
};

#[cfg(any(feature = "lua54", feature = "lua53"))]
pub use self::lua::{
    LUA_OPBAND, LUA_OPBNOT, LUA_OPBOR, LUA_OPBXOR, LUA_OPIDIV, LUA_OPSHL, LUA_OPSHR,
};

#[cfg(feature = "lua54")]
pub use self::lua::{LUA_GCGEN, LUA_GCINC};

#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
pub use self::lua::{LUA_GCISRUNNING, LUA_RIDX_GLOBALS, LUA_RIDX_MAINTHREAD};

#[cfg(any(feature = "lua53", feature = "lua52"))]
pub use self::lua::LUA_ERRGCMM;

#[cfg(any(feature = "lua51", feature = "luajit"))]
pub use self::lua::{LUA_ENVIRONINDEX, LUA_GLOBALSINDEX};

// constants from lauxlib.h
pub use self::lauxlib::{LUA_ERRFILE, LUA_NOREF, LUA_REFNIL};

// constants from lualib.h
pub use self::lualib::{
    LUA_COLIBNAME, LUA_DBLIBNAME, LUA_IOLIBNAME, LUA_LOADLIBNAME, LUA_MATHLIBNAME, LUA_OSLIBNAME,
    LUA_STRLIBNAME, LUA_TABLIBNAME,
};

#[cfg(any(feature = "lua54", feature = "lua53"))]
pub use self::lualib::LUA_UTF8LIBNAME;

#[cfg(any(feature = "lua52", feature = "luajit"))]
pub use self::lualib::LUA_BITLIBNAME;

#[cfg(feature = "luajit")]
pub use self::lualib::{LUA_FFILIBNAME, LUA_JITLIBNAME};

// Not actually defined in lua.h / luaconf.h
pub const LUA_MAX_UPVALUES: c_int = 255;

// Copied from https://github.com/rust-lang/rust/blob/master/src/libstd/sys_common/alloc.rs
#[cfg(all(any(
    target_arch = "x86",
    target_arch = "arm",
    target_arch = "mips",
    target_arch = "powerpc",
    target_arch = "powerpc64",
    target_arch = "asmjs",
    target_arch = "wasm32",
    target_arch = "hexagon"
)))]
pub const SYS_MIN_ALIGN: usize = 8;
#[cfg(all(any(
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "mips64",
    target_arch = "s390x",
    target_arch = "sparc64",
    target_arch = "riscv64"
)))]
pub const SYS_MIN_ALIGN: usize = 16;

#[allow(unused_imports, dead_code, non_camel_case_types)]
#[allow(clippy::unreadable_literal)]
mod glue {
    include!(concat!(env!("OUT_DIR"), "/glue.rs"));
}

#[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
mod compat53;

mod lauxlib;
mod lua;
mod luaconf;
mod lualib;
