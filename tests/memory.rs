use std::rc::Rc;

use mlua::{Lua, Result, UserData};

#[test]
fn test_gc_control() -> Result<()> {
    let lua = Lua::new();
    let globals = lua.globals();

    #[cfg(any(feature = "lua53", feature = "lua52"))]
    {
        assert!(lua.gc_is_running());
        lua.gc_stop();
        assert!(!lua.gc_is_running());
        lua.gc_restart();
        assert!(lua.gc_is_running());
    }

    struct MyUserdata(Rc<()>);
    impl UserData for MyUserdata {}

    let rc = Rc::new(());
    globals.set("userdata", lua.create_userdata(MyUserdata(rc.clone()))?)?;
    globals.raw_remove("userdata")?;

    assert_eq!(Rc::strong_count(&rc), 2);
    lua.gc_collect()?;
    lua.gc_collect()?;
    assert_eq!(Rc::strong_count(&rc), 1);

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
