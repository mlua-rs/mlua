use mlua::{Lua, Table};

fn main() {
    let lua = Lua::new();
    lua.scope(|scope| {
        let mut inner: Option<Table> = None;
        let f = scope
            .create_function_mut(move |_, t: Table| {
                if let Some(old) = inner.take() {
                    // Access old callback `Lua`.
                }
                inner = Some(t);
                Ok(())
            })?;
        f.call::<_, ()>(lua.create_table()?)?;
        Ok(())
    });
}
