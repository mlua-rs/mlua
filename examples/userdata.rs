use mlua::{Lua, Result, UserData, chunk};

#[derive(Default, UserData)]
struct Rectangle {
    length: u32,
    width: u32,
}

#[mlua::userdata_impl]
impl Rectangle {
    const NAME: &str = "Rectangle";

    #[lua(infallible)]
    fn new(length: u32, width: u32) -> Self {
        Self { length, width }
    }

    #[lua(getter, name = "area", infallible)]
    fn calculate_area(&self) -> u32 {
        self.length * self.width
    }

    fn diagonal(&self) -> Result<f64> {
        Ok((self.length.pow(2) as f64 + self.width.pow(2) as f64).sqrt())
    }

    // Constructor via `__call` metamethod
    #[lua(meta, infallible)]
    fn __call(length: u32, width: u32) -> Self {
        Rectangle::new(length, width)
    }
}

fn main() -> Result<()> {
    let lua = Lua::new();
    lua.globals().set("Rectangle", lua.create_proxy::<Rectangle>()?)?;
    lua.load(chunk! {
        local rect = Rectangle(10, 5)
        rect.width = rect.width + 5
        rect.length = rect.length + 5
        assert(rect.NAME == "Rectangle")
        assert(rect.area == 150)
        assert(math.floor(rect:diagonal()) == 18)
    })
    .exec()
}
