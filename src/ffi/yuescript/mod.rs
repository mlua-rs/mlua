use super::lua::lua_State;
use std::os::raw::c_int;

pub const LUA_YUELIBNAME: &str = "yue";

extern "C" {
    pub fn luaopen_yue(L: *mut lua_State) -> c_int;
}
