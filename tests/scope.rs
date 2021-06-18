use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use mlua::{
    AnyUserData, Error, Function, Lua, MetaMethod, Result, String, UserData, UserDataFields,
    UserDataMethods,
};

#[test]
fn test_scope_func() -> Result<()> {
    let lua = Lua::new();

    let rc = Rc::new(Cell::new(0));
    lua.scope(|scope| {
        let r = rc.clone();
        let f = scope.create_function(move |_, ()| {
            r.set(42);
            Ok(())
        })?;
        lua.globals().set("bad", f.clone())?;
        f.call::<_, ()>(())?;
        assert_eq!(Rc::strong_count(&rc), 2);
        Ok(())
    })?;
    assert_eq!(rc.get(), 42);
    assert_eq!(Rc::strong_count(&rc), 1);

    match lua.globals().get::<_, Function>("bad")?.call::<_, ()>(()) {
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
            .call::<_, ()>(())
    })?;
    assert_eq!(i, 42);

    Ok(())
}

#[test]
fn test_scope_outer_lua_access() -> Result<()> {
    let lua = Lua::new();

    let table = lua.create_table()?;
    lua.scope(|scope| {
        scope
            .create_function_mut(|_, ()| table.set("a", "b"))?
            .call::<_, ()>(())
    })?;
    assert_eq!(table.get::<_, String>("a")?, "b");

    Ok(())
}

#[test]
fn test_scope_userdata_fields() -> Result<()> {
    struct MyUserData<'a>(&'a Cell<i64>);

    impl<'a> UserData for MyUserData<'a> {
        fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
            fields.add_field_method_get("val", |_, data| Ok(data.0.get()));
            fields.add_field_method_set("val", |_, data, val| {
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
                assert(u.val == 42)
                u.val = 44
            end
        "#,
        )
        .eval()?;

    lua.scope(|scope| f.call::<_, ()>(scope.create_nonstatic_userdata(MyUserData(&i))?))?;

    assert_eq!(i.get(), 44);

    Ok(())
}

#[test]
fn test_scope_userdata_methods() -> Result<()> {
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

    lua.scope(|scope| f.call::<_, ()>(scope.create_nonstatic_userdata(MyUserData(&i))?))?;

    assert_eq!(i.get(), 44);

    Ok(())
}

#[test]
fn test_scope_userdata_functions() -> Result<()> {
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

    let dummy = 0;
    let f = lua
        .load(
            r#"
            i = 0
            return function(u)
                _ = u + u
                _ = u - 1
                _ = 1 + u
            end
        "#,
        )
        .eval::<Function>()?;

    lua.scope(|scope| f.call::<_, ()>(scope.create_nonstatic_userdata(MyUserData(&dummy))?))?;

    assert_eq!(lua.globals().get::<_, i64>("i")?, 3);

    Ok(())
}

#[test]
fn test_scope_userdata_mismatch() -> Result<()> {
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

    lua.load(
        r#"
        function okay(a, b)
            a.inc(a)
            b.inc(b)
        end
        function bad(a, b)
            a.inc(b)
        end
    "#,
    )
    .exec()?;

    let a = Cell::new(1);
    let b = Cell::new(1);

    let okay: Function = lua.globals().get("okay")?;
    let bad: Function = lua.globals().get("bad")?;

    lua.scope(|scope| {
        let au = scope.create_nonstatic_userdata(MyUserData(&a))?;
        let bu = scope.create_nonstatic_userdata(MyUserData(&b))?;
        assert!(okay.call::<_, ()>((au.clone(), bu.clone())).is_ok());
        match bad.call::<_, ()>((au, bu)) {
            Err(Error::CallbackError { ref cause, .. }) => match *cause.as_ref() {
                Error::UserDataTypeMismatch => {}
                ref other => panic!("wrong error type {:?}", other),
            },
            Err(other) => panic!("wrong error type {:?}", other),
            Ok(_) => panic!("incorrectly returned Ok"),
        }
        Ok(())
    })?;

    Ok(())
}

#[test]
fn test_scope_userdata_drop() -> Result<()> {
    let lua = Lua::new();

    struct MyUserData(Rc<()>);

    impl UserData for MyUserData {
        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_method("method", |_, _, ()| Ok(()));
        }
    }

    struct MyUserDataArc(Arc<()>);

    impl UserData for MyUserDataArc {}

    let rc = Rc::new(());
    let arc = Arc::new(());
    lua.scope(|scope| {
        let ud = scope.create_userdata(MyUserData(rc.clone()))?;
        ud.set_user_value(MyUserDataArc(arc.clone()))?;
        lua.globals().set("ud", ud)?;
        assert_eq!(Rc::strong_count(&rc), 2);
        assert_eq!(Arc::strong_count(&arc), 2);
        Ok(())
    })?;

    lua.gc_collect()?;
    assert_eq!(Rc::strong_count(&rc), 1);
    assert_eq!(Arc::strong_count(&arc), 1);

    match lua.load("ud:method()").exec() {
        Err(Error::CallbackError { ref cause, .. }) => match cause.as_ref() {
            Error::CallbackDestructed => {}
            err => panic!("expected CallbackDestructed, got {:?}", err),
        },
        r => panic!("improper return for destructed userdata: {:?}", r),
    };

    let ud = lua.globals().get::<_, AnyUserData>("ud")?;
    match ud.borrow::<MyUserData>() {
        Ok(_) => panic!("succesfull borrow for destructed userdata"),
        Err(Error::UserDataDestructed) => {}
        Err(err) => panic!("improper borrow error for destructed userdata: {:?}", err),
    }

    match ud.get_metatable() {
        Ok(_) => panic!("successful metatable retrieval of destructed userdata"),
        Err(Error::UserDataDestructed) => {}
        Err(err) => panic!(
            "improper metatable error for destructed userdata: {:?}",
            err
        ),
    }

    Ok(())
}

#[test]
fn test_scope_nonstatic_userdata_drop() -> Result<()> {
    let lua = Lua::new();

    struct MyUserData<'a>(&'a Cell<i64>, Arc<()>);

    impl<'a> UserData for MyUserData<'a> {
        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_method("inc", |_, data, ()| {
                data.0.set(data.0.get() + 1);
                Ok(())
            });
        }
    }

    struct MyUserDataArc(Arc<()>);

    impl UserData for MyUserDataArc {}

    let i = Cell::new(1);
    let arc = Arc::new(());
    lua.scope(|scope| {
        let ud = scope.create_nonstatic_userdata(MyUserData(&i, arc.clone()))?;
        ud.set_user_value(MyUserDataArc(arc.clone()))?;
        lua.globals().set("ud", ud)?;
        lua.load("ud:inc()").exec()?;
        assert_eq!(Arc::strong_count(&arc), 3);
        Ok(())
    })?;

    lua.gc_collect()?;
    assert_eq!(Arc::strong_count(&arc), 1);

    match lua.load("ud:inc()").exec() {
        Err(Error::CallbackError { ref cause, .. }) => match cause.as_ref() {
            Error::CallbackDestructed => {}
            err => panic!("expected CallbackDestructed, got {:?}", err),
        },
        r => panic!("improper return for destructed userdata: {:?}", r),
    };

    let ud = lua.globals().get::<_, AnyUserData>("ud")?;
    match ud.borrow::<MyUserData>() {
        Ok(_) => panic!("succesfull borrow for destructed userdata"),
        Err(Error::UserDataDestructed) => {}
        Err(err) => panic!("improper borrow error for destructed userdata: {:?}", err),
    }
    match ud.get_metatable() {
        Ok(_) => panic!("successful metatable retrieval of destructed userdata"),
        Err(Error::UserDataDestructed) => {}
        Err(err) => panic!(
            "improper metatable error for destructed userdata: {:?}",
            err
        ),
    }

    Ok(())
}
