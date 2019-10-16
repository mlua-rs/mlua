use mlua::{Lua, Table, Result};

struct Test {
    field: i32,
}

fn main() {
    let lua = Lua::new();
    let mut outer: Option<Table> = None;
    lua.scope(|scope| -> Result<()> {
        let f = scope
            .create_function_mut(|_, t: Table| {
                outer = Some(t);
                Ok(())
            })?;
        f.call::<_, ()>(lua.create_table()?)?;
        Ok(())
    });
}
