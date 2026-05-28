use mlua::Result;

#[derive(Clone, Debug)]
#[mlua::userdata]
struct Foo;

#[mlua::userdata_impl]
impl Foo {
    #[lua(field)]
    async fn description() -> Result<String> {
        Ok("foo".into())
    }
}

fn main() {}
