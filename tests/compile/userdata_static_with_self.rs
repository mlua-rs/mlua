#[derive(Default)]
#[mlua::userdata]
struct Foo {
    x: u32,
}

#[mlua::userdata_impl]
impl Foo {
    #[lua(field)]
    fn get_x(&self) -> mlua::Result<u32> {
        Ok(self.x)
    }
}

fn main() {}
