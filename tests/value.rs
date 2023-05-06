use std::ptr;

use mlua::{Lua, MultiValue, Result, Value};

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
