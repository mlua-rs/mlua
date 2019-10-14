use mlua::{Lua, Nil, Result, Table, Value};

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
        table4.pairs().collect::<Result<Vec<(i64, i64)>>>()?,
        vec![(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]
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
