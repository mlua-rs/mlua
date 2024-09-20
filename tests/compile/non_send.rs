use std::cell::Cell;
use std::rc::Rc;

use mlua::{Lua, Result};

fn main() -> Result<()> {
    let lua = Lua::new();

    let data = Rc::new(Cell::new(0));

    lua.create_function(move |_, ()| Ok(data.get()))?
        .call::<i32>(())?;

    Ok(())
}
