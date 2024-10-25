#![cfg(feature = "send")]

use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::string::String as StdString;

use mlua::{AnyUserData, Error, Lua, Result, UserDataRef};
use static_assertions::{assert_impl_all, assert_not_impl_all};

#[test]
fn test_userdata_multithread_access() -> Result<()> {
    let lua = Lua::new();

    // This type is `Send` but not `Sync`.
    struct MyUserData(#[allow(unused)] StdString, PhantomData<UnsafeCell<()>>);

    assert_impl_all!(MyUserData: Send);
    assert_not_impl_all!(MyUserData: Sync);

    lua.globals().set(
        "ud",
        AnyUserData::wrap(MyUserData("hello".to_string(), PhantomData)),
    )?;
    // We acquired the exclusive reference.
    let _ud1 = lua.globals().get::<UserDataRef<MyUserData>>("ud")?;

    std::thread::scope(|s| {
        s.spawn(|| {
            let res = lua.globals().get::<UserDataRef<MyUserData>>("ud");
            assert!(matches!(res, Err(Error::UserDataBorrowError)));
        });
    });

    Ok(())
}
