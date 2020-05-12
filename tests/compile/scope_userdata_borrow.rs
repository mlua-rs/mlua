use mlua::{Lua, UserData};

fn main() {
    // Should not allow userdata borrow to outlive lifetime of AnyUserData handle
    struct MyUserData<'a>(&'a i32);
    impl<'a> UserData for MyUserData<'a> {};

    let igood = 1;

    let lua = Lua::new();
    lua.scope(|scope| {
        let _ugood = scope.create_nonstatic_userdata(MyUserData(&igood)).unwrap();
        let _ubad = {
            let ibad = 42;
            scope.create_nonstatic_userdata(MyUserData(&ibad)).unwrap();
        };
        Ok(())
    });
}
