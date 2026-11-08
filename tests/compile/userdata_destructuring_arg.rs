#[derive(mlua::UserData)]
struct Foo {
    x: u32,
}

#[mlua::userdata_impl]
impl Foo {
    #[lua(infallible)]
    fn takes_pattern(&self, (a, b): (u32, u32)) -> u32 {
        self.x + a + b
    }
}

fn main() {}
