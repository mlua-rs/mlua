#![cfg(not(feature = "luau"))]

use std::cell::RefCell;
use std::ops::Deref;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use mlua::{DebugEvent, Error, HookTriggers, Lua, Result, Value};

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
        hook_output.lock().unwrap().push(debug.curr_line());
        Ok(())
    });
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
        Ok(())
    });

    lua.load(
        r#"
            local v = string.len("Hello World")
        "#,
    )
    .exec()?;

    lua.remove_hook();

    let output = output.lock().unwrap();
    if cfg!(feature = "luajit") && lua.load("jit.version_num").eval::<i64>()? >= 20100 {
        assert_eq!(
            *output,
            vec![(None, "main"), (Some("len".to_string()), "Lua")]
        );
    } else {
        assert_eq!(
            *output,
            vec![(None, "main"), (Some("len".to_string()), "C")]
        );
    }

    Ok(())
}

#[test]
fn test_error_within_hook() -> Result<()> {
    let lua = Lua::new();

    lua.set_hook(HookTriggers::EVERY_LINE, |_lua, _debug| {
        Err(Error::runtime("Something happened in there!"))
    });

    let err = lua
        .load("x = 1")
        .exec()
        .expect_err("panic didn't propagate");

    match err {
        Error::CallbackError { cause, .. } => match cause.deref() {
            Error::RuntimeError(s) => assert_eq!(s, "Something happened in there!"),
            _ => panic!("wrong callback error kind caught"),
        },
        _ => panic!("wrong error kind caught"),
    };

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
                Ok(())
            }
        },
    );

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

    lua.set_hook(
        HookTriggers::new().every_nth_instruction(1),
        |_lua, _debug| {
            Err(Error::runtime(
                "this hook should've been removed by this time",
            ))
        },
    );

    assert!(lua.load("local x = 1").exec().is_err());
    lua.remove_hook();
    assert!(lua.load("local x = 1").exec().is_ok());

    Ok(())
}

#[test]
fn test_hook_swap_within_hook() -> Result<()> {
    thread_local! {
        static TL_LUA: RefCell<Option<Lua>> = RefCell::new(None);
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
                    tl.borrow().as_ref().unwrap().set_hook(
                        HookTriggers::EVERY_LINE,
                        move |lua, _debug| {
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
                            Ok(())
                        },
                    )
                });
                Ok(())
            })
    });

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
        assert_eq!(lua.globals().get::<_, i64>("ok")?, 2);
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
        hook_output.lock().unwrap().push(debug.curr_line());
        Ok(())
    });

    co.resume(())?;
    lua.remove_hook();

    let output = output.lock().unwrap();
    if cfg!(feature = "luajit") && lua.load("jit.version_num").eval::<i64>()? >= 20100 {
        assert_eq!(*output, vec![2, 3, 4, 0, 4]);
    } else {
        assert_eq!(*output, vec![2, 3, 4]);
    }

    Ok(())
}
