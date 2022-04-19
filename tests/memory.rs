use std::sync::Arc;

use mlua::{GCMode, Lua, Result, UserData};

#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
use mlua::Error;

#[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
#[test]
fn test_memory_limit() -> Result<()> {
    let lua = Lua::new();

    let initial_memory = lua.used_memory();
    assert!(
        initial_memory > 0,
        "used_memory reporting is wrong, lua uses memory for stdlib"
    );

    let f = lua
        .load("local t = {}; for i = 1,10000 do t[i] = i end")
        .into_function()?;
    f.call::<_, ()>(()).expect("should trigger no memory limit");

    lua.set_memory_limit(initial_memory + 10000)?;
    match f.call::<_, ()>(()) {
        Err(Error::MemoryError(_)) => {}
        something_else => panic!("did not trigger memory error: {:?}", something_else),
    };

    lua.set_memory_limit(0)?;
    f.call::<_, ()>(()).expect("should trigger no memory limit");

    Ok(())
}

#[test]
fn test_gc_control() -> Result<()> {
    let lua = Lua::new();
    let globals = lua.globals();

    #[cfg(feature = "lua54")]
    {
        assert_eq!(lua.gc_gen(0, 0), GCMode::Incremental);
        assert_eq!(lua.gc_inc(0, 0, 0), GCMode::Generational);
    }

    #[cfg(any(
        feature = "lua54",
        feature = "lua53",
        feature = "lua52",
        feature = "luau"
    ))]
    {
        assert!(lua.gc_is_running());
        lua.gc_stop();
        assert!(!lua.gc_is_running());
        lua.gc_restart();
        assert!(lua.gc_is_running());
    }

    assert_eq!(lua.gc_inc(200, 100, 13), GCMode::Incremental);

    struct MyUserdata(Arc<()>);
    impl UserData for MyUserdata {}

    let rc = Arc::new(());
    globals.set("userdata", lua.create_userdata(MyUserdata(rc.clone()))?)?;
    globals.raw_remove("userdata")?;

    assert_eq!(Arc::strong_count(&rc), 2);
    lua.gc_collect()?;
    lua.gc_collect()?;
    assert_eq!(Arc::strong_count(&rc), 1);

    Ok(())
}

#[cfg(any(feature = "lua53", feature = "lua52"))]
#[test]
fn test_gc_error() {
    use mlua::Error;

    let lua = Lua::new();
    match lua
        .load(
            r#"
            val = nil
            table = {}
            setmetatable(table, {
                __gc = function()
                    error("gcwascalled")
                end
            })
            table = nil
            collectgarbage("collect")
    "#,
        )
        .exec()
    {
        Err(Error::GarbageCollectorError(_)) => {}
        Err(e) => panic!("__gc error did not result in correct error, instead: {}", e),
        Ok(()) => panic!("__gc error did not result in error"),
    }
}
