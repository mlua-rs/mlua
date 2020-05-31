use mlua::prelude::*;

fn sum(_: &Lua, (a, b): (i64, i64)) -> LuaResult<i64> {
    Ok(a + b)
}

#[mlua_derive::lua_module]
fn rust_module(lua: &Lua) -> LuaResult<LuaTable> {
    let exports = lua.create_table()?;
    exports.set("sum", lua.create_function(sum)?)?;
    Ok(exports)
}
