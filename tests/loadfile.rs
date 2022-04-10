use mlua::{Lua, Value};

#[test]
fn test_loadfile() {
    let lua = Lua::new();

    let env = lua.create_table().unwrap();
    env.set("test", 42u8).unwrap();

    let file = lua
        .load_file(file!().replace(".rs", ".lua"), Some(Value::Table(env)))
        .unwrap();

    let result = file.call::<_, u8>(()).unwrap();

    assert_eq!(result, 42);
}
