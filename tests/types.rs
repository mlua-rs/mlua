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
