#![cfg(feature = "luau")]

use mlua::{Lua, Result, Value};

#[test]
fn test_buffer() -> Result<()> {
    let lua = Lua::new();

    let buf1 = lua
        .load(
            r#"
        local buf = buffer.fromstring("hello")
        assert(buffer.len(buf) == 5)
        return buf
    "#,
        )
        .eval::<Value>()?;
    assert!(buf1.is_buffer());
    assert_eq!(buf1.type_name(), "buffer");

    let buf2 = lua.load("buffer.fromstring('hello')").eval::<Value>()?;
    assert_ne!(buf1, buf2);

    // Check that we can pass buffer type to Lua
    let buf1 = buf1.as_buffer().unwrap();
    let func = lua.create_function(|_, buf: Value| return buf.to_string())?;
    assert!(func.call::<String>(buf1)?.starts_with("buffer:"));

    // Check buffer methods
    assert_eq!(buf1.len(), 5);
    assert_eq!(buf1.to_vec(), b"hello");
    assert_eq!(buf1.read_bytes::<3>(1), [b'e', b'l', b'l']);
    buf1.write_bytes(1, b"i");
    assert_eq!(buf1.to_vec(), b"hillo");

    let buf3 = lua.create_buffer(b"")?;
    assert!(buf3.is_empty());
    assert!(!Value::Buffer(buf3).to_pointer().is_null());

    Ok(())
}

#[test]
#[should_panic(expected = "range end index 14 out of range for slice of length 13")]
fn test_buffer_out_of_bounds_read() {
    let lua = Lua::new();
    let buf = lua.create_buffer(b"hello, world!").unwrap();
    _ = buf.read_bytes::<1>(13);
}

#[test]
#[should_panic(expected = "range end index 16 out of range for slice of length 13")]
fn test_buffer_out_of_bounds_write() {
    let lua = Lua::new();
    let buf = lua.create_buffer(b"hello, world!").unwrap();
    buf.write_bytes(14, b"!!");
}
