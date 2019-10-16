use mlua::{Lua, Table, Result};

struct Test {
    field: i32,
}

fn main() {
    let lua = Lua::new();
    lua.scope(|scope| -> Result<()> {
        let mut inner: Option<Table> = None;
        let f = scope
            .create_function_mut(|_, t: Table| {
                inner = Some(t);
                Ok(())
            })?;
        f.call::<_, ()>(lua.create_table()?)?;
        Ok(())
    });
}
