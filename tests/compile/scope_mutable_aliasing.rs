use mlua::{Lua, UserData};

fn main() {
    struct MyUserData<'a>(&'a mut i32);
    impl UserData for MyUserData<'_> {}

    let mut i = 1;

    let lua = Lua::new();
    lua.scope(|scope| {
        let _a = scope.create_userdata(MyUserData(&mut i)).unwrap();
        let _b = scope.create_userdata(MyUserData(&mut i)).unwrap();
        Ok(())
    });
}
