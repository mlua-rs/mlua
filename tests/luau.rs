#![cfg(feature = "luau")]

use std::env;
use std::fs;

use mlua::{Error, Lua, Result, Value};

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

#[test]
fn test_vectors() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    globals.set(
        "vector",
        lua.create_function(|_, (x, y, z)| Ok(Value::Vector(x, y, z)))?,
    )?;

    let v: [f32; 3] = lua
        .load("return vector(1, 2, 3) + vector(3, 2, 1)")
        .eval()?;
    assert_eq!(v, [4.0, 4.0, 4.0]);

    Ok(())
}

#[test]
fn test_readonly_table() -> Result<()> {
    let lua = Lua::new();

    let t = lua.create_table()?;
    assert!(!t.is_readonly());
    t.set_readonly(true);
    assert!(t.is_readonly());

    match t.set("key", "value") {
        Err(Error::RuntimeError(err)) if err.contains("Attempt to modify a readonly table") => {}
        r => panic!(
            "expected RuntimeError(...) with a specific message, got {:?}",
            r
        ),
    };

    Ok(())
}
