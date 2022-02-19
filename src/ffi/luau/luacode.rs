//! Contains definitions from `luacode.h`.

use std::os::raw::{c_char, c_int};

#[repr(C)]
pub struct lua_CompileOptions {
    pub optimizationLevel: c_int,
    pub debugLevel: c_int,
    pub coverageLevel: c_int,
    pub vectorLib: *const c_char,
    pub vectorCtor: *const c_char,
    pub mutableGlobals: *mut *const c_char,
}

extern "C" {
    pub fn luau_compile(
        source: *const c_char,
        size: usize,
        options: *mut lua_CompileOptions,
        outsize: *mut usize,
    ) -> *mut c_char;
}
