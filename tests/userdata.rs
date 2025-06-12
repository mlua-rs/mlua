use std::any::TypeId;
use std::collections::HashMap;
use std::string::String as StdString;
use std::sync::Arc;

#[cfg(feature = "lua54")]
use std::sync::atomic::{AtomicI64, Ordering};

use mlua::{
    AnyUserData, Error, ExternalError, Function, Lua, MetaMethod, Nil, ObjectLike, Result, String, UserData,
    UserDataFields, UserDataMethods, UserDataRef, Value, Variadic,
};

#[test]
fn test_userdata() -> Result<()> {
    struct UserData1(i64);
    struct UserData2(Box<i64>);

    impl UserData for UserData1 {}
    impl UserData for UserData2 {}

    let lua = Lua::new();
    let userdata1 = lua.create_userdata(UserData1(1))?;
    let userdata2 = lua.create_userdata(UserData2(Box::new(2)))?;

    assert!(userdata1.is::<UserData1>());
    assert!(userdata1.type_id() == Some(TypeId::of::<UserData1>()));
    assert!(!userdata1.is::<UserData2>());
    assert!(userdata2.is::<UserData2>());
    assert!(!userdata2.is::<UserData1>());
    assert!(userdata2.type_id() == Some(TypeId::of::<UserData2>()));

    assert_eq!(userdata1.borrow::<UserData1>()?.0, 1);
    assert_eq!(*userdata2.borrow::<UserData2>()?.0, 2);

    Ok(())
}

#[test]
fn test_methods() -> Result<()> {
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    struct MyUserData(i64);

    impl UserData for MyUserData {
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_method("get_value", |_, data, ()| Ok(data.0));
            methods.add_method_mut("set_value", |_, data, args| {
                data.0 = args;
                Ok(())
            });
        }
    }

    fn check_methods(lua: &Lua, userdata: AnyUserData) -> Result<()> {
        let globals = lua.globals();
        globals.set("userdata", &userdata)?;
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
        let get = globals.get::<Function>("get_it")?;
        let set = globals.get::<Function>("set_it")?;
        assert_eq!(get.call::<i64>(())?, 42);
        userdata.borrow_mut::<MyUserData>()?.0 = 64;
        assert_eq!(get.call::<i64>(())?, 64);
        set.call::<()>(100)?;
        assert_eq!(get.call::<i64>(())?, 100);
        Ok(())
    }

    let lua = Lua::new();

    check_methods(&lua, lua.create_userdata(MyUserData(42))?)?;

    // Additionally check serializable userdata
    #[cfg(feature = "serde")]
    check_methods(&lua, lua.create_ser_userdata(MyUserData(42))?)?;

    Ok(())
}

#[test]
fn test_method_variadic() -> Result<()> {
    struct MyUserData(i64);

    impl UserData for MyUserData {
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_method("get", |_, data, ()| Ok(data.0));
            methods.add_method_mut("add", |_, data, vals: Variadic<i64>| {
                data.0 += vals.into_iter().sum::<i64>();
                Ok(())
            });
        }
    }

    let lua = Lua::new();
    let globals = lua.globals();
    globals.set("userdata", MyUserData(0))?;
    lua.load("userdata:add(1, 5, -10)").exec()?;
    let ud: UserDataRef<MyUserData> = globals.get("userdata")?;
    assert_eq!(ud.0, -4);

    Ok(())
}

#[test]
fn test_metamethods() -> Result<()> {
    #[derive(Copy, Clone)]
    struct MyUserData(i64);

    impl UserData for MyUserData {
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_method("get", |_, data, ()| Ok(data.0));
            methods.add_meta_function(
                MetaMethod::Add,
                |_, (lhs, rhs): (UserDataRef<Self>, UserDataRef<Self>)| Ok(MyUserData(lhs.0 + rhs.0)),
            );
            methods.add_meta_function(
                MetaMethod::Sub,
                |_, (lhs, rhs): (UserDataRef<Self>, UserDataRef<Self>)| Ok(MyUserData(lhs.0 - rhs.0)),
            );
            methods.add_meta_function(
                MetaMethod::Eq,
                |_, (lhs, rhs): (UserDataRef<Self>, UserDataRef<Self>)| Ok(lhs.0 == rhs.0),
            );
            methods.add_meta_method(MetaMethod::Index, |_, data, index: String| {
                if index.to_str()? == "inner" {
                    Ok(data.0)
                } else {
                    Err("no such custom index".into_lua_err())
                }
            });
            #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luajit52"))]
            methods.add_meta_method(MetaMethod::Pairs, |lua, data, ()| {
                use std::iter::FromIterator;
                let stateless_iter = lua.create_function(|_, (data, i): (UserDataRef<Self>, i64)| {
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
        lua.load("userdata1 + userdata2")
            .eval::<UserDataRef<MyUserData>>()?
            .0,
        10
    );

    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luajit52"))]
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

    assert_eq!(
        lua.load("userdata1 - userdata2")
            .eval::<UserDataRef<MyUserData>>()?
            .0,
        4
    );
    assert_eq!(lua.load("userdata1:get()").eval::<i64>()?, 7);
    assert_eq!(lua.load("userdata2.inner").eval::<i64>()?, 3);
    assert!(lua.load("userdata2.nonexist_field").eval::<()>().is_err());

    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luajit52"))]
    assert_eq!(pairs_it.call::<i64>(())?, 28);

    let userdata2: Value = globals.get("userdata2")?;
    let userdata3: Value = globals.get("userdata3")?;

    assert!(lua.load("userdata2 == userdata3").eval::<bool>()?);
    assert!(userdata2 != userdata3); // because references are differ
    assert!(userdata2.equals(&userdata3)?);

    let userdata1: AnyUserData = globals.get("userdata1")?;
    assert!(userdata1.metatable()?.contains(MetaMethod::Add)?);
    assert!(userdata1.metatable()?.contains(MetaMethod::Sub)?);
    assert!(userdata1.metatable()?.contains(MetaMethod::Index)?);
    assert!(!userdata1.metatable()?.contains(MetaMethod::Pow)?);

    Ok(())
}

#[cfg(feature = "lua54")]
#[test]
fn test_metamethod_close() -> Result<()> {
    #[derive(Clone)]
    struct MyUserData(Arc<AtomicI64>);

    impl UserData for MyUserData {
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
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
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_method("access", |_, this, ()| {
                assert_eq!(this.id, 123);
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
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_method("num", |_, this, ()| Ok(*this.0))
        }
    }

    #[cfg(feature = "serde")]
    impl serde::Serialize for MyUserdata {
        fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            serializer.serialize_i64(*self.0)
        }
    }

    fn check_userdata_take(lua: &Lua, userdata: AnyUserData, rc: Arc<i64>) -> Result<()> {
        lua.globals().set("userdata", &userdata)?;
        assert_eq!(Arc::strong_count(&rc), 3);
        {
            let _value = userdata.borrow::<MyUserdata>()?;
            // We should not be able to take userdata if it's borrowed
            match userdata.take::<MyUserdata>() {
                Err(Error::UserDataBorrowMutError) => {}
                r => panic!("expected `UserDataBorrowMutError` error, got {:?}", r),
            }
        }

        let value = userdata.take::<MyUserdata>()?;
        assert_eq!(*value.0, 18);
        drop(value);
        assert_eq!(Arc::strong_count(&rc), 2);

        match userdata.borrow::<MyUserdata>() {
            Err(Error::UserDataDestructed) => {}
            r => panic!("expected `UserDataDestructed` error, got {:?}", r),
        }
        match lua.load("userdata:num()").exec() {
            Err(Error::CallbackError { ref cause, .. }) => match cause.as_ref() {
                Error::UserDataDestructed => {}
                err => panic!("expected `UserDataDestructed`, got {:?}", err),
            },
            r => panic!("improper return for destructed userdata: {:?}", r),
        }

        assert!(!userdata.is::<MyUserdata>());

        drop(userdata);
        lua.globals().raw_remove("userdata")?;
        lua.gc_collect()?;
        lua.gc_collect()?;
        assert_eq!(Arc::strong_count(&rc), 1);

        Ok(())
    }

    let lua = Lua::new();

    let rc = Arc::new(18);
    let userdata = lua.create_userdata(MyUserdata(rc.clone()))?;
    userdata.set_nth_user_value(2, MyUserdata(rc.clone()))?;
    check_userdata_take(&lua, userdata, rc)?;

    // Additionally check serializable userdata
    #[cfg(feature = "serde")]
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
    struct MyUserdata(#[allow(unused)] Arc<()>);

    impl UserData for MyUserdata {
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_method("try_destroy", |lua, _this, ()| {
                let ud = lua.globals().get::<AnyUserData>("ud")?;
                match ud.destroy() {
                    Err(Error::UserDataBorrowMutError) => {}
                    r => panic!("expected `UserDataBorrowMutError` error, got {:?}", r),
                }
                Ok(())
            });
        }
    }

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

    let ud = lua.create_userdata(MyUserdata(rc.clone()))?;
    assert_eq!(Arc::strong_count(&rc), 2);
    let ud_ref = ud.borrow::<MyUserdata>()?;
    // With active `UserDataRef` this methods only marks userdata as destructed
    // without running destructor
    ud.destroy().unwrap();
    assert_eq!(Arc::strong_count(&rc), 2);
    drop(ud_ref);
    assert_eq!(Arc::strong_count(&rc), 1);

    // We cannot destroy (internally) borrowed userdata
    let ud = lua.create_userdata(MyUserdata(rc.clone()))?;
    lua.globals().set("ud", &ud)?;
    lua.load("ud:try_destroy()").exec().unwrap();
    ud.destroy().unwrap();
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
    assert_eq!(ud.nth_user_value::<String>(1)?, "hello");
    assert_eq!(ud.nth_user_value::<String>(2)?, "world");
    assert_eq!(ud.nth_user_value::<Value>(3)?, Value::Nil);
    assert_eq!(ud.nth_user_value::<i32>(65535)?, 321);

    assert!(ud.nth_user_value::<Value>(0).is_err());
    assert!(ud.nth_user_value::<Value>(65536).is_err());

    // Named user values
    let ud = lua.create_userdata(MyUserData)?;
    ud.set_named_user_value("name", "alex")?;
    ud.set_named_user_value("age", 10)?;

    assert_eq!(ud.named_user_value::<String>("name")?, "alex");
    assert_eq!(ud.named_user_value::<i32>("age")?, 10);
    assert_eq!(ud.named_user_value::<Value>("nonexist")?, Value::Nil);

    Ok(())
}

#[test]
fn test_functions() -> Result<()> {
    struct MyUserData(i64);

    impl UserData for MyUserData {
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_function("get_value", |_, ud: AnyUserData| Ok(ud.borrow::<MyUserData>()?.0));
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
    globals.set("userdata", &userdata)?;
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
    let get = globals.get::<Function>("get_it")?;
    let set = globals.get::<Function>("set_it")?;
    let get_constant = globals.get::<Function>("get_constant")?;
    assert_eq!(get.call::<i64>(())?, 42);
    userdata.borrow_mut::<MyUserData>()?.0 = 64;
    assert_eq!(get.call::<i64>(())?, 64);
    set.call::<()>(100)?;
    assert_eq!(get.call::<i64>(())?, 100);
    assert_eq!(get_constant.call::<i64>(())?, 7);

    Ok(())
}

#[test]
fn test_fields() -> Result<()> {
    let lua = Lua::new();
    let globals = lua.globals();

    #[derive(Copy, Clone)]
    struct MyUserData(i64);

    impl UserData for MyUserData {
        fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
            fields.add_field("static", "constant");
            fields.add_field_method_get("val", |_, data| Ok(data.0));
            fields.add_field_method_set("val", |_, data, val| {
                data.0 = val;
                Ok(())
            });

            // Use userdata "uservalue" storage
            fields.add_field_function_get("uval", |_, ud| ud.user_value::<Option<String>>());
            fields.add_field_function_set("uval", |_, ud, s: Option<String>| ud.set_user_value(s));

            fields.add_meta_field(MetaMethod::Index, HashMap::from([("f", 321)]));
            fields.add_meta_field_with(MetaMethod::NewIndex, |lua| {
                lua.create_function(|lua, (_, field, val): (AnyUserData, String, Value)| {
                    lua.globals().set(field, val)?;
                    Ok(())
                })
            })
        }
    }

    globals.set("ud", MyUserData(7))?;
    lua.load(
        r#"
        assert(ud.static == "constant")
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

    // Case: fields + __index metamethod (function)
    struct MyUserData2(i64);

    impl UserData for MyUserData2 {
        fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
            fields.add_field("z", 0);
            fields.add_field_method_get("x", |_, data| Ok(data.0));
        }

        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_meta_method(MetaMethod::Index, |_, _, name: StdString| match &*name {
                "y" => Ok(Some(-1)),
                _ => Ok(None),
            });
        }
    }

    globals.set("ud", MyUserData2(1))?;
    lua.load(
        r#"
        assert(ud.x == 1)
        assert(ud.y == -1)
        assert(ud.z == 0)
    "#,
    )
    .exec()?;

    Ok(())
}

#[test]
fn test_metatable() -> Result<()> {
    #[derive(Copy, Clone)]
    struct MyUserData;

    impl UserData for MyUserData {
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_function("my_type_name", |_, data: AnyUserData| {
                let metatable = data.metatable()?;
                metatable.get::<String>(MetaMethod::Type)
            });
        }
    }

    let lua = Lua::new();
    let globals = lua.globals();
    globals.set("ud", MyUserData)?;
    lua.load(r#"assert(ud:my_type_name() == "MyUserData")"#).exec()?;

    #[cfg(any(feature = "lua54", feature = "lua53", feature = "luau"))]
    lua.load(r#"assert(tostring(ud):sub(1, 11) == "MyUserData:")"#)
        .exec()?;
    #[cfg(feature = "luau")]
    lua.load(r#"assert(typeof(ud) == "MyUserData")"#).exec()?;

    let ud: AnyUserData = globals.get("ud")?;
    let metatable = ud.metatable()?;

    match metatable.get::<Value>("__gc") {
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
        .map(|kv: Result<(_, Value)>| Ok(kv?.0))
        .collect::<Result<Vec<_>>>()?;
    methods.sort();
    assert_eq!(methods, vec!["__index", MetaMethod::Type.name()]);

    #[derive(Copy, Clone)]
    struct MyUserData2;

    impl UserData for MyUserData2 {
        fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
            fields.add_meta_field_with("__index", |_| Ok(1));
        }
    }

    match lua.create_userdata(MyUserData2) {
        Ok(_) => panic!("expected MetaMethodTypeError, got no error"),
        Err(Error::MetaMethodTypeError { .. }) => {}
        Err(e) => panic!("expected MetaMethodTypeError, got {:?}", e),
    }

    #[derive(Copy, Clone)]
    struct MyUserData3;

    impl UserData for MyUserData3 {
        fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
            fields.add_meta_field_with(MetaMethod::Type, |_| Ok("CustomName"));
        }
    }

    let ud = lua.create_userdata(MyUserData3)?;
    let metatable = ud.metatable()?;
    assert_eq!(metatable.get::<String>(MetaMethod::Type)?.to_str()?, "CustomName");

    Ok(())
}

#[test]
fn test_userdata_proxy() -> Result<()> {
    struct MyUserData(i64);

    impl UserData for MyUserData {
        fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
            fields.add_field("static_field", 123);
            fields.add_field_method_get("n", |_, this| Ok(this.0));
        }

        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
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

#[test]
fn test_any_userdata() -> Result<()> {
    let lua = Lua::new();

    lua.register_userdata_type::<StdString>(|reg| {
        reg.add_method("get", |_, this, ()| Ok(this.clone()));
        reg.add_method_mut("concat", |_, this, s: String| {
            this.push_str(&s.to_string_lossy());
            Ok(())
        });
    })?;

    let ud = lua.create_any_userdata("hello".to_string())?;
    assert_eq!(&*ud.borrow::<StdString>()?, "hello");

    lua.globals().set("ud", ud)?;
    lua.load(
        r#"
        assert(ud:get() == "hello")
        ud:concat(", world")
        assert(ud:get() == "hello, world")
    "#,
    )
    .exec()
    .unwrap();

    Ok(())
}

#[test]
fn test_any_userdata_wrap() -> Result<()> {
    let lua = Lua::new();

    lua.register_userdata_type::<StdString>(|reg| {
        reg.add_method("get", |_, this, ()| Ok(this.clone()));
    })?;

    lua.globals().set("s", AnyUserData::wrap("hello".to_string()))?;
    lua.load(
        r#"
        assert(s:get() == "hello")
    "#,
    )
    .exec()
    .unwrap();

    Ok(())
}

#[test]
fn test_userdata_object_like() -> Result<()> {
    let lua = Lua::new();

    #[derive(Clone, Copy)]
    struct MyUserData(u32);

    impl UserData for MyUserData {
        fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
            fields.add_field_method_get("n", |_, this| Ok(this.0));
            fields.add_field_method_set("n", |_, this, val| {
                this.0 = val;
                Ok(())
            });
        }

        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_meta_method(MetaMethod::Call, |_, _this, ()| Ok("called"));
            methods.add_method_mut("add", |_, this, x: u32| {
                this.0 += x;
                Ok(())
            });
        }
    }

    let ud = lua.create_userdata(MyUserData(123))?;

    assert_eq!(ud.get::<u32>("n")?, 123);
    ud.set("n", 321)?;
    assert_eq!(ud.get::<u32>("n")?, 321);
    assert_eq!(ud.get::<Option<u32>>("non-existent")?, None);
    match ud.set("non-existent", 123) {
        Err(Error::RuntimeError(_)) => {}
        r => panic!("expected RuntimeError, got {r:?}"),
    }

    assert_eq!(ud.call::<String>(())?, "called");

    ud.call_method::<()>("add", 2)?;
    assert_eq!(ud.get::<u32>("n")?, 323);

    match ud.call_method::<()>("non_existent", ()) {
        Err(Error::RuntimeError(err)) => {
            assert!(err.contains("attempt to call a nil value (function 'non_existent')"))
        }
        r => panic!("expected RuntimeError, got {r:?}"),
    }

    assert!(ud.to_string()?.starts_with("MyUserData"));

    Ok(())
}

#[test]
fn test_userdata_method_errors() -> Result<()> {
    struct MyUserData(i64);

    impl UserData for MyUserData {
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_method("get_value", |_, data, ()| Ok(data.0));
        }
    }

    let lua = Lua::new();

    let ud = lua.create_userdata(MyUserData(123))?;
    let res = ud.call_function::<()>("get_value", "not a userdata");
    match res {
        Err(Error::CallbackError { cause, .. }) => match cause.as_ref() {
            Error::BadArgument {
                to,
                name,
                cause: cause2,
                ..
            } => {
                assert_eq!(to.as_deref(), Some("MyUserData.get_value"));
                assert_eq!(name.as_deref(), Some("self"));
                assert_eq!(
                    cause2.to_string(),
                    "error converting Lua string to userdata (expected userdata of type 'MyUserData')"
                );
            }
            err => panic!("expected BadArgument, got {err:?}"),
        },
        r => panic!("expected CallbackError, got {r:?}"),
    }

    Ok(())
}

#[test]
fn test_userdata_pointer() -> Result<()> {
    let lua = Lua::new();

    let ud1 = lua.create_any_userdata("hello")?;
    let ud2 = lua.create_any_userdata("hello")?;

    assert_eq!(ud1.to_pointer(), ud1.clone().to_pointer());
    // Different userdata objects with the same value should have different pointers
    assert_ne!(ud1.to_pointer(), ud2.to_pointer());

    Ok(())
}

#[cfg(feature = "macros")]
#[test]
fn test_userdata_derive() -> Result<()> {
    let lua = Lua::new();

    // Simple struct

    #[derive(Clone, Copy, mlua::FromLua)]
    struct MyUserData(i32);

    lua.register_userdata_type::<MyUserData>(|reg| {
        reg.add_function("val", |_, this: MyUserData| Ok(this.0));
    })?;

    lua.globals().set("ud", AnyUserData::wrap(MyUserData(123)))?;
    lua.load("assert(ud:val() == 123)").exec()?;

    // More complex struct where generics and where clause

    #[derive(Clone, Copy, mlua::FromLua)]
    struct MyUserData2<'a, T: ?Sized>(&'a T)
    where
        T: Copy;

    lua.register_userdata_type::<MyUserData2<'static, i32>>(|reg| {
        reg.add_function("val", |_, this: MyUserData2<'static, i32>| Ok(*this.0));
    })?;

    lua.globals().set("ud", AnyUserData::wrap(MyUserData2(&321)))?;
    lua.load("assert(ud:val() == 321)").exec()?;

    Ok(())
}

#[test]
fn test_nested_userdata_gc() -> Result<()> {
    let lua = Lua::new();

    let counter = Arc::new(());
    let arr = vec![lua.create_any_userdata(counter.clone())?];
    let arr_ud = lua.create_any_userdata(arr)?;

    assert_eq!(Arc::strong_count(&counter), 2);
    drop(arr_ud);
    // On first iteration Lua will destroy the array, on second - userdata
    lua.gc_collect()?;
    lua.gc_collect()?;
    assert_eq!(Arc::strong_count(&counter), 1);

    Ok(())
}

#[cfg(feature = "userdata-wrappers")]
#[test]
fn test_userdata_wrappers() -> Result<()> {
    #[derive(Debug)]
    struct MyUserData(i64);

    impl UserData for MyUserData {
        fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
            fields.add_field("static", "constant");
            fields.add_field_method_get("data", |_, this| Ok(this.0));
            fields.add_field_method_set("data", |_, this, val| {
                this.0 = val;
                Ok(())
            })
        }

        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_method("dbg", |_, this, ()| Ok(format!("{this:?}")));
        }
    }

    let lua = Lua::new();
    let globals = lua.globals();

    // Rc<T>
    #[cfg(not(feature = "send"))]
    {
        use std::rc::Rc;

        let ud = Rc::new(MyUserData(1));
        globals.set("ud", ud.clone())?;
        lua.load(
            r#"
            assert(ud.static == "constant")
            local ok, err = pcall(function() ud.data = 2 end)
            assert(
                tostring(err):find("error mutably borrowing userdata") ~= nil,
                "expected 'error mutably borrowing userdata', got '" .. tostring(err) .. "'"
            )
            assert(ud.data == 1)
            assert(ud:dbg(), "MyUserData(1)")
        "#,
        )
        .exec()
        .unwrap();

        // Test borrowing original userdata
        {
            let ud = globals.get::<AnyUserData>("ud")?;
            assert!(ud.is::<Rc<MyUserData>>());
            assert!(!ud.is::<MyUserData>());

            assert_eq!(ud.borrow::<MyUserData>()?.0, 1);
            assert!(matches!(
                ud.borrow_mut::<MyUserData>(),
                Err(Error::UserDataBorrowMutError)
            ));
            assert!(ud.borrow_mut::<Rc<MyUserData>>().is_ok());

            assert_eq!(ud.borrow_scoped::<MyUserData, _>(|x| x.0)?, 1);
            assert!(matches!(
                ud.borrow_mut_scoped::<MyUserData, _>(|_| ()),
                Err(Error::UserDataBorrowMutError)
            ));
        }

        // Collect userdata
        globals.set("ud", Nil)?;
        lua.gc_collect()?;
        assert_eq!(Rc::strong_count(&ud), 1);

        // We must be able to mutate userdata when having one reference only
        globals.set("ud", ud)?;
        lua.load(
            r#"
            ud.data = 2
            assert(ud.data == 2)
        "#,
        )
        .exec()
        .unwrap();
    }

    // Rc<RefCell<T>>
    #[cfg(not(feature = "send"))]
    {
        use std::cell::RefCell;
        use std::rc::Rc;

        let ud = Rc::new(RefCell::new(MyUserData(2)));
        globals.set("ud", ud.clone())?;
        lua.load(
            r#"
            assert(ud.static == "constant")
            assert(ud.data == 2)
            ud.data = 10
            assert(ud.data == 10)
            assert(ud:dbg() == "MyUserData(10)")
            "#,
        )
        .exec()
        .unwrap();

        // Test borrowing original userdata
        {
            let ud = globals.get::<AnyUserData>("ud")?;
            assert!(ud.is::<Rc<RefCell<MyUserData>>>());
            assert!(!ud.is::<MyUserData>());

            assert_eq!(ud.borrow::<MyUserData>()?.0, 10);
            assert_eq!(ud.borrow_mut::<MyUserData>()?.0, 10);
            ud.borrow_mut::<MyUserData>()?.0 = 20;
            assert_eq!(ud.borrow::<MyUserData>()?.0, 20);

            assert_eq!(ud.borrow_scoped::<MyUserData, _>(|x| x.0)?, 20);
            ud.borrow_mut_scoped::<MyUserData, _>(|x| x.0 = 30)?;
            assert_eq!(ud.borrow::<MyUserData>()?.0, 30);

            // Double (read) borrow is okay
            let _borrow = ud.borrow::<MyUserData>()?;
            assert_eq!(ud.borrow::<MyUserData>()?.0, 30);
            assert!(matches!(
                ud.borrow_mut::<MyUserData>(),
                Err(Error::UserDataBorrowMutError)
            ));
        }

        // Collect userdata
        globals.set("ud", Nil)?;
        lua.gc_collect()?;
        assert_eq!(Rc::strong_count(&ud), 1);

        // Check destroying wrapped UserDataRef without references in Lua
        let ud = lua.convert::<UserDataRef<MyUserData>>(ud)?;
        lua.gc_collect()?;
        assert_eq!(ud.0, 30);
        drop(ud);
    }

    // Arc<T>
    {
        let ud = Arc::new(MyUserData(3));
        globals.set("ud", ud.clone())?;
        lua.load(
            r#"
            assert(ud.static == "constant")
            local ok, err = pcall(function() ud.data = 4 end)
            assert(
                tostring(err):find("error mutably borrowing userdata") ~= nil,
                "expected 'error mutably borrowing userdata', got '" .. tostring(err) .. "'"
            )
            assert(ud.data == 3)
            assert(ud:dbg() == "MyUserData(3)")
            "#,
        )
        .exec()
        .unwrap();

        // Test borrowing original userdata
        {
            let ud = globals.get::<AnyUserData>("ud")?;
            assert!(ud.is::<Arc<MyUserData>>());
            assert!(!ud.is::<MyUserData>());

            assert_eq!(ud.borrow::<MyUserData>()?.0, 3);
            assert!(matches!(
                ud.borrow_mut::<MyUserData>(),
                Err(Error::UserDataBorrowMutError)
            ));
            assert!(ud.borrow_mut::<Arc<MyUserData>>().is_ok());

            assert_eq!(ud.borrow_scoped::<MyUserData, _>(|x| x.0)?, 3);
            assert!(matches!(
                ud.borrow_mut_scoped::<MyUserData, _>(|_| ()),
                Err(Error::UserDataBorrowMutError)
            ));
        }

        // Collect userdata
        globals.set("ud", Nil)?;
        lua.gc_collect()?;
        assert_eq!(Arc::strong_count(&ud), 1);

        // We must be able to mutate userdata when having one reference only
        globals.set("ud", ud)?;
        lua.load(
            r#"
            ud.data = 4
            assert(ud.data == 4)
            "#,
        )
        .exec()
        .unwrap();
    }

    // Arc<Mutex<T>>
    {
        use std::sync::Mutex;

        let ud = Arc::new(Mutex::new(MyUserData(5)));
        globals.set("ud", ud.clone())?;
        lua.load(
            r#"
            assert(ud.static == "constant")
            assert(ud.data == 5)
            ud.data = 6
            assert(ud.data == 6)
            assert(ud:dbg() == "MyUserData(6)")
            "#,
        )
        .exec()
        .unwrap();

        // Test borrowing original userdata
        {
            let ud = globals.get::<AnyUserData>("ud")?;
            assert!(ud.is::<Arc<Mutex<MyUserData>>>());
            assert!(!ud.is::<MyUserData>());

            #[rustfmt::skip]
            assert!(matches!(ud.borrow::<MyUserData>(), Err(Error::UserDataTypeMismatch)));
            #[rustfmt::skip]
            assert!(matches!(ud.borrow_mut::<MyUserData>(), Err(Error::UserDataTypeMismatch)));

            assert_eq!(ud.borrow_scoped::<MyUserData, _>(|x| x.0)?, 6);
            ud.borrow_mut_scoped::<MyUserData, _>(|x| x.0 = 8)?;
            assert_eq!(ud.borrow_scoped::<MyUserData, _>(|x| x.0)?, 8);
        }

        // Collect userdata
        globals.set("ud", Nil)?;
        lua.gc_collect()?;
        assert_eq!(Arc::strong_count(&ud), 1);
    }

    // Arc<RwLock<T>>
    {
        use std::sync::RwLock;

        let ud = Arc::new(RwLock::new(MyUserData(9)));
        globals.set("ud", ud.clone())?;
        lua.load(
            r#"
            assert(ud.static == "constant")
            assert(ud.data == 9)
            ud.data = 10
            assert(ud.data == 10)
            assert(ud:dbg() == "MyUserData(10)")
            "#,
        )
        .exec()
        .unwrap();

        // Test borrowing original userdata
        {
            let ud = globals.get::<AnyUserData>("ud")?;
            assert!(ud.is::<Arc<RwLock<MyUserData>>>());
            assert!(!ud.is::<MyUserData>());

            #[rustfmt::skip]
            assert!(matches!(ud.borrow::<MyUserData>(), Err(Error::UserDataTypeMismatch)));
            #[rustfmt::skip]
            assert!(matches!(ud.borrow_mut::<MyUserData>(), Err(Error::UserDataTypeMismatch)));

            assert_eq!(ud.borrow_scoped::<MyUserData, _>(|x| x.0)?, 10);
            ud.borrow_mut_scoped::<MyUserData, _>(|x| x.0 = 12)?;
            assert_eq!(ud.borrow_scoped::<MyUserData, _>(|x| x.0)?, 12);
        }

        // Collect userdata
        globals.set("ud", Nil)?;
        lua.gc_collect()?;
        assert_eq!(Arc::strong_count(&ud), 1);
    }

    // Arc<parking_lot::Mutex<T>>
    {
        use parking_lot::Mutex;

        let ud = Arc::new(Mutex::new(MyUserData(13)));
        globals.set("ud", ud.clone())?;
        lua.load(
            r#"
            assert(ud.static == "constant")
            assert(ud.data == 13)
            ud.data = 14
            assert(ud.data == 14)
            assert(ud:dbg() == "MyUserData(14)")
            "#,
        )
        .exec()
        .unwrap();

        // Test borrowing original userdata
        {
            let ud = globals.get::<AnyUserData>("ud")?;
            assert!(ud.is::<Arc<Mutex<MyUserData>>>());
            assert!(!ud.is::<MyUserData>());

            assert_eq!(ud.borrow::<MyUserData>()?.0, 14);
            assert_eq!(ud.borrow_mut::<MyUserData>()?.0, 14);
            ud.borrow_mut::<MyUserData>()?.0 = 15;
            assert_eq!(ud.borrow::<MyUserData>()?.0, 15);

            assert_eq!(ud.borrow_scoped::<MyUserData, _>(|x| x.0)?, 15);
            ud.borrow_mut_scoped::<MyUserData, _>(|x| x.0 = 16)?;
            assert_eq!(ud.borrow::<MyUserData>()?.0, 16);

            // Double borrow is not allowed
            let _borrow = ud.borrow::<MyUserData>()?;
            assert!(matches!(
                ud.borrow::<MyUserData>(),
                Err(Error::UserDataBorrowError)
            ));
        }

        // Collect userdata
        globals.set("ud", Nil)?;
        lua.gc_collect()?;
        assert_eq!(Arc::strong_count(&ud), 1);

        // Check destroying wrapped UserDataRef without references in Lua
        let ud = lua.convert::<UserDataRef<MyUserData>>(ud)?;
        lua.gc_collect()?;
        assert_eq!(ud.0, 16);
        drop(ud);
    }

    // Arc<parking_lot::RwLock<T>>
    {
        use parking_lot::RwLock;

        let ud = Arc::new(RwLock::new(MyUserData(17)));
        globals.set("ud", ud.clone())?;
        lua.load(
            r#"
            assert(ud.static == "constant")
            assert(ud.data == 17)
            ud.data = 18
            assert(ud.data == 18)
            assert(ud:dbg() == "MyUserData(18)")
            "#,
        )
        .exec()
        .unwrap();

        // Test borrowing original userdata
        {
            let ud = globals.get::<AnyUserData>("ud")?;
            assert!(ud.is::<Arc<RwLock<MyUserData>>>());
            assert!(!ud.is::<MyUserData>());

            assert_eq!(ud.borrow::<MyUserData>()?.0, 18);
            assert_eq!(ud.borrow_mut::<MyUserData>()?.0, 18);
            ud.borrow_mut::<MyUserData>()?.0 = 19;
            assert_eq!(ud.borrow::<MyUserData>()?.0, 19);

            assert_eq!(ud.borrow_scoped::<MyUserData, _>(|x| x.0)?, 19);
            ud.borrow_mut_scoped::<MyUserData, _>(|x| x.0 = 20)?;
            assert_eq!(ud.borrow::<MyUserData>()?.0, 20);

            // Multiple read borrows are allowed with parking_lot::RwLock
            let _borrow1 = ud.borrow::<MyUserData>()?;
            let _borrow2 = ud.borrow::<MyUserData>()?;
            assert!(matches!(
                ud.borrow_mut::<MyUserData>(),
                Err(Error::UserDataBorrowMutError)
            ));
        }

        // Collect userdata
        globals.set("ud", Nil)?;
        lua.gc_collect()?;
        assert_eq!(Arc::strong_count(&ud), 1);

        // Check destroying wrapped UserDataRef without references in Lua
        let ud = lua.convert::<UserDataRef<MyUserData>>(ud)?;
        lua.gc_collect()?;
        assert_eq!(ud.0, 20);
        drop(ud);
    }

    Ok(())
}
