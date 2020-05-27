// The MIT License (MIT)
//
// Copyright (c) 2020 A. Orlenko
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

use std::os::raw::*;

use crate::ffi::{lua_Alloc, lua_CFunction, lua_Hook, lua_Number, lua_State};

#[repr(C)]
struct lua_StateExt {
    next: *mut c_void,
    tt: u8,
    marked: u8,
    status: u8,
    top: *mut c_void,
    base: *mut c_void,
    l_G: *mut global_State,
    ci: *mut c_void,
    savedpc: *const c_void,
    stack_last: *mut c_void,
    stack: *mut c_void,
    end_ci: *mut c_void,
    base_ci: *mut c_void,
    stacksize: c_int,
    size_ci: c_int,
    nCcalls: c_ushort,
    baseCcalls: c_ushort,
    hookmask: u8,
    allowhook: u8,
    basehookcount: c_int,
    hookcount: c_int,
    hook: Option<lua_Hook>,
    l_gt: TValue,
    env: TValue,
    openupval: *mut c_void,
    gclist: *mut c_void,
    errorJmp: *mut c_void,
    errfunc: isize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TValue {
    value: Value,
    tt: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
union Value {
    gc: *mut c_void,
    p: *mut c_void,
    n: lua_Number,
    b: c_int,
}

#[repr(C)]
struct global_State {
    strt: stringtable,
    frealloc: Option<lua_Alloc>,
    ud: *mut c_void,
    currentwhite: u8,
    gcstate: u8,
    sweepstrgc: c_int,
    rootgc: *mut c_void,
    sweepgc: *mut c_void,
    gray: *mut c_void,
    grayagain: *mut c_void,
    weak: *mut c_void,
    tmudata: *mut c_void,
    buff: Mbuffer,
    GCthreshold: usize,
    totalbytes: usize,
    estimate: usize,
    gcdept: usize,
    gcpause: c_int,
    gcstepmul: c_int,
    panic: Option<lua_CFunction>,
    l_registry: TValue,
    mainthread: *mut lua_State,
    // Other fields ommited
}

#[repr(C)]
struct stringtable {
    hash: *mut c_void,
    nuse: c_uint,
    size: c_int,
}

#[repr(C)]
struct Mbuffer {
    buffer: *mut c_char,
    n: usize,
    buffsize: usize,
}

pub unsafe fn lua_getmainstate(state: *mut lua_State) -> *mut lua_State {
    let state = state as *mut lua_StateExt;
    let global = (*state).l_G;
    (*global).mainthread
}
