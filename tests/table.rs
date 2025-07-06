use mlua::{Error, Lua, ObjectLike, Result, Table, Value};

#[test]
fn test_globals_set_get() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    globals.set("foo", "bar")?;
    globals.set("baz", "baf")?;
    assert_eq!(globals.get::<String>("foo")?, "bar");
    assert_eq!(globals.get::<String>("baz")?, "baf");

    lua.load(r#"assert(foo == "bar")"#).exec().unwrap();

    Ok(())
}

#[test]
fn test_table() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();

    lua.load(
        r#"
        table1 = {1, 2, 3, 4, 5}
        table2 = {}
        table3 = {1, 2, nil, 4, 5}
    "#,
    )
    .exec()?;

    let table1 = globals.get::<Table>("table1")?;
    assert_eq!(table1.len()?, 5);
    assert!(!table1.is_empty());
    assert_eq!(
        table1.pairs().collect::<Result<Vec<(i64, i64)>>>()?,
        vec![(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]
    );
    assert_eq!(
        table1.sequence_values().collect::<Result<Vec<i64>>>()?,
        vec![1, 2, 3, 4, 5]
    );
    assert_eq!(table1, [1, 2, 3, 4, 5]);
    assert_eq!(table1, [1, 2, 3, 4, 5].as_slice());

    let table2 = globals.get::<Table>("table2")?;
    assert_eq!(table2.len()?, 0);
    assert!(table2.is_empty());
    assert_eq!(table2.pairs().collect::<Result<Vec<(i64, i64)>>>()?, vec![]);
    assert_eq!(table2, [0; 0]);

    let table3 = globals.get::<Table>("table3")?;
    // sequence_values should only iterate until the first border
    assert_eq!(table3, [1, 2]);
    assert_eq!(
        table3.sequence_values().collect::<Result<Vec<i64>>>()?,
        vec![1, 2]
    );

    Ok(())
}

#[test]
fn test_table_push_pop() -> Result<()> {
    let lua = Lua::new();

    // Test raw access
    let table1 = lua.create_sequence_from([123])?;
    table1.raw_push(321)?;
    assert_eq!(table1, [123, 321]);
    assert_eq!(table1.raw_pop::<i64>()?, 321);
    assert_eq!(table1.raw_pop::<i64>()?, 123);
    assert_eq!(table1.raw_pop::<Value>()?, Value::Nil); // An extra pop should do nothing
    assert_eq!(table1.raw_len(), 0);
    assert_eq!(table1, [0; 0]);

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
    assert_eq!(
        table2.sequence_values::<i64>().collect::<Result<Vec<_>>>()?,
        vec![]
    );
    assert_eq!(table2.pop::<i64>()?, 345);
    assert_eq!(table2.pop::<i64>()?, 234);
    assert_eq!(table2.pop::<Value>()?, Value::Nil);
    assert_eq!(table2.len()?, 0);

    Ok(())
}

#[test]
fn test_table_insert_remove() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();

    globals.set("table4", [1, 2, 3, 4, 5])?;
    let table4 = globals.get::<Table>("table4")?;
    assert_eq!(
        table4.pairs().collect::<Result<Vec<(i64, i64)>>>()?,
        vec![(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]
    );
    table4.raw_insert(4, 35)?;
    table4.raw_insert(7, 7)?;
    assert_eq!(
        table4.pairs().collect::<Result<Vec<(i64, i64)>>>()?,
        vec![(1, 1), (2, 2), (3, 3), (4, 35), (5, 4), (6, 5), (7, 7)]
    );
    table4.raw_remove(1)?;
    assert_eq!(
        table4.pairs().collect::<Result<Vec<(i64, i64)>>>()?,
        vec![(1, 2), (2, 3), (3, 35), (4, 4), (5, 5), (6, 7)]
    );

    // Wrong index, tables are 1-indexed
    assert!(table4.raw_insert(0, "123").is_err());

    Ok(())
}

#[test]
fn test_table_clear() -> Result<()> {
    let lua = Lua::new();

    let t = lua.create_table()?;

    // Check readonly error
    #[cfg(feature = "luau")]
    {
        t.set_readonly(true);
        assert!(matches!(
            t.clear(),
            Err(Error::RuntimeError(err)) if err.contains("attempt to modify a readonly table")
        ));
        t.set_readonly(false);
    }

    // Set array and hash parts
    t.push("abc")?;
    t.push("bcd")?;
    t.set("a", "1")?;
    t.set("b", "2")?;
    t.clear()?;
    assert_eq!(t.len()?, 0);
    assert_eq!(t.pairs::<Value, Value>().count(), 0);

    // Test table with metamethods
    let t2 = lua
        .load(
            r#"
        setmetatable({1, 2, 3, a = "1"}, {
            __index = function() error("index error") end,
            __newindex = function() error("newindex error") end,
            __len = function() error("len error") end,
            __pairs = function() error("pairs error") end,
        })
    "#,
        )
        .eval::<Table>()?;
    assert_eq!(t2.raw_len(), 3);
    assert!(!t2.is_empty());
    t2.clear()?;
    assert_eq!(t2.raw_len(), 0);
    assert!(t2.is_empty());
    assert_eq!(t2.raw_get::<Value>("a")?, Value::Nil);
    assert_ne!(t2.metatable(), None);

    Ok(())
}

#[test]
fn test_table_sequence_from() -> Result<()> {
    let lua = Lua::new();

    let get_table = lua.create_function(|_, t: Table| Ok(t))?;

    assert_eq!(get_table.call::<Table>(vec![1, 2, 3])?, [1, 2, 3]);
    assert_eq!(get_table.call::<Table>([4, 5, 6])?, [4, 5, 6]);
    assert_eq!(get_table.call::<Table>([7, 8, 9].as_slice())?, [7, 8, 9]);

    Ok(())
}

#[test]
fn test_table_pairs() -> Result<()> {
    let lua = Lua::new();

    let table = lua
        .load(
            r#"
    {
        foo = "bar",
        baz = "baf",
        [123] = 456,
        [789] = 101112,
        5,
    }
    "#,
        )
        .eval::<Table>()?;

    for (i, kv) in table.pairs::<String, Value>().enumerate() {
        let (k, _v) = kv.unwrap();
        match i {
            // Try to add a new key
            0 => table.set("new_key", "new_value")?,
            // Try to delete the 2nd key
            1 => {
                table.set(k, Value::Nil)?;
                lua.gc_collect()?;
            }
            _ => {}
        }
    }

    Ok(())
}

#[test]
fn test_table_for_each() -> Result<()> {
    let lua = Lua::new();

    let table = lua
        .load(
            r#"
    {
        foo = "bar",
        baz = "baf",
        [123] = 456,
        [789] = 101112,
        5,
    }
    "#,
        )
        .eval::<Table>()?;

    let mut i = 0;
    table.for_each::<String, Value>(|k, _| {
        if i == 0 {
            // Delete first key
            table.set(k, Value::Nil)?;
            lua.gc_collect()?;
        }
        Ok(i += 1)
    })?;
    assert_eq!(i, 5);

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
        let touter = globals.get::<Table>("touter")?;
        tin = touter.get::<Table>("tin")?;
    }

    assert_eq!(tin.get::<i64>(1)?, 1);
    assert_eq!(tin.get::<i64>(2)?, 2);
    assert_eq!(tin.get::<i64>(3)?, 3);

    Ok(())
}

#[test]
fn test_metatable() -> Result<()> {
    let lua = Lua::new();

    let table = lua.create_table()?;
    let metatable = lua.create_table()?;
    metatable.set("__index", lua.create_function(|_, ()| Ok("index_value"))?)?;
    table.set_metatable(Some(metatable))?;
    assert_eq!(table.get::<String>("any_key")?, "index_value");
    assert_eq!(table.raw_get::<Value>("any_key")?, Value::Nil);
    table.set_metatable(None)?;
    assert_eq!(table.get::<Value>("any_key")?, Value::Nil);

    Ok(())
}

#[test]
fn test_table_equals() -> Result<()> {
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

    let table1 = globals.get::<Table>("table1")?;
    let table2 = globals.get::<Table>("table2")?;
    let table3 = globals.get::<Table>("table3")?;
    let table4 = globals.get::<Table>("table4")?;

    assert!(table1 != table2);
    assert!(!table1.equals(&table2)?);
    assert!(table1 == table3);
    assert!(table1.equals(&table3)?);
    assert!(table1 != table4);
    assert!(table1.equals(&table4)?);

    Ok(())
}

#[test]
fn test_table_pointer() -> Result<()> {
    let lua = Lua::new();

    let table1 = lua.create_table()?;
    let table2 = lua.create_table()?;

    // Clone should not create a new table
    assert_eq!(table1.to_pointer(), table1.clone().to_pointer());
    assert_ne!(table1.to_pointer(), table2.to_pointer());

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
    assert!(bad_table.get::<i32>(1).is_err());
    assert!(bad_table.len().is_err());
    assert!(bad_table.raw_set(1, 1).is_ok());
    assert!(bad_table.raw_get::<i32>(1).is_ok());
    assert_eq!(bad_table.raw_len(), 1);

    Ok(())
}

#[test]
fn test_table_fmt() -> Result<()> {
    let lua = Lua::new();

    let table = lua
        .load(
            r#"
        local t = {1, 2, 3, a = 5, b = { 6 }}
        t["special-<chars>"] = 10
        t[9.2] = 9.2
        t[1.99] = 1.99
        t[true] = true
        t[false] = false
        return t
    "#,
        )
        .eval::<Table>()?;
    assert!(format!("{table:?}").starts_with("Table(Ref("));

    // Pretty print
    assert_eq!(
        format!("{table:#?}"),
        "{\n  [false] = false,\n  [true] = true,\n  [1] = 1,\n  [1.99] = 1.99,\n  [2] = 2,\n  [3] = 3,\n  [9.2] = 9.2,\n  a = 5,\n  b = {\n    6,\n  },\n  [\"special-<chars>\"] = 10,\n}"
    );

    let table2 = lua.create_table_from([("1", "first"), ("2", "second")])?;
    assert_eq!(
        format!("{table2:#?}"),
        "{\n  [\"1\"] = \"first\",\n  [\"2\"] = \"second\",\n}"
    );

    Ok(())
}

#[test]
fn test_table_object_like() -> Result<()> {
    let lua = Lua::new();

    lua.load(
        r#"
        table = {a = 1, b = 2}
        setmetatable(table, {
            __call = function(t, key)
                return "call_"..t[key]
            end,

            __tostring = function()
                return "table object"
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

    <Table as ObjectLike>::set(&table, "c", 3)?;
    assert_eq!(<Table as ObjectLike>::get::<i32>(&table, "c")?, 3);
    assert_eq!(table.call::<String>("b")?, "call_2");
    assert_eq!(table.call_function::<String>("func", "a")?, "func_a");
    assert_eq!(table.call_method::<String>("method", "a")?, "method_1");
    assert_eq!(table.to_string()?, "table object");

    match table.call_method::<()>("non_existent", ()) {
        Err(Error::RuntimeError(err)) => {
            assert!(err.contains("attempt to call a nil value (function 'non_existent')"))
        }
        r => panic!("expected RuntimeError, got {r:?}"),
    }

    // Test calling non-callable table
    let table2 = lua.create_table()?;
    assert!(matches!(table2.call::<()>(()), Err(Error::RuntimeError(_))));

    Ok(())
}
