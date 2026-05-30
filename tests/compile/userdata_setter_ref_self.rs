#[derive(Default, mlua::UserData)]
struct Foo {
    x: u32,
}

#[mlua::userdata_impl]
impl Foo {
    #[lua(setter)]
    fn set_x(self, val: u32) -> mlua::Result<()> {
        let _ = val;
        Ok(())
    }
}

fn main() {}
