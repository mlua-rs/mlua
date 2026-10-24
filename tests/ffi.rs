use mlua::ffi;

#[test]
fn test_buffer_not_unpin() {
    static_assertions::assert_not_impl_any!(ffi::luaL_Buffer: Unpin);
}
