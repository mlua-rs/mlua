use std::panic::catch_unwind;

use mlua::Lua;

fn main() {
    let lua = Lua::new();
    let table = lua.create_table().unwrap();
    catch_unwind(move || table.set("a", "b").unwrap());
}
