use std::sync::Arc;

use mlua::{Error, Result, UserData};

include!("_lua.rs");

#[test]
fn test_gc_control() -> Result<()> {
    let lua = make_lua();

    assert!(lua.gc_is_running());
    lua.gc_stop();
    assert!(!lua.gc_is_running());
    lua.gc_restart();
    assert!(lua.gc_is_running());

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

#[test]
fn test_gc_error() {
    let lua = make_lua();
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
