use std::panic::catch_unwind;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use mlua::{Error, Function, IntoLua, Lua, Result, Thread, ThreadEvent, ThreadTriggers, Value};

#[test]
fn test_thread() -> Result<()> {
    let lua = Lua::new();

    let thread = lua.create_thread(
        lua.load(
            r#"
            function (s)
                local sum = s
                for i = 1,4 do
                    sum = sum + coroutine.yield(sum)
                end
                return sum
            end
            "#,
        )
        .eval()?,
    )?;

    assert!(thread.is_resumable());
    assert_eq!(thread.resume::<i64>(0)?, 0);
    assert!(thread.is_resumable());
    assert_eq!(thread.resume::<i64>(1)?, 1);
    assert!(thread.is_resumable());
    assert_eq!(thread.resume::<i64>(2)?, 3);
    assert!(thread.is_resumable());
    assert_eq!(thread.resume::<i64>(3)?, 6);
    assert!(thread.is_resumable());
    assert_eq!(thread.resume::<i64>(4)?, 10);
    assert!(thread.is_finished());

    let accumulate = lua.create_thread(
        lua.load(
            r#"
            function (sum)
                while true do
                    sum = sum + coroutine.yield(sum)
                end
            end
            "#,
        )
        .eval::<Function>()?,
    )?;

    for i in 0..4 {
        accumulate.resume::<()>(i)?;
    }
    assert_eq!(accumulate.resume::<i64>(4)?, 10);
    assert!(accumulate.is_resumable());
    assert!(accumulate.resume::<()>("error").is_err());
    assert!(accumulate.is_error());

    let thread = lua
        .load(
            r#"
            coroutine.create(function ()
                while true do
                    coroutine.yield(42)
                end
            end)
        "#,
        )
        .eval::<Thread>()?;
    assert!(thread.is_resumable());
    assert_eq!(thread.resume::<i64>(())?, 42);

    let thread: Thread = lua
        .load(
            r#"
            coroutine.create(function(arg)
                assert(arg == 42)
                local yieldarg = coroutine.yield(123)
                assert(yieldarg == 43)
                return 987
            end)
        "#,
        )
        .eval()?;

    assert_eq!(thread.resume::<u32>(42)?, 123);
    assert_eq!(thread.resume::<u32>(43)?, 987);

    match thread.resume::<u32>(()) {
        Err(Error::CoroutineUnresumable) => {}
        Err(_) => panic!("resuming dead coroutine error is not CoroutineInactive kind"),
        _ => panic!("resuming dead coroutine did not return error"),
    }

    // Already running thread must be unresumable
    let thread = lua.create_thread(lua.create_function(|lua, ()| {
        assert!(lua.current_thread().is_running());
        let result = lua.current_thread().resume::<()>(());
        assert!(
            matches!(result, Err(Error::CoroutineUnresumable)),
            "unexpected result: {result:?}",
        );
        Ok(())
    })?)?;
    let result = thread.resume::<()>(());
    assert!(result.is_ok(), "unexpected result: {result:?}");

    // A thread that has resumed another thread (still running) is "normal".
    let check_outer = lua.create_function(|lua, ()| {
        let outer: Thread = lua.globals().get("outer")?;
        assert!(outer.is_normal());
        assert!(
            matches!(outer.resume::<()>(()), Err(Error::CoroutineUnresumable)),
            "resuming a `normal` thread must be unresumable",
        );
        Ok(())
    })?;
    lua.globals().set("check_outer", check_outer)?;
    let outer = lua.create_thread(
        lua.load(
            r#"
            function()
                local inner = coroutine.create(function() check_outer() end)
                assert(coroutine.resume(inner))
            end
            "#,
        )
        .eval()?,
    )?;
    lua.globals().set("outer", &outer)?;
    outer.resume::<()>(())?;
    assert!(outer.is_finished());

    Ok(())
}

#[test]
fn test_thread_reset() -> Result<()> {
    use mlua::{AnyUserData, UserData};
    use std::sync::Arc;

    let lua = Lua::new();

    struct MyUserData(#[allow(unused)] Arc<()>);
    impl UserData for MyUserData {}

    let arc = Arc::new(());

    let func: Function = lua.load(r#"function(ud) coroutine.yield(ud) end"#).eval()?;
    let thread = lua.create_thread(lua.load("return 0").into_function()?)?; // Dummy function first
    assert!(thread.reset(func.clone()).is_ok());

    for _ in 0..2 {
        assert!(thread.is_resumable());
        let _ = thread.resume::<AnyUserData>(MyUserData(arc.clone()))?;
        assert!(thread.is_resumable());
        assert_eq!(Arc::strong_count(&arc), 2);
        thread.resume::<()>(())?;
        assert!(thread.is_finished());
        thread.reset(func.clone())?;
        lua.gc_collect()?;
        assert_eq!(Arc::strong_count(&arc), 1);
    }

    // Check for errors
    let func: Function = lua.load(r#"function(ud) error("test error") end"#).eval()?;
    let thread = lua.create_thread(func.clone())?;
    let _ = thread.resume::<AnyUserData>(MyUserData(arc.clone()));
    assert!(thread.is_error());
    assert_eq!(Arc::strong_count(&arc), 2);
    #[cfg(any(feature = "lua55", feature = "lua54"))]
    {
        assert!(thread.reset(func.clone()).is_err());
        // Reset behavior has changed in Lua v5.4.4
        // It's became possible to force reset thread by popping error object
        assert!(thread.is_finished());
        assert!(thread.reset(func.clone()).is_ok());
        assert!(thread.is_resumable());
    }
    #[cfg(any(feature = "lua55", feature = "lua54", feature = "luau"))]
    {
        assert!(thread.reset(func.clone()).is_ok());
        assert!(thread.is_resumable());
    }

    // Try reset running thread
    let thread = lua.create_thread(lua.create_function(|lua, ()| {
        let this = lua.current_thread();
        this.reset(lua.create_function(|_, ()| Ok(()))?)?;
        Ok(())
    })?)?;
    let result = thread.resume::<()>(());
    assert!(
        matches!(result, Err(Error::CallbackError{ ref cause, ..})
            if matches!(cause.as_ref(), Error::RuntimeError(err)
                if err == "cannot reset a running thread")
        ),
        "unexpected result: {result:?}",
    );

    Ok(())
}

#[test]
fn test_coroutine_from_closure() -> Result<()> {
    let lua = Lua::new();

    let thrd_main = lua.create_function(|_, ()| Ok(()))?;
    lua.globals().set("main", thrd_main)?;

    #[cfg(any(
        feature = "lua55",
        feature = "lua54",
        feature = "lua53",
        feature = "lua52",
        feature = "luajit",
        feature = "luau"
    ))]
    let thrd: Thread = lua.load("coroutine.create(main)").eval()?;
    #[cfg(feature = "lua51")]
    let thrd: Thread = lua
        .load("coroutine.create(function(...) return main(unpack(arg)) end)")
        .eval()?;

    thrd.resume::<()>(())?;

    Ok(())
}

#[test]
#[cfg(not(panic = "abort"))]
fn test_coroutine_panic() {
    match catch_unwind(|| -> Result<()> {
        // check that coroutines propagate panics correctly
        let lua = Lua::new();
        let thrd_main = lua.create_function(|_, ()| -> Result<()> {
            panic!("test_panic");
        })?;
        lua.globals().set("main", &thrd_main)?;
        let thrd: Thread = lua.create_thread(thrd_main)?;
        thrd.resume(())
    }) {
        Ok(r) => panic!("coroutine panic not propagated, instead returned {:?}", r),
        Err(p) => assert!(*p.downcast::<&str>().unwrap() == "test_panic"),
    }
}

#[test]
fn test_thread_pointer() -> Result<()> {
    let lua = Lua::new();

    let func = lua.load("return 123").into_function()?;
    let thread = lua.create_thread(func.clone())?;

    assert_eq!(thread.to_pointer(), thread.clone().to_pointer());
    assert_ne!(thread.to_pointer(), lua.current_thread().to_pointer());

    Ok(())
}

#[test]
#[cfg(feature = "luau")]
fn test_thread_resume_error() -> Result<()> {
    let lua = Lua::new();

    let thread = lua
        .load(
            r#"
        coroutine.create(function()
            local ok, err = pcall(coroutine.yield, 123)
            assert(not ok, "yield should fail")
            assert(err == "myerror", "unexpected error: " .. tostring(err))
            return "success"
        end)
    "#,
        )
        .eval::<Thread>()?;

    assert_eq!(thread.resume::<i64>(())?, 123);
    let status = thread.resume_error::<String>("myerror").unwrap();
    assert_eq!(status, "success");

    Ok(())
}

#[test]
fn test_thread_resume_bad_arg() -> Result<()> {
    let lua = Lua::new();

    struct BadArg;

    impl IntoLua for BadArg {
        fn into_lua(self, _lua: &Lua) -> Result<Value> {
            Err(Error::runtime("bad arg"))
        }
    }

    let f = lua.create_thread(lua.create_function(|_, ()| Ok("okay"))?)?;
    let res = f.resume::<()>((123, BadArg));
    assert!(matches!(res, Err(Error::RuntimeError(msg)) if msg == "bad arg"));
    let res = f.resume::<String>(()).unwrap();
    assert_eq!(res, "okay");

    Ok(())
}

#[test]
fn test_thread_event_create() -> Result<()> {
    let lua = Lua::new();

    let created = Arc::new(AtomicBool::new(false));
    let created2 = created.clone();
    lua.set_thread_event_callback(ThreadTriggers::ON_CREATE, move |_lua, event| {
        assert!(matches!(event, ThreadEvent::Create(_)));
        created2.store(true, Ordering::Relaxed);
        Ok(())
    });

    let _thread = lua.create_thread(lua.create_function(|_, ()| Ok(()))?)?;
    assert!(created.load(Ordering::Relaxed));

    Ok(())
}

#[test]
fn test_thread_event_create_recursive() -> Result<()> {
    let lua = Lua::new();

    let count = Arc::new(AtomicU32::new(0));
    let count2 = count.clone();
    lua.set_thread_event_callback(ThreadTriggers::ON_CREATE, move |lua, event| {
        assert!(matches!(event, ThreadEvent::Create(_)));
        count2.fetch_add(1, Ordering::Relaxed);
        // Creating a thread inside the callback
        let _ = lua.create_thread(lua.load("return 321").into_function().unwrap())?;
        Ok(())
    });

    let _t = lua.create_thread(lua.load("return 123").into_function()?)?;
    assert_eq!(count.load(Ordering::Relaxed), 1);

    Ok(())
}

#[test]
fn test_thread_event_create_error() -> Result<()> {
    let lua = Lua::new();

    lua.set_thread_event_callback(ThreadTriggers::ON_CREATE, move |_, _| Err(Error::runtime("blah")));

    let result = lua.create_thread(lua.load("return 123").into_function()?);
    assert!(result.is_err());
    assert!(matches!(result, Err(Error::RuntimeError(err)) if err.contains("blah")));

    Ok(())
}

#[test]
fn test_thread_event_resume() -> Result<()> {
    let lua = Lua::new();

    let count = Arc::new(AtomicBool::new(false));
    let count2 = count.clone();
    lua.set_thread_event_callback(ThreadTriggers::ON_RESUME, move |_lua, event| {
        assert!(matches!(event, ThreadEvent::Resume(_)));
        count2.store(true, Ordering::Relaxed);
        Ok(())
    });

    let thread = lua.create_thread(lua.load("return 42").into_function()?)?;
    thread.resume::<()>(())?;

    assert!(count.load(Ordering::Relaxed));
    Ok(())
}

#[test]
fn test_thread_event_resume_error() -> Result<()> {
    let lua = Lua::new();

    lua.set_thread_event_callback(ThreadTriggers::ON_RESUME, move |_lua, _event| {
        Err(Error::runtime("abort resume"))
    });

    let thread = lua.create_thread(lua.load("return 42").into_function()?)?;
    let err = thread.resume::<()>(()).unwrap_err();
    assert!(matches!(err, Error::RuntimeError(msg) if msg == "abort resume"));
    assert!(thread.is_resumable());

    Ok(())
}

#[test]
fn test_thread_event_yield() -> Result<()> {
    let lua = Lua::new();

    let count = Arc::new(AtomicBool::new(false));
    let count2 = count.clone();
    lua.set_thread_event_callback(ThreadTriggers::ON_YIELD, move |_lua, event| {
        assert!(matches!(event, ThreadEvent::Yield(_)));
        count2.store(true, Ordering::Relaxed);
        Ok(())
    });

    let thread = lua.create_thread(lua.load("coroutine.yield(1) return 2").into_function()?)?;
    let val = thread.resume::<i32>(())?;
    assert_eq!(val, 1);
    assert!(count.load(Ordering::Relaxed));

    // Reset flag and resume to completion
    count.store(false, Ordering::Relaxed);
    let val = thread.resume::<i32>(())?;
    assert_eq!(val, 2);
    // Yield hook should not fire on the final return
    assert!(!count.load(Ordering::Relaxed));
    assert!(thread.is_finished());

    Ok(())
}

#[test]
fn test_thread_event_yield_error() -> Result<()> {
    let lua = Lua::new();

    lua.set_thread_event_callback(ThreadTriggers::ON_YIELD, move |_lua, _event| {
        Err(Error::runtime("yield error"))
    });

    let thread = lua.create_thread(lua.load("coroutine.yield(1)").into_function()?)?;
    let err = thread.resume::<()>(()).unwrap_err();
    assert!(matches!(err, Error::RuntimeError(msg) if msg == "yield error"));

    Ok(())
}

#[test]
fn test_thread_event_swap() -> Result<()> {
    let lua = Lua::new();

    let count = Arc::new(AtomicU32::new(0));
    let count2 = count.clone();
    lua.set_thread_event_callback(ThreadTriggers::ON_RESUME, move |_lua, _event| {
        count2.fetch_add(1, Ordering::Relaxed);
        Ok(())
    });

    let thread = lua.create_thread(lua.load("coroutine.yield(1) return 2").into_function()?)?;
    thread.resume::<i32>(())?;
    assert_eq!(count.load(Ordering::Relaxed), 1);

    // Replace callback with a new one
    let count3 = Arc::new(AtomicU32::new(0));
    let count4 = count3.clone();
    lua.set_thread_event_callback(ThreadTriggers::new().on_resume(), move |_lua, _event| {
        count4.fetch_add(10, Ordering::Relaxed);
        Ok(())
    });

    thread.resume::<i32>(())?;
    assert_eq!(count.load(Ordering::Relaxed), 1);
    assert_eq!(count3.load(Ordering::Relaxed), 10);

    // Remove callback
    lua.remove_thread_event_callback();
    thread.reset(lua.load("return 0").into_function()?)?;
    thread.resume::<()>(())?;
    assert_eq!(count3.load(Ordering::Relaxed), 10); // unchanged

    Ok(())
}

#[cfg(feature = "luau")]
#[test]
fn test_thread_event_luau_resume_error() -> Result<()> {
    let lua = Lua::new();

    let fired = Arc::new(AtomicBool::new(false));
    let fired2 = fired.clone();
    lua.set_thread_event_callback(ThreadTriggers::ON_RESUME, move |_lua, event| {
        assert!(matches!(event, ThreadEvent::Resume(_)));
        fired2.store(true, Ordering::Relaxed);
        Ok(())
    });

    let thread = lua.create_thread(lua.load("return 42").into_function()?)?;
    let _ = thread.resume_error::<()>("test error");
    assert!(fired.load(Ordering::Relaxed));

    Ok(())
}

#[cfg(feature = "luau")]
#[test]
fn test_thread_event_create_from_lua() -> Result<()> {
    let lua = Lua::new();

    let count = std::cell::Cell::new(0);
    lua.set_thread_event_callback(ThreadTriggers::ON_CREATE, move |_, _| {
        count.set(count.get() + 1);
        if count.get() == 2 {
            return Err(Error::runtime("thread limit exceeded"));
        }
        Ok(())
    });
    let result = lua
        .load(
            r#"
            local co = coroutine.wrap(function() return coroutine.create(print) end)
            co()
    "#,
        )
        .exec();
    assert!(result.is_err());
    assert!(matches!(result, Err(Error::RuntimeError(err)) if err.contains("thread limit exceeded")));

    Ok(())
}
