#![cfg(feature = "pluto")]

use mlua::{Lua, Result};

#[test]
fn test_pluto_libs() -> Result<()> {
    let lua = Lua::new();

    lua.load(
        r#"
        local json = require("pluto:json")
        local assert = require("pluto:assert")
        local data = { foo = "bar" }
        local str = json.encode(data)
        assert.equal(str, '{"foo":"bar"}')
    "#,
    )
    .exec()
    .unwrap();

    Ok(())
}
