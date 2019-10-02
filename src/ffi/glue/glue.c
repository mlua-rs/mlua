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

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <lauxlib.h>
#include <lua.h>
#include <lualib.h>

// Macros taken from https://gcc.gnu.org/onlinedocs/cpp/Stringification.html
#define xstr(s) str(s)
#define str(s) #s

typedef struct rs_item {
  int type;
  const char *name;
  union {
    int int_val;
    const char *str_val;
    LUA_INTEGER lua_int_val;
  };
} rs_item;

#define TY_INT 0
#define TY_LUAINT 1
#define TY_STR 2
#define TY_TYPE 3
#define TY_COMMENT 4
#define TY_RAW 5

#define RS_INT(name, val)                                                      \
  { TY_INT, name, .int_val = val }
#define RS_LUAINT(name, val)                                                   \
  { TY_LUAINT, name, .lua_int_val = val }
#define RS_STR(name, val)                                                      \
  { TY_STR, name, .str_val = val }
#define RS_TYPE(name, val)                                                     \
  { TY_TYPE, name, .str_val = val }
#define RS_COMMENT(val)                                                        \
  { TY_COMMENT, NULL, .str_val = val }
#define RS_RAW(val)                                                            \
  { TY_RAW, NULL, .str_val = val }

const char *rs_int_type(int width) {
  switch (width) {
  default:
  case 2:
    return "i16";
  case 4:
    return "i32";
  case 8:
    return "i64";
  }
}

const char *rs_uint_type(int width) {
  switch (width) {
  default:
  case 2:
    return "u16";
  case 4:
    return "u32";
  case 8:
    return "u64";
  }
}

int try_write(char **str, char c, size_t n, size_t *written, size_t szstr) {
  if (szstr - *written < n) {
    return 0;
  }
  for (; n; n--, *written++)
    *(*str)++ = c;
  return 1;
}

// converts \ in a string to \\ so that it can be used as a rust string literal
// ensures that `out` will always have a null terminating character
size_t escape(const char *in, char *out, size_t szout) {
  size_t written = 0;
  char cur;

  while (cur = *in++) {
    switch (cur) {
    case '\\':
      if (!try_write(&out, cur, 2, &written, szout))
        goto finalize;
      break;
    default:
      if (!try_write(&out, cur, 1, &written, szout))
        goto finalize;
      break;
    }
  }

finalize:
  if (written + 1 <= szout) {
    *out++ = '\0';
    written++;
  }
  return written;
}

int write_int_item(FILE *f, const char *name, int value) {
  return fprintf(f, "pub const %s: c_int = %d;\n", name, value);
}

int write_lua_int_item(FILE *f, const char *name, LUA_INTEGER value) {
  return fprintf(f, "pub const %s: LUA_INTEGER = " LUA_INTEGER_FMT ";\n", name,
                 value);
}

int write_str_item(FILE *f, const char *name, const char *value) {
  size_t len = strlen(value);
  size_t bufsz = len * 2 + 1;
  char *buf = malloc(bufsz);
  int ret;
  escape(value, buf, bufsz);
  ret = fprintf(f, "pub const %s: &'static str = \"%s\";\n", name, buf);
  free(buf);
  return ret;
}

int write_type(FILE *f, const char *name, const char *value) {
  return fprintf(f, "pub type %s = %s;\n", name, value);
}

int write_comment(FILE *f, const char *value) {
  return fprintf(f, "/* %s */\n", value);
}

int write_raw(FILE *f, const char *value) { return fputs(value, f) >= 0; }

int write_item(FILE *f, const rs_item *c) {
  switch (c->type) {
  case TY_INT:
    return write_int_item(f, c->name, c->int_val);
  case TY_LUAINT:
    return write_lua_int_item(f, c->name, c->lua_int_val);
  case TY_STR:
    return write_str_item(f, c->name, c->str_val);
  case TY_TYPE:
    return write_type(f, c->name, c->str_val);
  case TY_COMMENT:
    return write_comment(f, c->str_val);
  case TY_RAW:
    return write_raw(f, c->str_val);
  default:
    return 0;
  }
}

int write_items_(FILE *f, const rs_item items[], size_t num) {
  size_t i;
  for (i = 0; i < num; i++) {
    if (!write_item(f, &items[i]))
      return 0;
  }
  return 1;
}

#define write_items(f, cs) write_items_(f, cs, sizeof(cs) / sizeof(cs[0]))

int main(int argc, const char **argv) {
  if (argc <= 1) {
    printf("usage: %s <filename>\n", argv[0]);
    return EXIT_FAILURE;
  }

  const char *filename = argv[1];

  FILE *f = fopen(filename, "w");

  if (!f) {
    printf("could not open file: errno = %d\n", errno);
    return EXIT_FAILURE;
  }

  const rs_item glue_entries[] = {
      RS_COMMENT("this file was generated by glue.c; do not modify it by hand"),
      RS_RAW("use std::os::raw::*;\n"),

      // == luaconf.h ==========================================================

      RS_COMMENT("luaconf.h"),
      RS_STR("LUA_VDIR", LUA_VDIR),
      RS_STR("LUA_PATH_DEFAULT", LUA_PATH_DEFAULT),
      RS_STR("LUA_CPATH_DEFAULT", LUA_CPATH_DEFAULT),
      RS_STR("LUA_DIRSEP", LUA_DIRSEP),
      RS_INT("LUA_EXTRASPACE", LUA_EXTRASPACE),
      RS_INT("LUA_IDSIZE", LUA_IDSIZE),
      // RS_INT("LUAI_MAXSHORTLEN", LUAI_MAXSHORTLEN),
      // RS_TYPE("LUA_KCONTEXT", xstr(LUA_KCONTEXT)),
      RS_INT("LUAI_BITSINT", LUAI_BITSINT),
      // LUA_INT32? LUAI_UMEM? LUAI_MEM?
      RS_INT("LUAI_MAXSTACK", LUAI_MAXSTACK),
      RS_INT("LUAL_BUFFERSIZE", LUAL_BUFFERSIZE),
      RS_TYPE("LUA_NUMBER",
              sizeof(LUA_NUMBER) > sizeof(float) ? "c_double" : "c_float"),
      RS_TYPE("LUA_UNSIGNED", rs_uint_type(sizeof(LUA_UNSIGNED))),
      RS_TYPE("LUA_INTEGER", rs_int_type(sizeof(LUA_INTEGER))),
      RS_LUAINT("LUA_MAXINTEGER", LUA_MAXINTEGER),
      RS_LUAINT("LUA_MININTEGER", LUA_MININTEGER),

      // == lua.h ==============================================================

      RS_COMMENT("lua.h"),
      RS_STR("LUA_VERSION_MAJOR", LUA_VERSION_MAJOR),
      RS_STR("LUA_VERSION_MINOR", LUA_VERSION_MINOR),
      RS_INT("LUA_VERSION_NUM", LUA_VERSION_NUM),
      RS_STR("LUA_VERSION_RELEASE", LUA_VERSION_RELEASE),
      RS_STR("LUA_VERSION", LUA_VERSION),
      RS_STR("LUA_RELEASE", LUA_RELEASE),
      RS_STR("LUA_COPYRIGHT", LUA_COPYRIGHT),
      RS_STR("LUA_AUTHORS", LUA_AUTHORS),
      RS_INT("LUA_REGISTRYINDEX", LUA_REGISTRYINDEX),

      // == lauxlib.h ==========================================================

      RS_COMMENT("lauxlib.h"),
      RS_INT("LUAL_NUMSIZES", LUAL_NUMSIZES),
      RS_STR("LUA_FILEHANDLE", LUA_FILEHANDLE),

      // == lualib.h ===========================================================

      RS_COMMENT("lualib.h"),
      RS_STR("LUA_COLIBNAME", LUA_COLIBNAME),
      RS_STR("LUA_TABLIBNAME", LUA_TABLIBNAME),
      RS_STR("LUA_IOLIBNAME", LUA_IOLIBNAME),
      RS_STR("LUA_OSLIBNAME", LUA_OSLIBNAME),
      RS_STR("LUA_STRLIBNAME", LUA_STRLIBNAME),
      RS_STR("LUA_UTF8LIBNAME", LUA_UTF8LIBNAME),
      RS_STR("LUA_BITLIBNAME", LUA_BITLIBNAME),
      RS_STR("LUA_MATHLIBNAME", LUA_MATHLIBNAME),
      RS_STR("LUA_DBLIBNAME", LUA_DBLIBNAME),
      RS_STR("LUA_LOADLIBNAME", LUA_LOADLIBNAME),
  };

  if (!write_items(f, glue_entries)) {
    printf("%s: error generating %s; aborting\n", argv[0], filename);
    return EXIT_FAILURE;
  }

  fclose(f);

  return EXIT_SUCCESS;
}
