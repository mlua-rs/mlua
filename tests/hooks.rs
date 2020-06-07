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

use std::cell::RefCell;
use std::ops::Deref;
use std::str;
use std::sync::{Arc, Mutex};

use mlua::{Error, HookTriggers, Lua, Result, Value};

#[test]
fn line_counts() -> Result<()> {
    let output = Arc::new(Mutex::new(Vec::new()));
    let hook_output = output.clone();

    let lua = Lua::new();
    lua.set_hook(
        HookTriggers {
            every_line: true,
            ..Default::default()
        },
        move |_lua, debug| {
            hook_output.lock().unwrap().push(debug.curr_line());
            Ok(())
        },
    )?;
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
fn function_calls() -> Result<()> {
    let output = Arc::new(Mutex::new(Vec::new()));
    let hook_output = output.clone();

    let lua = Lua::new();
    lua.set_hook(
        HookTriggers {
            on_calls: true,
            ..Default::default()
        },
        move |_lua, debug| {
            let names = debug.names();
            let source = debug.source();
            let name = names.name.map(|s| str::from_utf8(s).unwrap().to_owned());
            let what = source.what.map(|s| str::from_utf8(s).unwrap().to_owned());
            hook_output.lock().unwrap().push((name, what));
            Ok(())
        },
    )?;

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
            vec![
                (None, Some("main".to_string())),
                (Some("len".to_string()), Some("Lua".to_string()))
            ]
        );
    } else {
        assert_eq!(
            *output,
            vec![
                (None, Some("main".to_string())),
                (Some("len".to_string()), Some("C".to_string()))
            ]
        );
    }

    Ok(())
}

#[test]
fn error_within_hook() -> Result<()> {
    let lua = Lua::new();
    lua.set_hook(
        HookTriggers {
            every_line: true,
            ..Default::default()
        },
        |_lua, _debug| {
            Err(Error::RuntimeError(
                "Something happened in there!".to_string(),
            ))
        },
    )?;

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
fn limit_execution_instructions() -> Result<()> {
    let lua = Lua::new();
    let mut max_instructions = 10000;

    #[cfg(feature = "luajit")]
    // For LuaJIT disable JIT, as compiled code does not trigger hooks
    lua.load("jit.off()").exec()?;

    lua.set_hook(
        HookTriggers {
            every_nth_instruction: Some(30),
            ..Default::default()
        },
        move |_lua, _debug| {
            max_instructions -= 30;
            if max_instructions < 0 {
                Err(Error::RuntimeError("time's up".to_string()))
            } else {
                Ok(())
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
fn hook_removal() -> Result<()> {
    let lua = Lua::new();

    lua.set_hook(
        HookTriggers {
            every_nth_instruction: Some(1),
            ..Default::default()
        },
        |_lua, _debug| {
            Err(Error::RuntimeError(
                "this hook should've been removed by this time".to_string(),
            ))
        },
    )?;

    assert!(lua.load("local x = 1").exec().is_err());
    lua.remove_hook();
    assert!(lua.load("local x = 1").exec().is_ok());

    Ok(())
}

#[test]
fn hook_swap_within_hook() -> Result<()> {
    thread_local! {
        static TL_LUA: RefCell<Option<Lua>> = RefCell::new(None);
    }

    TL_LUA.with(|tl| {
        *tl.borrow_mut() = Some(Lua::new());
    });

    TL_LUA.with(|tl| {
        tl.borrow().as_ref().unwrap().set_hook(
            HookTriggers {
                every_line: true,
                ..Default::default()
            },
            move |lua, _debug| {
                lua.globals().set("ok", 1i64)?;
                TL_LUA.with(|tl| {
                    tl.borrow().as_ref().unwrap().set_hook(
                        HookTriggers {
                            every_line: true,
                            ..Default::default()
                        },
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
                })
            },
        )
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
        assert_eq!(lua.globals().get::<_, i64>("ok")?, 2);
        Ok(())
    })
}
