#![cfg(feature = "luau")]

use std::env;
use std::fs;

use mlua::{Lua, Result};

#[test]
fn test_require() -> Result<()> {
    let lua = Lua::new();

    let temp_dir = tempfile::tempdir().unwrap();
    fs::write(
        temp_dir.path().join("module.luau"),
        r#"
        counter = counter or 0
        return counter + 1
    "#,
    )?;

    env::set_var("LUAU_PATH", temp_dir.path().join("?.luau"));
    lua.load(
        r#"
        local module = require("module")
        assert(module == 1)
        module = require("module")
        assert(module == 1)
    "#,
    )
    .exec()
}
