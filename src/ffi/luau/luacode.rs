//! Contains definitions from `luacode.h`.

use std::os::raw::{c_char, c_int, c_void};
use std::slice;

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
    #[link_name = "luau_compile"]
    pub fn luau_compile_(
        source: *const c_char,
        size: usize,
        options: *mut lua_CompileOptions,
        outsize: *mut usize,
    ) -> *mut c_char;

    fn free(p: *mut c_void);
}

pub unsafe fn luau_compile(source: &[u8], mut options: lua_CompileOptions) -> Vec<u8> {
    let mut outsize = 0;
    let data_ptr = luau_compile_(
        source.as_ptr() as *const c_char,
        source.len(),
        &mut options,
        &mut outsize,
    );
    let data = slice::from_raw_parts(data_ptr as *mut u8, outsize).to_vec();
    free(data_ptr as *mut c_void);
    data
}
