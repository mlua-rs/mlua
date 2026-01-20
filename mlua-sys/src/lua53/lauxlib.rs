//! Contains definitions from `lauxlib.h`.

use std::os::raw::{c_char, c_int, c_void};
use std::{mem, ptr};

use super::lua::{self, lua_CFunction, lua_Integer, lua_Number, lua_State};

// Extra error code for 'luaL_loadfilex'
pub const LUA_ERRFILE: c_int = lua::LUA_ERRERR + 1;

// Key, in the registry, for table of loaded modules
pub const LUA_LOADED_TABLE: *const c_char = cstr!("_LOADED");

// Key, in the registry, for table of preloaded loaders
pub const LUA_PRELOAD_TABLE: *const c_char = cstr!("_PRELOAD");

#[repr(C)]
pub struct luaL_Reg {
    pub name: *const c_char,
    pub func: lua_CFunction,
}

#[cfg_attr(all(windows, raw_dylib), link(name = "lua53", kind = "raw-dylib"))]
unsafe extern "C-unwind" {
    pub fn luaL_checkversion_(L: *mut lua_State, ver: lua_Number, sz: usize);

    pub fn luaL_getmetafield(L: *mut lua_State, obj: c_int, e: *const c_char) -> c_int;
    pub fn luaL_callmeta(L: *mut lua_State, obj: c_int, e: *const c_char) -> c_int;
    #[link_name = "luaL_tolstring"]
    pub fn luaL_tolstring_(L: *mut lua_State, idx: c_int, len: *mut usize) -> *const c_char;
    pub fn luaL_argerror(L: *mut lua_State, arg: c_int, extramsg: *const c_char) -> c_int;
    pub fn luaL_checklstring(L: *mut lua_State, arg: c_int, l: *mut usize) -> *const c_char;
    pub fn luaL_optlstring(L: *mut lua_State, arg: c_int, def: *const c_char, l: *mut usize)
        -> *const c_char;
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

    pub fn luaL_checkoption(
        L: *mut lua_State,
        arg: c_int,
        def: *const c_char,
        lst: *const *const c_char,
    ) -> c_int;

    pub fn luaL_fileresult(L: *mut lua_State, stat: c_int, fname: *const c_char) -> c_int;
    pub fn luaL_execresult(L: *mut lua_State, stat: c_int) -> c_int;
}

// Pre-defined references
pub const LUA_NOREF: c_int = -2;
pub const LUA_REFNIL: c_int = -1;

#[cfg_attr(all(windows, raw_dylib), link(name = "lua53", kind = "raw-dylib"))]
unsafe extern "C-unwind" {
    pub fn luaL_ref(L: *mut lua_State, t: c_int) -> c_int;
    pub fn luaL_unref(L: *mut lua_State, t: c_int, r#ref: c_int);

    pub fn luaL_loadfilex(L: *mut lua_State, filename: *const c_char, mode: *const c_char) -> c_int;
}

#[inline(always)]
pub unsafe fn luaL_loadfile(L: *mut lua_State, f: *const c_char) -> c_int {
    luaL_loadfilex(L, f, ptr::null())
}

#[cfg_attr(all(windows, raw_dylib), link(name = "lua53", kind = "raw-dylib"))]
unsafe extern "C-unwind" {
    pub fn luaL_loadbufferx(
        L: *mut lua_State,
        buff: *const c_char,
        sz: usize,
        name: *const c_char,
        mode: *const c_char,
    ) -> c_int;
    pub fn luaL_loadstring(L: *mut lua_State, s: *const c_char) -> c_int;

    pub fn luaL_newstate() -> *mut lua_State;

    pub fn luaL_len(L: *mut lua_State, idx: c_int) -> lua_Integer;

    pub fn luaL_gsub(
        L: *mut lua_State,
        s: *const c_char,
        p: *const c_char,
        r: *const c_char,
    ) -> *const c_char;

    pub fn luaL_setfuncs(L: *mut lua_State, l: *const luaL_Reg, nup: c_int);

    pub fn luaL_getsubtable(L: *mut lua_State, idx: c_int, fname: *const c_char) -> c_int;

    pub fn luaL_traceback(L: *mut lua_State, L1: *mut lua_State, msg: *const c_char, level: c_int);

    pub fn luaL_requiref(L: *mut lua_State, modname: *const c_char, openf: lua_CFunction, glb: c_int);
}

//
// Some useful macros (implemented as Rust functions)
//

// TODO: luaL_newlibtable, luaL_newlib

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

#[inline(always)]
pub unsafe fn luaL_tolstring(L: *mut lua_State, idx: c_int, len: *mut usize) -> *const c_char {
    luaL_tolstring_(L, lua::lua_absindex(L, idx), len)
}

#[inline(always)]
pub unsafe fn luaL_loadbuffer(L: *mut lua_State, s: *const c_char, sz: usize, n: *const c_char) -> c_int {
    luaL_loadbufferx(L, s, sz, n, ptr::null())
}

#[inline(always)]
pub unsafe fn luaL_opt<T>(
    L: *mut lua_State,
    f: unsafe extern "C-unwind" fn(*mut lua_State, c_int) -> T,
    n: c_int,
    d: T,
) -> T {
    if lua::lua_isnoneornil(L, n) != 0 {
        d
    } else {
        f(L, n)
    }
}

//
// Generic Buffer Manipulation
//

// The buffer size used by the lauxlib buffer system.
// In Lua 5.3: LUAL_BUFFERSIZE = (int)(0x80 * sizeof(void*) * sizeof(lua_Integer))
#[rustfmt::skip]
pub const LUAL_BUFFERSIZE: usize = 0x80 * mem::size_of::<*const ()>() * mem::size_of::<lua_Integer>();

#[repr(C)]
pub struct luaL_Buffer {
    pub b: *mut c_char, // buffer address
    pub size: usize,    // buffer size
    pub n: usize,       // number of characters in buffer
    pub L: *mut lua_State,
    pub initb: [c_char; LUAL_BUFFERSIZE], // initial buffer space
}

#[cfg_attr(all(windows, raw_dylib), link(name = "lua53", kind = "raw-dylib"))]
unsafe extern "C-unwind" {
    pub fn luaL_buffinit(L: *mut lua_State, B: *mut luaL_Buffer);
    pub fn luaL_prepbuffsize(B: *mut luaL_Buffer, sz: usize) -> *mut c_char;
    pub fn luaL_addlstring(B: *mut luaL_Buffer, s: *const c_char, l: usize);
    pub fn luaL_addstring(B: *mut luaL_Buffer, s: *const c_char);
    pub fn luaL_addvalue(B: *mut luaL_Buffer);
    pub fn luaL_pushresult(B: *mut luaL_Buffer);
    pub fn luaL_pushresultsize(B: *mut luaL_Buffer, sz: usize);
    pub fn luaL_buffinitsize(L: *mut lua_State, B: *mut luaL_Buffer, sz: usize) -> *mut c_char;
}

// Macro implementations as inline functions

#[inline(always)]
pub unsafe fn luaL_prepbuffer(B: *mut luaL_Buffer) -> *mut c_char {
    luaL_prepbuffsize(B, LUAL_BUFFERSIZE)
}

#[inline(always)]
pub unsafe fn luaL_addchar(B: *mut luaL_Buffer, c: c_char) {
    if (*B).n >= (*B).size {
        luaL_prepbuffsize(B, 1);
    }
    *(*B).b.add((*B).n) = c;
    (*B).n += 1;
}

#[inline(always)]
pub unsafe fn luaL_addsize(B: *mut luaL_Buffer, n: usize) {
    (*B).n += n;
}
