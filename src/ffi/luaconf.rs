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

//! Contains definitions from `luaconf.h`.

pub use super::glue::LUAL_BUFFERSIZE;
pub use super::glue::LUA_NUMBER;
pub use super::glue::LUA_INTEGER;
pub use super::glue::LUA_UNSIGNED;

pub use super::glue::{LUA_IDSIZE};
pub use super::glue::{LUA_MININTEGER, LUA_MAXINTEGER};

pub use super::glue::LUAI_MAXSTACK;
pub use super::glue::LUAL_NUMSIZES;

pub use super::glue::LUA_KCONTEXT;

use libc::c_int;

#[inline(always)]
pub unsafe fn lua_numtointeger(n: LUA_NUMBER, p: *mut LUA_INTEGER) -> c_int {
  if n >= (LUA_MININTEGER as LUA_NUMBER) && n < -(LUA_MININTEGER as LUA_NUMBER) {
    *p = n as LUA_INTEGER;
    1
  } else {
    0
  }
}

