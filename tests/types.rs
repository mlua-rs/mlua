#![cfg_attr(
    all(feature = "luajit", target_os = "macos", target_arch = "x86_64"),
    feature(link_args)
)]

#[cfg_attr(
    all(feature = "luajit", target_os = "macos", target_arch = "x86_64"),
    link_args = "-pagezero_size 10000 -image_base 100000000",
    allow(unused_attributes)
)]
extern "system" {}

use std::os::raw::c_void;

use mlua::{Function, LightUserData, Lua, Result};

#[test]
fn test_lightuserdata() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        function id(a)
            return a
        end
    "#,
    )
    .exec()?;

    let res = globals
        .get::<_, Function>("id")?
        .call::<_, LightUserData>(LightUserData(42 as *mut c_void))?;

    assert_eq!(res, LightUserData(42 as *mut c_void));

    Ok(())
}
