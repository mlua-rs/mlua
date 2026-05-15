#[derive(Default)]
#[mlua::userdata]
struct Foo(Vec<u8>);

#[mlua::userdata_impl]
impl Foo {
    fn first(&self, data: &mut [u8]) -> mlua::Result<u8> {
        Ok(data[0])
    }
}

fn main() {}
