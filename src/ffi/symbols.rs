use std::ffi::c_void;

extern "C" {
    static LUA_ALL_SYMBOLS: *const c_void;
}

// Hack to avoid stripping unused Lua symbols from binary in order to load C modules
pub fn keep_lua_symbols() {
    assert_ne!(format!("{:p}", unsafe { LUA_ALL_SYMBOLS }), "");
}
