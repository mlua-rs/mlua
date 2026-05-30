#[derive(Default, mlua::UserData)]
struct Foo;

#[mlua::userdata_impl]
impl Foo {
    #[lua(getter, meta)]
    fn bar(&self) -> mlua::Result<u32> {
        Ok(42)
    }
}

fn main() {}
