use mlua::{Lua, UserData, Result};

struct MyUserData<'a>(&'a mut i32);
impl<'a> UserData for MyUserData<'a> {}

fn main() {
    let mut i = 1;

    let lua = Lua::new();
    lua.scope(|scope| -> Result<()> {
        let _a = scope.create_nonstatic_userdata(MyUserData(&mut i))?;
        let _b = scope.create_nonstatic_userdata(MyUserData(&mut i))?;
        Ok(())
    });
}
