use mlua::{Function, Lua, Result, String};

#[test]
fn test_function() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        function concat(arg1, arg2)
            return arg1 .. arg2
        end
    "#,
    )
    .exec()?;

    let concat = globals.get::<_, Function>("concat")?;
    assert_eq!(concat.call::<_, String>(("foo", "bar"))?, "foobar");

    Ok(())
}

#[test]
fn test_bind() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        function concat(...)
            local res = ""
            for _, s in pairs({...}) do
                res = res..s
            end
            return res
        end
    "#,
    )
    .exec()?;

    let mut concat = globals.get::<_, Function>("concat")?;
    concat = concat.bind("foo")?;
    concat = concat.bind("bar")?;
    concat = concat.bind(("baz", "baf"))?;
    assert_eq!(
        concat.call::<_, String>(("hi", "wut"))?,
        "foobarbazbafhiwut"
    );

    Ok(())
}

#[test]
fn test_rust_function() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        function lua_function()
            return rust_function()
        end

        -- Test to make sure chunk return is ignored
        return 1
    "#,
    )
    .exec()?;

    let lua_function = globals.get::<_, Function>("lua_function")?;
    let rust_function = lua.create_function(|_, ()| Ok("hello"))?;

    globals.set("rust_function", rust_function)?;
    assert_eq!(lua_function.call::<_, String>(())?, "hello");

    Ok(())
}

#[test]
fn test_c_function() -> Result<()> {
    let lua = Lua::new();

    unsafe extern "C" fn c_function(state: *mut mlua::lua_State) -> std::os::raw::c_int {
        let lua = Lua::init_from_ptr(state);
        lua.globals().set("c_function", true).unwrap();
        0
    }

    let func = unsafe { lua.create_c_function(c_function)? };
    func.call(())?;
    assert_eq!(lua.globals().get::<_, bool>("c_function")?, true);

    Ok(())
}

#[test]
fn test_dump() -> Result<()> {
    let lua = unsafe { Lua::unsafe_new() };

    let concat_lua = lua
        .load(r#"function(arg1, arg2) return arg1 .. arg2 end"#)
        .eval::<Function>()?;
    let concat = lua.load(&concat_lua.dump(false)).into_function()?;

    assert_eq!(concat.call::<_, String>(("foo", "bar"))?, "foobar");

    Ok(())
}
