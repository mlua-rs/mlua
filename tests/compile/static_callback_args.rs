use std::cell::RefCell;

use mlua::{Lua, Result, Table};

fn main() -> Result<()> {
    thread_local! {
        static BAD_TIME: RefCell<Option<Table<'static>>> = RefCell::new(None);
    }

    let lua = Lua::new();

    lua.create_function(|_, table: Table| {
        BAD_TIME.with(|bt| {
            *bt.borrow_mut() = Some(table);
        });
        Ok(())
    })?
    .call::<_, ()>(lua.create_table()?)?;

    // In debug, this will panic with a reference leak before getting to the next part but
    // it segfaults anyway.
    drop(lua);

    BAD_TIME.with(|bt| {
        println!(
            "you're gonna have a bad time: {}",
            bt.borrow().as_ref().unwrap().len().unwrap()
        );
    });

    Ok(())
}
