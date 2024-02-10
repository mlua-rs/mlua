use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::{CStr, CString};

use maplit::{btreemap, btreeset, hashmap, hashset};
use mlua::{
    AnyUserData, Error, Function, IntoLua, Lua, RegistryKey, Result, Table, Thread, UserDataRef,
    Value,
};

#[test]
fn test_value_into_lua() -> Result<()> {
    let lua = Lua::new();

    // Direct conversion
    let v = Value::Boolean(true);
    let v2 = (&v).into_lua(&lua)?;
    assert_eq!(v, v2);

    // Push into stack
    let table = lua.create_table()?;
    table.set("v", &v)?;
    assert_eq!(v, table.get::<_, Value>("v")?);

    Ok(())
}

#[test]
fn test_string_into_lua() -> Result<()> {
    let lua = Lua::new();

    // Direct conversion
    let s = lua.create_string("hello, world!")?;
    let s2 = (&s).into_lua(&lua)?;
    assert_eq!(s, s2.as_string().unwrap());

    // Push into stack
    let table = lua.create_table()?;
    table.set("s", &s)?;
    assert_eq!(s, table.get::<_, String>("s")?);

    Ok(())
}

#[cfg(all(feature = "unstable", not(feature = "send")))]
#[test]
fn test_owned_string_into_lua() -> Result<()> {
    let lua = Lua::new();

    // Direct conversion
    let s = lua.create_string("hello, world")?.into_owned();
    let s2 = (&s).into_lua(&lua)?;
    assert_eq!(s.to_ref(), *s2.as_string().unwrap());

    // Push into stack
    let table = lua.create_table()?;
    table.set("s", &s)?;
    assert_eq!(s.to_ref(), table.get::<_, String>("s")?);

    Ok(())
}

#[cfg(all(feature = "unstable", not(feature = "send")))]
#[test]
fn test_owned_string_from_lua() -> Result<()> {
    let lua = Lua::new();

    let s = lua.unpack::<mlua::OwnedString>(lua.pack("hello, world")?)?;
    assert_eq!(s.to_ref(), "hello, world");

    Ok(())
}

#[test]
fn test_table_into_lua() -> Result<()> {
    let lua = Lua::new();

    // Direct conversion
    let t = lua.create_table()?;
    let t2 = (&t).into_lua(&lua)?;
    assert_eq!(&t, t2.as_table().unwrap());

    // Push into stack
    let f = lua.create_function(|_, (t, s): (Table, String)| t.set("s", s))?;
    f.call((&t, "hello"))?;
    assert_eq!("hello", t.get::<_, String>("s")?);

    Ok(())
}

#[cfg(all(feature = "unstable", not(feature = "send")))]
#[test]
fn test_owned_table_into_lua() -> Result<()> {
    let lua = Lua::new();

    // Direct conversion
    let t = lua.create_table()?.into_owned();
    let t2 = (&t).into_lua(&lua)?;
    assert_eq!(t.to_ref(), *t2.as_table().unwrap());

    // Push into stack
    let f = lua.create_function(|_, (t, s): (Table, String)| t.set("s", s))?;
    f.call((&t, "hello"))?;
    assert_eq!("hello", t.to_ref().get::<_, String>("s")?);

    Ok(())
}

#[test]
fn test_function_into_lua() -> Result<()> {
    let lua = Lua::new();

    // Direct conversion
    let f = lua.create_function(|_, ()| Ok::<_, Error>(()))?;
    let f2 = (&f).into_lua(&lua)?;
    assert_eq!(&f, f2.as_function().unwrap());

    // Push into stack
    let table = lua.create_table()?;
    table.set("f", &f)?;
    assert_eq!(f, table.get::<_, Function>("f")?);

    Ok(())
}

#[cfg(all(feature = "unstable", not(feature = "send")))]
#[test]
fn test_owned_function_into_lua() -> Result<()> {
    let lua = Lua::new();

    // Direct conversion
    let f = lua
        .create_function(|_, ()| Ok::<_, Error>(()))?
        .into_owned();
    let f2 = (&f).into_lua(&lua)?;
    assert_eq!(f.to_ref(), *f2.as_function().unwrap());

    // Push into stack
    let table = lua.create_table()?;
    table.set("f", &f)?;
    assert_eq!(f.to_ref(), table.get::<_, Function>("f")?);

    Ok(())
}

#[test]
fn test_thread_into_lua() -> Result<()> {
    let lua = Lua::new();

    // Direct conversion
    let f = lua.create_function(|_, ()| Ok::<_, Error>(()))?;
    let th = lua.create_thread(f)?;
    let th2 = (&th).into_lua(&lua)?;
    assert_eq!(&th, th2.as_thread().unwrap());

    // Push into stack
    let table = lua.create_table()?;
    table.set("th", &th)?;
    assert_eq!(th, table.get::<_, Thread>("th")?);

    Ok(())
}

#[cfg(all(feature = "unstable", not(feature = "send")))]
#[test]
fn test_owned_thread_into_lua() -> Result<()> {
    let lua = Lua::new();

    // Direct conversion
    let f = lua.create_function(|_, ()| Ok::<_, Error>(()))?;
    let th = lua.create_thread(f)?.into_owned();
    let th2 = (&th).into_lua(&lua)?;
    assert_eq!(&th.to_ref(), th2.as_thread().unwrap());

    // Push into stack
    let table = lua.create_table()?;
    table.set("th", &th)?;
    assert_eq!(th.to_ref(), table.get::<_, Thread>("th")?);

    Ok(())
}

#[cfg(all(feature = "unstable", not(feature = "send")))]
#[test]
fn test_owned_thread_from_lua() -> Result<()> {
    let lua = Lua::new();

    let th = lua.unpack::<mlua::OwnedThread>(Value::Thread(lua.current_thread()))?;
    assert_eq!(th.to_ref(), lua.current_thread());

    Ok(())
}

#[test]
fn test_anyuserdata_into_lua() -> Result<()> {
    let lua = Lua::new();

    // Direct conversion
    let ud = lua.create_any_userdata(String::from("hello"))?;
    let ud2 = (&ud).into_lua(&lua)?;
    assert_eq!(&ud, ud2.as_userdata().unwrap());

    // Push into stack
    let table = lua.create_table()?;
    table.set("ud", &ud)?;
    assert_eq!(ud, table.get::<_, AnyUserData>("ud")?);
    assert_eq!("hello", *table.get::<_, UserDataRef<String>>("ud")?);

    Ok(())
}

#[cfg(all(feature = "unstable", not(feature = "send")))]
#[test]
fn test_owned_anyuserdata_into_lua() -> Result<()> {
    let lua = Lua::new();

    // Direct conversion
    let ud = lua.create_any_userdata(String::from("hello"))?.into_owned();
    let ud2 = (&ud).into_lua(&lua)?;
    assert_eq!(ud.to_ref(), *ud2.as_userdata().unwrap());

    // Push into stack
    let table = lua.create_table()?;
    table.set("ud", &ud)?;
    assert_eq!(ud.to_ref(), table.get::<_, AnyUserData>("ud")?);
    assert_eq!("hello", *table.get::<_, UserDataRef<String>>("ud")?);

    Ok(())
}

#[test]
fn test_registry_value_into_lua() -> Result<()> {
    let lua = Lua::new();

    let t = lua.create_table()?;
    let r = lua.create_registry_value(t)?;
    let f = lua.create_function(|_, t: Table| t.raw_set("hello", "world"))?;

    f.call(&r)?;
    let v = r.into_lua(&lua)?;
    let t = v.as_table().unwrap();
    assert_eq!(t.get::<_, String>("hello")?, "world");

    // Try to set nil registry key
    let r_nil = lua.create_registry_value(Value::Nil)?;
    t.set("hello", &r_nil)?;
    assert_eq!(t.get::<_, Value>("hello")?, Value::Nil);

    // Check non-owned registry key
    let lua2 = Lua::new();
    let r2 = lua2.create_registry_value("abc")?;
    assert!(matches!(
        f.call::<_, ()>(&r2),
        Err(Error::MismatchedRegistryKey)
    ));

    Ok(())
}

#[test]
fn test_registry_key_from_lua() -> Result<()> {
    let lua = Lua::new();

    let fkey = lua.load("function() return 1 end").eval::<RegistryKey>()?;
    let f = lua.registry_value::<Function>(&fkey)?;
    assert_eq!(f.call::<_, i32>(())?, 1);

    Ok(())
}

#[test]
fn test_conv_vec() -> Result<()> {
    let lua = Lua::new();

    let v = vec![1, 2, 3];
    lua.globals().set("v", v.clone())?;
    let v2: Vec<i32> = lua.globals().get("v")?;
    assert_eq!(v, v2);

    Ok(())
}

#[test]
fn test_conv_hashmap() -> Result<()> {
    let lua = Lua::new();

    let map = hashmap! {"hello".to_string() => "world".to_string()};
    lua.globals().set("map", map.clone())?;
    let map2: HashMap<String, String> = lua.globals().get("map")?;
    assert_eq!(map, map2);

    Ok(())
}

#[test]
fn test_conv_hashset() -> Result<()> {
    let lua = Lua::new();

    let set = hashset! {"hello".to_string(), "world".to_string()};
    lua.globals().set("set", set.clone())?;
    let set2: HashSet<String> = lua.globals().get("set")?;
    assert_eq!(set, set2);

    let set3 = lua.load(r#"{"a", "b", "c"}"#).eval::<HashSet<String>>()?;
    assert_eq!(set3, hashset! { "a".into(), "b".into(), "c".into() });

    Ok(())
}

#[test]
fn test_conv_btreemap() -> Result<()> {
    let lua = Lua::new();

    let map = btreemap! {"hello".to_string() => "world".to_string()};
    lua.globals().set("map", map.clone())?;
    let map2: BTreeMap<String, String> = lua.globals().get("map")?;
    assert_eq!(map, map2);

    Ok(())
}

#[test]
fn test_conv_btreeset() -> Result<()> {
    let lua = Lua::new();

    let set = btreeset! {"hello".to_string(), "world".to_string()};
    lua.globals().set("set", set.clone())?;
    let set2: BTreeSet<String> = lua.globals().get("set")?;
    assert_eq!(set, set2);

    let set3 = lua.load(r#"{"a", "b", "c"}"#).eval::<BTreeSet<String>>()?;
    assert_eq!(set3, btreeset! { "a".into(), "b".into(), "c".into() });

    Ok(())
}

#[test]
fn test_conv_cstring() -> Result<()> {
    let lua = Lua::new();

    let s = CString::new(b"hello".to_vec()).unwrap();
    lua.globals().set("s", s.clone())?;
    let s2: CString = lua.globals().get("s")?;
    assert_eq!(s, s2);

    let cs = CStr::from_bytes_with_nul(b"hello\0").unwrap();
    lua.globals().set("cs", cs)?;
    let cs2: CString = lua.globals().get("cs")?;
    assert_eq!(cs, cs2.as_c_str());

    Ok(())
}

#[test]
fn test_conv_cow() -> Result<()> {
    let lua = Lua::new();

    let s = Cow::from("hello");
    lua.globals().set("s", s.clone())?;
    let s2: String = lua.globals().get("s")?;
    assert_eq!(s, s2);

    Ok(())
}

#[test]
fn test_conv_boxed_str() -> Result<()> {
    let lua = Lua::new();

    let s = String::from("hello").into_boxed_str();
    lua.globals().set("s", s.clone())?;
    let s2: Box<str> = lua.globals().get("s")?;
    assert_eq!(s, s2);

    Ok(())
}

#[test]
fn test_conv_boxed_slice() -> Result<()> {
    let lua = Lua::new();

    let v = vec![1, 2, 3].into_boxed_slice();
    lua.globals().set("v", v.clone())?;
    let v2: Box<[i32]> = lua.globals().get("v")?;
    assert_eq!(v, v2);

    Ok(())
}

#[test]
fn test_conv_array() -> Result<()> {
    let lua = Lua::new();

    let v = [1, 2, 3];
    lua.globals().set("v", v)?;
    let v2: [i32; 3] = lua.globals().get("v")?;
    assert_eq!(v, v2);

    let v2 = lua.globals().get::<_, [i32; 4]>("v");
    assert!(matches!(v2, Err(Error::FromLuaConversionError { .. })));

    Ok(())
}
