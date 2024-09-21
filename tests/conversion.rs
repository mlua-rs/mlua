use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::{CStr, CString, OsStr, OsString};
use std::path::{Path, PathBuf};

use bstr::BString;
use maplit::{btreemap, btreeset, hashmap, hashset};
use mlua::{
    AnyUserData, Error, Function, IntoLua, Lua, RegistryKey, Result, Table, Thread, UserDataRef, Value,
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
    assert_eq!(v, table.get::<Value>("v")?);

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
    assert_eq!(s, table.get::<String>("s")?);

    Ok(())
}

#[test]
fn test_string_from_lua() -> Result<()> {
    let lua = Lua::new();

    // From stack
    let f = lua.create_function(|_, s: mlua::String| Ok(s))?;
    let s = f.call::<String>("hello, world!")?;
    assert_eq!(s, "hello, world!");

    // Should fallback to default conversion
    let s = f.call::<String>(42)?;
    assert_eq!(s, "42");

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
    f.call::<()>((&t, "hello"))?;
    assert_eq!("hello", t.get::<String>("s")?);

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
    assert_eq!(f, table.get::<Function>("f")?);

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
    assert_eq!(th, table.get::<Thread>("th")?);

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
    assert_eq!(ud, table.get::<AnyUserData>("ud")?);
    assert_eq!("hello", *table.get::<UserDataRef<String>>("ud")?);

    Ok(())
}

#[test]
fn test_registry_value_into_lua() -> Result<()> {
    let lua = Lua::new();

    // Direct conversion
    let s = lua.create_string("hello, world")?;
    let r = lua.create_registry_value(&s)?;
    let value1 = lua.pack(&r)?;
    let value2 = lua.pack(r)?;
    assert_eq!(value1.as_str().as_deref(), Some("hello, world"));
    assert_eq!(value2.to_pointer(), value2.to_pointer());

    // Push into stack
    let t = lua.create_table()?;
    let r = lua.create_registry_value(&t)?;
    let f = lua.create_function(|_, (t, k, v): (Table, Value, Value)| t.set(k, v))?;
    f.call::<()>((&r, "hello", "world"))?;
    f.call::<()>((r, "welcome", "to the jungle"))?;
    assert_eq!(t.get::<String>("hello")?, "world");
    assert_eq!(t.get::<String>("welcome")?, "to the jungle");

    // Try to set nil registry key
    let r_nil = lua.create_registry_value(Value::Nil)?;
    t.set("hello", &r_nil)?;
    assert_eq!(t.get::<Value>("hello")?, Value::Nil);

    // Check non-owned registry key
    let lua2 = Lua::new();
    let r2 = lua2.create_registry_value("abc")?;
    assert!(matches!(f.call::<()>(&r2), Err(Error::MismatchedRegistryKey)));

    Ok(())
}

#[test]
fn test_registry_key_from_lua() -> Result<()> {
    let lua = Lua::new();

    let fkey = lua.load("function() return 1 end").eval::<RegistryKey>()?;
    let f = lua.registry_value::<Function>(&fkey)?;
    assert_eq!(f.call::<i32>(())?, 1);

    Ok(())
}

#[test]
fn test_integer_from_lua() -> Result<()> {
    let lua = Lua::new();

    // From stack
    let f = lua.create_function(|_, i: i32| Ok(i))?;
    assert_eq!(f.call::<i32>(42)?, 42);

    // Out of range
    match f.call::<i32>(i64::MAX).err() {
        Some(Error::CallbackError { cause, .. }) => match cause.as_ref() {
            Error::BadArgument { cause, .. } => match cause.as_ref() {
                Error::FromLuaConversionError { message, .. } => {
                    assert_eq!(message.as_ref().unwrap(), "out of range");
                }
                err => panic!("expected Error::FromLuaConversionError, got {err:?}"),
            },
            err => panic!("expected Error::BadArgument, got {err:?}"),
        },
        err => panic!("expected Error::CallbackError, got {err:?}"),
    }

    // Should fallback to default conversion
    assert_eq!(f.call::<i32>("42")?, 42);

    Ok(())
}

#[test]
fn test_float_from_lua() -> Result<()> {
    let lua = Lua::new();

    // From stack
    let f = lua.create_function(|_, f: f32| Ok(f))?;
    assert_eq!(f.call::<f32>(42.0)?, 42.0);

    // Out of range (but never fails)
    let val = f.call::<f32>(f64::MAX)?;
    assert!(val.is_infinite());

    // Should fallback to default conversion
    assert_eq!(f.call::<f32>("42.0")?, 42.0);

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

    let v2 = lua.globals().get::<[i32; 4]>("v");
    assert!(matches!(v2, Err(Error::FromLuaConversionError { .. })));

    Ok(())
}

#[test]
fn test_bstring_from_lua() -> Result<()> {
    let lua = Lua::new();

    let s = lua.create_string("hello, world")?;
    let bstr = lua.unpack::<BString>(Value::String(s))?;
    assert_eq!(bstr, "hello, world");

    let bstr = lua.unpack::<BString>(Value::Integer(123))?;
    assert_eq!(bstr, "123");

    let bstr = lua.unpack::<BString>(Value::Number(-123.55))?;
    assert_eq!(bstr, "-123.55");

    // Test from stack
    let f = lua.create_function(|_, bstr: BString| Ok(bstr))?;
    let bstr = f.call::<BString>("hello, world")?;
    assert_eq!(bstr, "hello, world");

    let bstr = f.call::<BString>(-43.22)?;
    assert_eq!(bstr, "-43.22");

    Ok(())
}

#[cfg(feature = "luau")]
#[test]
fn test_bstring_from_lua_buffer() -> Result<()> {
    let lua = Lua::new();

    let b = lua.create_buffer("hello, world")?;
    let bstr = lua.unpack::<BString>(Value::UserData(b))?;
    assert_eq!(bstr, "hello, world");

    // Test from stack
    let f = lua.create_function(|_, bstr: BString| Ok(bstr))?;
    let buf = lua.create_buffer("hello, world")?;
    let bstr = f.call::<BString>(buf)?;
    assert_eq!(bstr, "hello, world");

    Ok(())
}

#[test]
fn test_osstring_into_from_lua() -> Result<()> {
    let lua = Lua::new();

    let s = OsString::from("hello, world");

    let v = lua.pack(s.as_os_str())?;
    assert!(v.is_string());
    assert_eq!(v.as_str().unwrap(), "hello, world");

    let v = lua.pack(s)?;
    assert!(v.is_string());
    assert_eq!(v.as_str().unwrap(), "hello, world");

    let s = lua.create_string("hello, world")?;
    let bstr = lua.unpack::<OsString>(Value::String(s))?;
    assert_eq!(bstr, "hello, world");

    let bstr = lua.unpack::<OsString>(Value::Integer(123))?;
    assert_eq!(bstr, "123");

    let bstr = lua.unpack::<OsString>(Value::Number(-123.55))?;
    assert_eq!(bstr, "-123.55");

    Ok(())
}

#[test]
fn test_pathbuf_into_from_lua() -> Result<()> {
    let lua = Lua::new();

    let pb = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let pb_str = pb.to_str().unwrap();

    let v = lua.pack(pb.as_path())?;
    assert!(v.is_string());
    assert_eq!(v.as_str().unwrap(), pb_str);

    let v = lua.pack(pb.clone())?;
    assert!(v.is_string());
    assert_eq!(v.as_str().unwrap(), pb_str);

    let s = lua.create_string(pb_str)?;
    let bstr = lua.unpack::<PathBuf>(Value::String(s))?;
    assert_eq!(bstr, pb);

    Ok(())
}

#[test]
fn test_option_into_from_lua() -> Result<()> {
    let lua = Lua::new();

    // Direct conversion
    let v = Some(42);
    let v2 = v.into_lua(&lua)?;
    assert_eq!(v, v2.as_i32());

    // Push into stack / get from stack
    let f = lua.create_function(|_, v: Option<i32>| Ok(v))?;
    assert_eq!(f.call::<Option<i32>>(Some(42))?, Some(42));
    assert_eq!(f.call::<Option<i32>>(Option::<i32>::None)?, None);
    assert_eq!(f.call::<Option<i32>>(())?, None);

    Ok(())
}
