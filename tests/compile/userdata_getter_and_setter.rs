#[derive(Default)]
#[mlua::userdata]
struct Foo {
    x: u32,
}

#[mlua::userdata_impl]
impl Foo {
    #[lua(getter, setter)]
    fn x(&self) -> mlua::Result<u32> {
        Ok(self.x)
    }
}

fn main() {}
