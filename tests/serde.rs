#![cfg(feature = "serde")]

use std::collections::HashMap;
use std::error::Error as StdError;

use bstr::BString;
use mlua::{
    AnyUserData, DeserializeOptions, Error, ExternalResult, IntoLua, Lua, LuaSerdeExt, Result as LuaResult,
    SerializeOptions, UserData, Value,
};
use serde::{Deserialize, Serialize};

#[test]
fn test_serialize() -> Result<(), Box<dyn StdError>> {
    #[derive(Serialize)]
    struct MyUserData(i64, String);

    impl UserData for MyUserData {}

    let lua = Lua::new();
    let globals = lua.globals();

    let ud = lua.create_ser_userdata(MyUserData(123, "test userdata".into()))?;
    globals.set("ud", ud)?;
    globals.set("null", lua.null())?;

    let empty_array = lua.create_table()?;
    empty_array.set_metatable(Some(lua.array_metatable()))?;
    globals.set("empty_array", empty_array)?;

    let val = lua
        .load(
            r#"
        {
            _bool = true,
            _integer = 123,
            _number = 321.99,
            _string = "test string serialization",
            _table_arr = {nil, "value 1", nil, "value 2", {}},
            _table_map = {["table"] = "map", ["null"] = null},
            _bytes = "\240\040\140\040",
            _userdata = ud,
            _null = null,
            _empty_map = {},
            _empty_array = empty_array,
        }
    "#,
        )
        .eval::<Value>()?;

    let json = serde_json::json!({
        "_bool": true,
        "_integer": 123,
        "_number": 321.99,
        "_string": "test string serialization",
        "_table_arr": [null, "value 1", null, "value 2", {}],
        "_table_map": {"table": "map", "null": null},
        "_bytes": [240, 40, 140, 40],
        "_userdata": [123, "test userdata"],
        "_null": null,
        "_empty_map": {},
        "_empty_array": [],
    });

    assert_eq!(serde_json::to_value(&val)?, json);

    // Test to-from loop
    let val = lua.to_value(&json)?;
    let expected_json = lua.from_value::<serde_json::Value>(val)?;
    assert_eq!(expected_json, json);

    Ok(())
}

#[test]
fn test_serialize_any_userdata() {
    let lua = Lua::new();

    let json_val = serde_json::json!({
        "a": 1,
        "b": "test",
    });
    let json_ud = lua.create_ser_any_userdata(json_val).unwrap();
    let json_str = serde_json::to_string_pretty(&json_ud).unwrap();
    assert_eq!(json_str, "{\n  \"a\": 1,\n  \"b\": \"test\"\n}");
}

#[test]
fn test_serialize_wrapped_any_userdata() {
    let lua = Lua::new();

    let json_val = serde_json::json!({
        "a": 1,
        "b": "test",
    });
    let ud = AnyUserData::wrap_ser(json_val);
    let json_ud = ud.into_lua(&lua).unwrap();
    let json_str = serde_json::to_string(&json_ud).unwrap();
    assert_eq!(json_str, "{\"a\":1,\"b\":\"test\"}");
}

#[test]
fn test_serialize_failure() -> Result<(), Box<dyn StdError>> {
    #[derive(Serialize)]
    struct MyUserData(i64);

    impl UserData for MyUserData {}

    let lua = Lua::new();

    let ud = Value::UserData(lua.create_userdata(MyUserData(123))?);
    match serde_json::to_value(&ud) {
        Ok(v) => panic!("expected serialization error, got {}", v),
        Err(serde_json::Error { .. }) => {}
    }

    let func = lua.create_function(|_, _: ()| Ok(()))?;
    match serde_json::to_value(&Value::Function(func.clone())) {
        Ok(v) => panic!("expected serialization error, got {}", v),
        Err(serde_json::Error { .. }) => {}
    }

    let thr = lua.create_thread(func)?;
    match serde_json::to_value(&Value::Thread(thr)) {
        Ok(v) => panic!("expected serialization error, got {}", v),
        Err(serde_json::Error { .. }) => {}
    }

    Ok(())
}

#[cfg(all(feature = "luau", not(feature = "luau-vector4")))]
#[test]
fn test_serialize_vector() -> Result<(), Box<dyn StdError>> {
    let lua = Lua::new();

    let val = lua.load("{_vector = vector.create(1, 2, 3)}").eval::<Value>()?;
    let json = serde_json::json!({
        "_vector": [1.0, 2.0, 3.0],
    });
    assert_eq!(serde_json::to_value(&val)?, json);

    let expected_json = lua.from_value::<serde_json::Value>(val)?;
    assert_eq!(expected_json, json);

    Ok(())
}

#[cfg(feature = "luau-vector4")]
#[test]
fn test_serialize_vector() -> Result<(), Box<dyn StdError>> {
    let lua = Lua::new();

    let val = lua
        .load("{_vector = vector.create(1, 2, 3, 4)}")
        .eval::<Value>()?;
    let json = serde_json::json!({
        "_vector": [1.0, 2.0, 3.0, 4.0],
    });
    assert_eq!(serde_json::to_value(&val)?, json);

    let expected_json = lua.from_value::<serde_json::Value>(val)?;
    assert_eq!(expected_json, json);

    Ok(())
}

#[test]
fn test_serialize_sorted() -> LuaResult<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    globals.set("null", lua.null())?;

    let empty_array = lua.create_table()?;
    empty_array.set_metatable(Some(lua.array_metatable()))?;
    globals.set("empty_array", empty_array)?;

    let value = lua
        .load(
            r#"
        {
            _bool = true,
            _integer = 123,
            _number = 321.99,
            _string = "test string serialization",
            _table_arr = {nil, "value 1", nil, "value 2", {}},
            _table_map = {["table"] = "map", ["null"] = null},
            _bytes = "\240\040\140\040",
            _null = null,
            _empty_map = {},
            _empty_array = empty_array,
        }
    "#,
        )
        .eval::<Value>()?;

    let json = serde_json::to_string(&value.to_serializable().sort_keys(true)).unwrap();
    assert_eq!(
        json,
        r#"{"_bool":true,"_bytes":[240,40,140,40],"_empty_array":[],"_empty_map":{},"_integer":123,"_null":null,"_number":321.99,"_string":"test string serialization","_table_arr":[null,"value 1",null,"value 2",{}],"_table_map":{"null":null,"table":"map"}}"#
    );

    Ok(())
}

#[test]
fn test_serialize_globals() -> LuaResult<()> {
    let lua = Lua::new();

    let globals = Value::Table(lua.globals());

    // By default it should not work
    if let Ok(v) = serde_json::to_value(&globals) {
        panic!("expected serialization error, got {v:?}");
    }

    // It should work with `deny_recursive_tables` and `deny_unsupported_types` disabled
    if let Err(err) = serde_json::to_value(
        globals
            .to_serializable()
            .deny_recursive_tables(false)
            .deny_unsupported_types(false),
    ) {
        panic!("expected no errors, got {err:?}");
    }

    Ok(())
}

#[test]
fn test_serialize_same_table_twice() -> LuaResult<()> {
    let lua = Lua::new();

    let value = lua
        .load(
            r#"
        local foo = {}
        return {
            a = foo,
            b = foo,
        }
    "#,
        )
        .eval::<Value>()?;
    let json = serde_json::to_string(&value.to_serializable().sort_keys(true)).unwrap();
    assert_eq!(json, r#"{"a":{},"b":{}}"#);

    Ok(())
}

#[test]
fn test_serialize_empty_table() -> LuaResult<()> {
    let lua = Lua::new();

    let table = Value::Table(lua.create_table()?);
    let json = serde_json::to_string(&table.to_serializable()).unwrap();
    assert_eq!(json, "{}");

    // Set the option to encode empty tables as array
    let json = serde_json::to_string(&table.to_serializable().encode_empty_tables_as_array(true)).unwrap();
    assert_eq!(json, "[]");

    // Check hashmap table with this option
    table.as_table().unwrap().set("hello", "world")?;
    let json = serde_json::to_string(&table.to_serializable().encode_empty_tables_as_array(true)).unwrap();
    assert_eq!(json, r#"{"hello":"world"}"#);

    Ok(())
}

#[test]
fn test_to_value_struct() -> LuaResult<()> {
    let lua = Lua::new();
    let globals = lua.globals();
    globals.set("null", lua.null())?;

    #[derive(Serialize)]
    struct Test {
        name: String,
        key: i64,
        data: Option<bool>,
    }

    let test = Test {
        name: "alex".to_string(),
        key: -16,
        data: None,
    };

    globals.set("value", lua.to_value(&test)?)?;
    lua.load(
        r#"
            assert(value["name"] == "alex")
            assert(value["key"] == -16)
            assert(value["data"] == null)
        "#,
    )
    .exec()
}

#[test]
fn test_to_value_enum() -> LuaResult<()> {
    let lua = Lua::new();
    let globals = lua.globals();

    #[derive(Serialize)]
    enum E {
        Unit,
        Integer(u32),
        Tuple(u32, u32),
        Struct { a: u32 },
    }

    let u = E::Unit;
    globals.set("value", lua.to_value(&u)?)?;
    lua.load(r#"assert(value == "Unit")"#).exec()?;

    let n = E::Integer(1);
    globals.set("value", lua.to_value(&n)?)?;
    lua.load(r#"assert(value["Integer"] == 1)"#).exec()?;

    let t = E::Tuple(1, 2);
    globals.set("value", lua.to_value(&t)?)?;
    lua.load(
        r#"
            assert(value["Tuple"][1] == 1)
            assert(value["Tuple"][2] == 2)
        "#,
    )
    .exec()?;

    let s = E::Struct { a: 1 };
    globals.set("value", lua.to_value(&s)?)?;
    lua.load(r#"assert(value["Struct"]["a"] == 1)"#).exec()?;
    Ok(())
}

#[test]
fn test_to_value_with_options() -> Result<(), Box<dyn StdError>> {
    let lua = Lua::new();
    let globals = lua.globals();
    globals.set("null", lua.null())?;

    // set_array_metatable
    let data = lua.to_value_with(
        &Vec::<i32>::new(),
        SerializeOptions::new().set_array_metatable(false),
    )?;
    globals.set("data", data)?;
    lua.load(
        r#"
        assert(type(data) == "table" and #data == 0)
        assert(getmetatable(data) == nil)
    "#,
    )
    .exec()?;

    #[derive(Serialize)]
    struct UnitStruct;

    #[derive(Serialize)]
    struct MyData {
        map: HashMap<&'static str, Option<i32>>,
        unit: (),
        unitstruct: UnitStruct,
    }

    // serialize_none_to_null
    let mut map = HashMap::new();
    map.insert("key", None);
    let mydata = MyData {
        map,
        unit: (),
        unitstruct: UnitStruct,
    };
    let data2 = lua.to_value_with(&mydata, SerializeOptions::new().serialize_none_to_null(false))?;
    globals.set("data2", data2)?;
    lua.load(
        r#"
        assert(data2.map.key == nil)
        assert(data2.unit == null)
        assert(data2.unitstruct == null)
    "#,
    )
    .exec()?;

    // serialize_unit_to_null
    let data3 = lua.to_value_with(&mydata, SerializeOptions::new().serialize_unit_to_null(false))?;
    globals.set("data3", data3)?;
    lua.load(
        r#"
        assert(data3.map.key == null)
        assert(data3.unit == nil)
        assert(data3.unitstruct == nil)
    "#,
    )
    .exec()?;

    Ok(())
}

#[test]
fn test_from_value_nested_tables() -> Result<(), Box<dyn StdError>> {
    let lua = Lua::new();

    let value = lua
        .load(
            r#"
            local table_a = {a = "a"}
            local table_b = {"b"}
            return {
                a = table_a,
                b = {table_b, table_b},
                ab = {a = table_a, b = table_b}
            }
        "#,
        )
        .eval::<Value>()?;
    let got = lua.from_value::<serde_json::Value>(value)?;
    assert_eq!(
        got,
        serde_json::json!({
            "a": {"a": "a"},
            "b": [["b"], ["b"]],
            "ab": {"a": {"a": "a"}, "b": ["b"]},
        })
    );

    Ok(())
}

#[test]
fn test_from_value_struct() -> Result<(), Box<dyn StdError>> {
    let lua = Lua::new();

    #[derive(Deserialize, PartialEq, Debug)]
    struct Test {
        int: u32,
        seq: Vec<String>,
        map: HashMap<i32, i32>,
        empty: Vec<()>,
        tuple: (u8, u8, u8),
        bytes: BString,
    }

    let value = lua
        .load(
            r#"
            {
                int = 1,
                seq = {"a", "b"},
                map = {2, [4] = 1},
                empty = {},
                tuple = {10, 20, 30},
                bytes = "\240\040\140\040",
            }
        "#,
        )
        .eval::<Value>()?;
    let got = lua.from_value(value)?;
    assert_eq!(
        Test {
            int: 1,
            seq: vec!["a".into(), "b".into()],
            map: vec![(1, 2), (4, 1)].into_iter().collect(),
            empty: vec![],
            tuple: (10, 20, 30),
            bytes: BString::from([240, 40, 140, 40]),
        },
        got
    );

    Ok(())
}

#[test]
fn test_from_value_newtype_struct() -> Result<(), Box<dyn StdError>> {
    let lua = Lua::new();

    #[derive(Deserialize, PartialEq, Debug)]
    struct Test(f64);

    let got = lua.from_value(Value::Number(123.456))?;
    assert_eq!(Test(123.456), got);

    Ok(())
}

#[test]
fn test_from_value_enum() -> Result<(), Box<dyn StdError>> {
    let lua = Lua::new();
    lua.globals().set("null", lua.null())?;

    #[derive(Deserialize, PartialEq, Debug)]
    struct UnitStruct;

    #[derive(Deserialize, PartialEq, Debug)]
    enum E<T = ()> {
        Unit,
        Integer(u32),
        Tuple(u32, u32),
        Struct { a: u32 },
        Wrap(T),
    }

    let value = lua.load(r#""Unit""#).eval()?;
    let got: E = lua.from_value(value)?;
    assert_eq!(E::Unit, got);

    let value = lua.load(r#"{Integer = 1}"#).eval()?;
    let got: E = lua.from_value(value)?;
    assert_eq!(E::Integer(1), got);

    let value = lua.load(r#"{Tuple = {1, 2}}"#).eval()?;
    let got: E = lua.from_value(value)?;
    assert_eq!(E::Tuple(1, 2), got);

    let value = lua.load(r#"{Struct = {a = 3}}"#).eval()?;
    let got: E = lua.from_value(value)?;
    assert_eq!(E::Struct { a: 3 }, got);

    let value = lua.load(r#"{Wrap = null}"#).eval()?;
    let got = lua.from_value(value)?;
    assert_eq!(E::Wrap(UnitStruct), got);

    let value = lua.load(r#"{Wrap = null}"#).eval()?;
    let got = lua.from_value(value)?;
    assert_eq!(E::Wrap(()), got);

    Ok(())
}

#[test]
fn test_from_value_enum_untagged() -> Result<(), Box<dyn StdError>> {
    let lua = Lua::new();
    lua.globals().set("null", lua.null())?;

    #[derive(Deserialize, PartialEq, Debug)]
    #[serde(untagged)]
    enum Eut {
        Unit,
        Integer(u64),
        Tuple(u32, u32),
        Struct { a: u32 },
    }

    let value = lua.load(r#"null"#).eval()?;
    let got = lua.from_value(value)?;
    assert_eq!(Eut::Unit, got);

    let value = lua.load(r#"1"#).eval()?;
    let got = lua.from_value(value)?;
    assert_eq!(Eut::Integer(1), got);

    let value = lua.load(r#"{3, 1}"#).eval()?;
    let got = lua.from_value(value)?;
    assert_eq!(Eut::Tuple(3, 1), got);

    let value = lua.load(r#"{a = 10}"#).eval()?;
    let got = lua.from_value(value)?;
    assert_eq!(Eut::Struct { a: 10 }, got);

    let value = lua.load(r#"{b = 12}"#).eval()?;
    match lua.from_value::<Eut>(value) {
        Ok(v) => panic!("expected Error::DeserializeError, got {:?}", v),
        Err(Error::DeserializeError(_)) => {}
        Err(e) => panic!("expected Error::DeserializeError, got {}", e),
    }

    Ok(())
}

#[test]
fn test_from_value_with_options() -> Result<(), Box<dyn StdError>> {
    let lua = Lua::new();

    // Deny unsupported types by default
    let value = Value::Function(lua.create_function(|_, ()| Ok(()))?);
    match lua.from_value::<Option<String>>(value) {
        Ok(v) => panic!("expected deserialization error, got {:?}", v),
        Err(Error::DeserializeError(err)) => {
            assert!(err.contains("unsupported value type"))
        }
        Err(err) => panic!("expected `DeserializeError` error, got {:?}", err),
    };

    // Allow unsupported types
    let value = Value::Function(lua.create_function(|_, ()| Ok(()))?);
    let options = DeserializeOptions::new().deny_unsupported_types(false);
    assert_eq!(lua.from_value_with::<()>(value, options)?, ());

    // Allow unsupported types (in a table seq)
    let value = lua.load(r#"{"a", "b", function() end, "c"}"#).eval()?;
    let options = DeserializeOptions::new().deny_unsupported_types(false);
    assert_eq!(
        lua.from_value_with::<Vec<String>>(value, options)?,
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );

    // Deny recursive tables by default
    let value = lua.load(r#"local t = {}; t.t = t; return t"#).eval()?;
    match lua.from_value::<HashMap<String, Option<String>>>(value) {
        Ok(v) => panic!("expected deserialization error, got {:?}", v),
        Err(Error::DeserializeError(err)) => {
            assert!(err.contains("recursive table detected"))
        }
        Err(err) => panic!("expected `DeserializeError` error, got {:?}", err),
    };

    // Check recursion when using `Serialize` impl
    let t = lua.create_table()?;
    t.set("t", &t)?;
    assert!(serde_json::to_string(&t).is_err());

    // Serialize Lua globals table
    #[derive(Debug, Deserialize)]
    struct Globals {
        hello: String,
    }
    let options = DeserializeOptions::new()
        .deny_unsupported_types(false)
        .deny_recursive_tables(false);
    lua.load(r#"hello = "world""#).exec()?;
    let globals: Globals = lua.from_value_with(Value::Table(lua.globals()), options)?;
    assert_eq!(globals.hello, "world");

    Ok(())
}

#[test]
fn test_from_value_userdata() -> Result<(), Box<dyn StdError>> {
    let lua = Lua::new();

    // Tuple struct
    #[derive(Serialize, Deserialize)]
    struct MyUserData(i64, String);

    impl UserData for MyUserData {}

    let ud = lua.create_ser_userdata(MyUserData(123, "test userdata".into()))?;

    match lua.from_value::<MyUserData>(Value::UserData(ud)) {
        Ok(_) => {}
        Err(err) => panic!("expected no errors, got {err:?}"),
    };

    // Newtype struct
    #[derive(Serialize, Deserialize)]
    struct NewtypeUserdata(String);

    impl UserData for NewtypeUserdata {}

    let ud = lua.create_ser_userdata(NewtypeUserdata("newtype userdata".into()))?;

    match lua.from_value::<NewtypeUserdata>(Value::UserData(ud)) {
        Ok(_) => {}
        Err(err) => panic!("expected no errors, got {err:?}"),
    };

    // Option
    #[derive(Serialize, Deserialize)]
    struct UnitUserdata;

    impl UserData for UnitUserdata {}

    let ud = lua.create_ser_userdata(UnitUserdata)?;

    match lua.from_value::<Option<()>>(Value::UserData(ud)) {
        Ok(Some(_)) => {}
        Ok(_) => panic!("expected `Some`, got `None`"),
        Err(err) => panic!("expected no errors, got {err:?}"),
    };

    // Destructed userdata with skip option
    let ud = lua.create_ser_userdata(NewtypeUserdata("newtype userdata".into()))?;
    let _ = ud.take::<NewtypeUserdata>()?;

    match lua.from_value_with::<()>(
        Value::UserData(ud),
        DeserializeOptions::new().deny_unsupported_types(false),
    ) {
        Ok(_) => {}
        Err(err) => panic!("expected no errors, got {err:?}"),
    };

    Ok(())
}

#[test]
fn test_from_value_empty_table() -> Result<(), Box<dyn StdError>> {
    let lua = Lua::new();

    // By default we encode empty tables as objects
    let t = lua.create_table()?;
    let got = lua.from_value::<serde_json::Value>(Value::Table(t.clone()))?;
    assert_eq!(got, serde_json::json!({}));

    // Set the option to encode empty tables as array
    let got = lua
        .from_value_with::<serde_json::Value>(
            Value::Table(t.clone()),
            DeserializeOptions::new().encode_empty_tables_as_array(true),
        )
        .unwrap();
    assert_eq!(got, serde_json::json!([]));

    // Check hashmap table with this option
    t.raw_set("hello", "world")?;
    let got = lua
        .from_value_with::<serde_json::Value>(
            Value::Table(t),
            DeserializeOptions::new().encode_empty_tables_as_array(true),
        )
        .unwrap();
    assert_eq!(got, serde_json::json!({"hello": "world"}));

    Ok(())
}

#[test]
fn test_from_value_sorted() -> Result<(), Box<dyn StdError>> {
    let lua = Lua::new();

    let to_json = lua.create_function(|lua, value| {
        let json_value: serde_json::Value =
            lua.from_value_with(value, DeserializeOptions::new().sort_keys(true))?;
        serde_json::to_string(&json_value).into_lua_err()
    })?;
    lua.globals().set("to_json", to_json)?;

    lua.load(
        r#"
        local json = to_json({c = 3, b = 2, hello = "world", x = {1}, ["0a"] = {z = "z", d = "d"}})
        assert(json == '{"0a":{"d":"d","z":"z"},"b":2,"c":3,"hello":"world","x":[1]}', "invalid json")
    "#,
    )
    .exec()
    .unwrap();

    Ok(())
}

#[test]
fn test_arbitrary_precision() {
    let lua = Lua::new();

    let opts = SerializeOptions::new().detect_serde_json_arbitrary_precision(true);

    // Number
    let num = serde_json::Value::Number(serde_json::Number::from_f64(1.244e2).unwrap());
    let num = lua.to_value_with(&num, opts).unwrap();
    assert_eq!(num, Value::Number(1.244e2));

    // Integer
    let num = serde_json::Value::Number(serde_json::Number::from_f64(123.0).unwrap());
    let num = lua.to_value_with(&num, opts).unwrap();
    assert_eq!(num, Value::Integer(123));

    // Max u64
    let num = serde_json::Value::Number(serde_json::Number::from(i64::MAX));
    let num = lua.to_value_with(&num, opts).unwrap();
    assert_eq!(num, Value::Number(i64::MAX as f64));

    // Check that the option is disabled by default
    let num = serde_json::Value::Number(serde_json::Number::from_f64(1.244e2).unwrap());
    let num = lua.to_value(&num).unwrap();
    assert_eq!(num.type_name(), "table");
    assert_eq!(
        format!("{:#?}", num),
        "{\n  [\"$serde_json::private::Number\"] = \"124.4\",\n}"
    );
}

#[cfg(feature = "luau")]
#[test]
fn test_buffer_serialize() -> LuaResult<()> {
    let lua = Lua::new();

    let buf = lua.create_buffer(&[1, 2, 3, 4])?;
    let val = serde_value::to_value(&buf).unwrap();
    assert_eq!(val, serde_value::Value::Bytes(vec![1, 2, 3, 4]));

    // Try empty buffer
    let buf = lua.create_buffer(&[])?;
    let val = serde_value::to_value(&buf).unwrap();
    assert_eq!(val, serde_value::Value::Bytes(vec![]));

    Ok(())
}

#[cfg(feature = "luau")]
#[test]
fn test_buffer_from_value() -> LuaResult<()> {
    let lua = Lua::new();

    let buf = lua.create_buffer(&[1, 2, 3, 4])?;
    let val = lua.from_value::<serde_value::Value>(Value::Buffer(buf)).unwrap();
    assert_eq!(val, serde_value::Value::Bytes(vec![1, 2, 3, 4]));

    Ok(())
}
