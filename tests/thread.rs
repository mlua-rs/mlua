use std::panic::catch_unwind;

use mlua::{Error, Function, Lua, Result, Thread, ThreadStatus};

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

    assert_eq!(thread.status(), ThreadStatus::Resumable);
    assert_eq!(thread.resume::<i64>(0)?, 0);
    assert_eq!(thread.status(), ThreadStatus::Resumable);
    assert_eq!(thread.resume::<i64>(1)?, 1);
    assert_eq!(thread.status(), ThreadStatus::Resumable);
    assert_eq!(thread.resume::<i64>(2)?, 3);
    assert_eq!(thread.status(), ThreadStatus::Resumable);
    assert_eq!(thread.resume::<i64>(3)?, 6);
    assert_eq!(thread.status(), ThreadStatus::Resumable);
    assert_eq!(thread.resume::<i64>(4)?, 10);
    assert_eq!(thread.status(), ThreadStatus::Finished);

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
    assert_eq!(accumulate.status(), ThreadStatus::Resumable);
    assert!(accumulate.resume::<()>("error").is_err());
    assert_eq!(accumulate.status(), ThreadStatus::Error);

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
    assert_eq!(thread.status(), ThreadStatus::Resumable);
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
        assert_eq!(lua.current_thread().status(), ThreadStatus::Running);
        let result = lua.current_thread().resume::<()>(());
        assert!(
            matches!(result, Err(Error::CoroutineUnresumable)),
            "unexpected result: {result:?}",
        );
        Ok(())
    })?)?;
    let result = thread.resume::<()>(());
    assert!(result.is_ok(), "unexpected result: {result:?}");

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
        assert_eq!(thread.status(), ThreadStatus::Resumable);
        let _ = thread.resume::<AnyUserData>(MyUserData(arc.clone()))?;
        assert_eq!(thread.status(), ThreadStatus::Resumable);
        assert_eq!(Arc::strong_count(&arc), 2);
        thread.resume::<()>(())?;
        assert_eq!(thread.status(), ThreadStatus::Finished);
        thread.reset(func.clone())?;
        lua.gc_collect()?;
        assert_eq!(Arc::strong_count(&arc), 1);
    }

    // Check for errors
    let func: Function = lua.load(r#"function(ud) error("test error") end"#).eval()?;
    let thread = lua.create_thread(func.clone())?;
    let _ = thread.resume::<AnyUserData>(MyUserData(arc.clone()));
    assert_eq!(thread.status(), ThreadStatus::Error);
    assert_eq!(Arc::strong_count(&arc), 2);
    #[cfg(feature = "lua54")]
    {
        assert!(thread.reset(func.clone()).is_err());
        // Reset behavior has changed in Lua v5.4.4
        // It's became possible to force reset thread by popping error object
        assert!(matches!(thread.status(), ThreadStatus::Finished));
        assert!(thread.reset(func.clone()).is_ok());
        assert_eq!(thread.status(), ThreadStatus::Resumable);
    }
    #[cfg(any(feature = "lua54", feature = "luau"))]
    {
        assert!(thread.reset(func.clone()).is_ok());
        assert_eq!(thread.status(), ThreadStatus::Resumable);
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
