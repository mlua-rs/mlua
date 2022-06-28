use std::sync::Arc;
#[cfg(not(feature = "parking_lot"))]
use std::sync::{Mutex, RwLock};

#[cfg(feature = "parking_lot")]
use parking_lot::{Mutex, RwLock};

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
    #[cfg_attr(feature = "serialize", derive(serde::Serialize))]
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

    fn check_methods(lua: &Lua, userdata: AnyUserData) -> Result<()> {
        let globals = lua.globals();
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

    let lua = Lua::new();

    check_methods(&lua, lua.create_userdata(MyUserData(42))?)?;

    // Additionally check serializable userdata
    #[cfg(feature = "serialize")]
    check_methods(&lua, lua.create_ser_userdata(MyUserData(42))?)?;

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
            #[cfg(any(
                feature = "lua54",
                feature = "lua53",
                feature = "lua52",
                feature = "luajit52"
            ))]
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

    #[cfg(any(
        feature = "lua54",
        feature = "lua53",
        feature = "lua52",
        feature = "luajit52"
    ))]
    let pairs_it = lua
        .load(
            r#"
            function()
                local r = 0
                for i, v in pairs(userdata1) do
                    r = r + v
                end
                return r
            end
        "#,
        )
        .eval::<Function>()?;

    assert_eq!(lua.load("userdata1 - userdata2").eval::<MyUserData>()?.0, 4);
    assert_eq!(lua.load("userdata1:get()").eval::<i64>()?, 7);
    assert_eq!(lua.load("userdata2.inner").eval::<i64>()?, 3);
    assert!(lua.load("userdata2.nonexist_field").eval::<()>().is_err());

    #[cfg(any(
        feature = "lua54",
        feature = "lua53",
        feature = "lua52",
        feature = "luajit52"
    ))]
    assert_eq!(pairs_it.call::<_, i64>(())?, 28);

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
fn test_userdata_take() -> Result<()> {
    #[derive(Debug)]
    struct MyUserdata(Arc<i64>);

    impl UserData for MyUserdata {
        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_method("num", |_, this, ()| Ok(*this.0))
        }
    }

    #[cfg(feature = "serialize")]
    impl serde::Serialize for MyUserdata {
        fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            serializer.serialize_i64(*self.0)
        }
    }

    fn check_userdata_take(lua: &Lua, userdata: AnyUserData, rc: Arc<i64>) -> Result<()> {
        lua.globals().set("userdata", userdata.clone())?;
        assert_eq!(Arc::strong_count(&rc), 3);
        let userdata_copy = userdata.clone();
        {
            let _value = userdata.borrow::<MyUserdata>()?;
            // We should not be able to take userdata if it's borrowed
            match userdata_copy.take::<MyUserdata>() {
                Err(Error::UserDataBorrowMutError) => {}
                r => panic!("expected `UserDataBorrowMutError` error, got {:?}", r),
            }
        }

        let value = userdata_copy.take::<MyUserdata>()?;
        assert_eq!(*value.0, 18);
        drop(value);
        lua.gc_collect()?;
        assert_eq!(Arc::strong_count(&rc), 1);

        match userdata.borrow::<MyUserdata>() {
            Err(Error::UserDataDestructed) => {}
            r => panic!("expected `UserDataDestructed` error, got {:?}", r),
        }
        match lua.load("userdata:num()").exec() {
            Err(Error::CallbackError { ref cause, .. }) => match cause.as_ref() {
                Error::CallbackDestructed => {}
                err => panic!("expected `CallbackDestructed`, got {:?}", err),
            },
            r => panic!("improper return for destructed userdata: {:?}", r),
        }
        Ok(())
    }

    let lua = Lua::new();

    let rc = Arc::new(18);
    let userdata = lua.create_userdata(MyUserdata(rc.clone()))?;
    userdata.set_nth_user_value(2, MyUserdata(rc.clone()))?;
    check_userdata_take(&lua, userdata, rc)?;

    // Additionally check serializable userdata
    #[cfg(feature = "serialize")]
    {
        let rc = Arc::new(18);
        let userdata = lua.create_ser_userdata(MyUserdata(rc.clone()))?;
        userdata.set_nth_user_value(2, MyUserdata(rc.clone()))?;
        check_userdata_take(&lua, userdata, rc)?;
    }

    Ok(())
}

#[test]
fn test_userdata_destroy() -> Result<()> {
    struct MyUserdata(Arc<()>);

    impl UserData for MyUserdata {}

    let rc = Arc::new(());

    let lua = Lua::new();
    let ud = lua.create_userdata(MyUserdata(rc.clone()))?;
    ud.set_user_value(MyUserdata(rc.clone()))?;
    lua.globals().set("userdata", ud)?;

    assert_eq!(Arc::strong_count(&rc), 3);

    // Should destroy all objects
    lua.globals().raw_remove("userdata")?;
    lua.gc_collect()?;
    lua.gc_collect()?;

    assert_eq!(Arc::strong_count(&rc), 1);

    Ok(())
}

#[test]
fn test_user_values() -> Result<()> {
    struct MyUserData;

    impl UserData for MyUserData {}

    let lua = Lua::new();
    let ud = lua.create_userdata(MyUserData)?;

    ud.set_nth_user_value(1, "hello")?;
    ud.set_nth_user_value(2, "world")?;
    ud.set_nth_user_value(65535, 321)?;
    assert_eq!(ud.get_nth_user_value::<String>(1)?, "hello");
    assert_eq!(ud.get_nth_user_value::<String>(2)?, "world");
    assert_eq!(ud.get_nth_user_value::<Value>(3)?, Value::Nil);
    assert_eq!(ud.get_nth_user_value::<i32>(65535)?, 321);

    assert!(ud.get_nth_user_value::<Value>(0).is_err());
    assert!(ud.get_nth_user_value::<Value>(65536).is_err());

    // Named user values
    ud.set_named_user_value("name", "alex")?;
    ud.set_named_user_value("age", 10)?;

    assert_eq!(ud.get_named_user_value::<_, String>("name")?, "alex");
    assert_eq!(ud.get_named_user_value::<_, i32>("age")?, 10);
    assert_eq!(ud.get_named_user_value::<_, Value>("nonexist")?, Value::Nil);

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
            fields.add_meta_field_with(MetaMethod::NewIndex, |lua| {
                lua.create_function(|lua, (_, field, val): (AnyUserData, String, Value)| {
                    lua.globals().set(field, val)?;
                    Ok(())
                })
            })
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

        ud.unknown = 789
        assert(unknown == 789)
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
        let ud1 = Rc::new(RefCell::new(MyUserData(1)));
        globals.set("rc_refcell_ud", ud1.clone())?;
        lua.load(
            r#"
            rc_refcell_ud.data = rc_refcell_ud.data + 1
            assert(rc_refcell_ud.data == 2)
        "#,
        )
        .exec()?;
        assert_eq!(ud1.borrow().0, 2);
        globals.set("rc_refcell_ud", Nil)?;
        lua.gc_collect()?;
        assert_eq!(Rc::strong_count(&ud1), 1);
    }

    let ud2 = Arc::new(Mutex::new(MyUserData(2)));
    globals.set("arc_mutex_ud", ud2.clone())?;
    lua.load(
        r#"
        arc_mutex_ud.data = arc_mutex_ud.data + 1
        assert(arc_mutex_ud.data == 3)
    "#,
    )
    .exec()?;
    #[cfg(not(feature = "parking_lot"))]
    assert_eq!(ud2.lock().unwrap().0, 3);
    #[cfg(feature = "parking_lot")]
    assert_eq!(ud2.lock().0, 3);

    let ud3 = Arc::new(RwLock::new(MyUserData(3)));
    globals.set("arc_rwlock_ud", ud3.clone())?;
    lua.load(
        r#"
        arc_rwlock_ud.data = arc_rwlock_ud.data + 1
        assert(arc_rwlock_ud.data == 4)
    "#,
    )
    .exec()?;
    #[cfg(not(feature = "parking_lot"))]
    assert_eq!(ud3.read().unwrap().0, 4);
    #[cfg(feature = "parking_lot")]
    assert_eq!(ud3.read().0, 4);

    // Test drop
    globals.set("arc_mutex_ud", Nil)?;
    globals.set("arc_rwlock_ud", Nil)?;
    lua.gc_collect()?;
    assert_eq!(Arc::strong_count(&ud2), 1);
    assert_eq!(Arc::strong_count(&ud3), 1);

    Ok(())
}

#[test]
fn test_userdata_proxy() -> Result<()> {
    struct MyUserData(i64);

    impl UserData for MyUserData {
        fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
            fields.add_field_function_get("static_field", |_, _| Ok(123));
            fields.add_field_method_get("n", |_, this| Ok(this.0));
        }

        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_function("new", |_, n| Ok(Self(n)));

            methods.add_method("plus", |_, this, n: i64| Ok(this.0 + n));
        }
    }

    let lua = Lua::new();
    let globals = lua.globals();
    globals.set("MyUserData", lua.create_proxy::<MyUserData>()?)?;

    lua.load(
        r#"
        assert(MyUserData.static_field == 123)
        local data = MyUserData.new(321)
        assert(data.static_field == 123)
        assert(data.n == 321)
        assert(data:plus(1) == 322)

        -- Error when accessing the proxy object fields and methods that require instance

        local ok = pcall(function() return MyUserData.n end)
        assert(not ok)

        ok = pcall(function() return MyUserData:plus(1) end)
        assert(not ok)
    "#,
    )
    .exec()
}
