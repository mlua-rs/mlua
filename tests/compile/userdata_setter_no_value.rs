#[derive(Default)]
#[mlua::userdata]
struct Foo {
    x: u32,
}

#[mlua::userdata_impl]
impl Foo {
    #[lua(setter)]
    fn set_x(&mut self) -> mlua::Result<()> {
        Ok(())
    }
}

fn main() {}
