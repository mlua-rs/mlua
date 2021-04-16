use std::cell::Cell;
use std::rc::Rc;

use mlua::{
    AnyUserData, Error, Function, Lua, MetaMethod, Result, String, UserData, UserDataMethods,
};

#[test]
fn scope_func() -> Result<()> {
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
        Err(Error::CallbackError { .. }) => {}
        r => panic!("improper return for destructed function: {:?}", r),
    };

    Ok(())
}

#[test]
fn scope_drop() -> Result<()> {
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
            .set("static_ud", scope.create_userdata(MyUserdata(rc.clone()))?)?;
        assert_eq!(Rc::strong_count(&rc), 2);
        Ok(())
    })?;
    assert_eq!(Rc::strong_count(&rc), 1);

    match lua.load("static_ud:method()").exec() {
        Err(Error::CallbackError { ref cause, .. }) => match cause.as_ref() {
            Error::CallbackDestructed => {}
            e => panic!("expected CallbackDestructed, got {:?}", e),
        },
        r => panic!("improper return for destructed userdata: {:?}", r),
    };

    let static_ud = lua.globals().get::<_, AnyUserData>("static_ud")?;
    match static_ud.borrow::<MyUserdata>() {
        Ok(_) => panic!("borrowed destructed userdata"),
        Err(Error::UserDataDestructed) => {}
        Err(e) => panic!("expected UserDataDestructed, got {:?}", e),
    }

    // Check non-static UserData drop
    struct MyUserDataRef<'a>(&'a Cell<i64>);

    impl<'a> UserData for MyUserDataRef<'a> {
        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_method("inc", |_, data, ()| {
                data.0.set(data.0.get() + 1);
                Ok(())
            });
        }
    }

    let i = Cell::new(1);
    lua.scope(|scope| {
        lua.globals().set(
            "nonstatic_ud",
            scope.create_nonstatic_userdata(MyUserDataRef(&i))?,
        )
    })?;

    match lua.load("nonstatic_ud:inc(1)").exec() {
        Err(Error::CallbackError { ref cause, .. }) => match cause.as_ref() {
            Error::CallbackDestructed => {}
            e => panic!("expected CallbackDestructed, got {:?}", e),
        },
        r => panic!("improper return for destructed userdata: {:?}", r),
    };

    let nonstatic_ud = lua.globals().get::<_, AnyUserData>("nonstatic_ud")?;
    match nonstatic_ud.borrow::<MyUserDataRef>() {
        Ok(_) => panic!("borrowed destructed userdata"),
        Err(Error::UserDataDestructed) => {}
        Err(e) => panic!("expected UserDataDestructed, got {:?}", e),
    }

    Ok(())
}

#[test]
fn scope_capture() -> Result<()> {
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
fn outer_lua_access() -> Result<()> {
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
fn scope_userdata_methods() -> Result<()> {
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
fn scope_userdata_functions() -> Result<()> {
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
fn scope_userdata_mismatch() -> Result<()> {
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
