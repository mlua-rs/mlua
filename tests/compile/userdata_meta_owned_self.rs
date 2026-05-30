#[derive(Default, mlua::UserData)]
struct Foo;

#[mlua::userdata_impl]
impl Foo {
    #[lua(meta)]
    fn __gc(self) -> mlua::Result<()> {
        Ok(())
    }
}

fn main() {}
