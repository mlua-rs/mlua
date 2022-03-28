#![cfg(feature = "luau")]

use std::env;
use std::fs;

use mlua::{Error, Lua, Result, Table, Value};

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

#[test]
fn test_sandbox() -> Result<()> {
    let lua = Lua::new();

    lua.sandbox(true)?;

    lua.load("global = 123").exec()?;
    let n: i32 = lua.load("return global").eval()?;
    assert_eq!(n, 123);
    assert_eq!(lua.globals().get::<_, Option<i32>>("global")?, Some(123));

    // Threads should inherit "main" globals
    let f = lua.create_function(|lua, ()| lua.globals().get::<_, i32>("global"))?;
    let co = lua.create_thread(f.clone())?;
    assert_eq!(co.resume::<_, Option<i32>>(())?, Some(123));

    // Sandboxed threads should also inherit "main" globals
    let co = lua.create_thread(f)?;
    co.sandbox()?;
    assert_eq!(co.resume::<_, Option<i32>>(())?, Some(123));

    lua.sandbox(false)?;

    // Previously set variable `global` should be cleared now
    assert_eq!(lua.globals().get::<_, Option<i32>>("global")?, None);

    // Readonly flags should be cleared as well
    let table = lua.globals().get::<_, Table>("table")?;
    table.set("test", "test")?;

    Ok(())
}

#[test]
fn test_sandbox_threads() -> Result<()> {
    let lua = Lua::new();

    let f = lua.create_function(|lua, v: Value| lua.globals().set("global", v))?;

    let co = lua.create_thread(f.clone())?;
    co.resume(321)?;
    // The main state should see the `global` variable (as the thread is not sandboxed)
    assert_eq!(lua.globals().get::<_, Option<i32>>("global")?, Some(321));

    let co = lua.create_thread(f.clone())?;
    co.sandbox()?;
    co.resume(123)?;
    // The main state should see the previous `global` value (as the thread is sandboxed)
    assert_eq!(lua.globals().get::<_, Option<i32>>("global")?, Some(321));

    // Try to reset the (sandboxed) thread
    co.reset(f)?;
    co.resume(111)?;
    assert_eq!(lua.globals().get::<_, Option<i32>>("global")?, Some(111));

    Ok(())
}
