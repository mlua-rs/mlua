#![cfg(not(feature = "luau"))]

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use mlua::{DebugEvent, Error, HookTriggers, Lua, Result, ThreadStatus, Value, VmState};

#[test]
fn test_hook_triggers() {
    let trigger = HookTriggers::new().on_calls().on_returns()
        | HookTriggers::new().every_line().every_nth_instruction(5);

    assert!(trigger.on_calls);
    assert!(trigger.on_returns);
    assert!(trigger.every_line);
    assert_eq!(trigger.every_nth_instruction, Some(5));
}

#[test]
fn test_line_counts() -> Result<()> {
    let output = Arc::new(Mutex::new(Vec::new()));
    let hook_output = output.clone();

    let lua = Lua::new();
    lua.set_hook(HookTriggers::EVERY_LINE, move |_lua, debug| {
        assert_eq!(debug.event(), DebugEvent::Line);
        hook_output.lock().unwrap().push(debug.current_line().unwrap());
        Ok(VmState::Continue)
    })?;
    lua.load(
        r#"
            local x = 2 + 3
            local y = x * 63
            local z = string.len(x..", "..y)
        "#,
    )
    .exec()?;

    lua.remove_hook();

    let output = output.lock().unwrap();
    if cfg!(feature = "luajit") && lua.load("jit.version_num").eval::<i64>()? >= 20100 {
        assert_eq!(*output, vec![2, 3, 4, 0, 4]);
    } else {
        assert_eq!(*output, vec![2, 3, 4]);
    }

    Ok(())
}

#[test]
fn test_function_calls() -> Result<()> {
    let output = Arc::new(Mutex::new(Vec::new()));
    let hook_output = output.clone();

    let lua = Lua::new();
    lua.set_hook(HookTriggers::ON_CALLS, move |_lua, debug| {
        assert_eq!(debug.event(), DebugEvent::Call);
        let names = debug.names();
        let source = debug.source();
        let name = names.name.map(|s| s.into_owned());
        hook_output.lock().unwrap().push((name, source.what));
        Ok(VmState::Continue)
    })?;

    lua.load(
        r#"
            local v = string.len("Hello World")
        "#,
    )
    .exec()?;

    lua.remove_hook();

    let output = output.lock().unwrap();
    if cfg!(feature = "luajit") && lua.load("jit.version_num").eval::<i64>()? >= 20100 {
        #[cfg(not(force_memory_limit))]
        assert_eq!(*output, vec![(None, "main"), (Some("len".to_string()), "Lua")]);
        #[cfg(force_memory_limit)]
        assert_eq!(
            *output,
            vec![(None, "C"), (None, "main"), (Some("len".to_string()), "Lua")]
        );
    } else {
        #[cfg(not(force_memory_limit))]
        assert_eq!(*output, vec![(None, "main"), (Some("len".to_string()), "C")]);
        #[cfg(force_memory_limit)]
        assert_eq!(
            *output,
            vec![(None, "C"), (None, "main"), (Some("len".to_string()), "C")]
        );
    }

    Ok(())
}

#[test]
fn test_error_within_hook() -> Result<()> {
    let lua = Lua::new();

    lua.set_hook(HookTriggers::EVERY_LINE, |_lua, _debug| {
        Err(Error::runtime("Something happened in there!"))
    })?;

    let err = lua.load("x = 1").exec().expect_err("panic didn't propagate");
    match err {
        Error::RuntimeError(msg) => assert_eq!(msg, "Something happened in there!"),
        err => panic!("expected `RuntimeError` with a specific message, got {err:?}"),
    }

    Ok(())
}

#[test]
fn test_limit_execution_instructions() -> Result<()> {
    let lua = Lua::new();

    // For LuaJIT disable JIT, as compiled code does not trigger hooks
    #[cfg(feature = "luajit")]
    lua.load("jit.off()").exec()?;

    let max_instructions = AtomicI64::new(10000);
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(30),
        move |_lua, debug| {
            assert_eq!(debug.event(), DebugEvent::Count);
            if max_instructions.fetch_sub(30, Ordering::Relaxed) <= 30 {
                Err(Error::runtime("time's up"))
            } else {
                Ok(VmState::Continue)
            }
        },
    )?;

    lua.globals().set("x", Value::Integer(0))?;
    let _ = lua
        .load(
            r#"
                for i = 1, 10000 do
                    x = x + 1
                end
            "#,
        )
        .exec()
        .expect_err("instruction limit didn't occur");

    Ok(())
}

#[test]
fn test_hook_removal() -> Result<()> {
    let lua = Lua::new();

    lua.set_hook(HookTriggers::new().every_nth_instruction(1), |_lua, _debug| {
        Err(Error::runtime("this hook should've been removed by this time"))
    })?;

    assert!(lua.load("local x = 1").exec().is_err());
    lua.remove_hook();
    assert!(lua.load("local x = 1").exec().is_ok());

    Ok(())
}

// Having the code compiled (even not run) on macos and luajit causes a memory reference issue
// See https://github.com/LuaJIT/LuaJIT/issues/1099
#[cfg(not(all(feature = "luajit", target_os = "macos")))]
#[test]
fn test_hook_swap_within_hook() -> Result<()> {
    thread_local! {
        static TL_LUA: std::cell::RefCell<Option<Lua>> = Default::default();
    }

    TL_LUA.with(|tl| {
        *tl.borrow_mut() = Some(Lua::new());
    });

    TL_LUA.with(|tl| {
        tl.borrow()
            .as_ref()
            .unwrap()
            .set_hook(HookTriggers::EVERY_LINE, move |lua, _debug| {
                lua.globals().set("ok", 1i64)?;
                TL_LUA.with(|tl| {
                    tl.borrow()
                        .as_ref()
                        .unwrap()
                        .set_hook(HookTriggers::EVERY_LINE, move |lua, _debug| {
                            lua.load(
                                r#"
                                    if ok ~= nil then
                                        ok = ok + 1
                                    end
                                "#,
                            )
                            .exec()
                            .expect("exec failure within hook");
                            TL_LUA.with(|tl| {
                                tl.borrow().as_ref().unwrap().remove_hook();
                            });
                            Ok(VmState::Continue)
                        })
                })?;
                Ok(VmState::Continue)
            })
    })?;

    TL_LUA.with(|tl| {
        let tl = tl.borrow();
        let lua = tl.as_ref().unwrap();
        lua.load(
            r#"
                local x = 1
                x = 2
                local y = 3
            "#,
        )
        .exec()?;
        assert_eq!(lua.globals().get::<i64>("ok")?, 2);
        Ok(())
    })
}

#[test]
fn test_hook_threads() -> Result<()> {
    let lua = Lua::new();

    let func = lua
        .load(
            r#"
            local x = 2 + 3
            local y = x * 63
            local z = string.len(x..", "..y)
        "#,
        )
        .into_function()?;
    let co = lua.create_thread(func)?;

    let output = Arc::new(Mutex::new(Vec::new()));
    let hook_output = output.clone();
    co.set_hook(HookTriggers::EVERY_LINE, move |_lua, debug| {
        assert_eq!(debug.event(), DebugEvent::Line);
        hook_output.lock().unwrap().push(debug.current_line().unwrap());
        Ok(VmState::Continue)
    })?;

    co.resume::<()>(())?;
    lua.remove_hook();

    let output = output.lock().unwrap();
    if cfg!(feature = "luajit") && lua.load("jit.version_num").eval::<i64>()? >= 20100 {
        assert_eq!(*output, vec![2, 3, 4, 0, 4]);
    } else {
        assert_eq!(*output, vec![2, 3, 4]);
    }

    Ok(())
}

#[test]
fn test_hook_yield() -> Result<()> {
    let lua = Lua::new();

    let func = lua
        .load(
            r#"
            local x = 2 + 3
            local y = x * 63
            local z = string.len(x..", "..y)
        "#,
        )
        .into_function()?;
    let co = lua.create_thread(func)?;

    co.set_hook(HookTriggers::EVERY_LINE, move |_lua, _debug| Ok(VmState::Yield))?;

    #[cfg(any(feature = "lua54", feature = "lua53"))]
    {
        assert!(co.resume::<()>(()).is_ok());
        assert!(co.resume::<()>(()).is_ok());
        assert!(co.resume::<()>(()).is_ok());
        assert!(co.resume::<()>(()).is_ok());
        assert!(co.status() == ThreadStatus::Finished);
    }
    #[cfg(any(feature = "lua51", feature = "lua52", feature = "luajit"))]
    {
        assert!(
            matches!(co.resume::<()>(()), Err(Error::RuntimeError(err)) if err.contains("attempt to yield from a hook"))
        );
        assert!(co.status() == ThreadStatus::Error);
    }

    Ok(())
}

#[test]
fn test_global_hook() -> Result<()> {
    let lua = Lua::new();

    let counter = Arc::new(AtomicI64::new(0));
    let hook_counter = counter.clone();
    lua.set_global_hook(HookTriggers::EVERY_LINE, move |_lua, debug| {
        assert_eq!(debug.event(), DebugEvent::Line);
        hook_counter.fetch_add(1, Ordering::Relaxed);
        Ok(VmState::Continue)
    })?;

    let thread = lua.create_thread(
        lua.load(
            r#"
            local x = 2 + 3
            local y = x * 63
            coroutine.yield()
            local z = string.len(x..", "..y)
        "#,
        )
        .into_function()?,
    )?;

    thread.resume::<()>(()).unwrap();
    lua.remove_global_hook();
    thread.resume::<()>(()).unwrap();
    assert_eq!(thread.status(), ThreadStatus::Finished);
    assert_eq!(counter.load(Ordering::Relaxed), 3);

    Ok(())
}
