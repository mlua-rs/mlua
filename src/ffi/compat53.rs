//! Lua 5.3 compatibility layer for Lua 5.1/5.2
//!
//! Based on github.com/keplerproject/lua-compat-5.3

#![allow(clippy::needless_return)]

use std::convert::TryInto;
use std::ffi::CStr;
use std::mem;
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;
use std::str::FromStr;

#[cfg(feature = "lua52")]
use super::lua52::*;

#[cfg(any(feature = "lua51", feature = "luajit"))]
use super::lua51::*;

#[cfg(feature = "luau")]
use super::luau::*;

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

const COMPAT53_LEVELS1: c_int = 12; // size of the first part of the stack
const COMPAT53_LEVELS2: c_int = 10; // size of the second part of the stack

#[cfg(not(feature = "luau"))]
unsafe fn compat53_countlevels(L: *mut lua_State) -> c_int {
    let mut ar: lua_Debug = mem::zeroed();
    let (mut li, mut le) = (1, 1);
    // find an upper bound
    while lua_getstack(L, le, &mut ar) != 0 {
        li = le;
        le *= 2;
    }
    // do a binary search
    while li < le {
        let m = (li + le) / 2;
        if lua_getstack(L, m, &mut ar) != 0 {
            li = m + 1;
        } else {
            le = m;
        }
    }
    le - 1
}

#[cfg(feature = "luau")]
unsafe fn compat53_countlevels(L: *mut lua_State) -> c_int {
    let mut ar: lua_Debug = mem::zeroed();
    let (mut li, mut le) = (1, 1);
    // find an upper bound
    while lua_getinfo(L, le, cstr!(""), &mut ar) != 0 {
        li = le;
        le *= 2;
    }
    // do a binary search
    while li < le {
        let m = (li + le) / 2;
        if lua_getinfo(L, m, cstr!(""), &mut ar) != 0 {
            li = m + 1;
        } else {
            le = m;
        }
    }
    le - 1
}

unsafe fn compat53_checkmode(
    L: *mut lua_State,
    mode: *const c_char,
    modename: *const c_char,
    err: c_int,
) -> c_int {
    unsafe fn strchr(s: *const c_char, c: c_char) -> *const c_char {
        let mut st = s;
        while *st != 0 && *st != c {
            st = st.offset(1);
        }
        if *st == c {
            st
        } else {
            ptr::null()
        }
    }

    if !mode.is_null() && strchr(mode, *modename).is_null() {
        lua_pushfstring(
            L,
            cstr!("attempt to load a %s chunk (mode is '%s')"),
            modename,
            mode,
        );
        return err;
    }
    LUA_OK
}

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
unsafe fn compat53_findfield(L: *mut lua_State, objidx: c_int, level: c_int) -> c_int {
    if level == 0 || lua_istable(L, -1) == 0 {
        return 0; // not found
    }

    lua_pushnil(L); // start 'next' loop
    while lua_next(L, -2) != 0 {
        // for each pair in table
        if lua_type(L, -2) == LUA_TSTRING {
            // ignore non-string keys
            if lua_rawequal(L, objidx, -1) != 0 {
                // found object?
                lua_pop(L, 1); // remove value (but keep name)
                return 1;
            } else if compat53_findfield(L, objidx, level - 1) != 0 {
                // try recursively
                lua_remove(L, -2); // remove table (but keep name)
                lua_pushliteral(L, ".");
                lua_insert(L, -2); // place '.' between the two names
                lua_concat(L, 3);
                return 1;
            }
        }
        lua_pop(L, 1); // remove value
    }
    return 0; // not found
}

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
unsafe fn compat53_pushglobalfuncname(
    L: *mut lua_State,
    level: c_int,
    ar: *mut lua_Debug,
) -> c_int {
    let top = lua_gettop(L);
    // push function
    #[cfg(not(feature = "luau"))]
    lua_getinfo(L, cstr!("f"), ar);
    #[cfg(feature = "luau")]
    lua_getinfo(L, level, cstr!("f"), ar);
    lua_pushvalue(L, LUA_GLOBALSINDEX);
    if compat53_findfield(L, top + 1, 2) != 0 {
        lua_copy(L, -1, top + 1); // move name to proper place
        lua_pop(L, 2); // remove pushed values
        return 1;
    } else {
        lua_settop(L, top); // remove function and global table
        return 0;
    }
}

#[cfg(feature = "luau")]
unsafe fn compat53_pushfuncname(L: *mut lua_State, level: c_int, ar: *mut lua_Debug) {
    /*
    if *(*ar).namewhat != b'\0' as c_char {
        // is there a name?
        lua_pushfstring(L, cstr!("function '%s'"), (*ar).name);
    } else
    */
    if *(*ar).what == b'm' as c_char {
        // main?
        lua_pushliteral(L, "main chunk");
    } else if *(*ar).what == b'C' as c_char {
        if compat53_pushglobalfuncname(L, level, ar) != 0 {
            lua_pushfstring(L, cstr!("function '%s'"), lua_tostring(L, -1));
            lua_remove(L, -2); // remove name
        } else {
            lua_pushliteral(L, "?");
        }
    } else {
        lua_pushfstring(
            L,
            cstr!("function <%s:%d>"),
            (*ar).short_src.as_ptr(),
            (*ar).linedefined,
        );
    }
}

#[cfg(any(feature = "lua51", feature = "luajit"))]
unsafe fn compat53_pushfuncname(L: *mut lua_State, ar: *mut lua_Debug) {
    if *(*ar).namewhat != b'\0' as c_char {
        // is there a name?
        lua_pushfstring(L, cstr!("function '%s'"), (*ar).name);
    } else if *(*ar).what == b'm' as c_char {
        // main?
        lua_pushliteral(L, "main chunk");
    } else if *(*ar).what == b'C' as c_char {
        if compat53_pushglobalfuncname(L, -1, ar) != 0 {
            lua_pushfstring(L, cstr!("function '%s'"), lua_tostring(L, -1));
            lua_remove(L, -2); // remove name
        } else {
            lua_pushliteral(L, "?");
        }
    } else {
        lua_pushfstring(
            L,
            cstr!("function <%s:%d>"),
            (*ar).short_src.as_ptr(),
            (*ar).linedefined,
        );
    }
}

unsafe fn compat53_call_lua(L: *mut lua_State, code: &str, nargs: c_int, nret: c_int) {
    lua_rawgetp(L, LUA_REGISTRYINDEX, code.as_ptr() as *const c_void);
    if lua_type(L, -1) != LUA_TFUNCTION {
        lua_pop(L, 1);
        if luaL_loadbuffer(
            L,
            code.as_ptr() as *const c_char,
            code.as_bytes().len(),
            cstr!("=none"),
        ) != 0
        {
            lua_error(L);
        }
        lua_pushvalue(L, -1);
        lua_rawsetp(L, LUA_REGISTRYINDEX, code.as_ptr() as *const c_void);
    }
    lua_insert(L, -nargs - 1);
    lua_call(L, nargs, nret);
}

//
// lua ported functions
//

#[cfg(any(feature = "lua51", feature = "luajit"))]
#[inline(always)]
pub unsafe fn lua_absindex(L: *mut lua_State, mut idx: c_int) -> c_int {
    if idx < 0 && idx > LUA_REGISTRYINDEX {
        idx += lua_gettop(L) + 1;
    }
    idx
}

// Comparison and arithmetic functions
#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
pub const LUA_OPADD: c_int = 0;
#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
pub const LUA_OPSUB: c_int = 1;
#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
pub const LUA_OPMUL: c_int = 2;
#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
pub const LUA_OPDIV: c_int = 3;
#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
pub const LUA_OPMOD: c_int = 4;
#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
pub const LUA_OPPOW: c_int = 5;
#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
pub const LUA_OPUNM: c_int = 6;

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
static COMPAT53_ARITH_CODE: &str = r#"
local op,a,b = ...
if op == 0 then return a+b
elseif op == 1 then return a-b
elseif op == 2 then return a*b
elseif op == 3 then return a/b
elseif op == 4 then return a%b
elseif op == 5 then return a^b
elseif op == 6 then return -a
end
"#;

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
pub unsafe fn lua_arith(L: *mut lua_State, op: c_int) {
    #[allow(clippy::manual_range_contains)]
    if op < LUA_OPADD || op > LUA_OPUNM {
        luaL_error(L, cstr!("invalid 'op' argument for lua_arith"));
    }
    luaL_checkstack(L, 5, cstr!("not enough stack slots"));
    if op == LUA_OPUNM {
        lua_pushvalue(L, -1);
    }
    lua_pushnumber(L, op as lua_Number);
    lua_insert(L, -3);
    compat53_call_lua(L, COMPAT53_ARITH_CODE, 3, 1);
}

pub unsafe fn lua_rotate(L: *mut lua_State, mut idx: c_int, mut n: c_int) {
    idx = lua_absindex(L, idx);
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

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
#[inline(always)]
pub unsafe fn lua_copy(L: *mut lua_State, fromidx: c_int, toidx: c_int) {
    let abs_to = lua_absindex(L, toidx);
    luaL_checkstack(L, 1, cstr!("not enough stack slots"));
    lua_pushvalue(L, fromidx);
    lua_replace(L, abs_to);
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
    return 0;
}

#[cfg(any(
    feature = "lua52",
    feature = "lua51",
    feature = "luajit",
    feature = "luau"
))]
#[inline(always)]
pub unsafe fn lua_tointeger(L: *mut lua_State, i: c_int) -> lua_Integer {
    lua_tointegerx(L, i, ptr::null_mut())
}

#[cfg(any(feature = "lua51", feature = "luajit"))]
#[inline(always)]
pub unsafe fn lua_tonumberx(L: *mut lua_State, i: c_int, isnum: *mut c_int) -> lua_Number {
    let n = lua_tonumber(L, i);
    if !isnum.is_null() {
        *isnum = if n != 0.0 || lua_isnumber(L, i) != 0 {
            1
        } else {
            0
        };
    }
    return n;
}

// Implemented for Lua 5.2 as well
// See https://github.com/keplerproject/lua-compat-5.3/issues/40
#[cfg(any(
    feature = "lua52",
    feature = "lua51",
    feature = "luajit",
    feature = "luau"
))]
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
    return 0;
}

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
#[inline(always)]
pub unsafe fn lua_rawlen(L: *mut lua_State, idx: c_int) -> usize {
    lua_objlen(L, idx)
}

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
pub const LUA_OPEQ: c_int = 0;
#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
pub const LUA_OPLT: c_int = 1;
#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
pub const LUA_OPLE: c_int = 2;

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
#[inline(always)]
pub unsafe fn lua_compare(L: *mut lua_State, mut idx1: c_int, mut idx2: c_int, op: c_int) -> c_int {
    match op {
        LUA_OPEQ => lua_equal(L, idx1, idx2),
        LUA_OPLT => lua_lessthan(L, idx1, idx2),
        LUA_OPLE => {
            luaL_checkstack(L, 5, cstr!("not enough stack slots"));
            idx1 = lua_absindex(L, idx1);
            idx2 = lua_absindex(L, idx2);
            lua_pushvalue(L, idx1);
            lua_pushvalue(L, idx2);
            compat53_call_lua(L, "local a,b=...\nreturn a<=b\n", 2, 1);
            let result = lua_toboolean(L, -1);
            lua_pop(L, 1);
            result
        }
        _ => luaL_error(L, cstr!("invalid 'op' argument for lua_compare")),
    }
}

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
#[inline(always)]
pub unsafe fn lua_pushlstring(L: *mut lua_State, s: *const c_char, l: usize) -> *const c_char {
    if l == 0 {
        lua_pushlstring_(L, cstr!(""), 0);
    } else {
        lua_pushlstring_(L, s, l);
    }
    lua_tostring(L, -1)
}

#[cfg(feature = "lua52")]
#[inline(always)]
pub unsafe fn lua_pushlstring(L: *mut lua_State, s: *const c_char, l: usize) -> *const c_char {
    if l == 0 {
        lua_pushlstring_(L, cstr!(""), 0)
    } else {
        lua_pushlstring_(L, s, l)
    }
}

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
#[inline(always)]
pub unsafe fn lua_pushstring(L: *mut lua_State, s: *const c_char) -> *const c_char {
    lua_pushstring_(L, s);
    lua_tostring(L, -1)
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
    lua_gettable(L, idx);
    lua_type(L, -1)
}

#[inline(always)]
pub unsafe fn lua_rawget(L: *mut lua_State, idx: c_int) -> c_int {
    lua_rawget_(L, idx);
    lua_type(L, -1)
}

#[inline(always)]
pub unsafe fn lua_rawgeti(L: *mut lua_State, idx: c_int, n: lua_Integer) -> c_int {
    lua_rawgeti_(L, idx, n.try_into().expect("cannot convert to lua_Integer"));
    lua_type(L, -1)
}

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
#[inline(always)]
pub unsafe fn lua_rawgetp(L: *mut lua_State, idx: c_int, p: *const c_void) -> c_int {
    let abs_i = lua_absindex(L, idx);
    lua_pushlightuserdata(L, p as *mut c_void);
    lua_rawget(L, abs_i);
    lua_type(L, -1)
}

#[cfg(feature = "lua52")]
#[inline(always)]
pub unsafe fn lua_rawgetp(L: *mut lua_State, idx: c_int, p: *const c_void) -> c_int {
    lua_rawgetp_(L, idx, p);
    lua_type(L, -1)
}

#[cfg(any(feature = "lua51", feature = "luajit"))]
#[inline(always)]
pub unsafe fn lua_getuservalue(L: *mut lua_State, idx: c_int) -> c_int {
    lua_getfenv(L, idx);
    lua_type(L, -1)
}

#[cfg(feature = "luau")]
#[inline(always)]
pub unsafe fn lua_getuservalue(L: *mut lua_State, mut idx: c_int) -> c_int {
    luaL_checkstack(L, 2, cstr!("not enough stack slots available"));
    idx = lua_absindex(L, idx);
    lua_pushliteral(L, "__mlua_uservalues");
    lua_rawget(L, LUA_REGISTRYINDEX);
    if lua_istable(L, -1) == 0 {
        return LUA_TNIL;
    }
    lua_pushvalue(L, idx);
    lua_rawget(L, -2);
    lua_remove(L, -2);
    lua_type(L, -1)
}

#[cfg(feature = "lua52")]
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
    lua_rawseti_(
        L,
        idx,
        n.try_into().expect("cannot convert index from lua_Integer"),
    )
}

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
#[inline(always)]
pub unsafe fn lua_rawsetp(L: *mut lua_State, idx: c_int, p: *const c_void) {
    let abs_i = lua_absindex(L, idx);
    luaL_checkstack(L, 1, cstr!("not enough stack slots"));
    lua_pushlightuserdata(L, p as *mut c_void);
    lua_insert(L, -2);
    lua_rawset(L, abs_i);
}

#[cfg(any(feature = "lua51", feature = "luajit"))]
#[inline(always)]
pub unsafe fn lua_setuservalue(L: *mut lua_State, idx: c_int) {
    luaL_checktype(L, -1, LUA_TTABLE);
    lua_setfenv(L, idx);
}

#[cfg(feature = "luau")]
#[inline(always)]
pub unsafe fn lua_setuservalue(L: *mut lua_State, mut idx: c_int) {
    luaL_checkstack(L, 4, cstr!("not enough stack slots available"));
    idx = lua_absindex(L, idx);
    lua_pushliteral(L, "__mlua_uservalues");
    lua_pushvalue(L, -1);
    lua_rawget(L, LUA_REGISTRYINDEX);
    if lua_istable(L, -1) == 0 {
        lua_pop(L, 1);
        lua_createtable(L, 0, 2); // main table
        lua_createtable(L, 0, 1); // metatable
        lua_pushliteral(L, "k");
        lua_setfield(L, -2, cstr!("__mode"));
        lua_setmetatable(L, -2);
        lua_pushvalue(L, -2);
        lua_pushvalue(L, -2);
        lua_rawset(L, LUA_REGISTRYINDEX);
    }
    lua_replace(L, -2);
    lua_pushvalue(L, idx);
    lua_pushvalue(L, -3);
    lua_remove(L, -4);
    lua_rawset(L, -3);
    lua_pop(L, 1);
}

#[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
#[inline(always)]
pub unsafe fn lua_dump(
    L: *mut lua_State,
    writer: lua_Writer,
    data: *mut c_void,
    _strip: c_int,
) -> c_int {
    lua_dump_(L, writer, data)
}

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
#[inline(always)]
pub unsafe fn lua_len(L: *mut lua_State, idx: c_int) {
    match lua_type(L, idx) {
        LUA_TSTRING => {
            lua_pushnumber(L, lua_objlen(L, idx) as lua_Number);
        }
        LUA_TTABLE => {
            if luaL_callmeta(L, idx, cstr!("__len")) == 0 {
                lua_pushnumber(L, lua_objlen(L, idx) as lua_Number);
            }
        }
        LUA_TUSERDATA if luaL_callmeta(L, idx, cstr!("__len")) != 0 => {}
        _ => {
            luaL_error(
                L,
                cstr!("attempt to get length of a %s value"),
                lua_typename(L, lua_type(L, idx)),
            );
        }
    }
}

#[inline(always)]
pub unsafe fn lua_stringtonumber(L: *mut lua_State, s: *const c_char) -> usize {
    let cs = CStr::from_ptr(s);
    if let Ok(rs) = cs.to_str() {
        if let Ok(n) = lua_Number::from_str(rs.trim()) {
            lua_pushnumber(L, n);
            return cs.to_bytes_with_nul().len();
        }
    }
    0
}

pub const LUA_EXTRASPACE: usize = mem::size_of::<*const ()>();

#[allow(clippy::branches_sharing_code)]
pub unsafe fn lua_getextraspace(L: *mut lua_State) -> *mut c_void {
    luaL_checkstack(L, 4, cstr!("not enough stack slots available"));
    lua_pushliteral(L, "__compat53_extraspace");
    lua_pushvalue(L, -1);
    lua_rawget(L, LUA_REGISTRYINDEX);
    if lua_istable(L, -1) == 0 {
        lua_pop(L, 1);
        lua_createtable(L, 0, 2);
        lua_createtable(L, 0, 1);
        lua_pushliteral(L, "k");
        lua_setfield(L, -2, cstr!("__mode"));
        lua_setmetatable(L, -2);
        lua_pushvalue(L, -2);
        lua_pushvalue(L, -2);
        lua_rawset(L, LUA_REGISTRYINDEX);
    }
    lua_replace(L, -2);
    let is_main = lua_pushthread(L);
    lua_rawget(L, -2);
    let mut _ptr = lua_touserdata(L, -1);
    if _ptr.is_null() {
        lua_pop(L, 1);
        _ptr = lua_newuserdata(L, LUA_EXTRASPACE);
        if is_main != 0 {
            // mem::size_of::<c_void>() == 1
            ptr::write_bytes(_ptr, 0, LUA_EXTRASPACE);
            lua_pushthread(L);
            lua_pushvalue(L, -2);
            lua_rawset(L, -4);
            lua_pushboolean(L, 1);
            lua_pushvalue(L, -2);
            lua_rawset(L, -4);
        } else {
            lua_pushboolean(L, 1);
            lua_rawget(L, -3);
            let mptr = lua_touserdata(L, -1);
            if !mptr.is_null() {
                ptr::copy_nonoverlapping(mptr, _ptr, LUA_EXTRASPACE)
            } else {
                ptr::write_bytes(_ptr, 0, LUA_EXTRASPACE);
            }
            lua_pop(L, 1);
            lua_pushthread(L);
            lua_pushvalue(L, -2);
            lua_rawset(L, -4);
        }
    }
    lua_pop(L, 2);
    return _ptr;
}

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
#[inline(always)]
pub unsafe fn lua_pushglobaltable(L: *mut lua_State) {
    lua_pushvalue(L, LUA_GLOBALSINDEX);
}

//
// lauxlib ported functions
//

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
#[inline(always)]
pub unsafe fn luaL_checkstack(L: *mut lua_State, sz: c_int, msg: *const c_char) {
    if lua_checkstack(L, sz + LUA_MINSTACK) == 0 {
        if !msg.is_null() {
            luaL_error(L, cstr!("stack overflow (%s)"), msg);
        } else {
            lua_pushliteral(L, "stack overflow");
            lua_error(L);
        }
    }
}

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

#[cfg(any(feature = "lua51", feature = "luajit"))]
#[inline(always)]
pub unsafe fn luaL_loadbufferx(
    L: *mut lua_State,
    buff: *const c_char,
    sz: usize,
    name: *const c_char,
    mode: *const c_char,
) -> c_int {
    let status = if sz > 0 && *buff as u8 == LUA_SIGNATURE[0] {
        compat53_checkmode(L, mode, cstr!("binary"), LUA_ERRSYNTAX)
    } else {
        compat53_checkmode(L, mode, cstr!("text"), LUA_ERRSYNTAX)
    };
    if status != LUA_OK {
        return status;
    }
    luaL_loadbuffer(L, buff, sz, name)
}

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
#[inline(always)]
pub unsafe fn luaL_len(L: *mut lua_State, idx: c_int) -> lua_Integer {
    let mut isnum = 0;
    luaL_checkstack(L, 1, cstr!("not enough stack slots"));
    lua_len(L, idx);
    let res = lua_tointegerx(L, -1, &mut isnum);
    lua_pop(L, 1);
    if isnum == 0 {
        luaL_error(L, cstr!("object length is not an integer"));
    }
    res
}

#[cfg(any(feature = "lua51", feature = "luajit"))]
pub unsafe fn luaL_traceback(
    L: *mut lua_State,
    L1: *mut lua_State,
    msg: *const c_char,
    mut level: c_int,
) {
    let mut ar: lua_Debug = mem::zeroed();
    let top = lua_gettop(L);
    let numlevels = compat53_countlevels(L1);
    let mark = if numlevels > COMPAT53_LEVELS1 + COMPAT53_LEVELS2 {
        COMPAT53_LEVELS1
    } else {
        0
    };

    if !msg.is_null() {
        lua_pushfstring(L, cstr!("%s\n"), msg);
    }
    lua_pushliteral(L, "stack traceback:");
    while lua_getstack(L1, level, &mut ar) != 0 {
        level += 1;
        if level == mark {
            // too many levels?
            lua_pushliteral(L, "\n\t..."); // add a '...'
            level = numlevels - COMPAT53_LEVELS2; // and skip to last ones
        } else {
            lua_getinfo(L1, cstr!("Slnt"), &mut ar);
            lua_pushfstring(L, cstr!("\n\t%s:"), ar.short_src.as_ptr());
            if ar.currentline > 0 {
                lua_pushfstring(L, cstr!("%d:"), ar.currentline);
            }
            lua_pushliteral(L, " in ");
            compat53_pushfuncname(L, &mut ar);
            lua_concat(L, lua_gettop(L) - top);
        }
    }
    lua_concat(L, lua_gettop(L) - top);
}

#[cfg(feature = "luau")]
pub unsafe fn luaL_traceback(
    L: *mut lua_State,
    L1: *mut lua_State,
    msg: *const c_char,
    mut level: c_int,
) {
    let mut ar: lua_Debug = mem::zeroed();
    let top = lua_gettop(L);
    let numlevels = compat53_countlevels(L1);
    let mark = if numlevels > COMPAT53_LEVELS1 + COMPAT53_LEVELS2 {
        COMPAT53_LEVELS1
    } else {
        0
    };

    if !msg.is_null() {
        lua_pushfstring(L, cstr!("%s\n"), msg);
    }
    lua_pushliteral(L, "stack traceback:");
    while lua_getinfo(L1, level, cstr!(""), &mut ar) != 0 {
        level += 1;
        if level == mark {
            // too many levels?
            lua_pushliteral(L, "\n\t..."); // add a '...'
            level = numlevels - COMPAT53_LEVELS2; // and skip to last ones
        } else {
            lua_getinfo(L1, level - 1, cstr!("sln"), &mut ar);
            lua_pushfstring(L, cstr!("\n\t%s:"), ar.short_src.as_ptr());
            if ar.currentline > 0 {
                lua_pushfstring(L, cstr!("%d:"), ar.currentline);
            }
            lua_pushliteral(L, " in ");
            compat53_pushfuncname(L, level - 1, &mut ar);
            lua_concat(L, lua_gettop(L) - top);
        }
    }
    lua_concat(L, lua_gettop(L) - top);
}

pub unsafe fn luaL_tolstring(L: *mut lua_State, idx: c_int, len: *mut usize) -> *const c_char {
    if luaL_callmeta(L, idx, cstr!("__tostring")) == 0 {
        let t = lua_type(L, idx);
        match t {
            LUA_TNIL => {
                lua_pushliteral(L, "nil");
            }
            LUA_TSTRING | LUA_TNUMBER => {
                lua_pushvalue(L, idx);
            }
            LUA_TBOOLEAN => {
                if lua_toboolean(L, idx) == 0 {
                    lua_pushliteral(L, "false");
                } else {
                    lua_pushliteral(L, "true");
                }
            }
            _ => {
                let tt = luaL_getmetafield(L, idx, cstr!("__name"));
                let name = if tt == LUA_TSTRING {
                    lua_tostring(L, -1)
                } else {
                    lua_typename(L, t)
                };
                lua_pushfstring(L, cstr!("%s: %p"), name, lua_topointer(L, idx));
                if tt != LUA_TNIL {
                    lua_replace(L, -2);
                }
            }
        };
    } else if lua_isstring(L, -1) == 0 {
        luaL_error(L, cstr!("'__tostring' must return a string"));
    }
    lua_tolstring(L, -1, len)
}

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
#[inline(always)]
pub unsafe fn luaL_setmetatable(L: *mut lua_State, tname: *const c_char) {
    luaL_checkstack(L, 1, cstr!("not enough stack slots"));
    luaL_getmetatable(L, tname);
    lua_setmetatable(L, -2);
}

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
#[inline(always)]
pub unsafe fn luaL_testudata(L: *mut lua_State, i: c_int, tname: *const c_char) -> *mut c_void {
    let mut p = lua_touserdata(L, i);
    luaL_checkstack(L, 2, cstr!("not enough stack slots"));
    if p.is_null() || lua_getmetatable(L, i) == 0 {
        return ptr::null_mut();
    } else {
        luaL_getmetatable(L, tname);
        let res = lua_rawequal(L, -1, -2);
        lua_pop(L, 2);
        if res == 0 {
            p = ptr::null_mut();
        }
    }
    return p;
}

#[cfg(any(feature = "lua51", feature = "luajit"))]
#[inline(always)]
pub unsafe fn luaL_setfuncs(L: *mut lua_State, mut l: *const luaL_Reg, nup: c_int) {
    luaL_checkstack(L, nup + 1, cstr!("too many upvalues"));
    while !(*l).name.is_null() {
        // fill the table with given functions
        l = l.offset(1);
        lua_pushstring(L, (*l).name);
        for _ in 0..nup {
            // copy upvalues to the top
            lua_pushvalue(L, -(nup + 1));
        }
        lua_pushcclosure(L, (*l).func, nup); // closure with those upvalues
        lua_settable(L, -(nup + 3)); // table must be below the upvalues, the name and the closure
    }
    lua_pop(L, nup); // remove upvalues
}

#[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
pub unsafe fn luaL_getsubtable(L: *mut lua_State, idx: c_int, fname: *const c_char) -> c_int {
    let abs_i = lua_absindex(L, idx);
    luaL_checkstack(L, 3, cstr!("not enough stack slots"));
    lua_pushstring(L, fname);
    lua_gettable(L, abs_i);
    if lua_istable(L, -1) != 0 {
        return 1;
    }
    lua_pop(L, 1);
    lua_newtable(L);
    lua_pushstring(L, fname);
    lua_pushvalue(L, -2);
    lua_settable(L, abs_i);
    return 0;
}

pub unsafe fn luaL_requiref(
    L: *mut lua_State,
    modname: *const c_char,
    openf: lua_CFunction,
    glb: c_int,
) {
    luaL_checkstack(L, 3, cstr!("not enough stack slots available"));
    luaL_getsubtable(L, LUA_REGISTRYINDEX, cstr!("_LOADED"));
    if lua_getfield(L, -1, modname) == LUA_TNIL {
        lua_pop(L, 1);
        lua_pushcfunction(L, openf);
        lua_pushstring(L, modname);
        #[cfg(any(feature = "lua52", feature = "lua51", feature = "luau"))]
        {
            lua_call(L, 1, 1);
            lua_pushvalue(L, -1);
            lua_setfield(L, -3, modname);
        }
        #[cfg(feature = "luajit")]
        {
            lua_call(L, 1, 0);
            lua_getfield(L, -1, modname);
        }
    }
    if cfg!(any(feature = "lua52", feature = "lua51", feature = "luau")) && glb != 0 {
        lua_pushvalue(L, -1);
        lua_setglobal(L, modname);
    }
    if cfg!(feature = "luajit") && glb == 0 {
        lua_pushnil(L);
        lua_setglobal(L, modname);
    }
    lua_replace(L, -2);
}
