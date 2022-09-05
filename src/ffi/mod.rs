//! Low level bindings to Lua 5.4/5.3/5.2/5.1 including LuaJIT.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use std::os::raw::c_int;

#[cfg(feature = "lua54")]
pub use lua54::*;

#[cfg(feature = "lua53")]
pub use lua53::*;

#[cfg(feature = "lua52")]
pub use lua52::*;

#[cfg(any(feature = "lua51", feature = "luajit"))]
pub use lua51::*;

#[cfg(feature = "luau")]
pub use luau::*;

#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
pub const LUA_MAX_UPVALUES: c_int = 255;

#[cfg(any(feature = "lua51", all(feature = "luajit", not(feature = "vendored"))))]
pub const LUA_MAX_UPVALUES: c_int = 60;

#[cfg(all(feature = "luajit", feature = "vendored"))]
pub const LUA_MAX_UPVALUES: c_int = 120;

#[cfg(feature = "luau")]
pub const LUA_MAX_UPVALUES: c_int = 200;

// I believe `luaL_traceback` < 5.4 requires this much free stack to not error.
// 5.4 uses `luaL_Buffer`
pub const LUA_TRACEBACK_STACK: c_int = 11;

// The minimum alignment guaranteed by the architecture. This value is used to
// add fast paths for low alignment values.
// Copied from https://github.com/rust-lang/rust/blob/master/library/std/src/sys/common/alloc.rs
#[cfg(all(any(
    target_arch = "x86",
    target_arch = "arm",
    target_arch = "mips",
    target_arch = "powerpc",
    target_arch = "powerpc64",
    target_arch = "sparc",
    target_arch = "asmjs",
    target_arch = "wasm32",
    target_arch = "hexagon",
    all(target_arch = "riscv32", not(target_os = "espidf")),
    all(target_arch = "xtensa", not(target_os = "espidf")),
)))]
pub const SYS_MIN_ALIGN: usize = 8;
#[cfg(all(any(
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "mips64",
    target_arch = "s390x",
    target_arch = "sparc64",
    target_arch = "riscv64",
    target_arch = "wasm64",
)))]
pub const SYS_MIN_ALIGN: usize = 16;
// The allocator on the esp-idf platform guarentees 4 byte alignment.
#[cfg(all(any(
    all(target_arch = "riscv32", target_os = "espidf"),
    all(target_arch = "xtensa", target_os = "espidf"),
)))]
pub const SYS_MIN_ALIGN: usize = 4;

// Hack to avoid stripping a few unused Lua symbols that could be imported
// by C modules in unsafe mode
#[cfg(not(feature = "luau"))]
pub(crate) fn keep_lua_symbols() {
    let mut symbols: Vec<*const extern "C" fn()> = Vec::new();
    symbols.push(lua_atpanic as _);
    symbols.push(lua_isuserdata as _);
    symbols.push(lua_tocfunction as _);
    symbols.push(luaL_loadstring as _);
    symbols.push(luaL_openlibs as _);
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    {
        symbols.push(lua_getglobal as _);
        symbols.push(lua_setglobal as _);
        symbols.push(luaL_setfuncs as _);
    }
}

#[cfg(feature = "lua54")]
pub mod lua54;

#[cfg(feature = "lua53")]
pub mod lua53;

#[cfg(feature = "lua52")]
pub mod lua52;

#[cfg(any(feature = "lua51", feature = "luajit"))]
pub mod lua51;

#[cfg(feature = "luau")]
pub mod luau;
