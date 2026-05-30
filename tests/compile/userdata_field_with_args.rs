#[derive(Default)]
#[mlua::userdata]
struct Foo {
    x: u32,
}

#[mlua::userdata_impl]
impl Foo {
    #[lua(field)]
    fn as_name(name: &str) -> String {
        name.to_string()
    }
}

fn main() {}
