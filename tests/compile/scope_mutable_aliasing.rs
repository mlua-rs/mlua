use mlua::{Lua, UserData};

fn main() {
    struct MyUserData<'a>(&'a mut i32);
    impl<'a> UserData for MyUserData<'a> {};

    let mut i = 1;

    let lua = Lua::new();
    lua.scope(|scope| {
        let _a = scope.create_nonstatic_userdata(MyUserData(&mut i)).unwrap();
        let _b = scope.create_nonstatic_userdata(MyUserData(&mut i)).unwrap();
        Ok(())
    });
}
