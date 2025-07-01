use mlua::{Error, ExternalError, Integer, IntoLuaMulti, Lua, MultiValue, Result, String, Value, Variadic};

#[test]
fn test_result_conversions() -> Result<()> {
    let lua = Lua::new();
    let globals = lua.globals();

    let ok = lua.create_function(|_, ()| Ok(Ok::<(), Error>(())))?;
    let err = lua.create_function(|_, ()| Ok(Err::<(), _>("failure1".into_lua_err())))?;
    let ok2 = lua.create_function(|_, ()| Ok(Ok::<_, Error>("!".to_owned())))?;
    let err2 = lua.create_function(|_, ()| Ok(Err::<String, _>("failure2".into_lua_err())))?;

    globals.set("ok", ok)?;
    globals.set("ok2", ok2)?;
    globals.set("err", err)?;
    globals.set("err2", err2)?;

    lua.load(
        r#"
        local r, e = ok()
        assert(r == nil and e == nil)

        local r, e = err()
        assert(r == nil)
        assert(tostring(e):find("failure1") ~= nil)

        local r, e = ok2()
        assert(r == "!")
        assert(e == nil)

        local r, e = err2()
        assert(r == nil)
        assert(tostring(e):find("failure2") ~= nil)
    "#,
    )
    .exec()?;

    // Try to convert Result into MultiValue
    let ok1 = Ok::<(), Error>(());
    let multi_ok1 = ok1.into_lua_multi(&lua)?;
    assert_eq!(multi_ok1.len(), 0);
    let err1 = Err::<(), _>("failure1");
    let multi_err1 = err1.into_lua_multi(&lua)?;
    assert_eq!(multi_err1.len(), 2);
    assert_eq!(multi_err1[0], Value::Nil);
    assert_eq!(multi_err1[1].as_string().unwrap(), "failure1");

    let ok2 = Ok::<_, Error>("!");
    let multi_ok2 = ok2.into_lua_multi(&lua)?;
    assert_eq!(multi_ok2.len(), 1);
    assert_eq!(multi_ok2[0].as_string().unwrap(), "!");
    let err2 = Err::<String, _>("failure2".into_lua_err());
    let multi_err2 = err2.into_lua_multi(&lua)?;
    assert_eq!(multi_err2.len(), 2);
    assert_eq!(multi_err2[0], Value::Nil);
    assert!(matches!(multi_err2[1], Value::Error(_)));
    assert_eq!(multi_err2[1].to_string()?, "failure2");

    Ok(())
}

#[test]
fn test_multivalue() {
    let mut multi = MultiValue::with_capacity(3);
    multi.push_back(Value::Integer(1));
    multi.push_back(Value::Integer(2));
    multi.push_front(Value::Integer(3));
    assert_eq!(multi.iter().filter_map(|v| v.as_integer()).sum::<Integer>(), 6);

    let vec = multi.into_vec();
    assert_eq!(&vec, &[Value::Integer(3), Value::Integer(1), Value::Integer(2)]);
    let _multi2 = MultiValue::from_vec(vec);
}

#[test]
fn test_variadic() {
    let mut var = Variadic::with_capacity(3);
    var.extend_from_slice(&[1, 2, 3]);
    assert_eq!(var.iter().sum::<u32>(), 6);

    let vec = Vec::<u32>::from(var);
    assert_eq!(&vec, &[1, 2, 3]);
    let var2 = Variadic::from(vec);
    assert_eq!(var2.as_slice(), &[1, 2, 3]);
}
