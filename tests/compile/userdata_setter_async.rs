use mlua::Result;

#[derive(Clone, Debug, mlua::UserData)]
struct Foo(u64);

#[mlua::userdata_impl]
impl Foo {
    #[lua(setter)]
    async fn set_value(&mut self, val: u64) -> Result<()> {
        self.0 = val;
        Ok(())
    }
}

fn main() {}
