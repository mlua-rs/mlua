#[test]
#[ignore]
fn test_compilation() {
    let t = trybuild::TestCases::new();

    t.compile_fail("tests/compile/function_borrow.rs");
    t.compile_fail("tests/compile/lua_norefunwindsafe.rs");
    t.compile_fail("tests/compile/ref_nounwindsafe.rs");
    t.compile_fail("tests/compile/scope_callback_capture.rs");
    t.compile_fail("tests/compile/scope_invariance.rs");
    t.compile_fail("tests/compile/scope_mutable_aliasing.rs");
    t.compile_fail("tests/compile/scope_userdata_borrow.rs");

    #[cfg(feature = "async")]
    {
        t.compile_fail("tests/compile/async_any_userdata_method.rs");
        t.compile_fail("tests/compile/async_nonstatic_userdata.rs");
    }

    #[cfg(feature = "send")]
    t.compile_fail("tests/compile/non_send.rs");
    #[cfg(not(feature = "send"))]
    t.pass("tests/compile/non_send.rs");

    #[cfg(feature = "macros")]
    {
        t.compile_fail("tests/compile/chunk_dollar_non_ident.rs");
        t.compile_fail("tests/compile/userdata_getter_and_meta.rs");
        t.compile_fail("tests/compile/userdata_getter_and_setter.rs");
        t.compile_fail("tests/compile/userdata_getter_mut_self.rs");
        t.compile_fail("tests/compile/userdata_getter_extra_arg.rs");
        t.compile_fail("tests/compile/userdata_setter_ref_self.rs");
        t.compile_fail("tests/compile/userdata_mut_slice_arg.rs");
        t.compile_fail("tests/compile/userdata_setter_no_value.rs");
        t.compile_fail("tests/compile/userdata_static_with_self.rs");
        t.compile_fail("tests/compile/userdata_meta_owned_self.rs");
        t.compile_fail("tests/compile/userdata_destructuring_arg.rs");
        t.compile_fail("tests/compile/userdata_generic_impl.rs");
        t.compile_fail("tests/compile/userdata_const_getter.rs");
        t.compile_fail("tests/compile/userdata_field_with_args.rs");
    }

    #[cfg(all(feature = "macros", feature = "async"))]
    {
        t.compile_fail("tests/compile/userdata_getter_async.rs");
        t.compile_fail("tests/compile/userdata_setter_async.rs");
        t.compile_fail("tests/compile/userdata_field_async.rs");
    }
}
