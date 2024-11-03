#![cfg(feature = "send")]

use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::string::String as StdString;

use mlua::{AnyUserData, Error, Lua, ObjectLike, Result, UserData, UserDataMethods, UserDataRef};
use static_assertions::{assert_impl_all, assert_not_impl_all};

#[test]
fn test_userdata_multithread_access_send_only() -> Result<()> {
    let lua = Lua::new();

    // This type is `Send` but not `Sync`.
    struct MyUserData(StdString, PhantomData<UnsafeCell<()>>);
    assert_impl_all!(MyUserData: Send);
    assert_not_impl_all!(MyUserData: Sync);

    impl UserData for MyUserData {
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_method("method", |lua, this, ()| {
                let ud = lua.globals().get::<AnyUserData>("ud")?;
                assert_eq!(ud.call_method::<String>("method2", ())?, "method2");
                Ok(this.0.clone())
            });

            methods.add_method("method2", |_, _, ()| Ok("method2"));
        }
    }

    lua.globals()
        .set("ud", MyUserData("hello".to_string(), PhantomData))?;

    // We acquired the exclusive reference.
    let ud = lua.globals().get::<UserDataRef<MyUserData>>("ud")?;

    std::thread::scope(|s| {
        s.spawn(|| {
            let res = lua.globals().get::<UserDataRef<MyUserData>>("ud");
            assert!(matches!(res, Err(Error::UserDataBorrowError)));
        });
    });

    drop(ud);
    lua.load("ud:method()").exec().unwrap();

    Ok(())
}

#[test]
fn test_userdata_multithread_access_sync() -> Result<()> {
    let lua = Lua::new();

    // This type is `Send` and `Sync`.
    struct MyUserData(StdString);
    assert_impl_all!(MyUserData: Send, Sync);

    impl UserData for MyUserData {
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_method("method", |lua, this, ()| {
                let ud = lua.globals().get::<AnyUserData>("ud")?;
                assert!(ud.call_method::<()>("method2", ()).is_ok());
                Ok(this.0.clone())
            });

            methods.add_method("method2", |_, _, ()| Ok(()));
        }
    }

    lua.globals().set("ud", MyUserData("hello".to_string()))?;

    // We acquired the shared reference.
    let _ud = lua.globals().get::<UserDataRef<MyUserData>>("ud")?;

    std::thread::scope(|s| {
        s.spawn(|| {
            // Getting another shared reference for `Sync` type is allowed.
            let _ = lua.globals().get::<UserDataRef<MyUserData>>("ud").unwrap();
        });
    });

    lua.load("ud:method()").exec().unwrap();

    Ok(())
}
