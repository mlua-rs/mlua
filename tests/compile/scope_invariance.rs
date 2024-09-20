use mlua::Lua;

struct Test {
    field: i32,
}

fn main() {
    let lua = Lua::new();
    lua.scope(|scope| {
        let f = {
            let mut test = Test { field: 0 };

            scope.create_function_mut(|_, ()| {
                test.field = 42;
                //~^ error: `test` does not live long enough
                Ok(())
            })?
        };

        f.call::<()>(())
    });
}
