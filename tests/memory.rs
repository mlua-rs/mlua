#![cfg_attr(
    all(feature = "luajit", target_os = "macos", target_arch = "x86_64"),
    feature(link_args)
)]

#[cfg_attr(
    all(feature = "luajit", target_os = "macos", target_arch = "x86_64"),
    link_args = "-pagezero_size 10000 -image_base 100000000"
)]
extern "system" {}

use std::sync::Arc;

use mlua::{Lua, Result, UserData};

#[test]
fn test_gc_control() -> Result<()> {
    let lua = Lua::new();

    #[cfg(feature = "lua53")]
    {
        assert!(lua.gc_is_running());
        lua.gc_stop();
        assert!(!lua.gc_is_running());
        lua.gc_restart();
        assert!(lua.gc_is_running());
    }

    struct MyUserdata(Arc<()>);
    impl UserData for MyUserdata {}

    let rc = Arc::new(());
    lua.globals()
        .set("userdata", lua.create_userdata(MyUserdata(rc.clone()))?)?;
    lua.globals().raw_remove("userdata")?;

    assert_eq!(Arc::strong_count(&rc), 2);
    lua.gc_collect()?;
    lua.gc_collect()?;
    assert_eq!(Arc::strong_count(&rc), 1);

    Ok(())
}

#[cfg(feature = "lua53")]
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
