// The MIT License (MIT)
//
// Copyright (c) 2019-2021 A. Orlenko
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

#include <lauxlib.h>
#include <lua.h>
#include <lualib.h>

void *LUA_ALL_SYMBOLS[] = {
    /*
     * lua.h
     */
    lua_newstate,
    lua_close,
    lua_newthread,
    lua_atpanic,
#if LUA_VERSION_NUM > 501
    lua_version,
#endif
#if LUA_VERSION_NUM > 503
    lua_resetthread,
#endif
#ifdef HAVE_LUA_RESETTHREAD
    lua_resetthread,
#endif

#if LUA_VERSION_NUM > 501
    lua_absindex,
#endif
    lua_gettop,
    lua_settop,
    lua_pushvalue,
#if LUA_VERSION_NUM > 502
    lua_rotate,
#else
    lua_remove,
    lua_insert,
    lua_replace,
#endif
#if LUA_VERSION_NUM > 501
    lua_copy,
#endif
    lua_checkstack,
    lua_xmove,

    lua_isnumber,
    lua_isstring,
    lua_iscfunction,
    lua_isuserdata,
    lua_type,
    lua_typename,

#if LUA_VERSION_NUM == 501
    lua_tonumber,
    lua_tointeger,
#else
    lua_tonumberx,
    lua_tointegerx,
#endif
#if LUA_VERSION_NUM == 502
    lua_tounsignedx,
#endif
    lua_toboolean,
    lua_tolstring,
#if LUA_VERSION_NUM == 501
    lua_objlen,
#else
    lua_rawlen,
#endif
    lua_tocfunction,
    lua_touserdata,
    lua_tothread,
    lua_topointer,

#if LUA_VERSION_NUM == 501
    lua_equal,
    lua_lessthan,
#else
    lua_arith,
    lua_compare,
#endif
    lua_rawequal,

    lua_pushnil,
    lua_pushnumber,
    lua_pushinteger,
#if LUA_VERSION_NUM == 502
    lua_pushunsigned,
#endif
    lua_pushlstring,
    lua_pushstring,
    lua_pushvfstring,
    lua_pushfstring,
    lua_pushcclosure,
    lua_pushboolean,
    lua_pushlightuserdata,
    lua_pushthread,

#if LUA_VERSION_NUM > 501
    lua_getglobal,
    lua_rawgetp,
#endif
#if LUA_VERSION_NUM > 503
    lua_getiuservalue,
#elif LUA_VERSION_NUM > 501
    lua_getuservalue,
#endif
    lua_gettable,
    lua_getfield,
#if LUA_VERSION_NUM > 502
    lua_geti,
#endif
    lua_rawget,
    lua_rawgeti,
    lua_createtable,
#if LUA_VERSION_NUM < 504
    lua_newuserdata,
#else
    lua_newuserdatauv,
#endif
#if LUA_VERSION_NUM == 501
    lua_getfenv,
#endif
    lua_getmetatable,

#if LUA_VERSION_NUM > 501
    lua_setglobal,
    lua_rawsetp,
#endif
#if LUA_VERSION_NUM > 503
    lua_setiuservalue,
#elif LUA_VERSION_NUM > 501
    lua_setuservalue,
#endif
    lua_settable,
    lua_setfield,
#if LUA_VERSION_NUM > 502
    lua_seti,
#endif
    lua_rawset,
    lua_rawseti,
#if LUA_VERSION_NUM == 501
    lua_setfenv,
#endif
    lua_setmetatable,

#if LUA_VERSION_NUM > 501
    lua_callk,
    lua_pcallk,
#else
    lua_call,
    lua_pcall,
    lua_cpcall,
#endif
#if LUA_VERSION_NUM == 502
    lua_getctx,
#endif
    lua_load,
    lua_dump,

#if LUA_VERSION_NUM > 501
    lua_yieldk,
#else
    lua_yield,
#endif
    lua_resume,
    lua_status,
#if LUA_VERSION_NUM > 502
    lua_isyieldable,
#endif

    lua_gc,

    lua_error,
    lua_next,
    lua_concat,
#if LUA_VERSION_NUM > 501
    lua_len,
#endif
#if LUA_VERSION_NUM > 502
    lua_stringtonumber,
#endif
    lua_getallocf,
    lua_setallocf,

    lua_getstack,
    lua_getinfo,
    lua_getlocal,
    lua_setlocal,
    lua_getupvalue,
    lua_setupvalue,
#if LUA_VERSION_NUM > 501
    lua_upvalueid,
    lua_upvaluejoin,
#endif
    lua_sethook,
    lua_gethook,
    lua_gethookmask,
    lua_gethookcount,

/*
 * lauxlib.h
 */
#if LUA_VERSION_NUM > 501
    luaL_checkversion_,
    luaL_tolstring,
#else
    luaL_register,
    luaL_typerror,
#endif
#if LUA_VERSION_NUM == 502
    luaL_checkunsigned,
    luaL_optunsigned,
#endif
    luaL_getmetafield,
    luaL_callmeta,
    luaL_argerror,
    luaL_checklstring,
    luaL_optlstring,
    luaL_checknumber,
    luaL_optnumber,
    luaL_checkinteger,
    luaL_optinteger,
    luaL_checkstack,
    luaL_checktype,
    luaL_checkany,

    luaL_newmetatable,
#if LUA_VERSION_NUM > 501
    luaL_setmetatable,
    luaL_testudata,
#endif
    luaL_checkudata,
    luaL_where,
    luaL_error,
    luaL_checkoption,
#if LUA_VERSION_NUM > 501
    luaL_fileresult,
    luaL_execresult,
    luaL_loadfilex,
    luaL_loadbufferx,
    luaL_len,
    luaL_setfuncs,
    luaL_getsubtable,
    luaL_traceback,
    luaL_requiref,
#else
    luaL_loadfile,
    luaL_loadbuffer,
    luaL_findtable,
#endif
    luaL_ref,
    luaL_unref,
    luaL_loadstring,
    luaL_newstate,
    luaL_gsub,

    luaL_buffinit,
#if LUA_VERSION_NUM > 501
    luaL_prepbuffsize,
    luaL_pushresultsize,
    luaL_buffinitsize,
#else
    luaL_prepbuffer,
#endif
    luaL_addlstring,
    luaL_addstring,
    luaL_addvalue,
    luaL_pushresult,

    /*
     * lualib.h
     */
    luaopen_base,
#if LUA_VERSION_NUM > 501
    luaopen_coroutine,
#endif
    luaopen_table,
    luaopen_io,
    luaopen_os,
    luaopen_string,
#if LUA_VERSION_NUM > 502
    luaopen_utf8,
#endif
#if LUA_VERSION_NUM == 502
    luaopen_bit32,
#endif
    luaopen_math,
    luaopen_debug,
    luaopen_package,
    luaL_openlibs,
};
