use std::collections::HashMap;
use std::os::raw::c_void;
use std::ptr;
use std::string::String as StdString;

use mlua::{Error, LightUserData, Lua, MultiValue, Result, UserData, UserDataMethods, Value};

#[test]
fn test_value_eq() -> Result<()> {
    let lua = Lua::new();
    let globals = lua.globals();

    lua.load(
        r#"
        table1 = {1}
        table2 = {1}
        string1 = "hello"
        string2 = "hello"
        num1 = 1
        num2 = 1.0
        num3 = "1"
        func1 = function() end
        func2 = func1
        func3 = function() end
        thread1 = coroutine.create(function() end)
        thread2 = thread1

        setmetatable(table1, {
            __eq = function(a, b) return a[1] == b[1] end
        })
    "#,
    )
    .exec()?;
    globals.set("null", Value::NULL)?;

    let table1: Value = globals.get("table1")?;
    let table2: Value = globals.get("table2")?;
    let string1: Value = globals.get("string1")?;
    let string2: Value = globals.get("string2")?;
    let num1: Value = globals.get("num1")?;
    let num2: Value = globals.get("num2")?;
    let num3: Value = globals.get("num3")?;
    let func1: Value = globals.get("func1")?;
    let func2: Value = globals.get("func2")?;
    let func3: Value = globals.get("func3")?;
    let thread1: Value = globals.get("thread1")?;
    let thread2: Value = globals.get("thread2")?;
    let null: Value = globals.get("null")?;

    assert!(table1 != table2);
    assert!(table1.equals(&table2)?);
    assert!(string1 == string2);
    assert!(string1.equals(&string2)?);
    assert!(num1 == num2);
    assert!(num1.equals(&num2)?);
    assert!(num1 != num3);
    assert!(func1 == func2);
    assert!(func1 != func3);
    assert!(!func1.equals(&func3)?);
    assert!(thread1 == thread2);
    assert!(thread1.equals(&thread2)?);
    assert!(null == Value::NULL);

    assert!(!table1.to_pointer().is_null());
    assert!(!ptr::eq(table1.to_pointer(), table2.to_pointer()));
    assert!(ptr::eq(string1.to_pointer(), string2.to_pointer()));
    assert!(ptr::eq(func1.to_pointer(), func2.to_pointer()));
    assert!(num1.to_pointer().is_null());

    Ok(())
}

#[test]
fn test_multi_value() {
    let mut multi_value = MultiValue::new();
    assert_eq!(multi_value.len(), 0);
    assert_eq!(multi_value.get(0), None);

    multi_value.push_front(Value::Number(2.));
    multi_value.push_front(Value::Number(1.));
    assert_eq!(multi_value.get(0), Some(&Value::Number(1.)));
    assert_eq!(multi_value.get(1), Some(&Value::Number(2.)));

    assert_eq!(multi_value.pop_front(), Some(Value::Number(1.)));
    assert_eq!(multi_value[0], Value::Number(2.));

    multi_value.clear();
    assert!(multi_value.is_empty());
}

#[test]
fn test_value_to_pointer() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        table = {}
        string = "hello"
        num = 1
        func = function() end
        thread = coroutine.create(function() end)
    "#,
    )
    .exec()?;
    globals.set("null", Value::NULL)?;

    let table: Value = globals.get("table")?;
    let string: Value = globals.get("string")?;
    let num: Value = globals.get("num")?;
    let func: Value = globals.get("func")?;
    let thread: Value = globals.get("thread")?;
    let null: Value = globals.get("null")?;
    let ud: Value = Value::UserData(lua.create_any_userdata(())?);

    assert!(!table.to_pointer().is_null());
    assert!(!string.to_pointer().is_null());
    assert!(num.to_pointer().is_null());
    assert!(!func.to_pointer().is_null());
    assert!(!thread.to_pointer().is_null());
    assert!(null.to_pointer().is_null());
    assert!(!ud.to_pointer().is_null());

    Ok(())
}

#[test]
fn test_value_to_string() -> Result<()> {
    let lua = Lua::new();

    assert_eq!(Value::Nil.to_string()?, "nil");
    assert_eq!(Value::Nil.type_name(), "nil");
    assert_eq!(Value::Boolean(true).to_string()?, "true");
    assert_eq!(Value::Boolean(true).type_name(), "boolean");
    assert_eq!(Value::NULL.to_string()?, "null");
    assert_eq!(Value::NULL.type_name(), "lightuserdata");
    assert_eq!(
        Value::LightUserData(LightUserData(0x1 as *const c_void as *mut _)).to_string()?,
        "lightuserdata: 0x1"
    );
    assert_eq!(Value::Integer(1).to_string()?, "1");
    assert_eq!(Value::Integer(1).type_name(), "integer");
    assert_eq!(Value::Number(34.59).to_string()?, "34.59");
    assert_eq!(Value::Number(34.59).type_name(), "number");
    #[cfg(all(feature = "luau", not(feature = "luau-vector4")))]
    assert_eq!(
        Value::Vector(mlua::Vector::new(10.0, 11.1, 12.2)).to_string()?,
        "vector(10, 11.1, 12.2)"
    );
    #[cfg(all(feature = "luau", not(feature = "luau-vector4")))]
    assert_eq!(
        Value::Vector(mlua::Vector::new(10.0, 11.1, 12.2)).type_name(),
        "vector"
    );
    #[cfg(feature = "luau-vector4")]
    assert_eq!(
        Value::Vector(mlua::Vector::new(10.0, 11.1, 12.2, 13.3)).to_string()?,
        "vector(10, 11.1, 12.2, 13.3)"
    );

    let s = Value::String(lua.create_string("hello")?);
    assert_eq!(s.to_string()?, "hello");
    assert_eq!(s.type_name(), "string");

    let table: Value = lua.load("{}").eval()?;
    assert!(table.to_string()?.starts_with("table:"));
    let table: Value = lua
        .load("setmetatable({}, {__tostring = function() return 'test table' end})")
        .eval()?;
    assert_eq!(table.to_string()?, "test table");
    assert_eq!(table.type_name(), "table");

    let func: Value = lua.load("function() end").eval()?;
    assert!(func.to_string()?.starts_with("function:"));
    assert_eq!(func.type_name(), "function");

    let thread: Value = lua.load("coroutine.create(function() end)").eval()?;
    assert!(thread.to_string()?.starts_with("thread:"));
    assert_eq!(thread.type_name(), "thread");

    lua.register_userdata_type::<StdString>(|reg| {
        reg.add_meta_method("__tostring", |_, this, ()| Ok(this.clone()));
    })?;
    let ud: Value = Value::UserData(lua.create_any_userdata(String::from("string userdata"))?);
    assert_eq!(ud.to_string()?, "string userdata");
    assert_eq!(ud.type_name(), "userdata");

    struct MyUserData;
    impl UserData for MyUserData {}
    let ud: Value = Value::UserData(lua.create_userdata(MyUserData)?);
    assert!(ud.to_string()?.starts_with("MyUserData:"));

    let err = Value::Error(Box::new(Error::runtime("test error")));
    assert_eq!(err.to_string()?, "runtime error: test error");
    assert_eq!(err.type_name(), "error");

    #[cfg(feature = "luau")]
    {
        let buf = Value::Buffer(lua.create_buffer(b"hello")?);
        assert!(buf.to_string()?.starts_with("buffer:"));
        assert_eq!(buf.type_name(), "buffer");

        // Set `__tostring` metamethod for buffer
        let mt = lua.load("{__tostring = buffer.tostring}").eval()?;
        lua.set_type_metatable::<mlua::Buffer>(mt);
        assert_eq!(buf.to_string()?, "hello");
    }

    Ok(())
}

#[test]
fn test_debug_format() -> Result<()> {
    let lua = Lua::new();

    lua.register_userdata_type::<HashMap<i32, StdString>>(|_| {})?;
    let ud = lua
        .create_any_userdata::<HashMap<i32, StdString>>(HashMap::new())
        .map(Value::UserData)?;
    assert!(format!("{ud:#?}").starts_with("HashMap<i32, String>:"));

    Ok(())
}

#[test]
fn test_value_conversions() -> Result<()> {
    let lua = Lua::new();

    assert!(Value::Nil.is_nil());
    assert!(!Value::NULL.is_nil());
    assert!(Value::NULL.is_null());
    assert!(Value::NULL.is_light_userdata());
    assert!(Value::NULL.as_light_userdata() == Some(LightUserData(ptr::null_mut())));
    assert!(Value::Boolean(true).is_boolean());
    assert_eq!(Value::Boolean(false).as_boolean(), Some(false));
    assert!(Value::Integer(1).is_integer());
    assert_eq!(Value::Integer(1).as_integer(), Some(1));
    assert_eq!(Value::Integer(1).as_i32(), Some(1i32));
    assert_eq!(Value::Integer(1).as_u32(), Some(1u32));
    assert_eq!(Value::Integer(1).as_i64(), Some(1i64));
    assert_eq!(Value::Integer(1).as_u64(), Some(1u64));
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    {
        assert_eq!(Value::Integer(mlua::Integer::MAX).as_i32(), None);
        assert_eq!(Value::Integer(mlua::Integer::MAX).as_u32(), None);
    }
    assert_eq!(Value::Integer(1).as_isize(), Some(1isize));
    assert_eq!(Value::Integer(1).as_usize(), Some(1usize));
    assert!(Value::Number(1.23).is_number());
    assert_eq!(Value::Number(1.23).as_number(), Some(1.23));
    assert_eq!(Value::Number(1.23).as_f32(), Some(1.23f32));
    assert_eq!(Value::Number(1.23).as_f64(), Some(1.23f64));
    assert!(Value::String(lua.create_string("hello")?).is_string());
    assert_eq!(
        Value::String(lua.create_string("hello")?).as_string().unwrap(),
        "hello"
    );
    assert_eq!(Value::String(lua.create_string("hello")?).to_string()?, "hello");
    assert!(Value::Table(lua.create_table()?).is_table());
    assert!(Value::Table(lua.create_table()?).as_table().is_some());
    assert!(Value::Function(lua.create_function(|_, ()| Ok(())).unwrap()).is_function());
    assert!(Value::Function(lua.create_function(|_, ()| Ok(())).unwrap())
        .as_function()
        .is_some());
    assert!(Value::Thread(lua.create_thread(lua.load("function() end").eval()?)?).is_thread());
    assert!(
        Value::Thread(lua.create_thread(lua.load("function() end").eval()?)?)
            .as_thread()
            .is_some()
    );
    assert!(Value::UserData(lua.create_any_userdata("hello")?).is_userdata());
    assert_eq!(
        Value::UserData(lua.create_any_userdata("hello")?)
            .as_userdata()
            .and_then(|ud| ud.borrow::<&str>().ok())
            .as_deref(),
        Some(&"hello")
    );

    assert!(Value::Error(Box::new(Error::runtime("some error"))).is_error());
    assert_eq!(
        (Value::Error(Box::new(Error::runtime("some error"))).as_error())
            .unwrap()
            .to_string(),
        "runtime error: some error"
    );

    Ok(())
}

#[test]
fn test_value_exhaustive_match() {
    match Value::Nil {
        Value::Nil => {}
        Value::Boolean(_) => {}
        Value::LightUserData(_) => {}
        Value::Integer(_) => {}
        Value::Number(_) => {}
        #[cfg(feature = "luau")]
        Value::Vector(_) => {}
        Value::String(_) => {}
        Value::Table(_) => {}
        Value::Function(_) => {}
        Value::Thread(_) => {}
        Value::UserData(_) => {}
        #[cfg(feature = "luau")]
        Value::Buffer(_) => {}
        Value::Error(_) => {}
        Value::Other(_) => {}
    }
}
