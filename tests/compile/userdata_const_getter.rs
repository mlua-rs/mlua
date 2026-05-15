#[derive(Default)]
#[mlua::userdata]
struct Foo;

#[mlua::userdata_impl]
impl Foo {
    #[lua(getter)]
    const X: u32 = 42;
}

fn main() {}
