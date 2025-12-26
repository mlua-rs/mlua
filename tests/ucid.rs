//! Tests for Unicode Identifier (UCID) support in Lua.
//!
//! This module tests the `ucid` feature flag, which enables Unicode identifiers
//! in Lua code. When enabled, variable names, function names, and other identifiers
//! can use non-ASCII characters (e.g., Japanese, Chinese, Korean, Cyrillic, etc.).
//!
//! The `ucid` feature propagates to the `lua-src` crate, which compiles Lua with
//! the `LUA_UCID` flag enabled.
#![cfg(feature = "ucid")]

use mlua::{Lua, Result};

/// Test that Japanese identifiers can be used for variables.
#[test]
fn test_japanese_variable_names() -> Result<()> {
    let lua = Lua::new();

    // Using Japanese characters for variable names
    lua.load(
        r#"
        数値 = 42
        文字列 = "こんにちは"
        配列 = {1, 2, 3}
        "#,
    )
    .exec()?;

    let globals = lua.globals();

    // Verify the values are correctly assigned
    let num: i32 = globals.get("数値")?;
    assert_eq!(num, 42);

    let text: String = globals.get("文字列")?;
    assert_eq!(text, "こんにちは");

    let arr: Vec<i32> = globals.get("配列")?;
    assert_eq!(arr, vec![1, 2, 3]);

    Ok(())
}

/// Test that Japanese identifiers can be used for function names.
#[test]
fn test_japanese_function_names() -> Result<()> {
    let lua = Lua::new();

    // Define a function with a Japanese name
    lua.load(
        r#"
        function 挨拶する(名前)
            return "こんにちは、" .. 名前 .. "さん！"
        end
        "#,
    )
    .exec()?;

    // Call the function and verify the result
    let result: String = lua.load(r#"挨拶する("太郎")"#).eval()?;
    assert_eq!(result, "こんにちは、太郎さん！");

    Ok(())
}

/// Test that Japanese identifiers can be used in table fields.
#[test]
fn test_japanese_table_fields() -> Result<()> {
    let lua = Lua::new();

    // Create a table with Japanese field names
    lua.load(
        r#"
        人物 = {
            名前 = "山田太郎",
            年齢 = 30,
            職業 = "エンジニア"
        }
        "#,
    )
    .exec()?;

    let globals = lua.globals();
    let person: mlua::Table = globals.get("人物")?;

    let name: String = person.get("名前")?;
    assert_eq!(name, "山田太郎");

    let age: i32 = person.get("年齢")?;
    assert_eq!(age, 30);

    let job: String = person.get("職業")?;
    assert_eq!(job, "エンジニア");

    Ok(())
}

/// Test that other Unicode scripts also work (Cyrillic example).
#[test]
fn test_cyrillic_identifiers() -> Result<()> {
    let lua = Lua::new();

    // Using Cyrillic characters for variable names (Russian)
    lua.load(
        r#"
        число = 100
        текст = "Привет"
        "#,
    )
    .exec()?;

    let globals = lua.globals();

    let num: i32 = globals.get("число")?;
    assert_eq!(num, 100);

    let text: String = globals.get("текст")?;
    assert_eq!(text, "Привет");

    Ok(())
}

/// Test that Chinese identifiers work.
#[test]
fn test_chinese_identifiers() -> Result<()> {
    let lua = Lua::new();

    // Using Chinese characters for variable and function names
    lua.load(
        r#"
        function 计算总和(数字列表)
            local 总和 = 0
            for _, 数字 in ipairs(数字列表) do
                总和 = 总和 + 数字
            end
            return 总和
        end
        "#,
    )
    .exec()?;

    let result: i32 = lua.load(r#"计算总和({1, 2, 3, 4, 5})"#).eval()?;
    assert_eq!(result, 15);

    Ok(())
}

/// Test complex expressions with mixed Unicode identifiers.
#[test]
fn test_mixed_unicode_identifiers() -> Result<()> {
    let lua = Lua::new();

    // Mix of Japanese, ASCII, and other Unicode in the same code
    lua.load(
        r#"
        local 合計 = 0
        local items = {10, 20, 30}
        
        for i, 値 in ipairs(items) do
            合計 = 合計 + 値
        end
        
        結果 = 合計
        "#,
    )
    .exec()?;

    let result: i32 = lua.globals().get("結果")?;
    assert_eq!(result, 60);

    Ok(())
}
