use mlua::Result;

#[derive(Clone, Debug)]
#[mlua::userdata]
struct Foo(u64);

#[mlua::userdata_impl]
impl Foo {
    #[lua(getter)]
    async fn value(&self) -> Result<u64> {
        Ok(self.0)
    }
}

fn main() {}
