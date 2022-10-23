use mlua::{Error, Lua, Nil, Result, Table, TableExt, Value};

#[test]
fn test_set_get() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    globals.set("foo", "bar")?;
    globals.set("baz", "baf")?;
    assert_eq!(globals.get::<_, String>("foo")?, "bar");
    assert_eq!(globals.get::<_, String>("baz")?, "baf");

    Ok(())
}

#[test]
fn test_table() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();

    globals.set("table", lua.create_table()?)?;
    let table1: Table = globals.get("table")?;
    let table2: Table = globals.get("table")?;

    table1.set("foo", "bar")?;
    table2.set("baz", "baf")?;

    assert_eq!(table2.get::<_, String>("foo")?, "bar");
    assert_eq!(table1.get::<_, String>("baz")?, "baf");

    lua.load(
        r#"
        table1 = {1, 2, 3, 4, 5}
        table2 = {}
        table3 = {1, 2, nil, 4, 5}
    "#,
    )
    .exec()?;

    let table1 = globals.get::<_, Table>("table1")?;
    let table2 = globals.get::<_, Table>("table2")?;
    let table3 = globals.get::<_, Table>("table3")?;

    assert_eq!(table1.len()?, 5);
    assert_eq!(
        table1
            .clone()
            .pairs()
            .collect::<Result<Vec<(i64, i64)>>>()?,
        vec![(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]
    );
    assert_eq!(
        table1
            .clone()
            .sequence_values()
            .collect::<Result<Vec<i64>>>()?,
        vec![1, 2, 3, 4, 5]
    );

    assert_eq!(table2.len()?, 0);
    assert_eq!(
        table2
            .clone()
            .pairs()
            .collect::<Result<Vec<(i64, i64)>>>()?,
        vec![]
    );
    assert_eq!(
        table2.sequence_values().collect::<Result<Vec<i64>>>()?,
        vec![]
    );

    // sequence_values should only iterate until the first border
    assert_eq!(
        table3.sequence_values().collect::<Result<Vec<i64>>>()?,
        vec![1, 2]
    );

    globals.set("table4", lua.create_sequence_from(vec![1, 2, 3, 4, 5])?)?;
    let table4 = globals.get::<_, Table>("table4")?;
    assert_eq!(
        table4
            .clone()
            .pairs()
            .collect::<Result<Vec<(i64, i64)>>>()?,
        vec![(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]
    );

    table4.raw_insert(4, 35)?;
    table4.raw_insert(7, 7)?;
    assert_eq!(
        table4
            .clone()
            .pairs()
            .collect::<Result<Vec<(i64, i64)>>>()?,
        vec![(1, 1), (2, 2), (3, 3), (4, 35), (5, 4), (6, 5), (7, 7)]
    );

    table4.raw_remove(1)?;
    assert_eq!(
        table4
            .clone()
            .pairs()
            .collect::<Result<Vec<(i64, i64)>>>()?,
        vec![(1, 2), (2, 3), (3, 35), (4, 4), (5, 5), (6, 7)]
    );

    Ok(())
}

#[test]
fn test_table_push_pop() -> Result<()> {
    let lua = Lua::new();

    // Test raw access
    let table1 = lua.create_sequence_from(vec![123])?;
    table1.raw_push(321)?;
    assert_eq!(
        table1
            .clone()
            .raw_sequence_values::<i64>()
            .collect::<Result<Vec<_>>>()?,
        vec![123, 321]
    );
    assert_eq!(table1.raw_pop::<i64>()?, 321);
    assert_eq!(table1.raw_pop::<i64>()?, 123);
    assert_eq!(table1.raw_pop::<Value>()?, Value::Nil); // An extra pop should do nothing
    assert_eq!(table1.raw_len(), 0);

    // Test access through metamethods
    let table2 = lua
        .load(
            r#"
        local proxy_table = {234}
        table2 = setmetatable({}, {
            __len = function() return #proxy_table end,
            __index = proxy_table,
            __newindex = proxy_table,
        })
        return table2
    "#,
        )
        .eval::<Table>()?;
    table2.push(345)?;
    assert_eq!(table2.len()?, 2);
    assert_eq!(table2.pop::<i64>()?, 345);
    assert_eq!(table2.pop::<i64>()?, 234);
    assert_eq!(table2.pop::<Value>()?, Value::Nil);
    assert_eq!(table2.len()?, 0);

    Ok(())
}

#[test]
fn test_table_sequence_from() -> Result<()> {
    let lua = Lua::new();

    let get_table = lua.create_function(|_, t: Table| Ok(t))?;

    assert_eq!(
        get_table
            .call::<_, Table>(vec![1, 2, 3])?
            .sequence_values()
            .collect::<Result<Vec<i64>>>()?,
        vec![1, 2, 3]
    );

    assert_eq!(
        get_table
            .call::<_, Table>([1, 2, 3].as_ref())?
            .sequence_values()
            .collect::<Result<Vec<i64>>>()?,
        vec![1, 2, 3]
    );

    assert_eq!(
        get_table
            .call::<_, Table>([1, 2, 3])?
            .sequence_values()
            .collect::<Result<Vec<i64>>>()?,
        vec![1, 2, 3]
    );

    Ok(())
}

#[test]
fn test_table_scope() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        touter = {
            tin = {1, 2, 3}
        }
    "#,
    )
    .exec()?;

    // Make sure that table gets do not borrow the table, but instead just borrow lua.
    let tin;
    {
        let touter = globals.get::<_, Table>("touter")?;
        tin = touter.get::<_, Table>("tin")?;
    }

    assert_eq!(tin.get::<_, i64>(1)?, 1);
    assert_eq!(tin.get::<_, i64>(2)?, 2);
    assert_eq!(tin.get::<_, i64>(3)?, 3);

    Ok(())
}

#[test]
fn test_metatable() -> Result<()> {
    let lua = Lua::new();

    let table = lua.create_table()?;
    let metatable = lua.create_table()?;
    metatable.set("__index", lua.create_function(|_, ()| Ok("index_value"))?)?;
    table.set_metatable(Some(metatable));
    assert_eq!(table.get::<_, String>("any_key")?, "index_value");
    match table.raw_get::<_, Value>("any_key")? {
        Nil => {}
        _ => panic!(),
    }
    table.set_metatable(None);
    match table.get::<_, Value>("any_key")? {
        Nil => {}
        _ => panic!(),
    };

    Ok(())
}

#[test]
fn test_table_eq() -> Result<()> {
    let lua = Lua::new();
    let globals = lua.globals();

    lua.load(
        r#"
        table1 = {1}
        table2 = {1}
        table3 = table1
        table4 = {1}

        setmetatable(table4, {
            __eq = function(a, b) return a[1] == b[1] end
        })
    "#,
    )
    .exec()?;

    let table1 = globals.get::<_, Table>("table1")?;
    let table2 = globals.get::<_, Table>("table2")?;
    let table3 = globals.get::<_, Table>("table3")?;
    let table4 = globals.get::<_, Table>("table4")?;

    assert!(table1 != table2);
    assert!(!table1.equals(&table2)?);
    assert!(table1 == table3);
    assert!(table1.equals(&table3)?);
    assert!(table1 != table4);
    assert!(table1.equals(&table4)?);

    Ok(())
}

#[test]
fn test_table_error() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        table = {}
        setmetatable(table, {
            __index = function()
                error("lua error")
            end,
            __newindex = function()
                error("lua error")
            end,
            __len = function()
                error("lua error")
            end
        })
    "#,
    )
    .exec()?;

    let bad_table: Table = globals.get("table")?;
    assert!(bad_table.set(1, 1).is_err());
    assert!(bad_table.get::<_, i32>(1).is_err());
    assert!(bad_table.len().is_err());
    assert!(bad_table.raw_set(1, 1).is_ok());
    assert!(bad_table.raw_get::<_, i32>(1).is_ok());
    assert_eq!(bad_table.raw_len(), 1);

    Ok(())
}

#[test]
fn test_table_call() -> Result<()> {
    let lua = Lua::new();

    lua.load(
        r#"
        table = {a = 1, b = 2}
        setmetatable(table, {
            __call = function(t, key)
                return "call_"..t[key]
            end
        })

        function table.func(key)
            return "func_"..key
        end

        function table:method(key)
            return "method_"..self[key]
        end
    "#,
    )
    .exec()?;

    let table: Table = lua.globals().get("table")?;

    assert_eq!(table.call::<_, String>("b")?, "call_2");
    assert_eq!(table.call_function::<_, _, String>("func", "a")?, "func_a");
    assert_eq!(
        table.call_method::<_, _, String>("method", "a")?,
        "method_1"
    );

    // Test calling non-callable table
    let table2 = lua.create_table()?;
    assert!(matches!(
        table2.call::<_, ()>(()),
        Err(Error::RuntimeError(_))
    ));

    Ok(())
}
