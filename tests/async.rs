#![cfg(feature = "async")]

use std::rc::Rc;
use std::time::Duration;

use futures_util::stream::TryStreamExt;

use mlua::{Error, Function, Lua, Result};

#[tokio::test]
async fn test_async_function() -> Result<()> {
    let lua = Lua::new();

    let f = lua
        .create_async_function(|_lua, (a, b, c): (i64, i64, i64)| async move { Ok((a + b) * c) })?;
    lua.globals().set("f", f)?;

    let res: i64 = lua.load("f(1, 2, 3)").eval_async().await?;
    assert_eq!(res, 9);

    Ok(())
}

#[tokio::test]
async fn test_async_sleep() -> Result<()> {
    let lua = Lua::new();

    let sleep = lua.create_async_function(move |_lua, n: u64| async move {
        futures_timer::Delay::new(Duration::from_millis(n)).await;
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

    let sleep = lua.create_async_function(|_lua, name: String| async move {
        futures_timer::Delay::new(Duration::from_millis(10)).await;
        Ok(format!("hello, {}!", name))
    })?;

    match sleep.call::<_, ()>("alex") {
        Err(Error::RuntimeError(_)) => {}
        _ => panic!(
            "non-async executing async function must fail on the yield stage with RuntimeError"
        ),
    };

    assert_eq!(sleep.call_async::<_, String>("alex").await?, "hello, alex!");

    // Executing non-async functions using async call is allowed
    let sum = lua.create_function(|_lua, (a, b): (i64, i64)| return Ok(a + b))?;
    assert_eq!(sum.call_async::<_, i64>((5, 1)).await?, 6);

    Ok(())
}

#[tokio::test]
async fn test_async_bind_call() -> Result<()> {
    let lua = Lua::new();

    let less = lua.create_async_function(|_lua, (a, b): (i64, i64)| async move { Ok(a < b) })?;

    let less_bound = less.bind(0)?;
    lua.globals().set("f", less_bound)?;

    assert_eq!(lua.load("f(-1)").eval_async::<bool>().await?, false);
    assert_eq!(lua.load("f(1)").eval_async::<bool>().await?, true);

    Ok(())
}

#[tokio::test]
async fn test_async_handle_yield() -> Result<()> {
    let lua = Lua::new();

    let sum = lua.create_async_function(|_lua, (a, b): (i64, i64)| async move {
        futures_timer::Delay::new(Duration::from_millis(100)).await;
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
    assert_eq!(min.call_async::<_, i64>((-1, 1)).await?, -1);

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

    let mut stream = thread.into_async::<_, i64>(1);
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

    let cnt = Rc::new(100); // sleep 100ms
    let cnt2 = cnt.clone();
    let f = lua.create_async_function(move |_lua, ()| {
        let cnt3 = cnt2.clone();
        async move {
            futures_timer::Delay::new(Duration::from_millis(*cnt3.as_ref())).await;
            Ok("done")
        }
    })?;

    let res: String = lua.create_thread(f)?.into_async(()).await?;

    assert_eq!(res, "done");

    assert_eq!(Rc::strong_count(&cnt), 2);
    lua.gc_collect()?; // thread_s is non-resumable and subject to garbage collection
    assert_eq!(Rc::strong_count(&cnt), 1);

    Ok(())
}
