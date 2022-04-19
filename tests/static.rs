use std::cell::RefCell;

use mlua::{Lua, Result, Table};

#[test]
fn test_static_lua() -> Result<()> {
    let lua = Lua::new().into_static();

    thread_local! {
        static TABLE: RefCell<Option<Table<'static>>> = RefCell::new(None);
    }

    let f = lua.create_function(|_, table: Table| {
        TABLE.with(|t| {
            table.raw_insert(1, "hello")?;
            *t.borrow_mut() = Some(table);
            Ok(())
        })
    })?;

    f.call(lua.create_table()?)?;
    drop(f);
    lua.gc_collect()?;

    TABLE.with(|t| {
        assert!(t.borrow().as_ref().unwrap().len().unwrap() == 1);
        *t.borrow_mut() = None;
    });

    // Consume the Lua instance
    unsafe { Lua::from_static(lua) };

    Ok(())
}

#[test]
fn test_static_lua_coroutine() -> Result<()> {
    let lua = Lua::new().into_static();

    thread_local! {
        static TABLE: RefCell<Option<Table<'static>>> = RefCell::new(None);
    }

    let f = lua.create_function(|_, table: Table| {
        TABLE.with(|t| {
            table.raw_insert(1, "hello")?;
            *t.borrow_mut() = Some(table);
            Ok(())
        })
    })?;

    let co = lua.create_thread(f)?;
    co.resume::<_, ()>(lua.create_table()?)?;
    drop(co);
    lua.gc_collect()?;

    TABLE.with(|t| {
        assert_eq!(
            t.borrow().as_ref().unwrap().get::<_, String>(1i32).unwrap(),
            "hello".to_string()
        );
        *t.borrow_mut() = None;
    });

    // Consume the Lua instance
    unsafe { Lua::from_static(lua) };

    Ok(())
}

#[cfg(feature = "async")]
#[tokio::test]
async fn test_static_async() -> Result<()> {
    let lua = Lua::new().into_static();

    let timer =
        lua.create_async_function(|_, (i, n, f): (u64, u64, mlua::Function)| async move {
            tokio::task::spawn_local(async move {
                let dur = std::time::Duration::from_millis(i);
                for _ in 0..n {
                    tokio::task::spawn_local(f.call_async::<(), ()>(()));
                    tokio::time::sleep(dur).await;
                }
            });
            Ok(())
        })?;
    lua.globals().set("timer", timer)?;

    {
        let local_set = tokio::task::LocalSet::new();
        local_set
            .run_until(
                lua.load(
                    r#"
                local cnt = 0
                timer(1, 100, function()
                    cnt = cnt + 1
                    if cnt % 10 == 0 then
                        collectgarbage()
                    end
                end)
                "#,
                )
                .exec_async(),
            )
            .await?;
        local_set.await;
    }

    // Consume the Lua instance
    unsafe { Lua::from_static(lua) };

    Ok(())
}
