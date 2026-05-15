#[derive(Default)]
#[mlua::userdata]
struct Foo;

#[mlua::userdata_impl]
impl Foo {
    #[lua(meta)]
    fn __gc(self) -> mlua::Result<()> {
        Ok(())
    }
}

fn main() {}
