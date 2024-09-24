use std::io;

use mlua::{Error, ErrorContext, Lua, Result};

#[test]
fn test_error_context() -> Result<()> {
    let lua = Lua::new();

    let func =
        lua.create_function(|_, ()| Err::<(), _>(Error::runtime("runtime error")).context("some context"))?;
    lua.globals().set("func", func)?;

    let msg = lua
        .load("local _, err = pcall(func); return tostring(err)")
        .eval::<String>()?;
    assert!(msg.contains("some context"));
    assert!(msg.contains("runtime error"));

    let func2 = lua.create_function(|lua, ()| {
        lua.globals()
            .get::<String>("nonextant")
            .with_context(|_| "failed to find global")
    })?;
    lua.globals().set("func2", func2)?;

    let msg2 = lua
        .load("local _, err = pcall(func2); return tostring(err)")
        .eval::<String>()?;
    assert!(msg2.contains("failed to find global"));
    println!("{msg2}");
    assert!(msg2.contains("error converting Lua nil to String"));

    // Rewrite context message and test `downcast_ref`
    let func3 = lua.create_function(|_, ()| {
        Err::<(), _>(Error::external(io::Error::new(io::ErrorKind::Other, "other")))
            .context("some context")
            .context("some new context")
    })?;
    let res = func3.call::<()>(()).err().unwrap();
    let Error::CallbackError { cause, .. } = &res else {
        unreachable!()
    };
    assert!(!res.to_string().contains("some context"));
    assert!(res.to_string().contains("some new context"));
    assert!(cause.downcast_ref::<io::Error>().is_some());

    Ok(())
}

#[test]
fn test_error_chain() -> Result<()> {
    let lua = Lua::new();

    // Check that `Error::ExternalError` creates a chain with a single element
    let io_err = io::Error::new(io::ErrorKind::Other, "other");
    assert_eq!(Error::external(io_err).chain().count(), 1);

    let func = lua.create_function(|_, ()| {
        let err = Error::external(io::Error::new(io::ErrorKind::Other, "other")).context("io error");
        Err::<(), _>(err)
    })?;
    let err = func.call::<()>(()).err().unwrap();
    assert_eq!(err.chain().count(), 3);
    for (i, err) in err.chain().enumerate() {
        match i {
            0 => assert!(matches!(err.downcast_ref(), Some(Error::CallbackError { .. }))),
            1 => assert!(matches!(err.downcast_ref(), Some(Error::WithContext { .. }))),
            2 => assert!(matches!(err.downcast_ref(), Some(io::Error { .. }))),
            _ => unreachable!(),
        }
    }

    Ok(())
}
