extern crate failure;
extern crate rlua;

use std::iter::FromIterator;
use std::panic::catch_unwind;
use std::sync::Arc;
use std::{error, f32, f64, fmt};

use failure::err_msg;
use rlua::{
    Error, ExternalError, Function, Lua, Nil, Result, String, Table, UserData, Value, Variadic,
};

#[test]
fn test_load() {
    let lua = Lua::new();
    let func = lua.load("return 1+2", None).unwrap();
    let result: i32 = func.call(()).unwrap();
    assert_eq!(result, 3);

    assert!(lua.load("§$%§&$%&", None).is_err());
}

#[test]
fn test_debug() {
    let lua = unsafe { Lua::new_with_debug() };
    match lua.eval("debug", None).unwrap() {
        Value::Table(_) => {}
        val => panic!("Expected table for debug library, got {:#?}", val),
    }
    let traceback_output = lua.eval::<_, String>("debug.traceback()", None).unwrap();
    assert_eq!(
        traceback_output.to_str().unwrap().split("\n").next(),
        "stack traceback:".into()
    );
}

#[test]
fn test_exec() {
    let lua = Lua::new();
    let globals = lua.globals();
    lua.exec::<_, ()>(
        r#"
            res = 'foo'..'bar'
        "#,
        None,
    )
    .unwrap();
    assert_eq!(globals.get::<_, String>("res").unwrap(), "foobar");

    let module: Table = lua
        .exec(
            r#"
            local module = {}

            function module.func()
                return "hello"
            end

            return module
        "#,
            None,
        )
        .unwrap();
    assert!(module.contains_key("func").unwrap());
    assert_eq!(
        module
            .get::<_, Function>("func")
            .unwrap()
            .call::<_, String>(())
            .unwrap(),
        "hello"
    );
}

#[test]
fn test_eval() {
    let lua = Lua::new();
    assert_eq!(lua.eval::<_, i32>("1 + 1", None).unwrap(), 2);
    assert_eq!(lua.eval::<_, bool>("false == false", None).unwrap(), true);
    assert_eq!(lua.eval::<_, i32>("return 1 + 2", None).unwrap(), 3);
    match lua.eval::<_, ()>("if true then", None) {
        Err(Error::SyntaxError {
            incomplete_input: true,
            ..
        }) => {}
        r => panic!(
            "expected SyntaxError with incomplete_input=true, got {:?}",
            r
        ),
    }
}

#[test]
fn test_lua_multi() {
    let lua = Lua::new();
    let globals = lua.globals();
    lua.exec::<_, ()>(
        r#"
            function concat(arg1, arg2)
                return arg1 .. arg2
            end

            function mreturn()
                return 1, 2, 3, 4, 5, 6
            end
        "#,
        None,
    )
    .unwrap();

    let concat = globals.get::<_, Function>("concat").unwrap();
    let mreturn = globals.get::<_, Function>("mreturn").unwrap();

    assert_eq!(concat.call::<_, String>(("foo", "bar")).unwrap(), "foobar");
    let (a, b) = mreturn.call::<_, (u64, u64)>(()).unwrap();
    assert_eq!((a, b), (1, 2));
    let (a, b, v) = mreturn.call::<_, (u64, u64, Variadic<u64>)>(()).unwrap();
    assert_eq!((a, b), (1, 2));
    assert_eq!(v[..], [3, 4, 5, 6]);
}

#[test]
fn test_coercion() {
    let lua = Lua::new();
    let globals = lua.globals();
    lua.exec::<_, ()>(
        r#"
            int = 123
            str = "123"
            num = 123.0
        "#,
        None,
    )
    .unwrap();

    assert_eq!(globals.get::<_, String>("int").unwrap(), "123");
    assert_eq!(globals.get::<_, i32>("str").unwrap(), 123);
    assert_eq!(globals.get::<_, i32>("num").unwrap(), 123);
}

#[test]
fn test_error() {
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

        fn cause(&self) -> Option<&error::Error> {
            None
        }
    }

    let lua = Lua::new();
    let globals = lua.globals();
    lua.exec::<_, ()>(
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
        None,
    )
    .unwrap();

    let rust_error_function = lua
        .create_function(|_, ()| -> Result<()> { Err(TestError.to_lua_err()) })
        .unwrap();
    globals
        .set("rust_error_function", rust_error_function)
        .unwrap();

    let no_error = globals.get::<_, Function>("no_error").unwrap();
    let lua_error = globals.get::<_, Function>("lua_error").unwrap();
    let rust_error = globals.get::<_, Function>("rust_error").unwrap();
    let return_error = globals.get::<_, Function>("return_error").unwrap();
    let return_string_error = globals.get::<_, Function>("return_string_error").unwrap();
    let test_pcall = globals.get::<_, Function>("test_pcall").unwrap();
    let understand_recursion = globals.get::<_, Function>("understand_recursion").unwrap();

    assert!(no_error.call::<_, ()>(()).is_ok());
    match lua_error.call::<_, ()>(()) {
        Err(Error::RuntimeError(_)) => {}
        Err(_) => panic!("error is not RuntimeError kind"),
        _ => panic!("error not returned"),
    }
    match rust_error.call::<_, ()>(()) {
        Err(Error::CallbackError { .. }) => {}
        Err(_) => panic!("error is not CallbackError kind"),
        _ => panic!("error not returned"),
    }

    match return_error.call::<_, Value>(()) {
        Ok(Value::Error(_)) => {}
        _ => panic!("Value::Error not returned"),
    }

    assert!(return_string_error.call::<_, Error>(()).is_ok());

    match lua.eval::<_, ()>("if youre happy and you know it syntax error", None) {
        Err(Error::SyntaxError {
            incomplete_input: false,
            ..
        }) => {}
        Err(_) => panic!("error is not LuaSyntaxError::Syntax kind"),
        _ => panic!("error not returned"),
    }
    match lua.eval::<_, ()>("function i_will_finish_what_i()", None) {
        Err(Error::SyntaxError {
            incomplete_input: true,
            ..
        }) => {}
        Err(_) => panic!("error is not LuaSyntaxError::IncompleteStatement kind"),
        _ => panic!("error not returned"),
    }

    test_pcall.call::<_, ()>(()).unwrap();

    assert!(understand_recursion.call::<_, ()>(()).is_err());

    match catch_unwind(|| -> Result<()> {
        let lua = Lua::new();
        let globals = lua.globals();

        lua.exec::<_, ()>(
            r#"
                function rust_panic()
                    pcall(function () rust_panic_function() end)
                end
            "#,
            None,
        )?;
        let rust_panic_function = lua
            .create_function(|_, ()| -> Result<()> { panic!("test_panic") })
            .unwrap();
        globals.set("rust_panic_function", rust_panic_function)?;

        let rust_panic = globals.get::<_, Function>("rust_panic")?;

        rust_panic.call::<_, ()>(())
    }) {
        Ok(Ok(_)) => panic!("no panic was detected, pcall caught it!"),
        Ok(Err(e)) => panic!("error during panic test {:?}", e),
        Err(p) => assert!(*p.downcast::<&str>().unwrap() == "test_panic"),
    };

    match catch_unwind(|| -> Result<()> {
        let lua = Lua::new();
        let globals = lua.globals();

        lua.exec::<_, ()>(
            r#"
                function rust_panic()
                    xpcall(function() rust_panic_function() end, function() end)
                end
            "#,
            None,
        )?;
        let rust_panic_function = lua
            .create_function(|_, ()| -> Result<()> { panic!("test_panic") })
            .unwrap();
        globals.set("rust_panic_function", rust_panic_function)?;

        let rust_panic = globals.get::<_, Function>("rust_panic")?;

        rust_panic.call::<_, ()>(())
    }) {
        Ok(Ok(_)) => panic!("no panic was detected, xpcall caught it!"),
        Ok(Err(e)) => panic!("error during panic test {:?}", e),
        Err(p) => assert!(*p.downcast::<&str>().unwrap() == "test_panic"),
    };
}

#[test]
fn test_result_conversions() {
    let lua = Lua::new();
    let globals = lua.globals();

    let err = lua
        .create_function(|_, ()| {
            Ok(Err::<String, _>(
                err_msg("only through failure can we succeed").to_lua_err(),
            ))
        })
        .unwrap();
    let ok = lua
        .create_function(|_, ()| Ok(Ok::<_, Error>("!".to_owned())))
        .unwrap();

    globals.set("err", err).unwrap();
    globals.set("ok", ok).unwrap();

    lua.exec::<_, ()>(
        r#"
            local r, e = err()
            assert(r == nil)
            assert(tostring(e) == "only through failure can we succeed")

            local r, e = ok()
            assert(r == "!")
            assert(e == nil)
        "#,
        None,
    )
    .unwrap();
}

#[test]
fn test_num_conversion() {
    let lua = Lua::new();

    assert_eq!(
        lua.coerce_integer(Value::String(lua.create_string("1").unwrap())),
        Some(1)
    );
    assert_eq!(
        lua.coerce_integer(Value::String(lua.create_string("1.0").unwrap())),
        Some(1)
    );
    assert_eq!(
        lua.coerce_integer(Value::String(lua.create_string("1.5").unwrap())),
        None
    );

    assert_eq!(
        lua.coerce_number(Value::String(lua.create_string("1").unwrap())),
        Some(1.0)
    );
    assert_eq!(
        lua.coerce_number(Value::String(lua.create_string("1.0").unwrap())),
        Some(1.0)
    );
    assert_eq!(
        lua.coerce_number(Value::String(lua.create_string("1.5").unwrap())),
        Some(1.5)
    );

    assert_eq!(lua.eval::<_, i64>("1.0", None).unwrap(), 1);
    assert_eq!(lua.eval::<_, f64>("1.0", None).unwrap(), 1.0);
    assert_eq!(lua.eval::<_, String>("1.0", None).unwrap(), "1.0");

    assert_eq!(lua.eval::<_, i64>("1.5", None).unwrap(), 1);
    assert_eq!(lua.eval::<_, f64>("1.5", None).unwrap(), 1.5);
    assert_eq!(lua.eval::<_, String>("1.5", None).unwrap(), "1.5");

    assert!(lua.eval::<_, u64>("-1", None).is_err());
    assert_eq!(lua.eval::<_, i64>("-1", None).unwrap(), -1);

    assert!(lua.unpack::<u64>(lua.pack(1u128 << 64).unwrap()).is_err());
    assert!(lua.eval::<_, i64>("math.huge", None).is_err());

    assert_eq!(
        lua.unpack::<f64>(lua.pack(f32::MAX).unwrap()).unwrap(),
        f32::MAX as f64
    );
    assert!(lua.unpack::<f32>(lua.pack(f64::MAX).unwrap()).is_err());

    assert_eq!(
        lua.unpack::<i128>(lua.pack(1i128 << 64).unwrap()).unwrap(),
        1i128 << 64
    );
}

#[test]
fn test_pcall_xpcall() {
    let lua = Lua::new();
    let globals = lua.globals();

    // make sure that we handle not enough arguments

    assert!(lua.exec::<_, ()>("pcall()", None).is_err());
    assert!(lua.exec::<_, ()>("xpcall()", None).is_err());
    assert!(lua.exec::<_, ()>("xpcall(function() end)", None).is_err());

    // Make sure that the return values from are correct on success

    let (r, e) = lua
        .eval::<_, (bool, String)>("pcall(function(p) return p end, 'foo')", None)
        .unwrap();
    assert!(r);
    assert_eq!(e, "foo");

    let (r, e) = lua
        .eval::<_, (bool, String)>("xpcall(function(p) return p end, print, 'foo')", None)
        .unwrap();
    assert!(r);
    assert_eq!(e, "foo");

    // Make sure that the return values are correct on errors, and that error handling works

    lua.exec::<_, ()>(
        r#"
            pcall_error = nil
            pcall_status, pcall_error = pcall(error, "testerror")

            xpcall_error = nil
            xpcall_status, _ = xpcall(error, function(err) xpcall_error = err end, "testerror")
        "#,
        None,
    )
    .unwrap();

    assert_eq!(globals.get::<_, bool>("pcall_status").unwrap(), false);
    assert_eq!(
        globals.get::<_, String>("pcall_error").unwrap(),
        "testerror"
    );

    assert_eq!(globals.get::<_, bool>("xpcall_statusr").unwrap(), false);
    assert_eq!(
        globals.get::<_, String>("xpcall_error").unwrap(),
        "testerror"
    );

    // Make sure that weird xpcall error recursion at least doesn't cause unsafety or panics.
    lua.exec::<_, ()>(
        r#"
            function xpcall_recursion()
                xpcall(error, function(err) error(err) end, "testerror")
            end
        "#,
        None,
    )
    .unwrap();
    let _ = globals
        .get::<_, Function>("xpcall_recursion")
        .unwrap()
        .call::<_, ()>(());
}

#[test]
fn test_recursive_mut_callback_error() {
    let lua = Lua::new();

    let mut v = Some(Box::new(123));
    let f = lua
        .create_function_mut::<_, (), _>(move |lua, mutate: bool| {
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
        })
        .unwrap();
    lua.globals().set("f", f).unwrap();
    match lua
        .globals()
        .get::<_, Function>("f")
        .unwrap()
        .call::<_, ()>(false)
    {
        Err(Error::CallbackError { ref cause, .. }) => match *cause.as_ref() {
            Error::CallbackError { ref cause, .. } => match *cause.as_ref() {
                Error::RecursiveMutCallback { .. } => {}
                ref other => panic!("incorrect result: {:?}", other),
            },
            ref other => panic!("incorrect result: {:?}", other),
        },
        other => panic!("incorrect result: {:?}", other),
    };
}

#[test]
fn test_set_metatable_nil() {
    let lua = Lua::new();
    lua.exec::<_, ()>(
        r#"
        a = {}
        setmetatable(a, nil)
    "#,
        None,
    )
    .unwrap();
}

#[test]
fn test_gc_error() {
    let lua = Lua::new();
    match lua.exec::<_, ()>(
        r#"
                val = nil
                table = {}
                setmetatable(table, {
                    __gc = function()
                        error("gcwascalled")
                    end
                })
                table = nil
                collectgarbage("collect")
            "#,
        None,
    ) {
        Err(Error::GarbageCollectorError(_)) => {}
        Err(e) => panic!("__gc error did not result in correct error, instead: {}", e),
        Ok(()) => panic!("__gc error did not result in error"),
    }
}

#[test]
fn test_named_registry_value() {
    let lua = Lua::new();

    lua.set_named_registry_value::<i32>("test", 42).unwrap();
    let f = lua
        .create_function(move |lua, ()| {
            assert_eq!(lua.named_registry_value::<i32>("test")?, 42);
            Ok(())
        })
        .unwrap();

    f.call::<_, ()>(()).unwrap();

    lua.unset_named_registry_value("test").unwrap();
    match lua.named_registry_value("test").unwrap() {
        Nil => {}
        val => panic!("registry value was not Nil, was {:?}", val),
    };
}

#[test]
fn test_registry_value() {
    let lua = Lua::new();

    let mut r = Some(lua.create_registry_value::<i32>(42).unwrap());
    let f = lua
        .create_function_mut(move |lua, ()| {
            if let Some(r) = r.take() {
                assert_eq!(lua.registry_value::<i32>(&r)?, 42);
                lua.remove_registry_value(r).unwrap();
            } else {
                panic!();
            }
            Ok(())
        })
        .unwrap();

    f.call::<_, ()>(()).unwrap();
}

#[test]
fn test_drop_registry_value() {
    struct MyUserdata(Arc<()>);

    impl UserData for MyUserdata {}

    let lua = Lua::new();

    let rc = Arc::new(());

    let r = lua.create_registry_value(MyUserdata(rc.clone())).unwrap();
    assert_eq!(Arc::strong_count(&rc), 2);

    drop(r);
    lua.expire_registry_values();

    lua.exec::<_, ()>(r#"collectgarbage("collect")"#, None)
        .unwrap();

    assert_eq!(Arc::strong_count(&rc), 1);
}

#[test]
#[should_panic]
fn test_mismatched_lua_ref() {
    let lua1 = Lua::new();
    let lua2 = Lua::new();

    let s = lua1.create_string("hello").unwrap();
    let f = lua2.create_function(|_, _: String| Ok(())).unwrap();

    f.call::<_, ()>(s).unwrap();
}

#[test]
fn test_lua_registry_ownership() {
    let lua1 = Lua::new();
    let lua2 = Lua::new();

    let r1 = lua1.create_registry_value("hello").unwrap();
    let r2 = lua2.create_registry_value("hello").unwrap();

    assert!(lua1.owns_registry_value(&r1));
    assert!(!lua2.owns_registry_value(&r1));
    assert!(lua2.owns_registry_value(&r2));
    assert!(!lua1.owns_registry_value(&r2));
}

#[test]
fn test_mismatched_registry_key() {
    let lua1 = Lua::new();
    let lua2 = Lua::new();

    let r = lua1.create_registry_value("hello").unwrap();
    match lua2.remove_registry_value(r) {
        Err(Error::MismatchedRegistryKey) => {}
        r => panic!("wrong result type for mismatched registry key, {:?}", r),
    };
}

#[test]
fn too_many_returns() {
    let lua = Lua::new();
    let f = lua
        .create_function(|_, ()| Ok(Variadic::from_iter(1..1000000)))
        .unwrap();
    assert!(f.call::<_, Vec<u32>>(()).is_err());
}

#[test]
fn too_many_arguments() {
    let lua = Lua::new();
    lua.exec::<_, ()>("function test(...) end", None).unwrap();
    let args = Variadic::from_iter(1..1000000);
    assert!(lua
        .globals()
        .get::<_, Function>("test")
        .unwrap()
        .call::<_, ()>(args)
        .is_err());
}

#[test]
fn too_many_recursions() {
    let lua = Lua::new();

    let f = lua
        .create_function(move |lua, ()| lua.globals().get::<_, Function>("f")?.call::<_, ()>(()))
        .unwrap();
    lua.globals().set("f", f).unwrap();

    assert!(lua
        .globals()
        .get::<_, Function>("f")
        .unwrap()
        .call::<_, ()>(())
        .is_err());
}

#[test]
fn too_many_binds() {
    let lua = Lua::new();
    let globals = lua.globals();
    lua.exec::<_, ()>(
        r#"
            function f(...)
            end
        "#,
        None,
    )
    .unwrap();

    let concat = globals.get::<_, Function>("f").unwrap();
    assert!(concat.bind(Variadic::from_iter(1..1000000)).is_err());
    assert!(concat
        .call::<_, ()>(Variadic::from_iter(1..1000000))
        .is_err());
}

#[test]
fn large_args() {
    let lua = Lua::new();
    let globals = lua.globals();

    globals
        .set(
            "c",
            lua.create_function(|_, args: Variadic<usize>| {
                let mut s = 0;
                for i in 0..args.len() {
                    s += i;
                    assert_eq!(i, args[i]);
                }
                Ok(s)
            })
            .unwrap(),
        )
        .unwrap();

    let f: Function = lua
        .eval(
            r#"
            return function(...)
                return c(...)
            end
        "#,
            None,
        )
        .unwrap();

    assert_eq!(
        f.call::<_, usize>((0..100).collect::<Variadic<usize>>())
            .unwrap(),
        4950
    );
}

#[test]
fn large_args_ref() {
    let lua = Lua::new();

    let f = lua
        .create_function(|_, args: Variadic<String>| {
            for i in 0..args.len() {
                assert_eq!(args[i], i.to_string());
            }
            Ok(())
        })
        .unwrap();

    f.call::<_, ()>((0..100).map(|i| i.to_string()).collect::<Variadic<_>>())
        .unwrap();
}
