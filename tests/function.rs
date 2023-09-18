use mlua::{Function, Lua, Result, String, Table};

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
    assert_eq!(concat.call::<_, String>(())?, "foobarbazbaf");
    assert_eq!(
        concat.call::<_, String>(("hi", "wut"))?,
        "foobarbazbafhiwut"
    );

    let mut concat2 = globals.get::<_, Function>("concat")?;
    concat2 = concat2.bind(())?;
    assert_eq!(concat2.call::<_, String>(())?, "");
    assert_eq!(concat2.call::<_, String>(("ab", "cd"))?, "abcd");

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

    unsafe extern "C-unwind" fn c_function(state: *mut mlua::lua_State) -> std::ffi::c_int {
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
fn test_function_environment() -> Result<()> {
    let lua = Lua::new();

    // We must not get or set environment for C functions
    let rust_func = lua.create_function(|_, ()| Ok("hello"))?;
    assert_eq!(rust_func.environment(), None);
    assert_eq!(rust_func.set_environment(lua.globals()).ok(), Some(false));

    // Test getting Lua function environment
    lua.globals().set("hello", "global")?;
    let lua_func = lua
        .load(
            r#"
        local t = ""
        return function()
            -- two upvalues
            return t .. hello
        end
    "#,
        )
        .eval::<Function>()?;
    let lua_func2 = lua.load("return hello").into_function()?;
    assert_eq!(lua_func.call::<_, String>(())?, "global");
    assert_eq!(lua_func.environment(), Some(lua.globals()));

    // Test changing the environment
    let env = lua.create_table_from([("hello", "local")])?;
    assert!(lua_func.set_environment(env.clone())?);
    assert_eq!(lua_func.call::<_, String>(())?, "local");
    assert_eq!(lua_func2.call::<_, String>(())?, "global");

    // More complex case
    lua.load(
        r#"
        local number = 15
        function lucky() return tostring("number is "..number) end
        new_env = {
            tostring = function() return tostring(number) end,
        }
    "#,
    )
    .exec()?;
    let lucky = lua.globals().get::<_, Function>("lucky")?;
    assert_eq!(lucky.call::<_, String>(())?, "number is 15");
    let new_env = lua.globals().get::<_, Table>("new_env")?;
    lucky.set_environment(new_env)?;
    assert_eq!(lucky.call::<_, String>(())?, "15");

    // Test inheritance
    let lua_func2 = lua
        .load(r#"return function() return (function() return hello end)() end"#)
        .eval::<Function>()?;
    assert!(lua_func2.set_environment(env.clone())?);
    lua.gc_collect()?;
    assert_eq!(lua_func2.call::<_, String>(())?, "local");

    Ok(())
}

#[test]
fn test_function_info() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        function function1()
            return function() end
        end
    "#,
    )
    .set_name("source1")
    .exec()?;

    let function1 = globals.get::<_, Function>("function1")?;
    let function2 = function1.call::<_, Function>(())?;
    let function3 = lua.create_function(|_, ()| Ok(()))?;

    let function1_info = function1.info();
    #[cfg(feature = "luau")]
    assert_eq!(function1_info.name.as_deref(), Some("function1"));
    assert_eq!(function1_info.source.as_deref(), Some("source1"));
    assert_eq!(function1_info.line_defined, Some(2));
    #[cfg(not(feature = "luau"))]
    assert_eq!(function1_info.last_line_defined, Some(4));
    #[cfg(feature = "luau")]
    assert_eq!(function1_info.last_line_defined, None);
    assert_eq!(function1_info.what, "Lua");

    let function2_info = function2.info();
    assert_eq!(function2_info.name, None);
    assert_eq!(function2_info.source.as_deref(), Some("source1"));
    assert_eq!(function2_info.line_defined, Some(3));
    #[cfg(not(feature = "luau"))]
    assert_eq!(function2_info.last_line_defined, Some(3));
    #[cfg(feature = "luau")]
    assert_eq!(function2_info.last_line_defined, None);
    assert_eq!(function2_info.what, "Lua");

    let function3_info = function3.info();
    assert_eq!(function3_info.name, None);
    assert_eq!(function3_info.source.as_deref(), Some("=[C]"));
    assert_eq!(function3_info.line_defined, None);
    assert_eq!(function3_info.last_line_defined, None);
    assert_eq!(function3_info.what, "C");

    let print_info = globals.get::<_, Function>("print")?.info();
    #[cfg(feature = "luau")]
    assert_eq!(print_info.name.as_deref(), Some("print"));
    assert_eq!(print_info.source.as_deref(), Some("=[C]"));
    assert_eq!(print_info.what, "C");
    assert_eq!(print_info.line_defined, None);

    Ok(())
}

#[test]
fn test_function_wrap() -> Result<()> {
    use mlua::Error;

    let lua = Lua::new();

    lua.globals()
        .set("f", Function::wrap(|_, s: String| Ok(s)))?;
    lua.load(r#"assert(f("hello") == "hello")"#).exec().unwrap();

    let mut _i = false;
    lua.globals().set(
        "f",
        Function::wrap_mut(move |lua, ()| {
            _i = true;
            lua.globals().get::<_, Function>("f")?.call::<_, ()>(())
        }),
    )?;
    match lua.globals().get::<_, Function>("f")?.call::<_, ()>(()) {
        Err(Error::CallbackError { ref cause, .. }) => match *cause.as_ref() {
            Error::CallbackError { ref cause, .. } => match *cause.as_ref() {
                Error::RecursiveMutCallback { .. } => {}
                ref other => panic!("incorrect result: {other:?}"),
            },
            ref other => panic!("incorrect result: {other:?}"),
        },
        other => panic!("incorrect result: {other:?}"),
    };

    Ok(())
}

#[cfg(all(feature = "unstable", not(feature = "send")))]
#[test]
fn test_owned_function() -> Result<()> {
    let lua = Lua::new();

    let f = lua
        .create_function(|_, ()| Ok("hello, world!"))?
        .into_owned();
    drop(lua);

    // We still should be able to call the function despite Lua is dropped
    let s = f.call::<_, String>(())?;
    assert_eq!(s.to_string_lossy(), "hello, world!");

    Ok(())
}

#[cfg(all(feature = "unstable", not(feature = "send")))]
#[test]
fn test_owned_function_drop() -> Result<()> {
    let rc = std::sync::Arc::new(());

    {
        let lua = Lua::new();

        lua.set_app_data(rc.clone());

        let f1 = lua
            .create_function(|_, ()| Ok("hello, world!"))?
            .into_owned();
        let f2 =
            lua.create_function(move |_, ()| f1.to_ref().call::<_, std::string::String>(()))?;
        assert_eq!(f2.call::<_, String>(())?.to_string_lossy(), "hello, world!");
    }

    // Check that Lua is properly destroyed
    // It works because we collect garbage when Lua goes out of scope
    assert_eq!(std::sync::Arc::strong_count(&rc), 1);

    Ok(())
}
