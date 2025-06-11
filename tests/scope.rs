use std::cell::Cell;
use std::rc::Rc;
use std::string::String as StdString;
use std::sync::Arc;

use mlua::{
    AnyUserData, Error, Function, Lua, MetaMethod, ObjectLike, Result, String, UserData, UserDataFields,
    UserDataMethods, UserDataRegistry,
};

#[test]
fn test_scope_func() -> Result<()> {
    let lua = Lua::new();

    let rc = Rc::new(Cell::new(0));
    lua.scope(|scope| {
        let rc2 = rc.clone();
        let f = scope.create_function(move |_, ()| {
            rc2.set(42);
            Ok(())
        })?;
        lua.globals().set("f", &f)?;
        f.call::<()>(())?;
        assert_eq!(Rc::strong_count(&rc), 2);
        Ok(())
    })?;
    assert_eq!(rc.get(), 42);
    assert_eq!(Rc::strong_count(&rc), 1);

    match lua.globals().get::<Function>("f")?.call::<()>(()) {
        Err(Error::CallbackError { ref cause, .. }) => match *cause.as_ref() {
            Error::CallbackDestructed => {}
            ref err => panic!("wrong error type {:?}", err),
        },
        r => panic!("improper return for destructed function: {:?}", r),
    };

    Ok(())
}

#[test]
fn test_scope_capture() -> Result<()> {
    let lua = Lua::new();

    let mut i = 0;
    lua.scope(|scope| {
        scope
            .create_function_mut(|_, ()| {
                i = 42;
                Ok(())
            })?
            .call::<()>(())
    })?;
    assert_eq!(i, 42);

    Ok(())
}

#[test]
fn test_scope_outer_lua_access() -> Result<()> {
    let lua = Lua::new();

    let table = lua.create_table()?;
    lua.scope(|scope| scope.create_function(|_, ()| table.set("a", "b"))?.call::<()>(()))?;
    assert_eq!(table.get::<String>("a")?, "b");

    Ok(())
}

#[test]
fn test_scope_capture_scope() -> Result<()> {
    let lua = Lua::new();

    let i = Cell::new(0);
    lua.scope(|scope| {
        let f = scope.create_function(|_, ()| {
            scope.create_function(|_, n: u32| {
                i.set(i.get() + n);
                Ok(())
            })
        })?;
        f.call::<Function>(())?.call::<()>(10)?;
        Ok(())
    })?;

    assert_eq!(i.get(), 10);

    Ok(())
}

#[test]
fn test_scope_userdata_fields() -> Result<()> {
    struct MyUserData<'a>(&'a Cell<i64>);

    impl UserData for MyUserData<'_> {
        fn register(reg: &mut UserDataRegistry<Self>) {
            reg.add_field("field", "hello");
            reg.add_field_method_get("val", |_, data| Ok(data.0.get()));
            reg.add_field_method_set("val", |_, data, val| {
                data.0.set(val);
                Ok(())
            });
        }
    }

    let lua = Lua::new();

    let i = Cell::new(42);
    let f: Function = lua
        .load(
            r#"
            function(u)
                assert(u.field == "hello")
                assert(u.val == 42)
                u.val = 44
            end
        "#,
        )
        .eval()?;

    lua.scope(|scope| f.call::<()>(scope.create_userdata(MyUserData(&i))?))?;

    assert_eq!(i.get(), 44);

    Ok(())
}

#[test]
fn test_scope_userdata_methods() -> Result<()> {
    struct MyUserData<'a>(&'a Cell<i64>);

    impl UserData for MyUserData<'_> {
        fn register(reg: &mut UserDataRegistry<Self>) {
            reg.add_method("inc", |_, data, ()| {
                data.0.set(data.0.get() + 1);
                Ok(())
            });

            reg.add_method("dec", |_, data, ()| {
                data.0.set(data.0.get() - 1);
                Ok(())
            });
        }
    }

    let lua = Lua::new();

    let i = Cell::new(42);
    let f: Function = lua
        .load(
            r#"
            function(u)
                u:inc()
                u:inc()
                u:inc()
                u:dec()
            end
        "#,
        )
        .eval()?;

    lua.scope(|scope| f.call::<()>(scope.create_userdata(MyUserData(&i))?))?;

    assert_eq!(i.get(), 44);

    Ok(())
}

#[test]
fn test_scope_userdata_ops() -> Result<()> {
    struct MyUserData<'a>(&'a i64);

    impl UserData for MyUserData<'_> {
        fn register(reg: &mut UserDataRegistry<Self>) {
            reg.add_meta_method(MetaMethod::Add, |lua, this, ()| {
                let globals = lua.globals();
                globals.set("i", globals.get::<i64>("i")? + this.0)?;
                Ok(())
            });
            reg.add_meta_method(MetaMethod::Sub, |lua, this, ()| {
                let globals = lua.globals();
                globals.set("i", globals.get::<i64>("i")? + this.0)?;
                Ok(())
            });
        }
    }

    let lua = Lua::new();

    let dummy = 1;
    let f = lua
        .load(
            r#"
            i = 0
            return function(u)
                _ = u + u
                _ = u - 1
                _ = u + 1
            end
        "#,
        )
        .eval::<Function>()?;

    lua.scope(|scope| f.call::<()>(scope.create_userdata(MyUserData(&dummy))?))?;

    assert_eq!(lua.globals().get::<i64>("i")?, 3);

    Ok(())
}

#[test]
fn test_scope_userdata_values() -> Result<()> {
    struct MyUserData<'a>(&'a i64);

    impl UserData for MyUserData<'_> {
        fn register(registry: &mut UserDataRegistry<Self>) {
            registry.add_method("get", |_, data, ()| Ok(*data.0));
        }
    }

    let lua = Lua::new();

    let i = 42;
    let data = MyUserData(&i);
    lua.scope(|scope| {
        let ud = scope.create_userdata(data)?;
        assert_eq!(ud.call_method::<i64>("get", &ud)?, 42);
        ud.set_user_value("user_value")?;
        assert_eq!(ud.user_value::<String>()?, "user_value");
        Ok(())
    })?;

    Ok(())
}

#[test]
fn test_scope_userdata_mismatch() -> Result<()> {
    struct MyUserData<'a>(&'a mut i64);

    impl<'a> UserData for MyUserData<'a> {
        fn register(reg: &mut UserDataRegistry<Self>) {
            reg.add_method("get", |_, data, ()| Ok(*data.0));

            reg.add_method_mut("inc", |_, data, ()| {
                *data.0 = data.0.wrapping_add(1);
                Ok(())
            });
        }
    }

    let lua = Lua::new();

    lua.load(
        r#"
        function inc(a, b) a.inc(b) end
        function get(a, b) a.get(b) end
    "#,
    )
    .exec()?;

    let mut a = 1;
    let mut b = 1;

    lua.scope(|scope| {
        let au = scope.create_userdata(MyUserData(&mut a))?;
        let bu = scope.create_userdata(MyUserData(&mut b))?;
        for method_name in ["get", "inc"] {
            let f: Function = lua.globals().get(method_name)?;
            let full_name = format!("MyUserData.{method_name}");
            let full_name = full_name.as_str();

            assert!(f.call::<()>((&au, &au)).is_ok());
            match f.call::<()>((&au, &bu)) {
                Err(Error::CallbackError { ref cause, .. }) => match cause.as_ref() {
                    Error::BadArgument { to, pos, name, cause } => {
                        assert_eq!(to.as_deref(), Some(full_name));
                        assert_eq!(*pos, 1);
                        assert_eq!(name.as_deref(), Some("self"));
                        assert!(matches!(*cause.as_ref(), Error::UserDataTypeMismatch));
                    }
                    other => panic!("wrong error type {other:?}"),
                },
                Err(other) => panic!("wrong error type {other:?}"),
                Ok(_) => panic!("incorrectly returned Ok"),
            }

            // Pass non-userdata type
            let err = f.call::<()>((&au, 321)).err().unwrap();
            match err {
                Error::CallbackError { ref cause, .. } => match cause.as_ref() {
                    Error::BadArgument { to, pos, name, cause } => {
                        assert_eq!(to.as_deref(), Some(full_name));
                        assert_eq!(*pos, 1);
                        assert_eq!(name.as_deref(), Some("self"));
                        assert!(matches!(*cause.as_ref(), Error::FromLuaConversionError { .. }));
                    }
                    other => panic!("wrong error type {other:?}"),
                },
                other => panic!("wrong error type {other:?}"),
            }
            let err_msg = format!("bad argument `self` to `{full_name}`: error converting Lua number to userdata (expected userdata of type 'MyUserData')");
            assert!(err.to_string().contains(&err_msg));
        }
        Ok(())
    })?;

    Ok(())
}

#[test]
fn test_scope_userdata_drop() -> Result<()> {
    let lua = Lua::new();

    struct MyUserData<'a>(&'a Cell<i64>, #[allow(unused)] Rc<()>);

    impl UserData for MyUserData<'_> {
        fn register(reg: &mut UserDataRegistry<Self>) {
            reg.add_method("inc", |_, data, ()| {
                data.0.set(data.0.get() + 1);
                Ok(())
            });
        }
    }

    let (i, rc) = (Cell::new(1), Rc::new(()));
    lua.scope(|scope| {
        let ud = scope.create_userdata(MyUserData(&i, rc.clone()))?;
        lua.globals().set("ud", ud)?;
        lua.load("ud:inc()").exec()?;
        assert_eq!(Rc::strong_count(&rc), 2);
        Ok(())
    })?;
    assert_eq!(Rc::strong_count(&rc), 1);
    assert_eq!(i.get(), 2);

    match lua.load("ud:inc()").exec() {
        Err(Error::CallbackError { ref cause, .. }) => match cause.as_ref() {
            Error::UserDataDestructed => {}
            err => panic!("expected UserDataDestructed, got {err:?}"),
        },
        r => panic!("improper return for destructed userdata: {r:?}"),
    };

    let ud = lua.globals().get::<AnyUserData>("ud")?;
    match ud.borrow_scoped::<MyUserData, _>(|_| Ok::<_, Error>(())) {
        Ok(_) => panic!("successful borrow for destructed userdata"),
        Err(Error::UserDataDestructed) => {}
        Err(err) => panic!("improper borrow error for destructed userdata: {err:?}"),
    }
    match ud.metatable() {
        Ok(_) => panic!("successful metatable retrieval of destructed userdata"),
        Err(Error::UserDataDestructed) => {}
        Err(err) => panic!("improper metatable error for destructed userdata: {err:?}"),
    }

    Ok(())
}

#[test]
fn test_scope_userdata_ref() -> Result<()> {
    let lua = Lua::new();

    struct MyUserData(Cell<i64>);

    impl UserData for MyUserData {
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
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

    let data = MyUserData(Cell::new(1));
    lua.scope(|scope| {
        let ud = scope.create_userdata_ref(&data)?;
        modify_userdata(&lua, &ud)?;

        // We can only borrow userdata scoped
        assert!((matches!(ud.borrow::<MyUserData>(), Err(Error::UserDataTypeMismatch))));
        ud.borrow_scoped::<MyUserData, ()>(|ud_inst| {
            assert_eq!(ud_inst.0.get(), 2);
        })?;

        Ok(())
    })?;
    assert_eq!(data.0.get(), 2);

    Ok(())
}

#[test]
fn test_scope_userdata_ref_mut() -> Result<()> {
    let lua = Lua::new();

    struct MyUserData(i64);

    impl UserData for MyUserData {
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_method_mut("inc", |_, data, ()| {
                data.0 += 1;
                Ok(())
            });

            methods.add_method_mut("dec", |_, data, ()| {
                data.0 -= 1;
                Ok(())
            });
        }
    }

    let mut data = MyUserData(1);
    lua.scope(|scope| {
        let ud = scope.create_userdata_ref_mut(&mut data)?;
        modify_userdata(&lua, &ud)?;

        assert!((matches!(ud.borrow_mut::<MyUserData>(), Err(Error::UserDataTypeMismatch))));
        ud.borrow_mut_scoped::<MyUserData, ()>(|ud_inst| {
            ud_inst.0 += 10;
        })?;

        Ok(())
    })?;
    assert_eq!(data.0, 12);

    Ok(())
}

#[test]
fn test_scope_any_userdata() -> Result<()> {
    let lua = Lua::new();

    fn register(reg: &mut UserDataRegistry<&mut StdString>) {
        reg.add_method_mut("push", |_, this, s: String| {
            this.push_str(&s.to_str()?);
            Ok(())
        });
        reg.add_meta_method("__tostring", |_, data, ()| Ok((*data).clone()));
    }

    let mut data = StdString::from("foo");
    lua.scope(|scope| {
        let ud = scope.create_any_userdata(&mut data, register)?;
        lua.globals().set("ud", ud)?;
        lua.load(
            r#"
            assert(tostring(ud) == "foo")
            ud:push("bar")
            assert(tostring(ud) == "foobar")
        "#,
        )
        .exec()
    })?;

    // Check that userdata is destructed
    match lua.load("tostring(ud)").exec() {
        Err(Error::CallbackError { ref cause, .. }) => match cause.as_ref() {
            Error::UserDataDestructed => {}
            err => panic!("expected CallbackDestructed, got {err:?}"),
        },
        r => panic!("improper return for destructed userdata: {r:?}"),
    };

    Ok(())
}

#[test]
fn test_scope_any_userdata_ref() -> Result<()> {
    let lua = Lua::new();

    lua.register_userdata_type::<Cell<i64>>(|reg| {
        reg.add_method("inc", |_, data, ()| {
            data.set(data.get() + 1);
            Ok(())
        });

        reg.add_method("dec", |_, data, ()| {
            data.set(data.get() - 1);
            Ok(())
        });
    })?;

    let data = Cell::new(1i64);
    lua.scope(|scope| {
        let ud = scope.create_any_userdata_ref(&data)?;
        modify_userdata(&lua, &ud)
    })?;
    assert_eq!(data.get(), 2);

    Ok(())
}

#[test]
fn test_scope_any_userdata_ref_mut() -> Result<()> {
    let lua = Lua::new();

    lua.register_userdata_type::<i64>(|reg| {
        reg.add_method_mut("inc", |_, data, ()| {
            *data += 1;
            Ok(())
        });

        reg.add_method_mut("dec", |_, data, ()| {
            *data -= 1;
            Ok(())
        });
    })?;

    let mut data = 1i64;
    lua.scope(|scope| {
        let ud = scope.create_any_userdata_ref_mut(&mut data)?;
        modify_userdata(&lua, &ud)
    })?;
    assert_eq!(data, 2);

    Ok(())
}

#[test]
fn test_scope_destructors() -> Result<()> {
    let lua = Lua::new();

    lua.register_userdata_type::<Arc<StdString>>(|reg| {
        reg.add_meta_method("__tostring", |_, data, ()| Ok(data.to_string()));
    })?;

    let arc_str = Arc::new(StdString::from("foo"));

    let ud = lua.create_any_userdata(arc_str.clone())?;
    lua.scope(|scope| {
        scope.add_destructor(|| {
            assert!(ud.destroy().is_ok());
        });
        Ok(())
    })?;
    assert_eq!(Arc::strong_count(&arc_str), 1);

    // Try destructing the userdata while it's borrowed
    let ud = lua.create_any_userdata(arc_str.clone())?;
    ud.borrow_scoped::<Arc<StdString>, _>(|arc_str| {
        assert_eq!(arc_str.as_str(), "foo");
        lua.scope(|scope| {
            scope.add_destructor(|| {
                assert!(ud.destroy().is_err());
            });
            Ok(())
        })
        .unwrap();
        assert_eq!(arc_str.as_str(), "foo");
    })?;

    Ok(())
}

fn modify_userdata(lua: &Lua, ud: &AnyUserData) -> Result<()> {
    lua.load(
        r#"
    local u = ...
    u:inc()
    u:dec()
    u:inc()
"#,
    )
    .call(ud)
}
