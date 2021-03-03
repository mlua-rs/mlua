#![cfg(feature = "serialize")]
#![cfg_attr(
    all(feature = "luajit", target_os = "macos", target_arch = "x86_64"),
    feature(link_args)
)]

#[cfg_attr(
    all(feature = "luajit", target_os = "macos", target_arch = "x86_64"),
    link_args = "-pagezero_size 10000 -image_base 100000000",
    allow(unused_attributes)
)]
extern "system" {}

use mlua::{Error, Lua, LuaSerdeExt, Result as LuaResult, UserData, Value};
use serde::{Deserialize, Serialize};

#[test]
fn test_serialize() -> Result<(), Box<dyn std::error::Error>> {
    #[derive(Serialize)]
    struct MyUserData(i64, String);

    impl UserData for MyUserData {}

    let lua = Lua::new();
    let globals = lua.globals();

    let ud = lua.create_ser_userdata(MyUserData(123, "test userdata".into()))?;
    globals.set("ud", ud)?;
    globals.set("null", lua.null()?)?;

    let empty_array = lua.create_table()?;
    empty_array.set_metatable(Some(lua.array_metatable()?));
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
fn test_serialize_in_scope() -> LuaResult<()> {
    #[derive(Serialize, Clone)]
    struct MyUserData(i64, String);

    impl UserData for MyUserData {}

    let lua = Lua::new();
    lua.scope(|scope| {
        let ud = scope.create_ser_userdata(MyUserData(-5, "test userdata".into()))?;
        assert_eq!(
            serde_json::to_value(&ud).unwrap(),
            serde_json::json!((-5, "test userdata"))
        );
        Ok(())
    })?;

    lua.scope(|scope| {
        let ud = scope.create_ser_userdata(MyUserData(-5, "test userdata".into()))?;
        lua.globals().set("ud", ud)
    })?;
    let val = lua.load("ud").eval::<Value>()?;
    match serde_json::to_value(&val) {
        Ok(v) => panic!("expected destructed error, got {}", v),
        Err(e) if e.to_string().contains("destructed") => {}
        Err(e) => panic!("expected destructed error, got {}", e),
    }

    Ok(())
}

#[test]
fn test_serialize_failure() -> Result<(), Box<dyn std::error::Error>> {
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

#[test]
fn test_to_value_struct() -> LuaResult<()> {
    let lua = Lua::new();
    let globals = lua.globals();
    globals.set("null", lua.null()?)?;

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
    globals.set("null", lua.null()?)?;

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
    .exec()?;

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
fn test_from_value_struct() -> Result<(), Box<dyn std::error::Error>> {
    let lua = Lua::new();

    #[derive(Deserialize, PartialEq, Debug)]
    struct Test {
        int: u32,
        seq: Vec<String>,
        map: std::collections::HashMap<i32, i32>,
        empty: Vec<()>,
        tuple: (u8, u8, u8),
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
        },
        got
    );

    Ok(())
}

#[test]
fn test_from_value_enum() -> Result<(), Box<dyn std::error::Error>> {
    let lua = Lua::new();

    #[derive(Deserialize, PartialEq, Debug)]
    enum E {
        Unit,
        Integer(u32),
        Tuple(u32, u32),
        Struct { a: u32 },
    }

    let value = lua.load(r#""Unit""#).eval()?;
    let got = lua.from_value(value)?;
    assert_eq!(E::Unit, got);

    let value = lua.load(r#"{Integer = 1}"#).eval()?;
    let got = lua.from_value(value)?;
    assert_eq!(E::Integer(1), got);

    let value = lua.load(r#"{Tuple = {1, 2}}"#).eval()?;
    let got = lua.from_value(value)?;
    assert_eq!(E::Tuple(1, 2), got);

    let value = lua.load(r#"{Struct = {a = 3}}"#).eval()?;
    let got = lua.from_value(value)?;
    assert_eq!(E::Struct { a: 3 }, got);

    Ok(())
}

#[test]
fn test_from_value_enum_untagged() -> Result<(), Box<dyn std::error::Error>> {
    let lua = Lua::new();
    lua.globals().set("null", lua.null()?)?;

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
