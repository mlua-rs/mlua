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

#[cfg(not(feature = "luau"))]
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

#[test]
#[cfg(not(feature = "luau"))]
fn test_function_info() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        function function_1()
            return function()
            end
        end
        function function_2()
            return function()
            end
        end
    "#,
    )
    .set_name("source 1")?
    .exec()?;
    lua.load(
        r#"
        function function_3()
            return function()
            end
        end
        function function_4()
            return function()
            end
        end
    "#,
    )
    .set_name("source 2")?
    .exec()?;

    let function_5 = lua.create_function(|_, ()| Ok(()))?;
    globals.set("function_5", function_5)?;

    let function_1 = globals.get::<_, Function>("function_1")?;
    let function_2 = globals.get::<_, Function>("function_2")?;
    let function_3 = globals.get::<_, Function>("function_3")?;
    let function_4 = globals.get::<_, Function>("function_4")?;
    let function_5 = globals.get::<_, Function>("function_5")?;

    let function_1a = function_1.call::<_, Function>(())?;
    let function_2a = function_2.call::<_, Function>(())?;
    let function_3a = function_3.call::<_, Function>(())?;
    let function_4a = function_4.call::<_, Function>(())?;

    let function_1_source = function_1.info();
    assert_eq!(function_1_source.source, "source 1".as_bytes());
    assert_eq!(function_1_source.line_defined, 2);
    assert_eq!(function_1_source.last_line_defined, 5);
    assert_eq!(function_1_source.what, "Lua");

    let function_1a_source = function_1a.info();
    assert_eq!(function_1a_source.source, "source 1".as_bytes());
    assert_eq!(function_1a_source.line_defined, 3);
    assert_eq!(function_1a_source.last_line_defined, 4);
    assert_eq!(function_1a_source.what, "Lua");

    let function_2_source = function_2.info();
    assert_eq!(function_2_source.source, "source 1".as_bytes());
    assert_eq!(function_2_source.line_defined, 6);
    assert_eq!(function_2_source.last_line_defined, 9);
    assert_eq!(function_2_source.what, "Lua");

    let function_2a_source = function_2a.info();
    assert_eq!(function_2a_source.source, "source 1".as_bytes());
    assert_eq!(function_2a_source.line_defined, 7);
    assert_eq!(function_2a_source.last_line_defined, 8);
    assert_eq!(function_2a_source.what, "Lua");

    let function_3_source = function_3.info();
    assert_eq!(function_3_source.source, "source 2".as_bytes());
    assert_eq!(function_3_source.line_defined, 2);
    assert_eq!(function_3_source.last_line_defined, 5);
    assert_eq!(function_3_source.what, "Lua");

    let function_3a_source = function_3a.info();
    assert_eq!(function_3a_source.source, "source 2".as_bytes());
    assert_eq!(function_3a_source.line_defined, 3);
    assert_eq!(function_3a_source.last_line_defined, 4);
    assert_eq!(function_3a_source.what, "Lua");

    let function_4_source = function_4.info();
    assert_eq!(function_4_source.source, "source 2".as_bytes());
    assert_eq!(function_4_source.line_defined, 6);
    assert_eq!(function_4_source.last_line_defined, 9);
    assert_eq!(function_4_source.what, "Lua");

    let function_4a_source = function_4a.info();
    assert_eq!(function_4a_source.source, "source 2".as_bytes());
    assert_eq!(function_4a_source.line_defined, 7);
    assert_eq!(function_4a_source.last_line_defined, 8);
    assert_eq!(function_4a_source.what, "Lua");

    let function_5_source = function_5.info();
    assert_eq!(function_5_source.source, "=[C]".as_bytes());
    assert_eq!(function_5_source.line_defined, -1);
    assert_eq!(function_5_source.last_line_defined, -1);
    assert_eq!(function_5_source.what, "C");

    Ok(())
}
