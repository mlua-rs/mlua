use std::fs;
use std::io;

use mlua::{Error, Lua, Result};

#[test]
fn test_chunk_path() -> Result<()> {
    let lua = Lua::new();

    let temp_dir = tempfile::tempdir().unwrap();
    fs::write(
        temp_dir.path().join("module.lua"),
        r#"
        return 321
    "#,
    )?;
    let i: i32 = lua.load(&temp_dir.path().join("module.lua")).eval()?;
    assert_eq!(i, 321);

    match lua.load(&temp_dir.path().join("module2.lua")).exec() {
        Err(Error::ExternalError(err))
            if err.downcast_ref::<io::Error>().unwrap().kind() == io::ErrorKind::NotFound => {}
        res => panic!("expected io::Error, got {:?}", res),
    };

    Ok(())
}

#[test]
#[cfg(feature = "macros")]
fn test_chunk_macro() -> Result<()> {
    let lua = Lua::new();

    let name = "Rustacean";
    let table = vec![1];

    let data = lua.create_table()?;
    data.raw_set("num", 1)?;

    lua.globals().set("g", 123)?;

    lua.load(mlua::chunk! {
        assert($name == "Rustacean")
        assert($table[1] == 1)
        assert($data.num == 1)
        assert(g == 123)
        s = 321
    })
    .exec()?;

    assert_eq!(lua.globals().get::<_, i32>("s")?, 321);

    Ok(())
}
