#![cfg_attr(
    all(feature = "luajit", target_os = "macos", target_arch = "x86_64"),
    feature(link_args)
)]

#[cfg_attr(
    all(feature = "luajit", target_os = "macos", target_arch = "x86_64"),
    link_args = "-pagezero_size 10000 -image_base 100000000",
    allow(unused_attributes)
)]
extern "system" {}

use std::iter::FromIterator;
use std::panic::catch_unwind;
use std::sync::Arc;
use std::{error, f32, f64, fmt};

use mlua::{
    ChunkMode, Error, ExternalError, Function, Lua, Nil, Result, StdLib, String, Table, UserData,
    Value, Variadic,
};

#[test]
fn test_safety() -> Result<()> {
    let lua = Lua::new();
    assert!(lua.load(r#"require "debug""#).exec().is_err());
    match lua.load_from_std_lib(StdLib::DEBUG) {
        Err(Error::SafetyError(_)) => {}
        Err(e) => panic!("expected SafetyError, got {:?}", e),
        Ok(_) => panic!("expected SafetyError, got no error"),
    }
    drop(lua);

    let lua = unsafe { Lua::unsafe_new() };
    assert!(lua.load(r#"require "debug""#).exec().is_ok());
    drop(lua);

    match Lua::new_with(StdLib::DEBUG) {
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

    match lua.load("1 + 1").set_mode(ChunkMode::Binary).exec() {
        Err(Error::SafetyError(msg)) => {
            assert!(msg.contains("binary chunks are disabled in safe mode"))
        }
        Err(e) => panic!("expected SafetyError, got {:?}", e),
        Ok(_) => panic!("expected SafetyError, got no error"),
    }

    let bytecode = lua.load("return 1 + 1").into_function()?.dump(true)?;
    match lua.load(&bytecode).exec() {
        Err(Error::SafetyError(msg)) => {
            assert!(msg.contains("binary chunks are disabled in safe mode"))
        }
        Err(e) => panic!("expected SafetyError, got {:?}", e),
        Ok(_) => panic!("expected SafetyError, got no error"),
    }
    drop(lua);

    // Test safety rules after dynamically loading `package` library
    let lua = Lua::new_with(StdLib::NONE)?;
    assert!(lua.globals().get::<_, Option<Value>>("require")?.is_none());
    lua.load_from_std_lib(StdLib::PACKAGE)?;
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
    let func = lua.load("return 1+2").into_function()?;
    let result: i32 = func.call(())?;
    assert_eq!(result, 3);

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
    assert_eq!(globals.get::<_, String>("res")?, "foobar");

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
    assert_eq!(
        module.get::<_, Function>("func")?.call::<_, String>(())?,
        "hello"
    );

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
        r => panic!(
            "expected SyntaxError with incomplete_input=true, got {:?}",
            r
        ),
    }

    Ok(())
}

#[test]
fn test_load_mode() -> Result<()> {
    let lua = unsafe { Lua::unsafe_new() };

    assert_eq!(
        lua.load("1 + 1").set_mode(ChunkMode::Text).eval::<i32>()?,
        2
    );
    match lua.load("1 + 1").set_mode(ChunkMode::Binary).exec() {
        Ok(_) => panic!("expected SyntaxError, got no error"),
        Err(Error::SyntaxError { message: msg, .. }) => {
            assert!(msg.contains("attempt to load a text chunk"))
        }
        Err(e) => panic!("expected SyntaxError, got {:?}", e),
    };

    let bytecode = lua.load("return 1 + 1").into_function()?.dump(true)?;
    assert_eq!(lua.load(&bytecode).eval::<i32>()?, 2);
    assert_eq!(
        lua.load(&bytecode)
            .set_mode(ChunkMode::Binary)
            .eval::<i32>()?,
        2
    );
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
    let concat = globals.get::<_, Function>("concat")?;
    let mreturn = globals.get::<_, Function>("mreturn")?;

    assert_eq!(concat.call::<_, String>(("foo", "bar"))?, "foobar");
    let (a, b) = mreturn.call::<_, (u64, u64)>(())?;
    assert_eq!((a, b), (1, 2));
    let (a, b, v) = mreturn.call::<_, (u64, u64, Variadic<u64>)>(())?;
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
    "#,
    )
    .exec()?;

    let globals = lua.globals();
    assert_eq!(globals.get::<_, String>("int")?, "123");
    assert_eq!(globals.get::<_, i32>("str")?, 123);
    assert_eq!(globals.get::<_, i32>("num")?, 123);

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

    impl error::Error for TestError {
        fn description(&self) -> &str {
            "test error"
        }

        fn cause(&self) -> Option<&dyn error::Error> {
            None
        }
    }

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
                if string.match(_VERSION, ' 5%.1$') or string.match(_VERSION, ' 5%.2$') then
                    -- Special case for Lua 5.1/5.2
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

    let rust_error_function =
        lua.create_function(|_, ()| -> Result<()> { Err(TestError.to_lua_err()) })?;
    globals.set("rust_error_function", rust_error_function)?;

    let no_error = globals.get::<_, Function>("no_error")?;
    let lua_error = globals.get::<_, Function>("lua_error")?;
    let rust_error = globals.get::<_, Function>("rust_error")?;
    let return_error = globals.get::<_, Function>("return_error")?;
    let return_string_error = globals.get::<_, Function>("return_string_error")?;
    let test_pcall = globals.get::<_, Function>("test_pcall")?;
    let understand_recursion = globals.get::<_, Function>("understand_recursion")?;

    assert!(no_error.call::<_, ()>(()).is_ok());
    match lua_error.call::<_, ()>(()) {
        Err(Error::RuntimeError(_)) => {}
        Err(e) => panic!("error is not RuntimeError kind, got {:?}", e),
        _ => panic!("error not returned"),
    }
    match rust_error.call::<_, ()>(()) {
        Err(Error::CallbackError { .. }) => {}
        Err(e) => panic!("error is not CallbackError kind, got {:?}", e),
        _ => panic!("error not returned"),
    }

    match return_error.call::<_, Value>(()) {
        Ok(Value::Error(_)) => {}
        _ => panic!("Value::Error not returned"),
    }

    assert!(return_string_error.call::<_, Error>(()).is_ok());

    match lua
        .load("if youre happy and you know it syntax error")
        .exec()
    {
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

    test_pcall.call::<_, ()>(())?;

    assert!(understand_recursion.call::<_, ()>(()).is_err());

    match catch_unwind(|| -> Result<()> {
        let lua = Lua::new();
        let globals = lua.globals();

        lua.load(
            r#"
            function rust_panic()
                local _, err = pcall(function () rust_panic_function() end)
                if err ~= nil then
                    error(err)
                end
            end
        "#,
        )
        .exec()?;
        let rust_panic_function =
            lua.create_function(|_, ()| -> Result<()> { panic!("test_panic") })?;
        globals.set("rust_panic_function", rust_panic_function)?;

        let rust_panic = globals.get::<_, Function>("rust_panic")?;

        rust_panic.call::<_, ()>(())
    }) {
        Ok(Ok(_)) => panic!("no panic was detected"),
        Ok(Err(e)) => panic!("error during panic test {:?}", e),
        Err(p) => assert!(*p.downcast::<&str>().unwrap() == "test_panic"),
    };

    match catch_unwind(|| -> Result<()> {
        let lua = Lua::new();
        let globals = lua.globals();

        lua.load(
            r#"
            function rust_panic()
                local _, err = pcall(function () rust_panic_function() end)
                if err ~= nil then
                    error(tostring(err))
                end
            end
        "#,
        )
        .exec()?;
        let rust_panic_function =
            lua.create_function(|_, ()| -> Result<()> { panic!("test_panic") })?;
        globals.set("rust_panic_function", rust_panic_function)?;

        let rust_panic = globals.get::<_, Function>("rust_panic")?;

        rust_panic.call::<_, ()>(())
    }) {
        Ok(Ok(_)) => panic!("no error was detected"),
        Ok(Err(Error::RuntimeError(_))) => {}
        Ok(Err(e)) => panic!("unexpected error during panic test {:?}", e),
        Err(_) => panic!("panic was detected"),
    };

    Ok(())
}

#[test]
fn test_result_conversions() -> Result<()> {
    let lua = Lua::new();
    let globals = lua.globals();

    let err = lua.create_function(|_, ()| {
        Ok(Err::<String, _>(
            "only through failure can we succeed".to_lua_err(),
        ))
    })?;
    let ok = lua.create_function(|_, ()| Ok(Ok::<_, Error>("!".to_owned())))?;

    globals.set("err", err)?;
    globals.set("ok", ok)?;

    lua.load(
        r#"
        local r, e = err()
        assert(r == nil)
        assert(tostring(e):find("only through failure can we succeed") ~= nil)

        local r, e = ok()
        assert(r == "!")
        assert(e == nil)
    "#,
    )
    .exec()?;

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
    #[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
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

    // Lua 5.3/5.2 / LuaJIT compatible version of xpcall
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

    assert_eq!(globals.get::<_, bool>("pcall_status")?, false);
    assert_eq!(globals.get::<_, String>("pcall_error")?, "testerror");

    assert_eq!(globals.get::<_, bool>("xpcall_statusr")?, false);
    #[cfg(any(
        feature = "lua54",
        feature = "lua53",
        feature = "lua52",
        feature = "luajit"
    ))]
    assert_eq!(
        globals.get::<_, std::string::String>("xpcall_error")?,
        "testerror"
    );
    #[cfg(feature = "lua51")]
    assert!(globals
        .get::<_, String>("xpcall_error")?
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
    let _ = globals
        .get::<_, Function>("xpcall_recursion")?
        .call::<_, ()>(());

    Ok(())
}

#[test]
fn test_recursive_mut_callback_error() -> Result<()> {
    let lua = Lua::new();

    let mut v = Some(Box::new(123));
    let f = lua.create_function_mut::<_, (), _>(move |lua, mutate: bool| {
        if mutate {
            v = None;
        } else {
            // Produce a mutable reference
            let r = v.as_mut().unwrap();
            // Whoops, this will recurse into the function and produce another mutable reference!
            lua.globals().get::<_, Function>("f")?.call::<_, ()>(true)?;
            println!("Should not get here, mutable aliasing has occurred!");
            println!("value at {:p}", r as *mut _);
            println!("value is {}", r);
        }

        Ok(())
    })?;
    lua.globals().set("f", f)?;
    match lua.globals().get::<_, Function>("f")?.call::<_, ()>(false) {
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

    lua.set_named_registry_value::<_, i32>("test", 42)?;
    let f = lua.create_function(move |lua, ()| {
        assert_eq!(lua.named_registry_value::<_, i32>("test")?, 42);
        Ok(())
    })?;

    f.call::<_, ()>(())?;

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

    let mut r = Some(lua.create_registry_value::<i32>(42)?);
    let f = lua.create_function_mut(move |lua, ()| {
        if let Some(r) = r.take() {
            assert_eq!(lua.registry_value::<i32>(&r)?, 42);
            lua.remove_registry_value(r).unwrap();
        } else {
            panic!();
        }
        Ok(())
    })?;

    f.call::<_, ()>(())?;

    Ok(())
}

#[test]
fn test_drop_registry_value() -> Result<()> {
    struct MyUserdata(Arc<()>);

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
fn too_many_returns() -> Result<()> {
    let lua = Lua::new();
    let f = lua.create_function(|_, ()| Ok(Variadic::from_iter(1..1000000)))?;
    assert!(f.call::<_, Vec<u32>>(()).is_err());
    Ok(())
}

#[test]
fn too_many_arguments() -> Result<()> {
    let lua = Lua::new();
    lua.load("function test(...) end").exec()?;
    let args = Variadic::from_iter(1..1000000);
    assert!(lua
        .globals()
        .get::<_, Function>("test")?
        .call::<_, ()>(args)
        .is_err());
    Ok(())
}

#[test]
#[cfg(not(feature = "luajit"))]
fn too_many_recursions() -> Result<()> {
    let lua = Lua::new();
    let f = lua
        .create_function(move |lua, ()| lua.globals().get::<_, Function>("f")?.call::<_, ()>(()))?;
    lua.globals().set("f", f)?;

    assert!(lua
        .globals()
        .get::<_, Function>("f")?
        .call::<_, ()>(())
        .is_err());

    Ok(())
}

#[test]
fn too_many_binds() -> Result<()> {
    let lua = Lua::new();
    let globals = lua.globals();
    lua.load(
        r#"
        function f(...)
        end
    "#,
    )
    .exec()?;

    let concat = globals.get::<_, Function>("f")?;
    assert!(concat.bind(Variadic::from_iter(1..1000000)).is_err());
    assert!(concat
        .call::<_, ()>(Variadic::from_iter(1..1000000))
        .is_err());

    Ok(())
}

#[test]
fn large_args() -> Result<()> {
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

    assert_eq!(
        f.call::<_, usize>((0..100).collect::<Variadic<usize>>())?,
        4950
    );

    Ok(())
}

#[test]
fn large_args_ref() -> Result<()> {
    let lua = Lua::new();

    let f = lua.create_function(|_, args: Variadic<String>| {
        for i in 0..args.len() {
            assert_eq!(args[i], i.to_string());
        }
        Ok(())
    })?;

    f.call::<_, ()>((0..100).map(|i| i.to_string()).collect::<Variadic<_>>())?;

    Ok(())
}

#[test]
fn chunk_env() -> Result<()> {
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
    .set_environment(env1.clone())?
    .exec()?;

    lua.load(
        r#"
        assert(test_var == nil)
        test_var = 2
    "#,
    )
    .set_environment(env2.clone())?
    .exec()?;

    assert_eq!(
        lua.load("test_var").set_environment(env1)?.eval::<i32>()?,
        1
    );

    assert_eq!(
        lua.load("test_var").set_environment(env2)?.eval::<i32>()?,
        2
    );

    Ok(())
}

#[test]
fn context_thread() -> Result<()> {
    let lua = Lua::new();

    let f = lua
        .load(
            r#"
            local thread = ...
            assert(coroutine.running() == thread)
        "#,
        )
        .into_function()?;

    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    f.call::<_, ()>(lua.current_thread())?;

    #[cfg(any(feature = "lua51", feature = "luajit"))]
    f.call::<_, ()>(Nil)?;

    Ok(())
}

#[test]
#[cfg(any(feature = "lua51", feature = "luajit"))]
fn context_thread_51() -> Result<()> {
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

    thread.resume::<_, ()>(thread.clone())?;

    Ok(())
}

#[test]
#[cfg(feature = "luajit")]
fn test_jit_version() -> Result<()> {
    let lua = Lua::new();
    let jit: Table = lua.globals().get("jit")?;
    assert!(jit
        .get::<_, String>("version")?
        .to_str()?
        .contains("LuaJIT"));
    Ok(())
}
