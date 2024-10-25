use mlua::{UserData, UserDataMethods};

fn main() {
    #[derive(Clone)]
    struct MyUserData<'a>(&'a i64);

    impl UserData for MyUserData<'_> {
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_async_method("print", |_, data, ()| async move {
                println!("{}", data.0);
                Ok(())
            });
        }
    }
}
