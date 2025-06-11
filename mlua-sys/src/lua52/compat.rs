//! MLua compatibility layer for Lua 5.2
//!
//! Based on github.com/keplerproject/lua-compat-5.3

use std::os::raw::{c_char, c_int, c_void};
use std::ptr;

use super::lauxlib::*;
use super::lua::*;

#[inline(always)]
unsafe fn compat53_reverse(L: *mut lua_State, mut a: c_int, mut b: c_int) {
    while a < b {
        lua_pushvalue(L, a);
        lua_pushvalue(L, b);
        lua_replace(L, a);
        lua_replace(L, b);
        a += 1;
        b -= 1;
    }
}

//
// lua ported functions
//

pub unsafe fn lua_rotate(L: *mut lua_State, mut idx: c_int, mut n: c_int) {
    idx = lua_absindex(L, idx);
    if n > 0 {
        // Faster version
        for _ in 0..n {
            lua_insert(L, idx);
        }
        return;
    }
    let n_elems = lua_gettop(L) - idx + 1;
    if n < 0 {
        n += n_elems;
    }
    if n > 0 && n < n_elems {
        luaL_checkstack(L, 2, cstr!("not enough stack slots available"));
        n = n_elems - n;
        compat53_reverse(L, idx, idx + n - 1);
        compat53_reverse(L, idx + n, idx + n_elems - 1);
        compat53_reverse(L, idx, idx + n_elems - 1);
    }
}

#[inline(always)]
pub unsafe fn lua_isinteger(L: *mut lua_State, idx: c_int) -> c_int {
    if lua_type(L, idx) == LUA_TNUMBER {
        let n = lua_tonumber(L, idx);
        let i = lua_tointeger(L, idx);
        if (n - i as lua_Number).abs() < lua_Number::EPSILON {
            return 1;
        }
    }
    0
}

#[inline(always)]
pub unsafe fn lua_tointeger(L: *mut lua_State, i: c_int) -> lua_Integer {
    lua_tointegerx(L, i, ptr::null_mut())
}

// Implemented for Lua 5.2 as well
// See https://github.com/keplerproject/lua-compat-5.3/issues/40
#[inline(always)]
pub unsafe fn lua_tointegerx(L: *mut lua_State, i: c_int, isnum: *mut c_int) -> lua_Integer {
    let mut ok = 0;
    let n = lua_tonumberx(L, i, &mut ok);
    let n_int = n as lua_Integer;
    if ok != 0 && (n - n_int as lua_Number).abs() < lua_Number::EPSILON {
        if !isnum.is_null() {
            *isnum = 1;
        }
        return n_int;
    }
    if !isnum.is_null() {
        *isnum = 0;
    }
    0
}

#[inline(always)]
pub unsafe fn lua_pushlstring(L: *mut lua_State, s: *const c_char, l: usize) -> *const c_char {
    if l == 0 {
        lua_pushlstring_(L, cstr!(""), 0)
    } else {
        lua_pushlstring_(L, s, l)
    }
}

#[inline(always)]
pub unsafe fn lua_getglobal(L: *mut lua_State, var: *const c_char) -> c_int {
    lua_getglobal_(L, var);
    lua_type(L, -1)
}

#[inline(always)]
pub unsafe fn lua_gettable(L: *mut lua_State, idx: c_int) -> c_int {
    lua_gettable_(L, idx);
    lua_type(L, -1)
}

#[inline(always)]
pub unsafe fn lua_getfield(L: *mut lua_State, idx: c_int, k: *const c_char) -> c_int {
    lua_getfield_(L, idx, k);
    lua_type(L, -1)
}

#[inline(always)]
pub unsafe fn lua_geti(L: *mut lua_State, mut idx: c_int, n: lua_Integer) -> c_int {
    idx = lua_absindex(L, idx);
    lua_pushinteger(L, n);
    lua_gettable(L, idx)
}

#[inline(always)]
pub unsafe fn lua_rawget(L: *mut lua_State, idx: c_int) -> c_int {
    lua_rawget_(L, idx);
    lua_type(L, -1)
}

#[inline(always)]
pub unsafe fn lua_rawgeti(L: *mut lua_State, idx: c_int, n: lua_Integer) -> c_int {
    let n = n.try_into().expect("cannot convert index to lua_Integer");
    lua_rawgeti_(L, idx, n);
    lua_type(L, -1)
}

#[inline(always)]
pub unsafe fn lua_rawgetp(L: *mut lua_State, idx: c_int, p: *const c_void) -> c_int {
    lua_rawgetp_(L, idx, p);
    lua_type(L, -1)
}

#[inline(always)]
pub unsafe fn lua_getuservalue(L: *mut lua_State, idx: c_int) -> c_int {
    lua_getuservalue_(L, idx);
    lua_type(L, -1)
}

#[inline(always)]
pub unsafe fn lua_seti(L: *mut lua_State, mut idx: c_int, n: lua_Integer) {
    luaL_checkstack(L, 1, cstr!("not enough stack slots available"));
    idx = lua_absindex(L, idx);
    lua_pushinteger(L, n);
    lua_insert(L, -2);
    lua_settable(L, idx);
}

#[inline(always)]
pub unsafe fn lua_rawseti(L: *mut lua_State, idx: c_int, n: lua_Integer) {
    let n = n.try_into().expect("cannot convert index from lua_Integer");
    lua_rawseti_(L, idx, n)
}

#[inline(always)]
pub unsafe fn lua_dump(L: *mut lua_State, writer: lua_Writer, data: *mut c_void, _strip: c_int) -> c_int {
    lua_dump_(L, writer, data)
}

#[inline(always)]
pub unsafe fn lua_resume(L: *mut lua_State, from: *mut lua_State, narg: c_int, nres: *mut c_int) -> c_int {
    let ret = lua_resume_(L, from, narg);
    if (ret == LUA_OK || ret == LUA_YIELD) && !(nres.is_null()) {
        *nres = lua_gettop(L);
    }
    ret
}

//
// lauxlib ported functions
//

#[inline(always)]
pub unsafe fn luaL_getmetafield(L: *mut lua_State, obj: c_int, e: *const c_char) -> c_int {
    if luaL_getmetafield_(L, obj, e) != 0 {
        lua_type(L, -1)
    } else {
        LUA_TNIL
    }
}

#[inline(always)]
pub unsafe fn luaL_newmetatable(L: *mut lua_State, tname: *const c_char) -> c_int {
    if luaL_newmetatable_(L, tname) != 0 {
        lua_pushstring(L, tname);
        lua_setfield(L, -2, cstr!("__name"));
        1
    } else {
        0
    }
}

pub unsafe fn luaL_tolstring(L: *mut lua_State, mut idx: c_int, len: *mut usize) -> *const c_char {
    idx = lua_absindex(L, idx);
    if luaL_callmeta(L, idx, cstr!("__tostring")) == 0 {
        match lua_type(L, idx) {
            LUA_TNIL => {
                lua_pushliteral(L, c"nil");
            }
            LUA_TSTRING | LUA_TNUMBER => {
                lua_pushvalue(L, idx);
            }
            LUA_TBOOLEAN => {
                if lua_toboolean(L, idx) == 0 {
                    lua_pushliteral(L, c"false");
                } else {
                    lua_pushliteral(L, c"true");
                }
            }
            t => {
                let tt = luaL_getmetafield(L, idx, cstr!("__name"));
                let name = if tt == LUA_TSTRING {
                    lua_tostring(L, -1)
                } else {
                    lua_typename(L, t)
                };
                lua_pushfstring(L, cstr!("%s: %p"), name, lua_topointer(L, idx));
                if tt != LUA_TNIL {
                    lua_replace(L, -2); // remove '__name'
                }
            }
        };
    } else if lua_isstring(L, -1) == 0 {
        luaL_error(L, cstr!("'__tostring' must return a string"));
    }
    lua_tolstring(L, -1, len)
}

pub unsafe fn luaL_requiref(L: *mut lua_State, modname: *const c_char, openf: lua_CFunction, glb: c_int) {
    luaL_checkstack(L, 3, cstr!("not enough stack slots available"));
    luaL_getsubtable(L, LUA_REGISTRYINDEX, LUA_LOADED_TABLE);
    if lua_getfield(L, -1, modname) == LUA_TNIL {
        lua_pop(L, 1);
        lua_pushcfunction(L, openf);
        lua_pushstring(L, modname);
        lua_call(L, 1, 1);
        lua_pushvalue(L, -1);
        lua_setfield(L, -3, modname);
    }
    if glb != 0 {
        lua_pushvalue(L, -1);
        lua_setglobal(L, modname);
    }
    lua_replace(L, -2);
}

pub unsafe fn luaL_loadbufferenv(
    L: *mut lua_State,
    data: *const c_char,
    size: usize,
    name: *const c_char,
    mode: *const c_char,
    mut env: c_int,
) -> c_int {
    if env != 0 {
        env = lua_absindex(L, env);
    }
    let status = luaL_loadbufferx(L, data, size, name, mode);
    if status == LUA_OK && env != 0 {
        lua_pushvalue(L, env);
        lua_setupvalue(L, -2, 1);
    }
    status
}
