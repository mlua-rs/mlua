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
    assert!(num1.equals(num2)?);
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
fn test_value_to_string() -> Result<()> {
    let lua = Lua::new();

    assert_eq!(Value::Nil.to_string()?, "nil");
    assert_eq!(Value::Boolean(true).to_string()?, "true");
    assert_eq!(Value::NULL.to_string()?, "null");
    assert_eq!(
        Value::LightUserData(LightUserData(0x1 as *const c_void as *mut _)).to_string()?,
        "lightuserdata: 0x1"
    );
    assert_eq!(Value::Integer(1).to_string()?, "1");
    assert_eq!(Value::Number(34.59).to_string()?, "34.59");
    #[cfg(feature = "luau")]
    assert_eq!(
        Value::Vector(10.0, 11.1, 12.2).to_string()?,
        "vector(10, 11.1, 12.2)"
    );
    assert_eq!(
        Value::String(lua.create_string("hello")?).to_string()?,
        "hello"
    );

    let table: Value = lua.load("{}").eval()?;
    assert!(table.to_string()?.starts_with("table:"));
    let table: Value = lua
        .load("setmetatable({}, {__tostring = function() return 'test table' end})")
        .eval()?;
    assert_eq!(table.to_string()?, "test table");

    let func: Value = lua.load("function() end").eval()?;
    assert!(func.to_string()?.starts_with("function:"));

    let thread: Value = lua.load("coroutine.create(function() end)").eval()?;
    assert!(thread.to_string()?.starts_with("thread:"));

    lua.register_userdata_type::<StdString>(|reg| {
        reg.add_meta_method("__tostring", |_, this, ()| Ok(this.clone()));
    })?;
    let ud: Value = Value::UserData(lua.create_any_userdata(String::from("string userdata"))?);
    assert_eq!(ud.to_string()?, "string userdata");

    struct MyUserData;
    impl UserData for MyUserData {}
    let ud: Value = Value::UserData(lua.create_userdata(MyUserData)?);
    assert!(ud.to_string()?.starts_with("MyUserData:"));

    let err = Value::Error(Error::RuntimeError("test error".to_string()));
    assert_eq!(err.to_string()?, "runtime error: test error");

    Ok(())
}
