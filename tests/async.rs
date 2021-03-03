#![cfg(feature = "async")]
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

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};
use std::time::Duration;

use futures_timer::Delay;
use futures_util::stream::TryStreamExt;

use mlua::{
    Error, Function, Lua, Result, Table, TableExt, Thread, UserData, UserDataMethods, Value,
};

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
        Delay::new(Duration::from_millis(n)).await;
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
        Delay::new(Duration::from_millis(10)).await;
        Ok(format!("hello, {}!", name))
    })?;

    match hello.call::<_, ()>("alex") {
        Err(Error::RuntimeError(_)) => {}
        _ => panic!(
            "non-async executing async function must fail on the yield stage with RuntimeError"
        ),
    };

    assert_eq!(hello.call_async::<_, String>("alex").await?, "hello, alex!");

    // Executing non-async functions using async call is allowed
    let sum = lua.create_function(|_lua, (a, b): (i64, i64)| return Ok(a + b))?;
    assert_eq!(sum.call_async::<_, i64>((5, 1)).await?, 6);

    Ok(())
}

#[tokio::test]
async fn test_async_bind_call() -> Result<()> {
    let lua = Lua::new();

    let sum = lua.create_async_function(|_lua, (a, b): (i64, i64)| async move { Ok(a + b) })?;

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
        Delay::new(Duration::from_millis(10)).await;
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
        Delay::new(Duration::from_millis(10)).await;

        let g = lua.create_async_function(move |_, b: i64| async move {
            Delay::new(Duration::from_millis(10)).await;
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

    let cnt = Arc::new(10); // sleep 10ms
    let cnt2 = cnt.clone();
    let f = lua.create_async_function(move |_lua, ()| {
        let cnt3 = cnt2.clone();
        async move {
            Delay::new(Duration::from_millis(*cnt3.as_ref())).await;
            Ok("done")
        }
    })?;

    let res: String = lua.create_thread(f)?.into_async(()).await?;

    assert_eq!(res, "done");

    assert_eq!(Arc::strong_count(&cnt), 2);
    lua.gc_collect()?; // thread_s is non-resumable and subject to garbage collection
    assert_eq!(Arc::strong_count(&cnt), 1);

    Ok(())
}

#[tokio::test]
async fn test_async_table() -> Result<()> {
    let lua = Lua::new();

    let table = lua.create_table()?;
    table.set("val", 10)?;

    let get_value = lua.create_async_function(|_, table: Table| async move {
        Delay::new(Duration::from_millis(10)).await;
        table.get::<_, i64>("val")
    })?;
    table.set("get_value", get_value)?;

    let set_value = lua.create_async_function(|_, (table, n): (Table, i64)| async move {
        Delay::new(Duration::from_millis(10)).await;
        table.set("val", n)
    })?;
    table.set("set_value", set_value)?;

    let sleep = lua.create_async_function(|_, n| async move {
        Delay::new(Duration::from_millis(n)).await;
        Ok(format!("elapsed:{}ms", n))
    })?;
    table.set("sleep", sleep)?;

    assert_eq!(
        table
            .call_async_method::<_, _, i64>("get_value", ())
            .await?,
        10
    );
    table.call_async_method("set_value", 15).await?;
    assert_eq!(
        table
            .call_async_method::<_, _, i64>("get_value", ())
            .await?,
        15
    );
    assert_eq!(
        table
            .call_async_function::<_, _, String>("sleep", 7)
            .await?,
        "elapsed:7ms"
    );

    Ok(())
}

#[tokio::test]
async fn test_async_userdata() -> Result<()> {
    #[derive(Clone)]
    struct MyUserData(Arc<AtomicI64>);

    impl UserData for MyUserData {
        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_async_method("get_value", |_, data, ()| async move {
                Delay::new(Duration::from_millis(10)).await;
                Ok(data.0.load(Ordering::Relaxed))
            });

            methods.add_async_method("set_value", |_, data, n| async move {
                Delay::new(Duration::from_millis(10)).await;
                data.0.store(n, Ordering::Relaxed);
                Ok(())
            });

            methods.add_async_function("sleep", |_, n| async move {
                Delay::new(Duration::from_millis(n)).await;
                Ok(format!("elapsed:{}ms", n))
            });
        }
    }

    let lua = Lua::new();
    let globals = lua.globals();

    let userdata = lua.create_userdata(MyUserData(Arc::new(AtomicI64::new(11))))?;
    globals.set("userdata", userdata.clone())?;

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

    Ok(())
}

#[tokio::test]
async fn test_async_scope() -> Result<()> {
    let ref lua = Lua::new();

    let ref rc = Rc::new(Cell::new(0));

    let fut = lua.async_scope(|scope| async move {
        let f = scope.create_async_function(move |_, n: u64| {
            let rc2 = rc.clone();
            async move {
                rc2.set(42);
                Delay::new(Duration::from_millis(n)).await;
                assert_eq!(Rc::strong_count(&rc2), 2);
                Ok(())
            }
        })?;

        lua.globals().set("f", f.clone())?;

        assert_eq!(Rc::strong_count(rc), 1);
        let _ = f.call_async::<u64, ()>(10).await?;
        assert_eq!(Rc::strong_count(rc), 1);

        // Create future in partialy polled state (Poll::Pending)
        let g = lua.create_thread(f)?;
        g.resume::<u64, ()>(10)?;
        lua.globals().set("g", g)?;
        assert_eq!(Rc::strong_count(rc), 2);

        Ok(())
    });

    assert_eq!(Rc::strong_count(rc), 1);
    let _ = fut.await?;
    assert_eq!(Rc::strong_count(rc), 1);

    match lua
        .globals()
        .get::<_, Function>("f")?
        .call_async::<_, ()>(10)
        .await
    {
        Err(Error::CallbackError { ref cause, .. }) => match cause.as_ref() {
            Error::CallbackDestructed => {}
            e => panic!("expected `CallbackDestructed` error cause, got {:?}", e),
        },
        r => panic!("improper return for destructed function: {:?}", r),
    };

    match lua.globals().get::<_, Thread>("g")?.resume::<_, Value>(()) {
        Err(Error::CallbackError { ref cause, .. }) => match cause.as_ref() {
            Error::CallbackDestructed => {}
            e => panic!("expected `CallbackDestructed` error cause, got {:?}", e),
        },
        r => panic!("improper return for destructed function: {:?}", r),
    };

    Ok(())
}

#[tokio::test]
async fn test_async_scope_userdata() -> Result<()> {
    #[derive(Clone)]
    struct MyUserData(Arc<AtomicI64>);

    impl UserData for MyUserData {
        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_async_method("get_value", |_, data, ()| async move {
                Delay::new(Duration::from_millis(10)).await;
                Ok(data.0.load(Ordering::Relaxed))
            });

            methods.add_async_method("set_value", |_, data, n| async move {
                Delay::new(Duration::from_millis(10)).await;
                data.0.store(n, Ordering::Relaxed);
                Ok(())
            });

            methods.add_async_function("sleep", |_, n| async move {
                Delay::new(Duration::from_millis(n)).await;
                Ok(format!("elapsed:{}ms", n))
            });
        }
    }

    let ref lua = Lua::new();

    let ref arc = Arc::new(AtomicI64::new(11));

    lua.async_scope(|scope| async move {
        let ud = scope.create_userdata(MyUserData(arc.clone()))?;
        lua.globals().set("userdata", ud)?;
        lua.load(
            r#"
            assert(userdata:get_value() == 11)
            userdata:set_value(12)
            assert(userdata.sleep(5) == "elapsed:5ms")
            assert(userdata:get_value() == 12)
        "#,
        )
        .exec_async()
        .await
    })
    .await?;

    assert_eq!(Arc::strong_count(arc), 1);

    match lua.load("userdata:get_value()").exec_async().await {
        Err(Error::CallbackError { ref cause, .. }) => match cause.as_ref() {
            Error::CallbackDestructed => {}
            e => panic!("expected `CallbackDestructed` error cause, got {:?}", e),
        },
        r => panic!("improper return for destructed userdata: {:?}", r),
    };

    Ok(())
}
