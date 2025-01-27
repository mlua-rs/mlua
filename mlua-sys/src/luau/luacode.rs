//! Contains definitions from `luacode.h`.

use std::marker::{PhantomData, PhantomPinned};
use std::os::raw::{c_char, c_int, c_void};
use std::{ptr, slice};

#[repr(C)]
#[non_exhaustive]
pub struct lua_CompileOptions {
    pub optimizationLevel: c_int,
    pub debugLevel: c_int,
    pub typeInfoLevel: c_int,
    pub coverageLevel: c_int,
    pub vectorLib: *const c_char,
    pub vectorCtor: *const c_char,
    pub vectorType: *const c_char,
    pub mutableGlobals: *const *const c_char,
    pub userdataTypes: *const *const c_char,
    pub librariesWithKnownMembers: *const *const c_char,
    pub libraryMemberTypeCallback: Option<lua_LibraryMemberTypeCallback>,
    pub libraryMemberConstantCallback: Option<lua_LibraryMemberConstantCallback>,
    pub disabledBuiltins: *const *const c_char,
}

impl Default for lua_CompileOptions {
    fn default() -> Self {
        Self {
            optimizationLevel: 1,
            debugLevel: 1,
            typeInfoLevel: 0,
            coverageLevel: 0,
            vectorLib: ptr::null(),
            vectorCtor: ptr::null(),
            vectorType: ptr::null(),
            mutableGlobals: ptr::null(),
            userdataTypes: ptr::null(),
            librariesWithKnownMembers: ptr::null(),
            libraryMemberTypeCallback: None,
            libraryMemberConstantCallback: None,
            disabledBuiltins: ptr::null(),
        }
    }
}

#[repr(C)]
pub struct lua_CompileConstant {
    _data: [u8; 0],
    _marker: PhantomData<(*mut u8, PhantomPinned)>,
}

pub type lua_LibraryMemberTypeCallback =
    extern "C" fn(library: *const c_char, member: *const c_char) -> c_int;

pub type lua_LibraryMemberConstantCallback =
    extern "C" fn(library: *const c_char, member: *const c_char, constant: *mut lua_CompileConstant);

extern "C" {
    fn luau_set_compile_constant_nil(constant: *mut lua_CompileConstant);
    fn luau_set_compile_constant_boolean(constant: *mut lua_CompileConstant, b: c_int);
    fn luau_set_compile_constant_number(constant: *mut lua_CompileConstant, n: f64);
    fn luau_set_compile_constant_vector(constant: *mut lua_CompileConstant, x: f32, y: f32, z: f32, w: f32);
    fn luau_set_compile_constant_string(constant: *mut lua_CompileConstant, s: *const c_char, l: usize);
}

extern "C-unwind" {
    #[link_name = "luau_compile"]
    pub fn luau_compile_(
        source: *const c_char,
        size: usize,
        options: *mut lua_CompileOptions,
        outsize: *mut usize,
    ) -> *mut c_char;
}

extern "C" {
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
    assert!(!data_ptr.is_null(), "luau_compile failed");
    let data = slice::from_raw_parts(data_ptr as *mut u8, outsize).to_vec();
    free(data_ptr as *mut c_void);
    data
}
