//! Contains definitions from `lauxlib.h`.

use std::os::raw::c_int;

#[cfg(feature = "lua54")]
pub use super::lua54::lauxlib::*;

#[cfg(feature = "lua53")]
pub use super::lua53::lauxlib::*;

#[cfg(feature = "lua52")]
pub use super::lua52::lauxlib::*;

#[cfg(any(feature = "lua51", feature = "luajit"))]
pub use super::lua51::lauxlib::*;

#[cfg(feature = "luau")]
pub use super::luau::lauxlib::*;

#[cfg(feature = "lua52")]
pub use super::compat53::{luaL_getmetafield, luaL_newmetatable, luaL_requiref, luaL_tolstring};

#[cfg(any(feature = "lua51", feature = "luajit"))]
pub use super::compat53::{
    luaL_checkstack, luaL_getmetafield, luaL_getsubtable, luaL_len, luaL_loadbufferx,
    luaL_newmetatable, luaL_requiref, luaL_setfuncs, luaL_setmetatable, luaL_testudata,
    luaL_tolstring, luaL_traceback,
};

#[cfg(feature = "luau")]
pub use super::compat53::{
    luaL_checkstack, luaL_getmetafield, luaL_getsubtable, luaL_len, luaL_newmetatable,
    luaL_requiref, luaL_setmetatable, luaL_testudata, luaL_tolstring, luaL_traceback,
};

// I believe `luaL_traceback` < 5.4 requires this much free stack to not error.
// 5.4 uses `luaL_Buffer`
pub const LUA_TRACEBACK_STACK: c_int = 11;
