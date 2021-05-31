use std::sync::{Arc, Mutex, RwLock};

#[cfg(not(feature = "send"))]
use std::{cell::RefCell, rc::Rc};

#[cfg(feature = "lua54")]
use std::sync::atomic::{AtomicI64, Ordering};

use mlua::{
    AnyUserData, Error, ExternalError, Function, Lua, MetaMethod, Nil, Result, String, UserData,
    UserDataFields, UserDataMethods, Value,
};

#[test]
fn test_user_data() -> Result<()> {
    struct UserData1(i64);
    struct UserData2(Box<i64>);

    impl UserData for UserData1 {}
    impl UserData for UserData2 {}

    let lua = Lua::new();
    let userdata1 = lua.create_userdata(UserData1(1))?;
    let userdata2 = lua.create_userdata(UserData2(Box::new(2)))?;

    assert!(userdata1.is::<UserData1>());
    assert!(!userdata1.is::<UserData2>());
    assert!(userdata2.is::<UserData2>());
    assert!(!userdata2.is::<UserData1>());

    assert_eq!(userdata1.borrow::<UserData1>()?.0, 1);
    assert_eq!(*userdata2.borrow::<UserData2>()?.0, 2);

    Ok(())
}

#[test]
fn test_methods() -> Result<()> {
    struct MyUserData(i64);

    impl UserData for MyUserData {
        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_method("get_value", |_, data, ()| Ok(data.0));
            methods.add_method_mut("set_value", |_, data, args| {
                data.0 = args;
                Ok(())
            });
        }
    }

    let lua = Lua::new();
    let globals = lua.globals();
    let userdata = lua.create_userdata(MyUserData(42))?;
    globals.set("userdata", userdata.clone())?;
    lua.load(
        r#"
        function get_it()
            return userdata:get_value()
        end

        function set_it(i)
            return userdata:set_value(i)
        end
    "#,
    )
    .exec()?;
    let get = globals.get::<_, Function>("get_it")?;
    let set = globals.get::<_, Function>("set_it")?;
    assert_eq!(get.call::<_, i64>(())?, 42);
    userdata.borrow_mut::<MyUserData>()?.0 = 64;
    assert_eq!(get.call::<_, i64>(())?, 64);
    set.call::<_, ()>(100)?;
    assert_eq!(get.call::<_, i64>(())?, 100);

    Ok(())
}

#[test]
fn test_metamethods() -> Result<()> {
    #[derive(Copy, Clone)]
    struct MyUserData(i64);

    impl UserData for MyUserData {
        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_method("get", |_, data, ()| Ok(data.0));
            methods.add_meta_function(
                MetaMethod::Add,
                |_, (lhs, rhs): (MyUserData, MyUserData)| Ok(MyUserData(lhs.0 + rhs.0)),
            );
            methods.add_meta_function(
                MetaMethod::Sub,
                |_, (lhs, rhs): (MyUserData, MyUserData)| Ok(MyUserData(lhs.0 - rhs.0)),
            );
            methods.add_meta_function(MetaMethod::Eq, |_, (lhs, rhs): (MyUserData, MyUserData)| {
                Ok(lhs.0 == rhs.0)
            });
            methods.add_meta_method(MetaMethod::Index, |_, data, index: String| {
                if index.to_str()? == "inner" {
                    Ok(data.0)
                } else {
                    Err("no such custom index".to_lua_err())
                }
            });
            #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
            methods.add_meta_method(MetaMethod::Pairs, |lua, data, ()| {
                use std::iter::FromIterator;
                let stateless_iter = lua.create_function(|_, (data, i): (MyUserData, i64)| {
                    let i = i + 1;
                    if i <= data.0 {
                        return Ok(mlua::Variadic::from_iter(vec![i, i]));
                    }
                    return Ok(mlua::Variadic::new());
                })?;
                Ok((stateless_iter, data.clone(), 0))
            });
        }
    }

    let lua = Lua::new();
    let globals = lua.globals();
    globals.set("userdata1", MyUserData(7))?;
    globals.set("userdata2", MyUserData(3))?;
    globals.set("userdata3", MyUserData(3))?;
    assert_eq!(
        lua.load("userdata1 + userdata2").eval::<MyUserData>()?.0,
        10
    );

    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    let pairs_it = {
        lua.load(
            r#"
            function pairs_it()
                local r = 0
                for i, v in pairs(userdata1) do
                    r = r + v
                end
                return r
            end
        "#,
        )
        .exec()?;
        globals.get::<_, Function>("pairs_it")?
    };

    assert_eq!(lua.load("userdata1 - userdata2").eval::<MyUserData>()?.0, 4);
    assert_eq!(lua.load("userdata1:get()").eval::<i64>()?, 7);
    assert_eq!(lua.load("userdata2.inner").eval::<i64>()?, 3);
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    assert_eq!(pairs_it.call::<_, i64>(())?, 28);
    assert!(lua.load("userdata2.nonexist_field").eval::<()>().is_err());

    let userdata2: Value = globals.get("userdata2")?;
    let userdata3: Value = globals.get("userdata3")?;

    assert!(lua.load("userdata2 == userdata3").eval::<bool>()?);
    assert!(userdata2 != userdata3); // because references are differ
    assert!(userdata2.equals(userdata3)?);

    let userdata1: AnyUserData = globals.get("userdata1")?;
    assert!(userdata1.get_metatable()?.contains(MetaMethod::Add)?);
    assert!(userdata1.get_metatable()?.contains(MetaMethod::Sub)?);
    assert!(userdata1.get_metatable()?.contains(MetaMethod::Index)?);
    assert!(!userdata1.get_metatable()?.contains(MetaMethod::Pow)?);

    Ok(())
}

#[test]
#[cfg(feature = "lua54")]
fn test_metamethod_close() -> Result<()> {
    #[derive(Clone)]
    struct MyUserData(Arc<AtomicI64>);

    impl UserData for MyUserData {
        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_method("get", |_, data, ()| Ok(data.0.load(Ordering::Relaxed)));
            methods.add_meta_method(MetaMethod::Close, |_, data, _err: Value| {
                data.0.store(0, Ordering::Relaxed);
                Ok(())
            });
        }
    }

    let lua = Lua::new();
    let globals = lua.globals();

    let ud = MyUserData(Arc::new(AtomicI64::new(-1)));
    let ud2 = ud.clone();

    globals.set(
        "new_userdata",
        lua.create_function(move |_lua, val: i64| {
            let ud = ud2.clone();
            ud.0.store(val, Ordering::Relaxed);
            Ok(ud)
        })?,
    )?;

    lua.load(
        r#"
        do
            local ud <close> = new_userdata(7)
            assert(ud:get() == 7)
        end
    "#,
    )
    .exec()?;

    assert_eq!(ud.0.load(Ordering::Relaxed), 0);

    Ok(())
}

#[test]
fn test_gc_userdata() -> Result<()> {
    struct MyUserdata {
        id: u8,
    }

    impl UserData for MyUserdata {
        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_method("access", |_, this, ()| {
                assert!(this.id == 123);
                Ok(())
            });
        }
    }

    let lua = Lua::new();
    lua.globals().set("userdata", MyUserdata { id: 123 })?;

    assert!(lua
        .load(
            r#"
            local tbl = setmetatable({
                userdata = userdata
            }, { __gc = function(self)
                -- resurrect userdata
                hatch = self.userdata
            end })

            tbl = nil
            userdata = nil  -- make table and userdata collectable
            collectgarbage("collect")
            hatch:access()
        "#
        )
        .exec()
        .is_err());

    Ok(())
}

#[test]
fn test_destroy_userdata() -> Result<()> {
    struct MyUserdata(Arc<()>);

    impl UserData for MyUserdata {}

    let rc = Arc::new(());

    let lua = Lua::new();
    lua.globals().set("userdata", MyUserdata(rc.clone()))?;

    assert_eq!(Arc::strong_count(&rc), 2);

    // should destroy all objects
    let _ = lua.globals().raw_remove("userdata")?;
    lua.gc_collect()?;

    assert_eq!(Arc::strong_count(&rc), 1);

    Ok(())
}

#[test]
fn test_user_value() -> Result<()> {
    struct MyUserData;

    impl UserData for MyUserData {}

    let lua = Lua::new();
    let ud = lua.create_userdata(MyUserData)?;
    ud.set_user_value("hello")?;
    assert_eq!(ud.get_user_value::<String>()?, "hello");
    assert!(ud.get_user_value::<u32>().is_err());

    Ok(())
}

#[test]
fn test_functions() -> Result<()> {
    struct MyUserData(i64);

    impl UserData for MyUserData {
        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_function("get_value", |_, ud: AnyUserData| {
                Ok(ud.borrow::<MyUserData>()?.0)
            });
            methods.add_function_mut("set_value", |_, (ud, value): (AnyUserData, i64)| {
                ud.borrow_mut::<MyUserData>()?.0 = value;
                Ok(())
            });
            methods.add_function("get_constant", |_, ()| Ok(7));
        }
    }

    let lua = Lua::new();
    let globals = lua.globals();
    let userdata = lua.create_userdata(MyUserData(42))?;
    globals.set("userdata", userdata.clone())?;
    lua.load(
        r#"
        function get_it()
            return userdata:get_value()
        end

        function set_it(i)
            return userdata:set_value(i)
        end

        function get_constant()
            return userdata.get_constant()
        end
    "#,
    )
    .exec()?;
    let get = globals.get::<_, Function>("get_it")?;
    let set = globals.get::<_, Function>("set_it")?;
    let get_constant = globals.get::<_, Function>("get_constant")?;
    assert_eq!(get.call::<_, i64>(())?, 42);
    userdata.borrow_mut::<MyUserData>()?.0 = 64;
    assert_eq!(get.call::<_, i64>(())?, 64);
    set.call::<_, ()>(100)?;
    assert_eq!(get.call::<_, i64>(())?, 100);
    assert_eq!(get_constant.call::<_, i64>(())?, 7);

    Ok(())
}

#[test]
fn test_fields() -> Result<()> {
    #[derive(Copy, Clone)]
    struct MyUserData(i64);

    impl UserData for MyUserData {
        fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
            fields.add_field_method_get("val", |_, data| Ok(data.0));
            fields.add_field_method_set("val", |_, data, val| {
                data.0 = val;
                Ok(())
            });

            // Use userdata "uservalue" storage
            fields.add_field_function_get("uval", |_, ud| ud.get_user_value::<Option<String>>());
            fields
                .add_field_function_set("uval", |_, ud, s| ud.set_user_value::<Option<String>>(s));

            fields.add_meta_field_with(MetaMethod::Index, |lua| {
                let index = lua.create_table()?;
                index.set("f", 321)?;
                Ok(index)
            });
        }
    }

    let lua = Lua::new();
    let globals = lua.globals();
    globals.set("ud", MyUserData(7))?;
    lua.load(
        r#"
        assert(ud.val == 7)
        ud.val = 10
        assert(ud.val == 10)

        assert(ud.uval == nil)
        ud.uval = "hello"
        assert(ud.uval == "hello")

        assert(ud.f == 321)
    "#,
    )
    .exec()?;

    Ok(())
}

#[test]
fn test_metatable() -> Result<()> {
    #[derive(Copy, Clone)]
    struct MyUserData(i64);

    impl UserData for MyUserData {
        fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
            fields.add_meta_field_with("__type_name", |_| Ok("MyUserData"));
        }

        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_function("my_type_name", |_, data: AnyUserData| {
                let metatable = data.get_metatable()?;
                metatable.get::<_, String>("__type_name")
            });
        }
    }

    let lua = Lua::new();
    let globals = lua.globals();
    globals.set("ud", MyUserData(7))?;
    lua.load(
        r#"
        assert(ud:my_type_name() == "MyUserData")
    "#,
    )
    .exec()?;

    let ud: AnyUserData = globals.get("ud")?;
    let metatable = ud.get_metatable()?;

    match metatable.get::<_, Value>("__gc") {
        Ok(_) => panic!("expected MetaMethodRestricted, got no error"),
        Err(Error::MetaMethodRestricted(_)) => {}
        Err(e) => panic!("expected MetaMethodRestricted, got {:?}", e),
    }

    match metatable.set(MetaMethod::Index, Nil) {
        Ok(_) => panic!("expected MetaMethodRestricted, got no error"),
        Err(Error::MetaMethodRestricted(_)) => {}
        Err(e) => panic!("expected MetaMethodRestricted, got {:?}", e),
    }

    let mut methods = metatable
        .pairs()
        .into_iter()
        .map(|kv: Result<(_, Value)>| Ok(kv?.0))
        .collect::<Result<Vec<_>>>()?;
    methods.sort_by_cached_key(|k| k.name().to_owned());
    assert_eq!(methods, vec![MetaMethod::Index, "__type_name".into()]);

    #[derive(Copy, Clone)]
    struct MyUserData2(i64);

    impl UserData for MyUserData2 {
        fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
            fields.add_meta_field_with("__index", |_| Ok(1));
        }
    }

    match lua.create_userdata(MyUserData2(1)) {
        Ok(_) => panic!("expected MetaMethodTypeError, got no error"),
        Err(Error::MetaMethodTypeError { .. }) => {}
        Err(e) => panic!("expected MetaMethodTypeError, got {:?}", e),
    }

    Ok(())
}

#[test]
fn test_userdata_wrapped() -> Result<()> {
    struct MyUserData(i64);

    impl UserData for MyUserData {
        fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
            fields.add_field_method_get("data", |_, this| Ok(this.0));
            fields.add_field_method_set("data", |_, this, val| {
                this.0 = val;
                Ok(())
            })
        }
    }

    let lua = Lua::new();
    let globals = lua.globals();

    #[cfg(not(feature = "send"))]
    {
        globals.set("rc_refcell_ud", Rc::new(RefCell::new(MyUserData(1))))?;
        lua.load(
            r#"
            rc_refcell_ud.data = rc_refcell_ud.data + 1
            assert(rc_refcell_ud.data == 2)
        "#,
        )
        .exec()?;
    }

    globals.set("arc_mutex_ud", Arc::new(Mutex::new(MyUserData(2))))?;
    lua.load(
        r#"
        arc_mutex_ud.data = arc_mutex_ud.data + 1
        assert(arc_mutex_ud.data == 3)
    "#,
    )
    .exec()?;

    globals.set("arc_rwlock_ud", Arc::new(RwLock::new(MyUserData(3))))?;
    lua.load(
        r#"
        arc_rwlock_ud.data = arc_rwlock_ud.data + 1
        assert(arc_rwlock_ud.data == 4)
    "#,
    )
    .exec()?;

    Ok(())
}
