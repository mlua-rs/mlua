use mlua::{Lua, Result};

#[test]
fn test_debug_format() -> Result<()> {
    let lua = Lua::new();

    // Globals
    let globals = lua.globals();
    let dump = format!("{globals:#?}");
    assert!(dump.starts_with("{\n  _G = table:"));

    // TODO: Other cases

    Ok(())
}
