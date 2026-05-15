#[derive(Default)]
#[mlua::userdata]
struct Foo {
    x: u32,
}

#[mlua::userdata_impl]
impl Foo {
    #[lua(getter)]
    fn x(&self, extra: u32) -> mlua::Result<u32> {
        Ok(self.x + extra)
    }
}

fn main() {}
