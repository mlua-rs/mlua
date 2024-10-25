use mlua::{Lua, Result};

struct Test(i32);

fn main() {
    let test = Test(0);

    let lua = Lua::new();
    let _ = lua.create_function(|_, ()| -> Result<i32> { Ok(test.0) });
}
