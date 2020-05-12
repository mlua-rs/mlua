use mlua::{Lua, Table};

fn main() {
    let lua = Lua::new();
    let mut outer: Option<Table> = None;
    lua.scope(|scope| {
        let f = scope
            .create_function_mut(|_, t: Table| {
                outer = Some(t);
                Ok(())
            })?;
        f.call::<_, ()>(lua.create_table()?)?;
        Ok(())
    });
}
