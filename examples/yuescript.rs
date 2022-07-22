use mlua::{Function, Lua, LuaOptions, MultiValue, Result, StdLib, Table, Value};

fn main() -> Result<()> {
    let lua = Lua::new_with(StdLib::ALL_SAFE | StdLib::YUE, LuaOptions::new())?;

    let source = r#"
f = ->
    print "hello world"
f!
    "#;

    let yue = lua
        .globals()
        .get::<_, Function>("require")?
        .call::<_, Table>("yue")?;
    let config = lua.create_table()?;
    config.set("implicit_return_root", true)?;
    config.set("reserve_line_number", true)?;
    config.set("lint_global", true)?;
    let res = yue
        .get::<_, Function>("to_lua")?
        .call::<_, MultiValue>(source)?;

    if let Value::String(code) = res.get(0).unwrap() {
        println!("{:?}", code.to_str());
    } else {
        println!("Result code is not string");
    }

    Ok(())
}
