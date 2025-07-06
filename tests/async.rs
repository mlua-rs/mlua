#![cfg(feature = "async")]

use std::string::String as StdString;
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::TryStreamExt;
use tokio::sync::Mutex;

use mlua::{
    Error, Function, Lua, LuaOptions, MultiValue, ObjectLike, Result, StdLib, Table, UserData,
    UserDataMethods, UserDataRef, Value,
};

#[cfg(not(target_arch = "wasm32"))]
async fn sleep_ms(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[cfg(target_arch = "wasm32")]
async fn sleep_ms(_ms: u64) {
    // I was unable to make sleep() work in wasm32-emscripten target
    tokio::task::yield_now().await;
}

#[tokio::test]
async fn test_async_function() -> Result<()> {
    let lua = Lua::new();

    let f = lua.create_async_function(|_lua, (a, b, c): (i64, i64, i64)| async move { Ok((a + b) * c) })?;
    lua.globals().set("f", f)?;

    let res: i64 = lua.load("f(1, 2, 3)").eval_async().await?;
    assert_eq!(res, 9);

    Ok(())
}

#[tokio::test]
async fn test_async_function_wrap() -> Result<()> {
    let lua = Lua::new();

    let f = Function::wrap_async(|s: StdString| async move {
        tokio::task::yield_now().await;
        Ok(s)
    });
    lua.globals().set("f", f)?;
    let res: String = lua.load(r#"f("hello")"#).eval_async().await?;
    assert_eq!(res, "hello");

    // Return error
    let ferr = Function::wrap_async(|| async move { Err::<(), _>(Error::runtime("some async error")) });
    lua.globals().set("ferr", ferr)?;
    lua.load(
        r#"
        local ok, err = pcall(ferr)
        assert(not ok and tostring(err):find("some async error"))
    "#,
    )
    .exec_async()
    .await
    .unwrap();

    Ok(())
}

#[tokio::test]
async fn test_async_function_wrap_raw() -> Result<()> {
    let lua = Lua::new();

    let f = Function::wrap_raw_async(|s: StdString| async move {
        tokio::task::yield_now().await;
        s
    });
    lua.globals().set("f", f)?;
    let res: String = lua.load(r#"f("hello")"#).eval_async().await?;
    assert_eq!(res, "hello");

    // Return error
    let ferr = Function::wrap_raw_async(|| async move {
        tokio::task::yield_now().await;
        Err::<(), _>("some error")
    });
    lua.globals().set("ferr", ferr)?;
    let (_, err): (Value, String) = lua.load(r#"ferr()"#).eval_async().await?;
    assert_eq!(err, "some error");

    Ok(())
}

#[tokio::test]
async fn test_async_sleep() -> Result<()> {
    let lua = Lua::new();

    let sleep = lua.create_async_function(move |_lua, n: u64| async move {
        sleep_ms(n).await;
        Ok(format!("elapsed:{}ms", n))
    })?;
    lua.globals().set("sleep", sleep)?;

    let res: String = lua.load(r"return sleep(...)").call_async(100).await?;
    assert_eq!(res, "elapsed:100ms");

    Ok(())
}

#[tokio::test]
async fn test_async_call() -> Result<()> {
    let lua = Lua::new();

    let hello = lua.create_async_function(|_lua, name: String| async move {
        sleep_ms(10).await;
        Ok(format!("hello, {}!", name))
    })?;

    match hello.call::<()>("alex") {
        Err(Error::RuntimeError(_)) => {}
        err => panic!("expected `RuntimeError`, got {err:?}"),
    };

    assert_eq!(hello.call_async::<String>("alex").await?, "hello, alex!");

    // Executing non-async functions using async call is allowed
    let sum = lua.create_function(|_lua, (a, b): (i64, i64)| return Ok(a + b))?;
    assert_eq!(sum.call_async::<i64>((5, 1)).await?, 6);

    Ok(())
}

#[tokio::test]
async fn test_async_call_many_returns() -> Result<()> {
    let lua = Lua::new();

    let hello = lua.create_async_function(|_lua, ()| async move {
        sleep_ms(10).await;
        Ok(("a", "b", "c", 1))
    })?;

    let vals = hello.call_async::<MultiValue>(()).await?;
    assert_eq!(vals.len(), 4);
    assert_eq!(vals[0].to_string()?, "a");
    assert_eq!(vals[1].to_string()?, "b");
    assert_eq!(vals[2].to_string()?, "c");
    assert_eq!(vals[3], Value::Integer(1));

    Ok(())
}

#[tokio::test]
async fn test_async_bind_call() -> Result<()> {
    let lua = Lua::new();

    let sum = lua.create_async_function(|_lua, (a, b): (i64, i64)| async move {
        tokio::task::yield_now().await;
        Ok(a + b)
    })?;

    let plus_10 = sum.bind(10)?;
    lua.globals().set("plus_10", plus_10)?;

    assert_eq!(lua.load("plus_10(-1)").eval_async::<i64>().await?, 9);
    assert_eq!(lua.load("plus_10(1)").eval_async::<i64>().await?, 11);

    Ok(())
}

#[tokio::test]
async fn test_async_handle_yield() -> Result<()> {
    let lua = Lua::new();

    let sum = lua.create_async_function(|_lua, (a, b): (i64, i64)| async move {
        sleep_ms(10).await;
        Ok(a + b)
    })?;

    lua.globals().set("sleep_sum", sum)?;

    let res: String = lua
        .load(
            r#"
        sum = sleep_sum(6, 7)
        assert(sum == 13)
        coroutine.yield("in progress")
        return "done"
    "#,
        )
        .call_async(())
        .await?;

    assert_eq!(res, "done");

    let min = lua
        .load(
            r#"
        function (a, b)
            coroutine.yield("ignore me")
            if a < b then return a else return b end
        end
    "#,
        )
        .eval::<Function>()?;
    assert_eq!(min.call_async::<i64>((-1, 1)).await?, -1);

    Ok(())
}

#[tokio::test]
async fn test_async_multi_return_nil() -> Result<()> {
    let lua = Lua::new();
    lua.globals().set(
        "func",
        lua.create_async_function(|_, _: ()| async { Ok((Option::<String>::None, "error")) })?,
    )?;

    lua.load(
        r#"
        local ok, err = func()
        assert(err == "error")
    "#,
    )
    .exec_async()
    .await
}

#[tokio::test]
async fn test_async_return_async_closure() -> Result<()> {
    let lua = Lua::new();

    let f = lua.create_async_function(|lua, a: i64| async move {
        sleep_ms(10).await;

        let g = lua.create_async_function(move |_, b: i64| async move {
            sleep_ms(10).await;
            return Ok(a + b);
        })?;

        Ok(g)
    })?;

    lua.globals().set("f", f)?;

    let res: i64 = lua
        .load("local g = f(1); return g(2) + g(3)")
        .call_async(())
        .await?;

    assert_eq!(res, 7);

    Ok(())
}

#[cfg(feature = "lua54")]
#[tokio::test]
async fn test_async_lua54_to_be_closed() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    globals.set("close_count", 0)?;

    let code = r#"
        local t <close> = setmetatable({}, {
            __close = function()
                close_count = close_count + 1
            end
        })
        error "test"
    "#;
    let f = lua.load(code).into_function()?;

    // Test close using call_async
    let _ = f.call_async::<()>(()).await;
    assert_eq!(globals.get::<usize>("close_count")?, 1);

    // Don't close by default when awaiting async threads
    let co = lua.create_thread(f.clone())?;
    let _ = co.clone().into_async::<()>(())?.await;
    assert_eq!(globals.get::<usize>("close_count")?, 1);
    let _ = co.reset(f);
    assert_eq!(globals.get::<usize>("close_count")?, 2);

    Ok(())
}

#[tokio::test]
async fn test_async_thread_stream() -> Result<()> {
    let lua = Lua::new();

    let thread = lua.create_thread(
        lua.load(
            r#"
            function (sum)
                for i = 1,10 do
                    sum = sum + i
                    coroutine.yield(sum)
                end
                return sum
            end
            "#,
        )
        .eval()?,
    )?;

    let mut stream = thread.into_async::<i64>(1)?;
    let mut sum = 0;
    while let Some(n) = stream.try_next().await? {
        sum += n;
    }

    assert_eq!(sum, 286);

    Ok(())
}

#[tokio::test]
async fn test_async_thread() -> Result<()> {
    let lua = Lua::new();

    let cnt = Arc::new(10); // sleep 10ms
    let cnt2 = cnt.clone();
    let f = lua.create_async_function(move |_lua, ()| {
        let cnt3 = cnt2.clone();
        async move {
            sleep_ms(*cnt3.as_ref()).await;
            Ok("done")
        }
    })?;

    let res: String = lua.create_thread(f)?.into_async(())?.await?;

    assert_eq!(res, "done");

    assert_eq!(Arc::strong_count(&cnt), 2);
    lua.gc_collect()?; // thread_s is non-resumable and subject to garbage collection
    assert_eq!(Arc::strong_count(&cnt), 1);

    Ok(())
}

#[test]
fn test_async_thread_capture() -> Result<()> {
    let lua = Lua::new();

    let f = lua.create_async_function(move |_lua, v: Value| async move {
        tokio::task::yield_now().await;
        drop(v);
        Ok(())
    })?;

    let thread = lua.create_thread(f)?;
    // After first resume, `v: Value` is captured in the coroutine
    thread.resume::<()>("abc").unwrap();
    drop(thread);

    Ok(())
}

#[tokio::test]
async fn test_async_table_object_like() -> Result<()> {
    let options = LuaOptions::new().thread_pool_size(4);
    let lua = Lua::new_with(StdLib::ALL_SAFE, options)?;

    let table = lua.create_table()?;
    table.set("val", 10)?;

    let get_value = lua.create_async_function(|_, table: Table| async move {
        sleep_ms(10).await;
        table.get::<i64>("val")
    })?;
    table.set("get_value", get_value)?;

    let set_value = lua.create_async_function(|_, (table, n): (Table, i64)| async move {
        sleep_ms(10).await;
        table.set("val", n)
    })?;
    table.set("set_value", set_value)?;

    assert_eq!(table.call_async_method::<i64>("get_value", ()).await?, 10);
    table.call_async_method::<()>("set_value", 15).await?;
    assert_eq!(table.call_async_method::<i64>("get_value", ()).await?, 15);

    let metatable = lua.create_table()?;
    metatable.set(
        "__call",
        lua.create_async_function(|_, table: Table| async move {
            sleep_ms(10).await;
            table.get::<i64>("val")
        })?,
    )?;
    table.set_metatable(Some(metatable))?;
    assert_eq!(table.call_async::<i64>(()).await.unwrap(), 15);

    match table.call_async_method::<()>("non_existent", ()).await {
        Err(Error::RuntimeError(err)) => {
            assert!(err.contains("attempt to call a nil value (function 'non_existent')"))
        }
        r => panic!("expected RuntimeError, got {r:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn test_async_thread_pool() -> Result<()> {
    let options = LuaOptions::new().thread_pool_size(4);
    let lua = Lua::new_with(StdLib::ALL_SAFE, options)?;

    let error_f = lua.create_async_function(|_, ()| async move {
        sleep_ms(10).await;
        Err::<(), _>(Error::runtime("test"))
    })?;

    let sleep = lua.create_async_function(|_, n| async move {
        sleep_ms(n).await;
        Ok(format!("elapsed:{}ms", n))
    })?;

    assert!(error_f.call_async::<()>(()).await.is_err());
    // Next call should use cached thread
    assert_eq!(sleep.call_async::<String>(3).await?, "elapsed:3ms");

    Ok(())
}

#[tokio::test]
async fn test_async_userdata() -> Result<()> {
    struct MyUserData(u64);

    impl UserData for MyUserData {
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_async_method("get_value", |_, data, ()| async move {
                sleep_ms(10).await;
                Ok(data.0)
            });

            methods.add_async_method_mut("set_value", |_, mut data, n| async move {
                sleep_ms(10).await;
                data.0 = n;
                Ok(())
            });

            methods.add_async_function("sleep", |_, n| async move {
                sleep_ms(n).await;
                Ok(format!("elapsed:{}ms", n))
            });

            #[cfg(not(any(feature = "lua51", feature = "luau")))]
            methods.add_async_meta_method(mlua::MetaMethod::Call, |_, data, ()| async move {
                let n = data.0;
                sleep_ms(n).await;
                Ok(format!("elapsed:{}ms", n))
            });

            #[cfg(not(any(feature = "lua51", feature = "luau")))]
            methods.add_async_meta_method(mlua::MetaMethod::Index, |_, data, key: String| async move {
                sleep_ms(10).await;
                match key.as_str() {
                    "ms" => Ok(Some(data.0 as f64)),
                    "s" => Ok(Some((data.0 as f64) / 1000.0)),
                    _ => Ok(None),
                }
            });

            #[cfg(not(any(feature = "lua51", feature = "luau")))]
            methods.add_async_meta_method_mut(
                mlua::MetaMethod::NewIndex,
                |_, mut data, (key, value): (String, f64)| async move {
                    sleep_ms(10).await;
                    match key.as_str() {
                        "ms" => data.0 = value as u64,
                        "s" => data.0 = (value * 1000.0) as u64,
                        _ => return Err(Error::external(format!("key '{}' not found", key))),
                    }
                    Ok(())
                },
            );
        }
    }

    let lua = Lua::new();
    let globals = lua.globals();

    let userdata = lua.create_userdata(MyUserData(11))?;
    globals.set("userdata", &userdata)?;

    lua.load(
        r#"
        assert(userdata:get_value() == 11)
        userdata:set_value(12)
        assert(userdata.sleep(5) == "elapsed:5ms")
        assert(userdata:get_value() == 12)
    "#,
    )
    .exec_async()
    .await?;

    #[cfg(not(any(feature = "lua51", feature = "luau")))]
    lua.load(
        r#"
        userdata:set_value(15)
        assert(userdata() == "elapsed:15ms")

        userdata.ms = 2000
        assert(userdata.s == 2)

        userdata.s = 15
        assert(userdata.ms == 15000)
    "#,
    )
    .exec_async()
    .await?;

    // ObjectLike methods
    userdata.call_async_method::<()>("set_value", 24).await?;
    let n: u64 = userdata.call_async_method("get_value", ()).await?;
    assert_eq!(n, 24);
    userdata.call_async_function::<()>("sleep", 15).await?;

    #[cfg(not(any(feature = "lua51", feature = "luau")))]
    assert_eq!(userdata.call_async::<String>(()).await?, "elapsed:24ms");

    Ok(())
}

#[tokio::test]
async fn test_async_thread_error() -> Result<()> {
    struct MyUserData;

    impl UserData for MyUserData {
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_meta_method("__tostring", |_, _this, ()| Ok("myuserdata error"))
        }
    }

    let lua = Lua::new();
    let result = lua
        .load("function x(...) error(...) end x(...)")
        .set_name("chunk")
        .call_async::<()>(MyUserData)
        .await;
    assert!(
        matches!(result, Err(Error::RuntimeError(cause)) if cause.contains("myuserdata error")),
        "improper error traceback from dead thread"
    );

    Ok(())
}

#[tokio::test]
async fn test_async_terminate() -> Result<()> {
    // Future captures `Lua` instance and dropped all together
    let mutex = Arc::new(Mutex::new(0u32));
    {
        let lua = Lua::new();
        let mutex2 = mutex.clone();
        let func = lua.create_async_function(move |lua, ()| {
            let mutex = mutex2.clone();
            async move {
                let _guard = mutex.lock().await;
                sleep_ms(100).await;
                drop(lua); // Move Lua to the future to test drop
                Ok(())
            }
        })?;

        let _ = tokio::time::timeout(Duration::from_millis(30), func.call_async::<()>(())).await;
    }
    assert!(mutex.try_lock().is_ok());

    // Future is dropped, but `Lua` instance is still alive
    let lua = Lua::new();
    let func = lua.create_async_function(move |_, mutex: UserDataRef<Arc<Mutex<u32>>>| async move {
        let _guard = mutex.lock().await;
        sleep_ms(100).await;
        Ok(())
    })?;
    let mutex2 = lua.create_any_userdata(mutex.clone())?;
    let _ = tokio::time::timeout(Duration::from_millis(30), func.call_async::<()>(mutex2)).await;
    assert!(mutex.try_lock().is_ok());

    Ok(())
}

#[tokio::test]
async fn test_async_task() -> Result<()> {
    let lua = Lua::new();

    let delay = lua.create_function(|lua, (secs, f, args): (f32, Function, MultiValue)| {
        let thread = lua.create_thread(f)?;
        let thread2 = thread.clone().into_async::<()>(args)?;
        tokio::task::spawn_local(async move {
            tokio::time::sleep(Duration::from_secs_f32(secs)).await;
            _ = thread2.await;
        });
        Ok(thread)
    })?;

    lua.globals().set("delay", delay)?;
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            _ = lua
                .load("delay(0.1, function(msg) global_msg = msg end, 'done')")
                .exec_async()
                .await;
        })
        .await;
    local.await;
    assert_eq!(lua.globals().get::<String>("global_msg")?, "done");

    Ok(())
}

#[tokio::test]
async fn test_async_task_abort() -> Result<()> {
    let lua = Lua::new();

    let sleep = lua.create_async_function(move |_lua, n: u64| async move {
        sleep_ms(n).await;
        Ok(())
    })?;
    lua.globals().set("sleep", sleep)?;

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let lua2 = lua.clone();
            let jh = tokio::task::spawn_local(async move {
                lua2.load("sleep(200) result = 'done'")
                    .exec_async()
                    .await
                    .unwrap();
            });
            sleep_ms(100).await; // Wait for the task to start
            jh.abort();
        })
        .await;
    local.await;
    assert_eq!(lua.globals().get::<Value>("result")?, Value::Nil);

    Ok(())
}

#[tokio::test]
#[cfg(not(feature = "luau"))]
async fn test_async_hook() -> Result<()> {
    use std::sync::atomic::{AtomicBool, Ordering};

    let lua = Lua::new();

    static HOOK_CALLED: AtomicBool = AtomicBool::new(false);
    lua.set_global_hook(mlua::HookTriggers::new().every_line(), move |_, _| {
        if !HOOK_CALLED.swap(true, Ordering::Relaxed) {
            #[cfg(any(feature = "lu53", feature = "lua54"))]
            return Ok(mlua::VmState::Yield);
        }
        Ok(mlua::VmState::Continue)
    })?;

    let sleep = lua.create_async_function(move |_lua, n: u64| async move {
        sleep_ms(n).await;
        Ok(())
    })?;
    lua.globals().set("sleep", sleep)?;

    lua.load(r"sleep(100)").exec_async().await?;
    assert!(HOOK_CALLED.load(Ordering::Relaxed));

    Ok(())
}
