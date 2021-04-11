use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::CString;

use maplit::{btreemap, btreeset, hashmap, hashset};
use mlua::{Lua, Result};

#[test]
fn test_conv_vec() -> Result<()> {
    let lua = Lua::new();

    let v = vec![1, 2, 3];
    lua.globals().set("v", v.clone())?;
    let v2: Vec<i32> = lua.globals().get("v")?;
    assert!(v == v2);

    Ok(())
}

#[test]
fn test_conv_hashmap() -> Result<()> {
    let lua = Lua::new();

    let map = hashmap! {"hello".to_string() => "world".to_string()};
    lua.globals().set("map", map.clone())?;
    let map2: HashMap<String, String> = lua.globals().get("map")?;
    assert!(map == map2);

    Ok(())
}

#[test]
fn test_conv_hashset() -> Result<()> {
    let lua = Lua::new();

    let set = hashset! {"hello".to_string(), "world".to_string()};
    lua.globals().set("set", set.clone())?;
    let set2: HashSet<String> = lua.globals().get("set")?;
    assert!(set == set2);

    Ok(())
}

#[test]
fn test_conv_btreemap() -> Result<()> {
    let lua = Lua::new();

    let map = btreemap! {"hello".to_string() => "world".to_string()};
    lua.globals().set("map", map.clone())?;
    let map2: BTreeMap<String, String> = lua.globals().get("map")?;
    assert!(map == map2);

    Ok(())
}

#[test]
fn test_conv_btreeset() -> Result<()> {
    let lua = Lua::new();

    let set = btreeset! {"hello".to_string(), "world".to_string()};
    lua.globals().set("set", set.clone())?;
    let set2: BTreeSet<String> = lua.globals().get("set")?;
    assert!(set == set2);

    Ok(())
}

#[test]
fn test_conv_cstring() -> Result<()> {
    let lua = Lua::new();

    let s = CString::new(b"hello".to_vec()).unwrap();
    lua.globals().set("s", s.clone())?;
    let s2: CString = lua.globals().get("s")?;
    assert!(s == s2);

    Ok(())
}

#[test]
fn test_conv_cow() -> Result<()> {
    let lua = Lua::new();

    let s = Cow::from("hello");
    lua.globals().set("s", s.clone())?;
    let s2: String = lua.globals().get("s")?;
    assert!(s == s2);

    Ok(())
}
