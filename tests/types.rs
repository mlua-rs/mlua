use std::os::raw::c_void;

use mlua::{Function, LightUserData, Lua, Number, Result, String as LuaString, Thread};

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
        .get::<Function>("id")?
        .call::<LightUserData>(LightUserData(42 as *mut c_void))?;

    assert_eq!(res, LightUserData(42 as *mut c_void));

    Ok(())
}

#[test]
fn test_boolean_type_metatable() -> Result<()> {
    let lua = Lua::new();

    let mt = lua.create_table()?;
    mt.set("__add", Function::wrap(|a, b| Ok(a || b)))?;
    lua.set_type_metatable::<bool>(Some(mt));

    lua.load(r#"assert(true + true == true)"#).exec().unwrap();
    lua.load(r#"assert(true + false == true)"#).exec().unwrap();
    lua.load(r#"assert(false + true == true)"#).exec().unwrap();
    lua.load(r#"assert(false + false == false)"#).exec().unwrap();

    Ok(())
}

#[test]
fn test_lightuserdata_type_metatable() -> Result<()> {
    let lua = Lua::new();

    let mt = lua.create_table()?;
    mt.set(
        "__add",
        Function::wrap(|a: LightUserData, b: LightUserData| {
            Ok(LightUserData((a.0 as usize + b.0 as usize) as *mut c_void))
        }),
    )?;
    lua.set_type_metatable::<LightUserData>(Some(mt));

    let res = lua
        .load(
            r#"
        local a, b = ...
        return a + b
    "#,
        )
        .call::<LightUserData>((
            LightUserData(42 as *mut c_void),
            LightUserData(100 as *mut c_void),
        ))
        .unwrap();
    assert_eq!(res, LightUserData(142 as *mut c_void));

    Ok(())
}

#[test]
fn test_number_type_metatable() -> Result<()> {
    let lua = Lua::new();

    let mt = lua.create_table()?;
    mt.set("__call", Function::wrap(|n1: f64, n2: f64| Ok(n1 * n2)))?;
    lua.set_type_metatable::<Number>(Some(mt));
    lua.load(r#"assert((1.5)(3.0) == 4.5)"#).exec().unwrap();
    lua.load(r#"assert((5)(5) == 25)"#).exec().unwrap();

    Ok(())
}

#[test]
fn test_string_type_metatable() -> Result<()> {
    let lua = Lua::new();

    let mt = lua.create_table()?;
    mt.set(
        "__add",
        Function::wrap(|a: String, b: String| Ok(format!("{a}{b}"))),
    )?;
    lua.set_type_metatable::<LuaString>(Some(mt));

    lua.load(r#"assert(("foo" + "bar") == "foobar")"#).exec().unwrap();

    Ok(())
}

#[test]
fn test_function_type_metatable() -> Result<()> {
    let lua = Lua::new();

    let mt = lua.create_table()?;
    mt.set(
        "__index",
        Function::wrap(|_: Function, key: String| Ok(format!("function.{key}"))),
    )?;
    lua.set_type_metatable::<Function>(Some(mt));

    lua.load(r#"assert((function() end).foo == "function.foo")"#)
        .exec()
        .unwrap();

    Ok(())
}

#[test]
fn test_thread_type_metatable() -> Result<()> {
    let lua = Lua::new();

    let mt = lua.create_table()?;
    mt.set(
        "__index",
        Function::wrap(|_: Thread, key: String| Ok(format!("thread.{key}"))),
    )?;
    lua.set_type_metatable::<Thread>(Some(mt));

    lua.load(r#"assert((coroutine.create(function() end)).foo == "thread.foo")"#)
        .exec()
        .unwrap();

    Ok(())
}
