use std::cell::Cell;
use std::rc::Rc;
use std::string::String as StdString;

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
    struct MyUserData<'a>(&'a Cell<i64>);

    impl<'a> UserData for MyUserData<'a> {
        fn register(reg: &mut UserDataRegistry<Self>) {
            reg.add_method("inc", |_, data, ()| {
                data.0.set(data.0.get() + 1);
                Ok(())
            });
        }
    }

    let lua = Lua::new();

    lua.load(
        r#"
        function inc(a, b) a.inc(b) end
    "#,
    )
    .exec()?;

    let a = Cell::new(1);
    let b = Cell::new(1);

    let inc: Function = lua.globals().get("inc")?;
    lua.scope(|scope| {
        let au = scope.create_userdata(MyUserData(&a))?;
        let bu = scope.create_userdata(MyUserData(&b))?;
        assert!(inc.call::<()>((&au, &au)).is_ok());
        match inc.call::<()>((&au, &bu)) {
            Err(Error::CallbackError { ref cause, .. }) => match cause.as_ref() {
                Error::BadArgument { to, pos, name, cause } => {
                    assert_eq!(to.as_deref(), Some("MyUserData.inc"));
                    assert_eq!(*pos, 1);
                    assert_eq!(name.as_deref(), Some("self"));
                    assert!(matches!(*cause.as_ref(), Error::UserDataTypeMismatch));
                }
                other => panic!("wrong error type {other:?}"),
            },
            Err(other) => panic!("wrong error type {other:?}"),
            Ok(_) => panic!("incorrectly returned Ok"),
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
        Ok(_) => panic!("succesfull borrow for destructed userdata"),
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
        modify_userdata(&lua, ud)
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
        modify_userdata(&lua, ud)
    })?;
    assert_eq!(data.0, 2);

    Ok(())
}

#[test]
fn test_scope_any_userdata() -> Result<()> {
    let lua = Lua::new();

    lua.register_userdata_type::<StdString>(|reg| {
        reg.add_meta_method("__tostring", |_, data, ()| Ok(data.clone()));
    })?;

    let data = StdString::from("foo");
    lua.scope(|scope| {
        let ud = scope.create_any_userdata_ref(&data)?;
        lua.globals().set("ud", ud)?;
        lua.load("assert(tostring(ud) == 'foo')").exec()
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
        modify_userdata(&lua, ud)
    })?;
    assert_eq!(data.get(), 2);

    Ok(())
}

fn modify_userdata(lua: &Lua, ud: AnyUserData) -> Result<()> {
    let f: Function = lua
        .load(
            r#"
    function(u)
        u:inc()
        u:dec()
        u:inc()
    end
"#,
        )
        .eval()?;

    f.call::<()>(ud)?;

    Ok(())
}
