use mlua::{UserData, UserDataMethods};

struct MyUserData;

impl UserData for MyUserData {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_async_method("method", |_, this: &'static Self, ()| async {
            Ok(())
        });
        // ^ lifetime may not live long enough
    }
}

fn main() {}
