extern crate rlua;

use rlua::{Lua, Nil, Result, Table, Value};

#[test]
fn test_set_get() {
    let lua = Lua::new();
    let globals = lua.globals();
    globals.set("foo", "bar").unwrap();
    globals.set("baz", "baf").unwrap();
    assert_eq!(globals.get::<_, String>("foo").unwrap(), "bar");
    assert_eq!(globals.get::<_, String>("baz").unwrap(), "baf");
}

#[test]
fn test_table() {
    let lua = Lua::new();
    let globals = lua.globals();

    globals.set("table", lua.create_table().unwrap()).unwrap();
    let table1: Table = globals.get("table").unwrap();
    let table2: Table = globals.get("table").unwrap();

    table1.set("foo", "bar").unwrap();
    table2.set("baz", "baf").unwrap();

    assert_eq!(table2.get::<_, String>("foo").unwrap(), "bar");
    assert_eq!(table1.get::<_, String>("baz").unwrap(), "baf");

    lua.exec::<_, ()>(
        r#"
                table1 = {1, 2, 3, 4, 5}
                table2 = {}
                table3 = {1, 2, nil, 4, 5}
            "#,
        None,
    )
    .unwrap();

    let table1 = globals.get::<_, Table>("table1").unwrap();
    let table2 = globals.get::<_, Table>("table2").unwrap();
    let table3 = globals.get::<_, Table>("table3").unwrap();

    assert_eq!(table1.len().unwrap(), 5);
    assert_eq!(
        table1
            .clone()
            .pairs()
            .collect::<Result<Vec<(i64, i64)>>>()
            .unwrap(),
        vec![(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]
    );
    assert_eq!(
        table1
            .clone()
            .sequence_values()
            .collect::<Result<Vec<i64>>>()
            .unwrap(),
        vec![1, 2, 3, 4, 5]
    );

    assert_eq!(table2.len().unwrap(), 0);
    assert_eq!(
        table2
            .clone()
            .pairs()
            .collect::<Result<Vec<(i64, i64)>>>()
            .unwrap(),
        vec![]
    );
    assert_eq!(
        table2
            .sequence_values()
            .collect::<Result<Vec<i64>>>()
            .unwrap(),
        vec![]
    );

    // sequence_values should only iterate until the first border
    assert_eq!(
        table3
            .sequence_values()
            .collect::<Result<Vec<i64>>>()
            .unwrap(),
        vec![1, 2]
    );

    globals
        .set(
            "table4",
            lua.create_sequence_from(vec![1, 2, 3, 4, 5]).unwrap(),
        )
        .unwrap();
    let table4 = globals.get::<_, Table>("table4").unwrap();
    assert_eq!(
        table4.pairs().collect::<Result<Vec<(i64, i64)>>>().unwrap(),
        vec![(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]
    );
}

#[test]
fn test_table_scope() {
    let lua = Lua::new();
    let globals = lua.globals();
    lua.exec::<_, ()>(
        r#"
            touter = {
                tin = {1, 2, 3}
            }
        "#,
        None,
    )
    .unwrap();

    // Make sure that table gets do not borrow the table, but instead just borrow lua.
    let tin;
    {
        let touter = globals.get::<_, Table>("touter").unwrap();
        tin = touter.get::<_, Table>("tin").unwrap();
    }

    assert_eq!(tin.get::<_, i64>(1).unwrap(), 1);
    assert_eq!(tin.get::<_, i64>(2).unwrap(), 2);
    assert_eq!(tin.get::<_, i64>(3).unwrap(), 3);
}

#[test]
fn test_metatable() {
    let lua = Lua::new();

    let table = lua.create_table().unwrap();
    let metatable = lua.create_table().unwrap();
    metatable
        .set(
            "__index",
            lua.create_function(|_, ()| Ok("index_value")).unwrap(),
        )
        .unwrap();
    table.set_metatable(Some(metatable));
    assert_eq!(table.get::<_, String>("any_key").unwrap(), "index_value");
    match table.raw_get::<_, Value>("any_key").unwrap() {
        Nil => {}
        _ => panic!(),
    }
    table.set_metatable(None);
    match table.get::<_, Value>("any_key").unwrap() {
        Nil => {}
        _ => panic!(),
    };
}

#[test]
fn test_table_error() {
    let lua = Lua::new();
    let globals = lua.globals();
    lua.exec::<_, ()>(
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
        None,
    )
    .unwrap();

    let bad_table: Table = globals.get("table").unwrap();
    assert!(bad_table.set(1, 1).is_err());
    assert!(bad_table.get::<_, i32>(1).is_err());
    assert!(bad_table.len().is_err());
    assert!(bad_table.raw_set(1, 1).is_ok());
    assert!(bad_table.raw_get::<_, i32>(1).is_ok());
    assert_eq!(bad_table.raw_len(), 1);
}
