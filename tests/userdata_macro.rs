#![cfg(feature = "macros")]

use mlua::{Lua, Result, UserData};

#[derive(Default, Clone, Debug, UserData)]
struct Rectangle {
    length: u32,
    #[lua]
    width: u32,

    #[lua(get, name = "version")]
    version_ro: u32,

    #[lua(skip)]
    _internal: u64,
}

#[mlua::userdata_impl]
impl Rectangle {
    const TYPE_NAME: &str = "Rectangle";

    #[lua(infallible)]
    fn new(length: u32, width: u32) -> Self {
        Rectangle {
            length,
            width,
            version_ro: 1,
            _internal: 0,
        }
    }

    fn area(&self) -> Result<u32> {
        Ok(self.length * self.width)
    }

    #[lua(getter, infallible, name = "perimeter")]
    fn calculate_perimeter(&self) -> u32 {
        2 * (self.length + self.width)
    }

    #[lua(infallible)]
    fn diagonal(&self) -> f64 {
        (self.length.pow(2) as f64 + self.width.pow(2) as f64).sqrt()
    }

    fn scale(&mut self, factor: u32) -> Result<()> {
        self.length *= factor;
        self.width *= factor;
        Ok(())
    }

    #[lua(setter, name = "size", infallible)]
    fn set_size(&mut self, _lua: &Lua, size: u32) {
        self.length = size;
        self.width = size;
    }

    #[lua(field)]
    fn description() -> &'static str {
        "A rectangle shape"
    }

    #[lua(meta)]
    fn __tostring(&self) -> Result<String> {
        Ok(format!("Rectangle({}x{})", self.length, self.width))
    }

    #[lua(meta, name = "__call")]
    fn call() -> Result<Self> {
        Ok(Rectangle::default())
    }

    #[lua(meta, infallible, name = "__add")]
    fn add(&self, other: &Rectangle) -> Rectangle {
        Rectangle {
            length: self.length + other.length,
            width: self.width + other.width,
            ..Default::default()
        }
    }

    #[lua(meta, field, name = "__answer")]
    fn answer() -> u32 {
        42
    }

    #[lua(skip)]
    #[allow(unused)]
    fn helper() -> u32 {
        42
    }

    fn default_size() -> Result<(u32, u32)> {
        Ok((100, 100))
    }

    fn get_width(&self, lua: &Lua) -> Result<u32> {
        let _ = lua.globals().len();
        Ok(self.width)
    }

    #[lua(getter, name = "lua_version")]
    fn get_lua_version(&self, lua: &::mlua::Lua) -> Result<String> {
        // `::mlua::Lua` is used to check that the type is correctly resolved in macros
        lua.globals().get("_VERSION")
    }

    fn into_tuple(self) -> Result<(u32, u32)> {
        Ok((self.length, self.width))
    }

    fn greet(&self, name: &str) -> Result<String> {
        Ok(format!("Hello, {name}!"))
    }

    fn transfer_length(&mut self, other: &mut Rectangle) -> Result<()> {
        other.length += self.length;
        self.length = 0;
        Ok(())
    }
}

#[mlua::userdata_impl]
impl Rectangle {
    #[lua(infallible)]
    fn double_length(&self) -> u32 {
        self.length * 2
    }
}

fn make_lua() -> Lua {
    let lua = unsafe { Lua::unsafe_new() };
    lua.globals()
        .set("Rectangle", lua.create_proxy::<Rectangle>().unwrap())
        .unwrap();
    lua
}

#[test]
fn test_rectangle() {
    let lua = make_lua();

    // Basic fields, getters, setters, methods
    lua.load(
        r#"
        rect = Rectangle.new(5, 10, 3)
        assert(rect.length == 5, "length should be 5")
        assert(rect.width == 10, "width should be 10")
        assert(rect.perimeter == 30, "perimeter should be 30")

        -- read-only field
        assert(rect.version == 1, "version should be 1")
        local ok, err = pcall(function() rect.version = 2 end)
        assert(not ok, "version should be read-only")

        -- skipped
        assert(rect._internal == nil, "_internal should be nil")
        assert(rect.helper == nil, "skipped method should be nil")

        rect.length = 15
        rect.width = 20
        assert(rect.length == 15, "length should be updated to 15")
        assert(rect.width == 20, "width should be updated to 20")
        assert(rect.perimeter == 70, "perimeter should be updated to 70")
        assert(rect:area() == 300, "area should return 300")

        rect:scale(2)
        assert(rect.length == 30, "length should be scaled to 30")
        assert(rect.width == 40, "width should be scaled to 40")
        assert(rect:diagonal() == 50.0, "diagonal should be 50.0")

        rect.size = 7
        assert(rect.length == 7, "length should be updated to 7")
        assert(rect.width == 7, "width should be updated to 7")

        -- static / associated items
        assert(rect.TYPE_NAME == 'Rectangle', "TYPE_NAME should be 'Rectangle'")
        assert(rect.description == 'A rectangle shape', "description should be 'A rectangle shape'")
        local w, h = rect.default_size()
        assert(w == 100, "default_size width should be 100")
        assert(h == 100, "default_size height should be 100")

        -- meta methods
        local r1 = Rectangle.new(5, 10, 0)
        assert(tostring(r1) == 'Rectangle(5x10)', "__tostring should return 'Rectangle(5x10)'")
        local r2 = r1()
        assert(r2:area() == 0, "__call should create a default rect")
        local r3 = Rectangle.new(3, 4, 0)
        local r4 = r1 + r3
        assert(r4.length == 8, "__add length should be 5 + 3 = 8")
        assert(r4.width == 14, "__add width should be 10 + 4 = 14")

        -- method with &mut self and &mut Rectangle param
        rect = Rectangle.new(5, 10, 3)
        other = Rectangle.new(2, 3, 0)
        rect:transfer_length(other)
        assert(rect.length == 0, "length should be 0 after transfer")
        assert(other.length == 7, "other length should be 7 after transfer")
        assert(other:double_length() == 14, "double_length should be 14")

        -- meta field
        if _VERSION:match("Lua ") then
            local mt = debug.getmetatable(rect)
            assert(mt.__answer == 42, "__answer meta field should be 42")
        end

        assert(rect.lua_version == _VERSION, "lua_version should match Lua's _VERSION")

        -- Consuming method
        local w, h = rect:into_tuple()
        assert(w == 0, "into_tuple width should be 0")
        assert(h == 10, "into_tuple height should be 7")
        local ok, err = pcall(function() rect:area() end)
        assert(not ok and tostring(err):match("userdata has been destructed"), "rect should be consumed and unusable after into_tuple")
    "#,
    )
    .exec()
    .unwrap();
}

#[derive(Clone, Debug, UserData)]
enum Color {
    Red,
    Green,
    Blue,
}

fn make_lua_color() -> Lua {
    let lua = Lua::new();
    lua.globals()
        .set("Color", lua.create_proxy::<Color>().unwrap())
        .unwrap();
    lua
}

#[mlua::userdata_impl]
impl Color {
    #[lua(infallible)]
    fn new(r: u8, g: u8, b: u8) -> Self {
        if r > 0 && g == 0 && b == 0 {
            Color::Red
        } else if g > 0 && r == 0 && b == 0 {
            Color::Green
        } else {
            Color::Blue
        }
    }

    #[lua(infallible)]
    fn name(&self) -> String {
        match self {
            Color::Red => "red".into(),
            Color::Green => "green".into(),
            Color::Blue => "blue".into(),
        }
    }

    #[lua(meta, infallible)]
    fn __tostring(&self) -> String {
        self.name()
    }
}

#[test]
fn test_color() {
    let lua = make_lua_color();
    lua.load(
        r#"
        red = Color.new(255, 0, 0)
        green = Color.new(0, 255, 0)
        blue = Color.new(0, 0, 255)

        assert(red:name() == 'red', "red name should be 'red'")
        assert(green:name() == 'green', "green name should be 'green'")
        assert(blue:name() == 'blue', "blue name should be 'blue'")

        assert(tostring(red) == 'red', "red tostring should be 'red'")
        assert(tostring(green) == 'green', "green tostring should be 'green'")
        assert(tostring(blue) == 'blue', "blue tostring should be 'blue'")
    "#,
    )
    .exec()
    .unwrap();
}

#[derive(Clone, Debug, UserData)]
struct Point(i32, i32);

fn make_lua_point() -> Lua {
    let lua = Lua::new();
    lua.globals()
        .set("Point", lua.create_proxy::<Point>().unwrap())
        .unwrap();
    lua
}

#[mlua::userdata_impl]
impl Point {
    #[lua(infallible)]
    fn new(x: i32, y: i32) -> Self {
        Point(x, y)
    }

    fn x(&self) -> Result<i32> {
        Ok(self.0)
    }

    fn y(&self) -> Result<i32> {
        Ok(self.1)
    }

    fn distance(&self, other: &Point) -> Result<f64> {
        let dx = (self.0 - other.0) as f64;
        let dy = (self.1 - other.1) as f64;
        Ok((dx * dx + dy * dy).sqrt())
    }
}

#[test]
fn test_point() {
    let lua = make_lua_point();
    lua.load(
        r#"
        p1 = Point.new(0, 0)
        p2 = Point.new(3, 4)

        assert(p1:x() == 0, "p1.x should be 0")
        assert(p1:y() == 0, "p1.y should be 0")
        assert(p2:x() == 3, "p2.x should be 3")
        assert(p2:y() == 4, "p2.y should be 4")

        assert(p1:distance(p2) == 5.0, "distance should be 5.0")
    "#,
    )
    .exec()
    .unwrap();
}

#[derive(Clone, Debug, UserData)]
struct Bytes(Vec<u8>);

#[mlua::userdata_impl]
impl Bytes {
    #[lua(meta, name = "__type")]
    const TYPE: &str = "MyBytes";

    #[lua(infallible)]
    fn new(data: &[u8]) -> Self {
        Bytes(data.to_vec())
    }

    fn first(&self) -> Result<Option<u8>> {
        Ok(self.0.first().copied())
    }

    fn len(&self) -> Result<usize> {
        Ok(self.0.len())
    }
}

#[test]
fn test_known_borrow_wrappers() -> Result<()> {
    let lua = Lua::new();
    lua.globals()
        .set("Bytes", lua.create_proxy::<Bytes>().unwrap())
        .unwrap();
    lua.load(
        r#"
        local b = Bytes.new('abc')
        assert(b:first() == 97, "first should return 97 ('a')")
        assert(b:len() == 3, "len should return 3")

        if _VERSION:match("Luau") then
            assert(typeof(b) == 'MyBytes', "type should be MyBytes in Luau")
        end
    "#,
    )
    .exec()
    .unwrap();
    Ok(())
}

#[cfg(feature = "async")]
mod async_tests {
    use mlua::{Lua, Result, UserData};

    #[derive(Clone, Debug, UserData)]
    struct AsyncCounter(u64);

    #[mlua::userdata_impl]
    impl AsyncCounter {
        #[lua(infallible)]
        fn new() -> Self {
            AsyncCounter(0)
        }

        async fn get_value(&self) -> Result<u64> {
            Ok(self.0)
        }

        async fn set_value(&mut self, value: u64) -> Result<()> {
            self.0 = value;
            Ok(())
        }

        async fn take_value(self) -> Result<u64> {
            Ok(self.0)
        }

        #[lua(infallible)]
        async fn get_value_infallible(&self) -> u64 {
            self.0
        }

        async fn multiply(&self, factor: u64) -> Result<u64> {
            Ok(self.0 * factor)
        }

        async fn default_value() -> Result<u64> {
            Ok(42)
        }

        #[cfg(not(any(feature = "lua51", feature = "luau")))]
        #[lua(meta)]
        async fn __tostring(&self) -> Result<String> {
            Ok(format!("Counter({})", self.0))
        }
    }

    #[tokio::test]
    async fn test_async_methods() {
        let lua = Lua::new();
        lua.globals()
            .set("AsyncCounter", lua.create_proxy::<AsyncCounter>().unwrap())
            .unwrap();

        lua.load(
            r#"
            local c = AsyncCounter.new()
            c:set_value(10)
            local val = c:get_value()
            assert(val == 10, "expected 10, got " .. tostring(val))
            local doubled = c:multiply(3)
            assert(doubled == 30, "expected 30, got " .. tostring(doubled))
            local inf = c:get_value_infallible()
            assert(inf == 10, "expected infallible 10, got " .. tostring(inf))
        "#,
        )
        .exec_async()
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_async_consume() {
        let lua = Lua::new();
        lua.globals()
            .set("AsyncCounter", lua.create_proxy::<AsyncCounter>().unwrap())
            .unwrap();

        lua.load(
            r#"
            local c = AsyncCounter.new()
            c:set_value(42)
            local val = c:take_value()
            assert(val == 42)
            local ok, err = pcall(function() c:get_value() end)
            assert(not ok and tostring(err):match("userdata has been destructed"))
        "#,
        )
        .exec_async()
        .await
        .unwrap();
    }

    #[cfg(not(any(feature = "lua51", feature = "luau")))]
    #[tokio::test]
    async fn test_async_meta() {
        let lua = Lua::new();
        lua.globals()
            .set("AsyncCounter", lua.create_proxy::<AsyncCounter>().unwrap())
            .unwrap();

        lua.load(
            r#"
            local c = AsyncCounter.new()
            c:set_value(7)
            assert(tostring(c) == "Counter(7)")
        "#,
        )
        .exec_async()
        .await
        .unwrap();
    }
}
