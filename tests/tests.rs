use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::iter::FromIterator;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::string::String as StdString;
use std::sync::Arc;
use std::{error, f32, f64, fmt};

use mlua::{
    ffi, ChunkMode, Error, ExternalError, Function, Lua, LuaOptions, Nil, Result, StdLib, String, Table,
    UserData, Value, Variadic,
};

#[test]
fn test_weak_lua() {
    let lua = Lua::new();
    let weak_lua = lua.weak();
    assert!(weak_lua.try_upgrade().is_some());
    drop(lua);
    assert!(weak_lua.try_upgrade().is_none());
}

#[test]
#[should_panic(expected = "Lua instance is destroyed")]
fn test_weak_lua_panic() {
    let lua = Lua::new();
    let weak_lua = lua.weak();
    drop(lua);
    let _ = weak_lua.upgrade();
}

#[cfg(not(feature = "luau"))]
#[test]
fn test_safety() -> Result<()> {
    let lua = Lua::new();
    assert!(lua.load(r#"require "debug""#).exec().is_err());
    match lua.load_std_libs(StdLib::DEBUG) {
        Err(Error::SafetyError(_)) => {}
        Err(e) => panic!("expected SafetyError, got {:?}", e),
        Ok(_) => panic!("expected SafetyError, got no error"),
    }
    drop(lua);

    let lua = unsafe { Lua::unsafe_new() };
    assert!(lua.load(r#"require "debug""#).exec().is_ok());
    drop(lua);

    match Lua::new_with(StdLib::DEBUG, LuaOptions::default()) {
        Err(Error::SafetyError(_)) => {}
        Err(e) => panic!("expected SafetyError, got {:?}", e),
        Ok(_) => panic!("expected SafetyError, got new Lua state"),
    }

    let lua = Lua::new();
    match lua.load(r#"package.loadlib()"#).exec() {
        Err(Error::CallbackError { ref cause, .. }) => match cause.as_ref() {
            Error::SafetyError(_) => {}
            e => panic!("expected SafetyError cause, got {:?}", e),
        },
        Err(e) => panic!("expected CallbackError, got {:?}", e),
        Ok(_) => panic!("expected CallbackError, got no error"),
    };
    match lua.load(r#"require "fake_ffi""#).exec() {
        Err(Error::RuntimeError(msg)) => assert!(msg.contains("can't load C modules in safe mode")),
        Err(e) => panic!("expected RuntimeError, got {:?}", e),
        Ok(_) => panic!("expected RuntimeError, got no error"),
    }
    drop(lua);

    // Test safety rules after dynamically loading `package` library
    let lua = Lua::new_with(StdLib::NONE, LuaOptions::default())?;
    assert!(lua.globals().get::<Option<Value>>("require")?.is_none());
    lua.load_std_libs(StdLib::PACKAGE)?;
    match lua.load(r#"package.loadlib()"#).exec() {
        Err(Error::CallbackError { ref cause, .. }) => match cause.as_ref() {
            Error::SafetyError(_) => {}
            e => panic!("expected SafetyError cause, got {:?}", e),
        },
        Err(e) => panic!("expected CallbackError, got {:?}", e),
        Ok(_) => panic!("expected CallbackError, got no error"),
    };

    Ok(())
}

#[test]
fn test_load() -> Result<()> {
    let lua = Lua::new();

    let func = lua.load("\treturn 1+2").into_function()?;
    let result: i32 = func.call(())?;
    assert_eq!(result, 3);

    assert!(lua.load("").exec().is_ok());
    assert!(lua.load("§$%§&$%&").exec().is_err());

    Ok(())
}

#[test]
fn test_exec() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        res = 'foo'..'bar'
    "#,
    )
    .exec()?;
    assert_eq!(globals.get::<String>("res")?, "foobar");

    let module: Table = lua
        .load(
            r#"
            local module = {}

            function module.func()
                return "hello"
            end

            return module
        "#,
        )
        .eval()?;
    assert!(module.contains_key("func")?);
    assert_eq!(module.get::<Function>("func")?.call::<String>(())?, "hello");

    Ok(())
}

#[test]
fn test_eval() -> Result<()> {
    let lua = Lua::new();

    assert_eq!(lua.load("1 + 1").eval::<i32>()?, 2);
    assert_eq!(lua.load("false == false").eval::<bool>()?, true);
    assert_eq!(lua.load("return 1 + 2").eval::<i32>()?, 3);
    match lua.load("if true then").eval::<()>() {
        Err(Error::SyntaxError {
            incomplete_input: true,
            ..
        }) => {}
        r => panic!("expected SyntaxError with incomplete_input=true, got {:?}", r),
    }

    Ok(())
}

#[test]
fn test_replace_globals() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.create_table()?;
    globals.set("foo", "bar")?;

    lua.set_globals(globals.clone())?;
    let val = lua.load("return foo").eval::<StdString>()?;
    assert_eq!(val, "bar");

    // Updating globals in sandboxed Lua state is not allowed
    #[cfg(feature = "luau")]
    {
        lua.sandbox(true)?;
        match lua.set_globals(globals) {
            Err(Error::RuntimeError(msg))
                if msg.contains("cannot change globals in a sandboxed Lua state") => {}
            r => panic!("expected RuntimeError(...) with a specific error message, got {r:?}"),
        }
    }

    Ok(())
}

#[test]
fn test_load_mode() -> Result<()> {
    let lua = unsafe { Lua::unsafe_new() };

    assert_eq!(lua.load("1 + 1").set_mode(ChunkMode::Text).eval::<i32>()?, 2);
    match lua.load("1 + 1").set_mode(ChunkMode::Binary).exec() {
        Ok(_) => panic!("expected SyntaxError, got no error"),
        Err(Error::SyntaxError { message: msg, .. }) => {
            assert!(msg.contains("attempt to load a text chunk"))
        }
        Err(e) => panic!("expected SyntaxError, got {:?}", e),
    };

    #[cfg(not(feature = "luau"))]
    let bytecode = lua.load("return 1 + 1").into_function()?.dump(true);
    #[cfg(feature = "luau")]
    let bytecode = mlua::Compiler::new().compile("return 1 + 1")?;
    assert_eq!(lua.load(&bytecode).eval::<i32>()?, 2);
    assert_eq!(lua.load(&bytecode).set_mode(ChunkMode::Binary).eval::<i32>()?, 2);
    match lua.load(&bytecode).set_mode(ChunkMode::Text).exec() {
        Ok(_) => panic!("expected SyntaxError, got no error"),
        Err(Error::SyntaxError { message: msg, .. }) => {
            assert!(msg.contains("attempt to load a binary chunk"))
        }
        Err(e) => panic!("expected SyntaxError, got {:?}", e),
    };

    Ok(())
}

#[test]
fn test_lua_multi() -> Result<()> {
    let lua = Lua::new();

    lua.load(
        r#"
        function concat(arg1, arg2)
            return arg1 .. arg2
        end

        function mreturn()
            return 1, 2, 3, 4, 5, 6
        end
    "#,
    )
    .exec()?;

    let globals = lua.globals();
    let concat = globals.get::<Function>("concat")?;
    let mreturn = globals.get::<Function>("mreturn")?;

    assert_eq!(concat.call::<String>(("foo", "bar"))?, "foobar");
    let (a, b) = mreturn.call::<(u64, u64)>(())?;
    assert_eq!((a, b), (1, 2));
    let (a, b, v) = mreturn.call::<(u64, u64, Variadic<u64>)>(())?;
    assert_eq!((a, b), (1, 2));
    assert_eq!(v[..], [3, 4, 5, 6]);

    Ok(())
}

#[test]
fn test_coercion() -> Result<()> {
    let lua = Lua::new();

    lua.load(
        r#"
        int = 123
        str = "123"
        num = 123.0
        func = function() end
    "#,
    )
    .exec()?;

    let globals = lua.globals();
    assert_eq!(globals.get::<String>("int")?, "123");
    assert_eq!(globals.get::<i32>("str")?, 123);
    assert_eq!(globals.get::<i32>("num")?, 123);
    assert!(globals.get::<String>("func").is_err());

    Ok(())
}

#[test]
fn test_error() -> Result<()> {
    #[derive(Debug)]
    pub struct TestError;

    impl fmt::Display for TestError {
        fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
            write!(fmt, "test error")
        }
    }

    impl error::Error for TestError {}

    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        function no_error()
        end

        function lua_error()
            error("this is a lua error")
        end

        function rust_error()
            rust_error_function()
        end

        function return_error()
            local status, res = pcall(rust_error_function)
            assert(not status)
            return res
        end

        function return_string_error()
            return "this should be converted to an error"
        end

        function test_pcall()
            local testvar = 0

            pcall(function(arg)
                testvar = testvar + arg
                error("should be ignored")
            end, 3)

            local function handler(err)
                if string.match(_VERSION, " 5%.1$")
                    or string.match(_VERSION, " 5%.2$")
                    or string.match(_VERSION, "Luau")
                then
                    -- Special case for Lua 5.1/5.2 and Luau
                    local caps = string.match(err, ': (%d+)$')
                    if caps then
                        err = caps
                    end
                end
                testvar = testvar + err
                return "should be ignored"
            end

            local status, res = xpcall(function()
                error(5)
            end, handler)
            assert(not status)

            if testvar ~= 8 then
                error("testvar had the wrong value, pcall / xpcall misbehaving "..testvar)
            end
        end

        function understand_recursion()
            understand_recursion()
        end
    "#,
    )
    .exec()?;

    let rust_error_function = lua.create_function(|_, ()| -> Result<()> { Err(TestError.into_lua_err()) })?;
    globals.set("rust_error_function", rust_error_function)?;

    let no_error = globals.get::<Function>("no_error")?;
    assert!(no_error.call::<()>(()).is_ok());

    let lua_error = globals.get::<Function>("lua_error")?;
    match lua_error.call::<()>(()) {
        Err(Error::RuntimeError(_)) => {}
        Err(e) => panic!("error is not RuntimeError kind, got {:?}", e),
        _ => panic!("error not returned"),
    }

    let rust_error = globals.get::<Function>("rust_error")?;
    match rust_error.call::<()>(()) {
        Err(Error::CallbackError { .. }) => {}
        Err(e) => panic!("error is not CallbackError kind, got {:?}", e),
        _ => panic!("error not returned"),
    }

    let return_error = globals.get::<Function>("return_error")?;
    match return_error.call::<Value>(()) {
        Ok(Value::Error(_)) => {}
        _ => panic!("Value::Error not returned"),
    }

    let return_string_error = globals.get::<Function>("return_string_error")?;
    assert!(return_string_error.call::<Error>(()).is_ok());

    match lua.load("if you are happy and you know it syntax error").exec() {
        Err(Error::SyntaxError {
            incomplete_input: false,
            ..
        }) => {}
        Err(_) => panic!("error is not LuaSyntaxError::Syntax kind"),
        _ => panic!("error not returned"),
    }
    match lua.load("function i_will_finish_what_i()").exec() {
        Err(Error::SyntaxError {
            incomplete_input: true,
            ..
        }) => {}
        Err(_) => panic!("error is not LuaSyntaxError::IncompleteStatement kind"),
        _ => panic!("error not returned"),
    }

    let test_pcall = globals.get::<Function>("test_pcall")?;
    test_pcall.call::<()>(())?;

    #[cfg(not(target_arch = "wasm32"))]
    {
        let understand_recursion = globals.get::<Function>("understand_recursion")?;
        assert!(understand_recursion.call::<()>(()).is_err());
    }

    Ok(())
}

#[test]
fn test_panic() -> Result<()> {
    fn make_lua(options: LuaOptions) -> Result<Lua> {
        let lua = Lua::new_with(StdLib::ALL_SAFE, options)?;
        let rust_panic_function = lua.create_function(|_, msg: Option<StdString>| -> Result<()> {
            if let Some(msg) = msg {
                panic!("{}", msg)
            }
            panic!("rust panic")
        })?;
        lua.globals().set("rust_panic_function", rust_panic_function)?;
        Ok(lua)
    }

    // Test triggering Lua error with sending Rust panic (must be resumed)
    {
        let lua = make_lua(LuaOptions::default())?;

        match catch_unwind(AssertUnwindSafe(|| -> Result<()> {
            lua.load(
                r#"
                _, err = pcall(rust_panic_function)
                error(err)
            "#,
            )
            .exec()
        })) {
            Ok(Ok(_)) => panic!("no panic was detected"),
            Ok(Err(e)) => panic!("error during panic test {:?}", e),
            Err(p) => assert!(*p.downcast::<&str>().unwrap() == "rust panic"),
        };

        // Trigger same panic again
        match lua.load("error(err)").exec() {
            Ok(_) => panic!("no error was detected"),
            Err(Error::PreviouslyResumedPanic) => {}
            Err(e) => panic!("expected PreviouslyResumedPanic, got {:?}", e),
        }
    }

    // Test returning Rust panic (must be resumed)
    {
        let lua = make_lua(LuaOptions::default())?;
        match catch_unwind(AssertUnwindSafe(|| -> Result<()> {
            let _catched_panic = lua
                .load(
                    r#"
                    -- Set global
                    _, err = pcall(rust_panic_function)
                    return err
                "#,
                )
                .eval::<Value>()?;
            Ok(())
        })) {
            Ok(_) => panic!("no panic was detected"),
            Err(_) => {}
        };

        assert!(lua.globals().get::<Value>("err")? == Value::Nil);
        match lua.load("tostring(err)").exec() {
            Ok(_) => panic!("no error was detected"),
            Err(Error::CallbackError { ref cause, .. }) => match cause.as_ref() {
                Error::PreviouslyResumedPanic => {}
                e => panic!("expected PreviouslyResumedPanic, got {:?}", e),
            },
            Err(e) => panic!("expected CallbackError, got {:?}", e),
        }
    }

    // Test representing Rust panic as a string
    match catch_unwind(|| -> Result<()> {
        let lua = make_lua(LuaOptions::default())?;
        lua.load(
            r#"
            local _, err = pcall(rust_panic_function)
            error(tostring(err))
        "#,
        )
        .exec()
    }) {
        Ok(Ok(_)) => panic!("no error was detected"),
        Ok(Err(Error::RuntimeError(_))) => {}
        Ok(Err(e)) => panic!("expected RuntimeError, got {:?}", e),
        Err(_) => panic!("panic was detected"),
    }

    // Test disabling `catch_rust_panics` option / pcall correctness
    match catch_unwind(|| -> Result<()> {
        let lua = make_lua(LuaOptions::new().catch_rust_panics(false))?;
        lua.load(
            r#"
            local ok, err = pcall(function(msg) error(msg) end, "hello")
            assert(not ok and err:find("hello") ~= nil)

            ok, err = pcall(rust_panic_function, "rust panic from lua")
            -- Nothing to return, panic should be automatically resumed
        "#,
        )
        .exec()
    }) {
        Ok(r) => panic!("no panic was detected: {:?}", r),
        Err(p) => assert!(*p.downcast::<StdString>().unwrap() == "rust panic from lua"),
    }

    // Test disabling `catch_rust_panics` option / xpcall correctness
    match catch_unwind(|| -> Result<()> {
        let lua = make_lua(LuaOptions::new().catch_rust_panics(false))?;
        lua.load(
            r#"
            local msgh_ok = false
            local msgh = function(err)
                msgh_ok = err ~= nil and err:find("hello") ~= nil
                return err
            end
            local ok, err = xpcall(function(msg) error(msg) end, msgh, "hello")
            assert(not ok and err:find("hello") ~= nil)
            assert(msgh_ok)

            ok, err = xpcall(rust_panic_function, msgh, "rust panic from lua")
            -- Nothing to return, panic should be automatically resumed
        "#,
        )
        .exec()
    }) {
        Ok(r) => panic!("no panic was detected: {:?}", r),
        Err(p) => assert!(*p.downcast::<StdString>().unwrap() == "rust panic from lua"),
    }

    Ok(())
}

#[cfg(target_pointer_width = "64")]
#[test]
fn test_safe_integers() -> Result<()> {
    const MAX_SAFE_INTEGER: i64 = 2i64.pow(53) - 1;
    const MIN_SAFE_INTEGER: i64 = -2i64.pow(53) + 1;

    let lua = Lua::new();
    let f = lua.load("return ...").into_function()?;

    assert_eq!(f.call::<i64>(MAX_SAFE_INTEGER)?, MAX_SAFE_INTEGER);
    assert_eq!(f.call::<i64>(MIN_SAFE_INTEGER)?, MIN_SAFE_INTEGER);

    // For Lua versions that does not support 64-bit integers, the values will be converted to f64
    #[cfg(any(feature = "luau", feature = "lua51", feature = "luajit"))]
    {
        assert_ne!(f.call::<i64>(MAX_SAFE_INTEGER + 2)?, MAX_SAFE_INTEGER + 2);
        assert_ne!(f.call::<i64>(MIN_SAFE_INTEGER - 2)?, MIN_SAFE_INTEGER - 2);
        assert_eq!(f.call::<f64>(i64::MAX)?, i64::MAX as f64);
    }

    Ok(())
}

#[test]
fn test_num_conversion() -> Result<()> {
    let lua = Lua::new();

    assert_eq!(
        lua.coerce_integer(Value::String(lua.create_string("1")?))?,
        Some(1)
    );
    assert_eq!(
        lua.coerce_integer(Value::String(lua.create_string("1.0")?))?,
        Some(1)
    );
    assert_eq!(
        lua.coerce_integer(Value::String(lua.create_string("1.5")?))?,
        None
    );

    assert_eq!(
        lua.coerce_number(Value::String(lua.create_string("1")?))?,
        Some(1.0)
    );
    assert_eq!(
        lua.coerce_number(Value::String(lua.create_string("1.0")?))?,
        Some(1.0)
    );
    assert_eq!(
        lua.coerce_number(Value::String(lua.create_string("1.5")?))?,
        Some(1.5)
    );

    assert_eq!(lua.load("1.0").eval::<i64>()?, 1);
    assert_eq!(lua.load("1.0").eval::<f64>()?, 1.0);
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    assert_eq!(lua.load("1.0").eval::<String>()?, "1.0");
    #[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit", feature = "luau"))]
    assert_eq!(lua.load("1.0").eval::<String>()?, "1");

    assert_eq!(lua.load("1.5").eval::<i64>()?, 1);
    assert_eq!(lua.load("1.5").eval::<f64>()?, 1.5);
    assert_eq!(lua.load("1.5").eval::<String>()?, "1.5");

    assert!(lua.load("-1").eval::<u64>().is_err());
    assert_eq!(lua.load("-1").eval::<i64>()?, -1);

    assert!(lua.unpack::<u64>(lua.pack(1u128 << 64)?).is_err());
    assert!(lua.load("math.huge").eval::<i64>().is_err());

    assert_eq!(lua.unpack::<f64>(lua.pack(f32::MAX)?)?, f32::MAX as f64);
    assert_eq!(lua.unpack::<f64>(lua.pack(f32::MIN)?)?, f32::MIN as f64);
    assert_eq!(lua.unpack::<f32>(lua.pack(f64::MAX)?)?, f32::INFINITY);
    assert_eq!(lua.unpack::<f32>(lua.pack(f64::MIN)?)?, f32::NEG_INFINITY);

    assert_eq!(lua.unpack::<i128>(lua.pack(1i128 << 64)?)?, 1i128 << 64);

    Ok(())
}

#[test]
fn test_pcall_xpcall() -> Result<()> {
    let lua = Lua::new();
    let globals = lua.globals();

    // make sure that we handle not enough arguments

    assert!(lua.load("pcall()").exec().is_err());
    assert!(lua.load("xpcall()").exec().is_err());
    assert!(lua.load("xpcall(function() end)").exec().is_err());

    // Lua >= 5.2 compatible version of xpcall for 5.1
    #[cfg(feature = "lua51")]
    lua.load(
        r#"
        local xpcall_orig = xpcall
        function xpcall(f, err, ...)
            return xpcall_orig(function() return f(unpack(arg)) end, err)
        end
    "#,
    )
    .exec()?;

    // Make sure that the return values from are correct on success

    let (r, e) = lua
        .load("pcall(function(p) return p end, 'foo')")
        .eval::<(bool, String)>()?;
    assert!(r);
    assert_eq!(e, "foo");

    let (r, e) = lua
        .load("xpcall(function(p) return p end, print, 'foo')")
        .eval::<(bool, String)>()?;
    assert!(r);
    assert_eq!(e, "foo");

    // Make sure that the return values are correct on errors, and that error handling works

    lua.load(
        r#"
        pcall_error = nil
        pcall_status, pcall_error = pcall(error, "testerror")

        xpcall_error = nil
        xpcall_status, _ = xpcall(error, function(err) xpcall_error = err end, "testerror")
    "#,
    )
    .exec()?;

    assert_eq!(globals.get::<bool>("pcall_status")?, false);
    assert_eq!(globals.get::<String>("pcall_error")?, "testerror");

    assert_eq!(globals.get::<bool>("xpcall_statusr")?, false);
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luajit"))]
    assert_eq!(globals.get::<std::string::String>("xpcall_error")?, "testerror");
    #[cfg(feature = "lua51")]
    assert!(globals
        .get::<String>("xpcall_error")?
        .to_str()?
        .ends_with(": testerror"));

    // Make sure that weird xpcall error recursion at least doesn't cause unsafety or panics.
    lua.load(
        r#"
        function xpcall_recursion()
            xpcall(error, function(err) error(err) end, "testerror")
        end
    "#,
    )
    .exec()?;
    let _ = globals.get::<Function>("xpcall_recursion")?.call::<()>(());

    Ok(())
}

#[test]
fn test_recursive_mut_callback_error() -> Result<()> {
    let lua = Lua::new();

    let mut v = Some(Box::new(123));
    let f = lua.create_function_mut(move |lua, mutate: bool| {
        if mutate {
            v = None;
        } else {
            // Produce a mutable reference
            let r = v.as_mut().unwrap();
            // Whoops, this will recurse into the function and produce another mutable reference!
            lua.globals().get::<Function>("f")?.call::<()>(true)?;
            println!("Should not get here, mutable aliasing has occurred!");
            println!("value at {:p} is {r}", r as *mut _);
        }

        Ok(())
    })?;
    lua.globals().set("f", f)?;
    match lua.globals().get::<Function>("f")?.call::<()>(false) {
        Err(Error::CallbackError { ref cause, .. }) => match *cause.as_ref() {
            Error::CallbackError { ref cause, .. } => match *cause.as_ref() {
                Error::RecursiveMutCallback { .. } => {}
                ref other => panic!("incorrect result: {:?}", other),
            },
            ref other => panic!("incorrect result: {:?}", other),
        },
        other => panic!("incorrect result: {:?}", other),
    };

    Ok(())
}

#[test]
fn test_set_metatable_nil() -> Result<()> {
    let lua = Lua::new();
    lua.load(
        r#"
        a = {}
        setmetatable(a, nil)
    "#,
    )
    .exec()?;
    Ok(())
}

#[test]
fn test_named_registry_value() -> Result<()> {
    let lua = Lua::new();

    lua.set_named_registry_value("test", 42)?;
    let f = lua.create_function(move |lua, ()| {
        assert_eq!(lua.named_registry_value::<i32>("test")?, 42);
        Ok(())
    })?;

    f.call::<()>(())?;

    lua.unset_named_registry_value("test")?;
    match lua.named_registry_value("test")? {
        Nil => {}
        val => panic!("registry value was not Nil, was {:?}", val),
    };

    Ok(())
}

#[test]
fn test_registry_value() -> Result<()> {
    let lua = Lua::new();

    let mut r = Some(lua.create_registry_value(42)?);
    let f = lua.create_function_mut(move |lua, ()| {
        if let Some(r) = r.take() {
            assert_eq!(lua.registry_value::<i32>(&r)?, 42);
            lua.remove_registry_value(r).unwrap();
        } else {
            panic!();
        }
        Ok(())
    })?;

    f.call::<()>(())?;

    Ok(())
}

#[test]
fn test_drop_registry_value() -> Result<()> {
    struct MyUserdata(#[allow(unused)] Arc<()>);

    impl UserData for MyUserdata {}

    let lua = Lua::new();
    let rc = Arc::new(());

    let r = lua.create_registry_value(MyUserdata(rc.clone()))?;
    assert_eq!(Arc::strong_count(&rc), 2);

    drop(r);
    lua.expire_registry_values();

    lua.load(r#"collectgarbage("collect")"#).exec()?;

    assert_eq!(Arc::strong_count(&rc), 1);

    Ok(())
}

#[test]
fn test_replace_registry_value() -> Result<()> {
    let lua = Lua::new();

    let mut key = lua.create_registry_value(42)?;
    lua.replace_registry_value(&mut key, "new value")?;
    assert_eq!(lua.registry_value::<String>(&key)?, "new value");
    lua.replace_registry_value(&mut key, Value::Nil)?;
    assert_eq!(lua.registry_value::<Value>(&key)?, Value::Nil);
    lua.replace_registry_value(&mut key, 123)?;
    assert_eq!(lua.registry_value::<i32>(&key)?, 123);

    let mut key2 = lua.create_registry_value(Value::Nil)?;
    lua.replace_registry_value(&mut key2, Value::Nil)?;
    assert_eq!(lua.registry_value::<Value>(&key2)?, Value::Nil);
    lua.replace_registry_value(&mut key2, "abc")?;
    assert_eq!(lua.registry_value::<String>(&key2)?, "abc");

    Ok(())
}

#[test]
fn test_lua_registry_hash() -> Result<()> {
    let lua = Lua::new();

    let r1 = Arc::new(lua.create_registry_value("value1")?);
    let r2 = Arc::new(lua.create_registry_value("value2")?);

    let mut map = HashMap::new();
    map.insert(r1.clone(), "value1");
    map.insert(r2.clone(), "value2");

    assert_eq!(map[&r1], "value1");
    assert_eq!(map[&r2], "value2");

    Ok(())
}

#[test]
fn test_lua_registry_ownership() -> Result<()> {
    let lua1 = Lua::new();
    let lua2 = Lua::new();

    let r1 = lua1.create_registry_value("hello")?;
    let r2 = lua2.create_registry_value("hello")?;

    assert!(lua1.owns_registry_value(&r1));
    assert!(!lua2.owns_registry_value(&r1));
    assert!(lua2.owns_registry_value(&r2));
    assert!(!lua1.owns_registry_value(&r2));

    Ok(())
}

#[test]
fn test_mismatched_registry_key() -> Result<()> {
    let lua1 = Lua::new();
    let lua2 = Lua::new();

    let r = lua1.create_registry_value("hello")?;
    match lua2.remove_registry_value(r) {
        Err(Error::MismatchedRegistryKey) => {}
        r => panic!("wrong result type for mismatched registry key, {:?}", r),
    };

    Ok(())
}

#[test]
fn test_registry_value_reuse() -> Result<()> {
    let lua = Lua::new();

    let r1 = lua.create_registry_value("value1")?;
    let r1_slot = format!("{r1:?}");
    drop(r1);

    // Previous slot must not be reused by nil value
    let r2 = lua.create_registry_value(Value::Nil)?;
    let r2_slot = format!("{r2:?}");
    assert_ne!(r1_slot, r2_slot);
    drop(r2);

    // But should be reused by non-nil value
    let r3 = lua.create_registry_value("value3")?;
    let r3_slot = format!("{r3:?}");
    assert_eq!(r1_slot, r3_slot);

    Ok(())
}

#[test]
fn test_application_data() -> Result<()> {
    let lua = Lua::new();

    lua.set_app_data("test1");
    lua.set_app_data(vec!["test2"]);

    // Borrow &str immutably and Vec<&str> mutably
    let s = lua.app_data_ref::<&str>().unwrap();
    let mut v = lua.app_data_mut::<Vec<&str>>().unwrap();
    v.push("test3");

    // Insert of new data or removal should fail now
    assert!(lua.try_set_app_data::<i32>(123).is_err());
    match catch_unwind(AssertUnwindSafe(|| lua.set_app_data::<i32>(123))) {
        Ok(_) => panic!("expected panic"),
        Err(_) => {}
    }
    match catch_unwind(AssertUnwindSafe(|| lua.remove_app_data::<i32>())) {
        Ok(_) => panic!("expected panic"),
        Err(_) => {}
    }

    // Check display and debug impls
    assert_eq!(format!("{s}"), "test1");
    assert_eq!(format!("{s:?}"), "\"test1\"");

    // Borrowing immutably and mutably of the same type is not allowed
    assert!(lua.try_app_data_mut::<&str>().is_err());
    match catch_unwind(AssertUnwindSafe(|| lua.app_data_mut::<&str>().unwrap())) {
        Ok(_) => panic!("expected panic"),
        Err(_) => {}
    }
    assert!(lua.try_app_data_ref::<Vec<&str>>().is_err());
    drop((s, v));

    // Test that application data is accessible from anywhere
    let f = lua.create_function(|lua, ()| {
        let mut data1 = lua.app_data_mut::<&str>().unwrap();
        assert_eq!(*data1, "test1");
        *data1 = "test4";

        let data2 = lua.app_data_ref::<Vec<&str>>().unwrap();
        assert_eq!(*data2, vec!["test2", "test3"]);

        Ok(())
    })?;
    f.call::<()>(())?;

    assert_eq!(*lua.app_data_ref::<&str>().unwrap(), "test4");
    assert_eq!(*lua.app_data_ref::<Vec<&str>>().unwrap(), vec!["test2", "test3"]);

    lua.remove_app_data::<Vec<&str>>();
    assert!(matches!(lua.app_data_ref::<Vec<&str>>(), None));

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

    extern "C-unwind" fn c_function(state: *mut mlua::lua_State) -> std::os::raw::c_int {
        unsafe {
            ffi::lua_pushboolean(state, 1);
            ffi::lua_setglobal(state, b"c_function\0" as *const _ as *const _);
        }
        0
    }

    let func = unsafe { lua.create_c_function(c_function)? };
    func.call::<()>(())?;
    assert_eq!(lua.globals().get::<bool>("c_function")?, true);

    Ok(())
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_recursion() -> Result<()> {
    let lua = Lua::new();

    let f = lua.create_function(move |lua, i: i32| {
        if i < 64 {
            lua.globals().get::<Function>("f")?.call::<()>(i + 1)?;
        }
        Ok(())
    })?;

    lua.globals().set("f", &f)?;
    f.call::<()>(1)?;

    Ok(())
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_too_many_returns() -> Result<()> {
    let lua = Lua::new();
    let f = lua.create_function(|_, ()| Ok(Variadic::from_iter(1..1000000)))?;
    assert!(f.call::<Variadic<u32>>(()).is_err());
    Ok(())
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_too_many_arguments() -> Result<()> {
    let lua = Lua::new();
    lua.load("function test(...) end").exec()?;
    let args = Variadic::from_iter(1..1000000);
    assert!(lua.globals().get::<Function>("test")?.call::<()>(args).is_err());
    Ok(())
}

#[test]
#[cfg(not(feature = "luajit"))]
#[cfg(not(target_arch = "wasm32"))]
fn test_too_many_recursions() -> Result<()> {
    let lua = Lua::new();

    let f = lua.create_function(move |lua, ()| lua.globals().get::<Function>("f")?.call::<()>(()))?;

    lua.globals().set("f", &f)?;
    assert!(f.call::<()>(()).is_err());

    Ok(())
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_ref_stack_exhaustion() {
    match catch_unwind(AssertUnwindSafe(|| -> Result<()> {
        let lua = Lua::new();
        let mut vals = Vec::new();
        for _ in 0..10000000 {
            vals.push(lua.create_table()?);
        }
        Ok(())
    })) {
        Ok(_) => panic!("no panic was detected"),
        Err(p) => assert!(p
            .downcast::<StdString>()
            .unwrap()
            .starts_with("cannot create a Lua reference, out of auxiliary stack space")),
    }
}

#[test]
fn test_large_args() -> Result<()> {
    let lua = Lua::new();
    let globals = lua.globals();

    globals.set(
        "c",
        lua.create_function(|_, args: Variadic<usize>| {
            let mut s = 0;
            for i in 0..args.len() {
                s += i;
                assert_eq!(i, args[i]);
            }
            Ok(s)
        })?,
    )?;

    let f: Function = lua
        .load(
            r#"
            return function(...)
                return c(...)
            end
        "#,
        )
        .eval()?;

    assert_eq!(f.call::<usize>((0..100).collect::<Variadic<usize>>())?, 4950);

    Ok(())
}

#[test]
fn test_large_args_ref() -> Result<()> {
    let lua = Lua::new();

    let f = lua.create_function(|_, args: Variadic<String>| {
        for i in 0..args.len() {
            assert_eq!(args[i], i.to_string());
        }
        Ok(())
    })?;

    f.call::<()>((0..100).map(|i| i.to_string()).collect::<Variadic<_>>())?;

    Ok(())
}

#[test]
fn test_chunk_env() -> Result<()> {
    let lua = Lua::new();

    let assert: Function = lua.globals().get("assert")?;

    let env1 = lua.create_table()?;
    env1.set("assert", assert.clone())?;

    let env2 = lua.create_table()?;
    env2.set("assert", assert)?;

    lua.load(
        r#"
        test_var = 1
    "#,
    )
    .set_environment(env1.clone())
    .exec()?;

    lua.load(
        r#"
        assert(test_var == nil)
        test_var = 2
    "#,
    )
    .set_environment(env2.clone())
    .exec()?;

    assert_eq!(lua.load("test_var").set_environment(env1).eval::<i32>()?, 1);
    assert_eq!(lua.load("test_var").set_environment(env2).eval::<i32>()?, 2);

    Ok(())
}

#[test]
fn test_context_thread() -> Result<()> {
    let lua = Lua::new();

    let f = lua
        .load(
            r#"
            local thread = ...
            assert(coroutine.running() == thread)
        "#,
        )
        .into_function()?;

    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luajit52"))]
    f.call::<()>(lua.current_thread())?;

    #[cfg(any(
        feature = "lua51",
        all(feature = "luajit", not(feature = "luajit52")),
        feature = "luau"
    ))]
    f.call::<()>(Nil)?;

    Ok(())
}

#[test]
#[cfg(any(feature = "lua51", all(feature = "luajit", not(feature = "luajit52"))))]
fn test_context_thread_51() -> Result<()> {
    let lua = Lua::new();

    let thread = lua.create_thread(
        lua.load(
            r#"
                function (thread)
                    assert(coroutine.running() == thread)
                end
            "#,
        )
        .eval()?,
    )?;

    thread.resume::<()>(thread.clone())?;

    Ok(())
}

#[test]
#[cfg(feature = "luajit")]
fn test_jit_version() -> Result<()> {
    let lua = Lua::new();
    let jit: Table = lua.globals().get("jit")?;
    assert!(jit.get::<String>("version")?.to_str()?.contains("LuaJIT"));
    Ok(())
}

#[test]
fn test_register_module() -> Result<()> {
    let lua = Lua::new();

    let t = lua.create_table()?;
    t.set("name", "my_module")?;
    lua.register_module("@my_module", &t)?;

    lua.load(
        r#"
        local my_module = require("@my_module")
        assert(my_module.name == "my_module")
    "#,
    )
    .exec()?;

    lua.unload_module("@my_module")?;
    lua.load(
        r#"
        local ok, err = pcall(function() return require("@my_module") end)
        assert(not ok)
        "#,
    )
    .exec()?;

    #[cfg(feature = "luau")]
    {
        // Luau registered modules must have '@' prefix
        let res = lua.register_module("my_module", 123);
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err().to_string(),
            "runtime error: module name must begin with '@'"
        );
    }

    Ok(())
}

#[test]
#[cfg(not(feature = "luau"))]
fn test_preload_module() -> Result<()> {
    let lua = Lua::new();

    let loader = lua.create_function(move |lua, modname: String| {
        let t = lua.create_table()?;
        t.set("name", modname)?;
        Ok(t)
    })?;

    lua.preload_module("@my_module", loader.clone())?;
    lua.load(
        r#"
        -- `my_module` is global for purposes of next test
        my_module = require("@my_module")
        assert(my_module.name == "@my_module")
        local my_module2 = require("@my_module")
        assert(my_module == my_module2)
    "#,
    )
    .exec()
    .unwrap();

    // Test unloading and loading again
    lua.unload_module("@my_module")?;
    lua.load(
        r#"
        local my_module3 = require("@my_module")
        -- `my_module` is not equal to `my_module3` because it was reloaded
        assert(my_module ~= my_module3)
        "#,
    )
    .exec()
    .unwrap();

    Ok(())
}

#[test]
fn test_inspect_stack() -> Result<()> {
    let lua = Lua::new();

    // Not inside any function
    assert!(lua.inspect_stack(0, |_| ()).is_none());

    let logline = lua.create_function(|lua, msg: StdString| {
        let r = lua
            .inspect_stack(1, |debug| {
                let source = debug.source().short_src;
                let source = source.as_deref().unwrap_or("?");
                let line = debug.current_line().unwrap();
                format!("{}:{} {}", source, line, msg)
            })
            .unwrap();
        Ok(r)
    })?;
    lua.globals().set("logline", logline)?;

    lua.load(
        r#"
        local function foo()
            local line = logline("hello")
            return line
        end
        local function bar()
            return foo()
        end

        assert(foo() == '[string "chunk"]:3 hello')
        assert(bar() == '[string "chunk"]:3 hello')
        assert(logline("world") == '[string "chunk"]:12 world')
    "#,
    )
    .set_name("chunk")
    .exec()?;

    let stack_info = lua.create_function(|lua, ()| {
        let stack_info = lua.inspect_stack(1, |debug| debug.stack()).unwrap();
        Ok(format!("{stack_info:?}"))
    })?;
    lua.globals().set("stack_info", stack_info)?;

    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luau"))]
    lua.load(
        r#"
        local stack_info = stack_info
        local function baz(a, b, c, ...)
            return stack_info()
        end
        assert(baz() == 'DebugStack { num_ups: 1, num_params: 3, is_vararg: true }')
    "#,
    )
    .exec()?;

    // LuaJIT does not pass this test for some reason
    #[cfg(feature = "lua51")]
    lua.load(
        r#"
        local stack_info = stack_info
        local function baz(a, b, c, ...)
            return stack_info()
        end
        assert(baz() == 'DebugStack { num_ups: 1 }')
    "#,
    )
    .exec()?;

    // Test retrieving currently running function
    let running_function =
        lua.create_function(|lua, ()| Ok(lua.inspect_stack(1, |debug| debug.function())))?;
    lua.globals().set("running_function", running_function)?;
    lua.load(
        r#"
        local function baz()
            return running_function()
        end
        if jit == nil then
            assert(baz() == baz)
        else
            -- luajit inline the "baz" function and returns the chunk itself
            assert(baz() == running_function())
        end
    "#,
    )
    .exec()?;

    Ok(())
}

#[test]
fn test_multi_states() -> Result<()> {
    let lua = Lua::new();

    let f = lua.create_function(|_, g: Option<Function>| {
        if let Some(g) = g {
            g.call::<()>(())?;
        }
        Ok(())
    })?;
    lua.globals().set("f", f)?;

    lua.load("f(function() coroutine.wrap(function() f() end)() end)")
        .exec()?;

    Ok(())
}

#[test]
#[cfg(feature = "lua54")]
fn test_warnings() -> Result<()> {
    let lua = Lua::new();
    lua.set_app_data::<Vec<(StdString, bool)>>(Vec::new());

    lua.set_warning_function(|lua, msg, incomplete| {
        lua.app_data_mut::<Vec<(StdString, bool)>>()
            .unwrap()
            .push((msg.to_string(), incomplete));
        Ok(())
    });

    lua.warning("native warning ...", true);
    lua.warning("finish", false);
    lua.warning("\0", false);
    lua.load(r#"warn("lua warning", "continue")"#).exec()?;

    lua.remove_warning_function();
    lua.warning("one more warning", false);

    let messages = lua.app_data_ref::<Vec<(StdString, bool)>>().unwrap();
    assert_eq!(
        *messages,
        vec![
            ("native warning ...".to_string(), true),
            ("finish".to_string(), false),
            ("".to_string(), false),
            ("lua warning".to_string(), true),
            ("continue".to_string(), false),
        ]
    );

    // Trigger error inside warning
    lua.set_warning_function(|_, _, _| Err(Error::runtime("warning error")));
    assert!(matches!(
        lua.load(r#"warn("test")"#).exec(),
        Err(Error::RuntimeError(ref err)) if err == "warning error"
    ));

    // Recursive warning
    lua.set_warning_function(|lua, _, _| {
        lua.warning("inner", false);
        Ok(())
    });
    lua.warning("hello", false);

    Ok(())
}

#[test]
#[cfg(feature = "luajit")]
fn test_luajit_cdata() -> Result<()> {
    let lua = unsafe { Lua::unsafe_new() };

    let cdata = lua
        .load(
            r#"
        local ffi = require("ffi")
        ffi.cdef[[
            void *malloc(size_t size);
            void free(void *ptr);
        ]]
        local ptr = ffi.C.malloc(1)
        ffi.C.free(ptr)
        return ptr
    "#,
        )
        .eval::<Value>()?;
    assert_eq!(cdata.type_name(), "other");
    assert!(cdata.to_string()?.starts_with("cdata<void *>:"));

    Ok(())
}

#[test]
#[cfg(feature = "send")]
#[cfg(not(target_arch = "wasm32"))]
fn test_multi_thread() -> Result<()> {
    let lua = Lua::new();

    lua.globals().set("i", 0)?;
    let func = lua.load("i = i + 1").into_function()?;

    std::thread::scope(|s| {
        s.spawn(|| {
            for _ in 0..5 {
                func.call::<()>(()).unwrap();
            }
        });
        s.spawn(|| {
            for _ in 0..5 {
                func.call::<()>(()).unwrap();
            }
        });
    });

    assert_eq!(lua.globals().get::<i32>("i")?, 10);

    Ok(())
}

#[test]
fn test_exec_raw() -> Result<()> {
    let lua = Lua::new();

    let sum = lua.create_function(|_, args: Variadic<i32>| {
        let mut sum = 0;
        for i in args {
            sum += i;
        }
        Ok(sum)
    })?;
    lua.globals().set("sum", sum)?;

    let n: i32 = unsafe {
        lua.exec_raw((), |state| {
            ffi::lua_getglobal(state, b"sum\0".as_ptr() as _);
            ffi::lua_pushinteger(state, 1);
            ffi::lua_pushinteger(state, 7);
            ffi::lua_call(state, 2, 1);
        })
    }?;
    assert_eq!(n, 8);

    // Test error handling
    let res: Result<()> = unsafe {
        lua.exec_raw("test error", |state| {
            ffi::lua_error(state);
        })
    };
    assert!(matches!(res, Err(Error::RuntimeError(err)) if err.contains("test error")));

    Ok(())
}

#[test]
fn test_gc_drop_ref_thread() -> Result<()> {
    let lua = Lua::new();

    let t = lua.create_table()?;
    lua.create_function(move |_, ()| {
        _ = &t;
        Ok(())
    })?;

    for _ in 0..10000 {
        // GC will run eventually to collect the function and the table above
        lua.create_table()?;
    }

    Ok(())
}

#[cfg(not(feature = "luau"))]
#[test]
fn test_get_or_init_from_ptr() -> Result<()> {
    // This would not work with Luau, the state must be init by mlua internally
    let state = unsafe { ffi::luaL_newstate() };

    let mut lua = unsafe { Lua::get_or_init_from_ptr(state) };
    lua.globals().set("hello", "world678")?;

    // The same Lua instance must be returned
    lua = unsafe { Lua::get_or_init_from_ptr(state) };
    assert_eq!(lua.globals().get::<String>("hello")?, "world678");

    unsafe { ffi::lua_close(state) };

    // Lua must not be accessed after closing

    Ok(())
}
