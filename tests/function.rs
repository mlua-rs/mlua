#![allow(unused_imports)]

use std::{string::String as StdString, time::Duration};

use futures_executor::block_on;

use mlua::{Error, Function, Lua, Result, String, Thread};

#[test]
fn test_function() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        function concat(arg1, arg2)
            return arg1 .. arg2
        end
    "#,
    )
    .exec()?;

    let concat = globals.get::<_, Function>("concat")?;
    assert_eq!(concat.call::<_, String>(("foo", "bar"))?, "foobar");

    Ok(())
}

#[test]
fn test_bind() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        function concat(...)
            local res = ""
            for _, s in pairs({...}) do
                res = res..s
            end
            return res
        end
    "#,
    )
    .exec()?;

    let mut concat = globals.get::<_, Function>("concat")?;
    concat = concat.bind("foo")?;
    concat = concat.bind("bar")?;
    concat = concat.bind(("baz", "baf"))?;
    assert_eq!(
        concat.call::<_, String>(("hi", "wut"))?,
        "foobarbazbafhiwut"
    );

    Ok(())
}

#[test]
fn test_rust_function() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        function lua_function()
            return rust_function()
        end

        -- Test to make sure chunk return is ignored
        return 1
    "#,
    )
    .exec()?;

    let lua_function = globals.get::<_, Function>("lua_function")?;
    let rust_function = lua.create_function(|_, ()| Ok("hello"))?;

    globals.set("rust_function", rust_function)?;
    assert_eq!(lua_function.call::<_, String>(())?, "hello");

    Ok(())
}

#[cfg(feature = "async")]
#[tokio::test]
async fn test_async_function() -> Result<()> {
    let lua = Lua::new();

    let f = lua.create_async_function(move |_lua, n: u64| async move {
        futures_timer::Delay::new(Duration::from_secs(n)).await;
        Ok("hello")
    })?;
    lua.globals().set("rust_async_sleep", f)?;

    let thread = lua
        .load(
            r#"
            coroutine.create(function ()
                ret = rust_async_sleep(1)
                assert(ret == "hello")
                coroutine.yield()
                return "world"
            end)
        "#,
        )
        .eval::<Thread>()?;

    let fut = thread.into_async(());
    let ret: StdString = fut.await?;
    assert_eq!(ret, "world");

    Ok(())
}
