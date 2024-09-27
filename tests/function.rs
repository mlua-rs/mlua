use mlua::{Error, Function, Lua, Result, String, Table};

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

    let concat = globals.get::<Function>("concat")?;
    assert_eq!(concat.call::<String>(("foo", "bar"))?, "foobar");

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

    let mut concat = globals.get::<Function>("concat")?;
    concat = concat.bind("foo")?;
    concat = concat.bind("bar")?;
    concat = concat.bind(("baz", "baf"))?;
    assert_eq!(concat.call::<String>(())?, "foobarbazbaf");
    assert_eq!(concat.call::<String>(("hi", "wut"))?, "foobarbazbafhiwut");

    let mut concat2 = globals.get::<Function>("concat")?;
    concat2 = concat2.bind(())?;
    assert_eq!(concat2.call::<String>(())?, "");
    assert_eq!(concat2.call::<String>(("ab", "cd"))?, "abcd");

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

    let lua_function = globals.get::<Function>("lua_function")?;
    let rust_function = lua.create_function(|_, ()| Ok("hello"))?;

    globals.set("rust_function", rust_function)?;
    assert_eq!(lua_function.call::<String>(())?, "hello");

    Ok(())
}

#[test]
fn test_c_function() -> Result<()> {
    let lua = Lua::new();

    unsafe extern "C-unwind" fn c_function(state: *mut mlua::lua_State) -> std::os::raw::c_int {
        ffi::lua_pushboolean(state, 1);
        ffi::lua_setglobal(state, b"c_function\0" as *const _ as *const _);
        0
    }

    let func = unsafe { lua.create_c_function(c_function)? };
    func.call::<()>(())?;
    assert_eq!(lua.globals().get::<bool>("c_function")?, true);

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

    assert_eq!(concat.call::<String>(("foo", "bar"))?, "foobar");

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
    assert_eq!(lua_func.call::<String>(())?, "global");
    assert_eq!(lua_func.environment(), Some(lua.globals()));

    // Test changing the environment
    let env = lua.create_table_from([("hello", "local")])?;
    assert!(lua_func.set_environment(env.clone())?);
    assert_eq!(lua_func.call::<String>(())?, "local");
    assert_eq!(lua_func2.call::<String>(())?, "global");

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
    let lucky = lua.globals().get::<Function>("lucky")?;
    assert_eq!(lucky.call::<String>(())?, "number is 15");
    let new_env = lua.globals().get::<Table>("new_env")?;
    lucky.set_environment(new_env)?;
    assert_eq!(lucky.call::<String>(())?, "15");

    // Test inheritance
    let lua_func2 = lua
        .load(r#"return function() return (function() return hello end)() end"#)
        .eval::<Function>()?;
    assert!(lua_func2.set_environment(env.clone())?);
    lua.gc_collect()?;
    assert_eq!(lua_func2.call::<String>(())?, "local");

    // Test getting environment set by chunk loader
    let chunk = lua
        .load("return hello")
        .set_environment(lua.create_table_from([("hello", "chunk")])?)
        .into_function()?;
    assert_eq!(chunk.environment().unwrap().get::<String>("hello")?, "chunk");

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

    let function1 = globals.get::<Function>("function1")?;
    let function2 = function1.call::<Function>(())?;
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

    let print_info = globals.get::<Function>("print")?.info();
    #[cfg(feature = "luau")]
    assert_eq!(print_info.name.as_deref(), Some("print"));
    assert_eq!(print_info.source.as_deref(), Some("=[C]"));
    assert_eq!(print_info.what, "C");
    assert_eq!(print_info.line_defined, None);

    Ok(())
}

#[test]
fn test_function_pointer() -> Result<()> {
    let lua = Lua::new();

    let func1 = lua.load("return function() end").into_function()?;
    let func2 = func1.call::<Function>(())?;

    assert_eq!(func1.to_pointer(), func1.clone().to_pointer());
    assert_ne!(func1.to_pointer(), func2.to_pointer());

    Ok(())
}

#[cfg(feature = "luau")]
#[test]
fn test_function_deep_clone() -> Result<()> {
    let lua = Lua::new();

    lua.globals().set("a", 1)?;
    let func1 = lua.load("a += 1; return a").into_function()?;
    let func2 = func1.deep_clone();

    assert_ne!(func1.to_pointer(), func2.to_pointer());
    assert_eq!(func1.call::<i32>(())?, 2);
    assert_eq!(func2.call::<i32>(())?, 3);

    // Check that for Rust functions deep_clone is just a clone
    let rust_func = lua.create_function(|_, ()| Ok(42))?;
    let rust_func2 = rust_func.deep_clone();
    assert_eq!(rust_func.to_pointer(), rust_func2.to_pointer());

    Ok(())
}

#[test]
fn test_function_wrap() -> Result<()> {
    let lua = Lua::new();

    let f = Function::wrap(|s: String, n| Ok(s.to_str().unwrap().repeat(n)));
    lua.globals().set("f", f)?;
    lua.load(r#"assert(f("hello", 2) == "hellohello")"#)
        .exec()
        .unwrap();

    // Return error
    let ferr = Function::wrap(|| Err::<(), _>(Error::runtime("some error")));
    lua.globals().set("ferr", ferr)?;
    lua.load(
        r#"
        local ok, err = pcall(ferr)
        assert(not ok and tostring(err):find("some error"))
    "#,
    )
    .exec()
    .unwrap();

    // Mutable callback
    let mut i = 0;
    let fmut = Function::wrap_mut(move || {
        i += 1;
        Ok(i)
    });
    lua.globals().set("fmut", fmut)?;
    lua.load(r#"fmut(); fmut(); assert(fmut() == 3)"#).exec().unwrap();

    // Check mutable callback with error
    let fmut_err = Function::wrap_mut(|| Err::<(), _>(Error::runtime("some error")));
    lua.globals().set("fmut_err", fmut_err)?;
    lua.load(
        r#"
        local ok, err = pcall(fmut_err)
        assert(not ok and tostring(err):find("some error"))
    "#,
    )
    .exec()
    .unwrap();

    // Check recursive mut callback error
    let fmut = Function::wrap_mut(|f: Function| match f.call::<()>(&f) {
        Err(Error::CallbackError { cause, .. }) => match cause.as_ref() {
            Error::RecursiveMutCallback { .. } => Ok(()),
            other => panic!("incorrect result: {other:?}"),
        },
        other => panic!("incorrect result: {other:?}"),
    });
    let fmut = lua.convert::<Function>(fmut)?;
    assert!(fmut.call::<()>(&fmut).is_ok());

    Ok(())
}

#[test]
fn test_function_wrap_raw() -> Result<()> {
    let lua = Lua::new();

    let f = Function::wrap_raw(|| "hello");
    lua.globals().set("f", f)?;
    lua.load(r#"assert(f() == "hello")"#).exec().unwrap();

    // Return error
    let ferr = Function::wrap_raw(|| Err::<(), _>("some error"));
    lua.globals().set("ferr", ferr)?;
    lua.load(
        r#"
        local _, err = ferr()
        assert(err == "some error")
    "#,
    )
    .exec()
    .unwrap();

    // Mutable callback
    let mut i = 0;
    let fmut = Function::wrap_raw_mut(move || {
        i += 1;
        i
    });
    lua.globals().set("fmut", fmut)?;
    lua.load(r#"fmut(); fmut(); assert(fmut() == 3)"#).exec().unwrap();

    // Check mutable callback with error
    let fmut_err = Function::wrap_raw_mut(|| Err::<(), _>("some error"));
    lua.globals().set("fmut_err", fmut_err)?;
    lua.load(
        r#"
        local _, err = fmut_err()
        assert(err == "some error")
    "#,
    )
    .exec()
    .unwrap();

    Ok(())
}
