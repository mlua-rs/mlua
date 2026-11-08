#[derive(mlua::UserData)]
struct Foo {
    x: u32,
}

#[mlua::userdata_impl]
impl<T> Foo<T> {
    #[lua(infallible)]
    fn get(&self) -> u32 {
        self.x
    }
}

#[mlua::userdata_impl]
impl Foo {
    fn generic<T>(&self, value: T) -> mlua::Result<T> {
        Ok(value)
    }
}

fn main() {}
