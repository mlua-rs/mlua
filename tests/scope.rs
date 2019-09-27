extern crate rlua;

use std::cell::Cell;
use std::rc::Rc;

use rlua::{Error, Function, Lua, MetaMethod, String, UserData, UserDataMethods};

#[test]
fn scope_func() {
    let lua = Lua::new();

    let rc = Rc::new(Cell::new(0));
    lua.scope(|scope| {
        let r = rc.clone();
        let f = scope
            .create_function(move |_, ()| {
                r.set(42);
                Ok(())
            })
            .unwrap();
        lua.globals().set("bad", f.clone()).unwrap();
        f.call::<_, ()>(()).unwrap();
        assert_eq!(Rc::strong_count(&rc), 2);
    });
    assert_eq!(rc.get(), 42);
    assert_eq!(Rc::strong_count(&rc), 1);

    match lua
        .globals()
        .get::<_, Function>("bad")
        .unwrap()
        .call::<_, ()>(())
    {
        Err(Error::CallbackError { .. }) => {}
        r => panic!("improper return for destructed function: {:?}", r),
    };
}

#[test]
fn scope_drop() {
    let lua = Lua::new();

    struct MyUserdata(Rc<()>);
    impl UserData for MyUserdata {
        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_method("method", |_, _, ()| Ok(()));
        }
    }

    let rc = Rc::new(());

    lua.scope(|scope| {
        lua.globals()
            .set(
                "test",
                scope
                    .create_static_userdata(MyUserdata(rc.clone()))
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(Rc::strong_count(&rc), 2);
    });
    assert_eq!(Rc::strong_count(&rc), 1);

    match lua.exec::<_, ()>("test:method()", None) {
        Err(Error::CallbackError { .. }) => {}
        r => panic!("improper return for destructed userdata: {:?}", r),
    };
}

#[test]
fn scope_capture() {
    let lua = Lua::new();

    let mut i = 0;
    lua.scope(|scope| {
        scope
            .create_function_mut(|_, ()| {
                i = 42;
                Ok(())
            })
            .unwrap()
            .call::<_, ()>(())
            .unwrap();
    });
    assert_eq!(i, 42);
}

#[test]
fn outer_lua_access() {
    let lua = Lua::new();
    let table = lua.create_table().unwrap();
    lua.scope(|scope| {
        scope
            .create_function_mut(|_, ()| {
                table.set("a", "b").unwrap();
                Ok(())
            })
            .unwrap()
            .call::<_, ()>(())
            .unwrap();
    });
    assert_eq!(table.get::<_, String>("a").unwrap(), "b");
}

#[test]
fn scope_userdata_methods() {
    struct MyUserData<'a>(&'a Cell<i64>);

    impl<'a> UserData for MyUserData<'a> {
        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_method("inc", |_, data, ()| {
                data.0.set(data.0.get() + 1);
                Ok(())
            });

            methods.add_method("dec", |_, data, ()| {
                data.0.set(data.0.get() - 1);
                Ok(())
            });
        }
    }

    let lua = Lua::new();

    let i = Cell::new(42);
    lua.scope(|scope| {
        let f: Function = lua
            .eval(
                r#"
                    function(u)
                        u:inc()
                        u:inc()
                        u:inc()
                        u:dec()
                    end
                "#,
                None,
            )
            .unwrap();

        f.call::<_, ()>(scope.create_nonstatic_userdata(MyUserData(&i)).unwrap())
            .unwrap();
    });

    assert_eq!(i.get(), 44);
}

#[test]
fn scope_userdata_functions() {
    struct MyUserData<'a>(&'a i64);

    impl<'a> UserData for MyUserData<'a> {
        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_meta_function(MetaMethod::Add, |lua, ()| {
                let globals = lua.globals();
                globals.set("i", globals.get::<_, i64>("i")? + 1)?;
                Ok(())
            });
            methods.add_meta_function(MetaMethod::Sub, |lua, ()| {
                let globals = lua.globals();
                globals.set("i", globals.get::<_, i64>("i")? + 1)?;
                Ok(())
            });
        }
    }

    let lua = Lua::new();
    let f = lua
        .exec::<_, Function>(
            r#"
                i = 0
                return function(u)
                    _ = u + u
                    _ = u - 1
                    _ = 1 + u
                end
            "#,
            None,
        )
        .unwrap();

    let dummy = 0;
    lua.scope(|scope| {
        f.call::<_, ()>(scope.create_nonstatic_userdata(MyUserData(&dummy)).unwrap())
            .unwrap();
    });

    assert_eq!(lua.globals().get::<_, i64>("i").unwrap(), 3);
}

#[test]
fn scope_userdata_mismatch() {
    struct MyUserData<'a>(&'a Cell<i64>);

    impl<'a> UserData for MyUserData<'a> {
        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_method("inc", |_, data, ()| {
                data.0.set(data.0.get() + 1);
                Ok(())
            });
        }
    }

    let lua = Lua::new();
    lua.exec::<_, ()>(
        r#"
            function okay(a, b)
                a.inc(a)
                b.inc(b)
            end

            function bad(a, b)
                a.inc(b)
            end
        "#,
        None,
    )
    .unwrap();

    let a = Cell::new(1);
    let b = Cell::new(1);

    let okay: Function = lua.globals().get("okay").unwrap();
    let bad: Function = lua.globals().get("bad").unwrap();

    lua.scope(|scope| {
        let au = scope.create_nonstatic_userdata(MyUserData(&a)).unwrap();
        let bu = scope.create_nonstatic_userdata(MyUserData(&b)).unwrap();
        assert!(okay.call::<_, ()>((au.clone(), bu.clone())).is_ok());
        match bad.call::<_, ()>((au, bu)) {
            Err(Error::CallbackError { ref cause, .. }) => match *cause.as_ref() {
                Error::UserDataTypeMismatch => {}
                ref other => panic!("wrong error type {:?}", other),
            },
            Err(other) => panic!("wrong error type {:?}", other),
            Ok(_) => panic!("incorrectly returned Ok"),
        }
    });
}
