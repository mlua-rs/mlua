use mlua::{Lua, UserData, UserDataMethods};

fn main() {
    let ref lua = Lua::new();

    #[derive(Clone)]
    struct MyUserData<'a>(&'a i64);

    impl<'a> UserData for MyUserData<'a> {
        fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_async_method("print", |_, data, ()| async move {
                println!("{}", data.0);
                Ok(())
            });
        }
    }
}
