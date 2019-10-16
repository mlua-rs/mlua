use mlua::{Lua, UserData, Result};

struct MyUserData<'a>(&'a i32);
impl<'a> UserData for MyUserData<'a> {}

fn main() {
    // Should not allow userdata borrow to outlive lifetime of AnyUserData handle

    let igood = 1;

    let lua = Lua::new();
    lua.scope(|scope| -> Result<()> {
        let _ugood = scope.create_nonstatic_userdata(MyUserData(&igood))?;
        let _ubad = {
            let ibad = 42;
            scope.create_nonstatic_userdata(MyUserData(&ibad))?;
        };
        Ok(())
    });
}
