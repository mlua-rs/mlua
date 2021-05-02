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

#include "compat-5.3.h"

size_t MLUA_WRAPPED_ERROR_SIZE = 0;
size_t MLUA_WRAPPED_PANIC_SIZE = 0;

const void *MLUA_WRAPPED_ERROR_KEY = NULL;
const void *MLUA_WRAPPED_PANIC_KEY = NULL;

extern void wrapped_error_traceback(lua_State *L, int error_idx, void *error_ud,
                                    int has_traceback);

extern int mlua_hook_proc(lua_State *L, lua_Debug *ar);

#define max(a, b) (a > b ? a : b)

typedef struct {
  const char *data;
  size_t len;
} StringArg;

// A wrapper around Rust function to protect from triggering longjmp in Rust.
// Rust callback expected to return -1 in case of errors or number of output
// values.
static int lua_call_rust(lua_State *L) {
  int nargs = lua_gettop(L);

  // We need one extra stack space to store preallocated memory, and at least 2
  // stack spaces overall for handling error metatables in rust fn
  int extra_stack = 1;
  if (nargs < 2) {
    extra_stack = 2 - nargs;
  }

  luaL_checkstack(L, extra_stack,
                  "not enough stack space for callback error handling");

  // We cannot shadow rust errors with Lua ones, we pre-allocate enough memory
  // to store a wrapped error or panic *before* we proceed.
  lua_newuserdata(L, max(MLUA_WRAPPED_ERROR_SIZE, MLUA_WRAPPED_PANIC_SIZE));
  lua_rotate(L, 1, 1);

  lua_CFunction rust_callback = lua_touserdata(L, lua_upvalueindex(1));

  int ret = rust_callback(L);
  if (ret == -1) {
    lua_error(L);
  }

  return ret;
}

void lua_call_mlua_hook_proc(lua_State *L, lua_Debug *ar) {
  luaL_checkstack(L, 2, "not enough stack space for callback error handling");
  lua_newuserdata(L, max(MLUA_WRAPPED_ERROR_SIZE, MLUA_WRAPPED_PANIC_SIZE));
  lua_rotate(L, 1, 1);
  int ret = mlua_hook_proc(L, ar);
  if (ret == -1) {
    lua_error(L);
  }
}

static inline lua_Integer lua_popinteger(lua_State *L) {
  lua_Integer index = lua_tointeger(L, -1);
  lua_pop(L, 1);
  return index;
}

//
// Common functions
//

int lua_gc_s(lua_State *L) {
  int data = lua_popinteger(L);
  int what = lua_popinteger(L);
  int ret = lua_gc(L, what, data);
  lua_pushinteger(L, ret);
  return 1;
}

int luaL_ref_s(lua_State *L) {
  int ret = luaL_ref(L, -2);
  lua_pushinteger(L, ret);
  return 1;
}

int lua_pushlstring_s(lua_State *L) {
  StringArg *s = lua_touserdata(L, -1);
  lua_pop(L, 1);
  lua_pushlstring(L, s->data, s->len);
  return 1;
}

int lua_tolstring_s(lua_State *L) {
  void *len = lua_touserdata(L, -1);
  lua_pop(L, 1);
  const char *s = lua_tolstring(L, -1, len);
  lua_pushlightuserdata(L, (void *)s);
  return 2;
}

int lua_newthread_s(lua_State *L) {
  lua_newthread(L);
  return 1;
}

int lua_newuserdata_s(lua_State *L) {
  size_t size = lua_tointeger(L, -1);
  lua_pop(L, 1);
  lua_newuserdata(L, size);
  return 1;
}

int lua_newwrappederror_s(lua_State *L) {
  lua_newuserdata(L, MLUA_WRAPPED_ERROR_SIZE);
  return 1;
}

int lua_pushcclosure_s(lua_State *L) {
  int n = lua_gettop(L) - 1;
  lua_CFunction fn = lua_touserdata(L, -1);
  lua_pop(L, 1);
  lua_pushcclosure(L, fn, n);
  return 1;
}

int lua_pushrclosure_s(lua_State *L) {
  int n = lua_gettop(L);
  lua_pushcclosure(L, lua_call_rust, n);
  return 1;
}

int luaL_requiref_s(lua_State *L) {
  const char *modname = lua_touserdata(L, -3);
  lua_CFunction openf = lua_touserdata(L, -2);
  int glb = lua_tointeger(L, -1);
  lua_pop(L, 3);
  luaL_requiref(L, modname, openf, glb);
  return 1;
}

//
// Table functions
//

int lua_newtable_s(lua_State *L) {
  lua_createtable(L, 0, 0);
  return 1;
}

int lua_createtable_s(lua_State *L) {
  int nrec = lua_popinteger(L);
  int narr = lua_popinteger(L);
  lua_createtable(L, narr, nrec);
  return 1;
}

int lua_gettable_s(lua_State *L) {
  lua_gettable(L, -2);
  return 1;
}

int lua_settable_s(lua_State *L) {
  lua_settable(L, -3);
  return 0;
}

int lua_geti_s(lua_State *L) {
  lua_Integer index = lua_popinteger(L);
  lua_geti(L, -1, index);
  return 1;
}

int lua_rawset_s(lua_State *L) {
  lua_rawset(L, -3);
  return 0;
}

int lua_rawseti_s(lua_State *L) {
  lua_Integer index = lua_popinteger(L);
  lua_rawseti(L, -2, index);
  return 0;
}

int lua_rawsetp_s(lua_State *L) {
  void *p = lua_touserdata(L, -1);
  lua_pop(L, 1);
  lua_rawsetp(L, -2, p);
  return 0;
}

int lua_rawsetfield_s(lua_State *L) {
  StringArg *s = lua_touserdata(L, -2);
  lua_pushlstring(L, s->data, s->len);
  lua_replace(L, -3);
  lua_rawset(L, -3);
  return 0;
}

int lua_rawinsert_s(lua_State *L) {
  lua_Integer index = lua_popinteger(L);
  lua_Integer size = lua_rawlen(L, -2);

  for (lua_Integer i = size; i >= index; i--) {
    // table[i+1] = table[i]
    lua_rawgeti(L, -2, i);
    lua_rawseti(L, -3, i + 1);
  }
  lua_rawseti(L, -2, index);

  return 0;
}

int lua_rawremove_s(lua_State *L) {
  lua_Integer index = lua_popinteger(L);
  lua_Integer size = lua_rawlen(L, -1);

  for (lua_Integer i = index; i < size; i++) {
    lua_rawgeti(L, -1, i + 1);
    lua_rawseti(L, -2, i);
  }
  lua_pushnil(L);
  lua_rawseti(L, -2, size);

  return 0;
}

int luaL_len_s(lua_State *L) {
  lua_pushinteger(L, luaL_len(L, -1));
  return 1;
}

int lua_next_s(lua_State *L) {
  int ret = lua_next(L, -2);
  lua_pushinteger(L, ret);
  return ret == 0 ? 1 : 3;
}

//
// Moved from Rust to C
//

// Wrapper to lookup in `field_getters` first, then `methods`, ending
// original `__index`. Used only if `field_getters` or `methods` set.
int meta_index_impl(lua_State *state) {
  // stack: self, key
  luaL_checkstack(state, 2, NULL);

  // lookup in `field_getters` table
  if (lua_isnil(state, lua_upvalueindex(2)) == 0) {
    lua_pushvalue(state, -1); // `key` arg
    if (lua_rawget(state, lua_upvalueindex(2)) != LUA_TNIL) {
      lua_insert(state, -3); // move function
      lua_pop(state, 1);     // remove `key`
      lua_call(state, 1, 1);
      return 1;
    }
    lua_pop(state, 1); // pop the nil value
  }
  // lookup in `methods` table
  if (lua_isnil(state, lua_upvalueindex(3)) == 0) {
    lua_pushvalue(state, -1); // `key` arg
    if (lua_rawget(state, lua_upvalueindex(3)) != LUA_TNIL) {
      lua_insert(state, -3);
      lua_pop(state, 2);
      return 1;
    }
    lua_pop(state, 1); // pop the nil value
  }

  // lookup in `__index`
  lua_pushvalue(state, lua_upvalueindex(1));
  switch (lua_type(state, -1)) {
  case LUA_TNIL:
    lua_pop(state, 1); // pop the nil value
    const char *field = lua_tostring(state, -1);
    luaL_error(state, "attempt to get an unknown field '%s'", field);
    break;

  case LUA_TTABLE:
    lua_insert(state, -2);
    lua_gettable(state, -2);
    break;

  case LUA_TFUNCTION:
    lua_insert(state, -3);
    lua_call(state, 2, 1);
    break;
  }

  return 1;
}

// Similar to `meta_index_impl`, checks `field_setters` table first, then
// `__newindex` metamethod. Used only if `field_setters` set.
int meta_newindex_impl(lua_State *state) {
  // stack: self, key, value
  luaL_checkstack(state, 2, NULL);

  // lookup in `field_setters` table
  lua_pushvalue(state, -2); // `key` arg
  if (lua_rawget(state, lua_upvalueindex(2)) != LUA_TNIL) {
    lua_remove(state, -3); // remove `key`
    lua_insert(state, -3); // move function
    lua_call(state, 2, 0);
    return 0;
  }
  lua_pop(state, 1); // pop the nil value

  // lookup in `__newindex`
  lua_pushvalue(state, lua_upvalueindex(1));
  switch (lua_type(state, -1)) {
  case LUA_TNIL:
    lua_pop(state, 1); // pop the nil value
    const char *field = lua_tostring(state, -2);
    luaL_error(state, "attempt to set an unknown field '%s'", field);
    break;

  case LUA_TTABLE:
    lua_insert(state, -3);
    lua_settable(state, -3);
    break;

  case LUA_TFUNCTION:
    lua_insert(state, -4);
    lua_call(state, 3, 0);
    break;
  }

  return 0;
}

// See Function::bind
int bind_call_impl(lua_State *state) {
  int nargs = lua_gettop(state);
  int nbinds = lua_tointeger(state, lua_upvalueindex(2));
  luaL_checkstack(state, nbinds + 2, NULL);

  lua_settop(state, nargs + nbinds + 1);
  lua_rotate(state, -(nargs + nbinds + 1), nbinds + 1);

  lua_pushvalue(state, lua_upvalueindex(1));
  lua_replace(state, 1);

  for (int i = 0; i < nbinds; i++) {
    lua_pushvalue(state, lua_upvalueindex(i + 3));
    lua_replace(state, i + 2);
  }

  lua_call(state, nargs + nbinds, LUA_MULTRET);
  return lua_gettop(state);
}

// Returns 1 if a value at index `index` is a special wrapped struct identified
// by `key`
int is_wrapped_struct(lua_State *state, int index, const void *key) {
  if (key == NULL) {
    // Not yet initialized?
    return 0;
  }

  void *ud = lua_touserdata(state, index);
  if (ud == NULL || lua_getmetatable(state, index) == 0) {
    return 0;
  }
  lua_rawgetp(state, LUA_REGISTRYINDEX, key);
  int res = lua_rawequal(state, -1, -2);
  lua_pop(state, 2);
  return res;
}

// Takes an error at the top of the stack, and if it is a WrappedError, converts
// it to an Error::CallbackError with a traceback, if it is some lua type,
// prints the error along with a traceback, and if it is a WrappedPanic, does
// not modify it. This function does its best to avoid triggering another error
// and shadowing previous rust errors, but it may trigger Lua errors that shadow
// rust errors under certain memory conditions. This function ensures that such
// behavior will *never* occur with a rust panic, however.
int error_traceback(lua_State *state) {
  // I believe luaL_traceback < 5.4 requires this much free stack to not error.
  // 5.4 uses luaL_Buffer
  const int LUA_TRACEBACK_STACK = 11;

  if (lua_checkstack(state, 2) == 0) {
    // If we don't have enough stack space to even check the error type, do
    // nothing so we don't risk shadowing a rust panic.
    return 1;
  }

  if (is_wrapped_struct(state, -1, MLUA_WRAPPED_ERROR_KEY) != 0) {
    int error_idx = lua_absindex(state, -1);
    // lua_newuserdata and luaL_traceback may error
    void *error_ud = lua_newuserdata(state, MLUA_WRAPPED_ERROR_SIZE);
    int has_traceback = 0;
    if (lua_checkstack(state, LUA_TRACEBACK_STACK) != 0) {
      luaL_traceback(state, state, NULL, 0);
      has_traceback = 1;
    }
    wrapped_error_traceback(state, error_idx, error_ud, has_traceback);
    return 1;
  }

  if (MLUA_WRAPPED_PANIC_KEY != NULL &&
      !is_wrapped_struct(state, -1, MLUA_WRAPPED_PANIC_KEY) &&
      lua_checkstack(state, LUA_TRACEBACK_STACK) != 0) {
    const char *s = luaL_tolstring(state, -1, NULL);
    luaL_traceback(state, state, s, 0);
    lua_remove(state, -2);
  }

  return 1;
}

int error_traceback_s(lua_State *L) {
  lua_State *L1 = lua_touserdata(L, -1);
  lua_pop(L, 1);
  return error_traceback(L1);
}
