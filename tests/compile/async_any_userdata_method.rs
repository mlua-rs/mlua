use mlua::{UserDataMethods, Lua};

fn main() {
    let lua = Lua::new();

    lua.register_userdata_type::<String>(|reg| {
        let s = String::new();
        let mut s = &s;
        reg.add_async_method("t", |_, this: &String, ()| async {
            s = this;
            Ok(())
        });
    }).unwrap();
}
