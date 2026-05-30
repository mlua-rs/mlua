#[derive(Default, mlua::UserData)]
struct Foo {
    x: u32,
}

#[mlua::userdata_impl]
impl Foo {
    #[lua(getter)]
    fn x(&mut self) -> mlua::Result<u32> {
        Ok(self.x)
    }
}

fn main() {}
