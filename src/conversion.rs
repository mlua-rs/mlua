use std::prelude::v1::*;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::convert::TryInto;
use std::ffi::c_int;
use std::ffi::{CStr, CString};
use std::hash::{BuildHasher, Hash};
use std::string::String as StdString;
use std::{slice, str};

use bstr::{BStr, BString};
use num_traits::cast;

use crate::error::{Error, Result};
use crate::function::Function;
use crate::lua::Lua;
use crate::string::String;
use crate::table::Table;
use crate::thread::Thread;
use crate::types::{LightUserData, MaybeSend};
use crate::userdata::{AnyUserData, UserData, UserDataRef, UserDataRefMut};
use crate::value::{FromLua, IntoLua, Nil, Value};

#[cfg(all(feature = "unstable", any(not(feature = "send"), doc)))]
use crate::{function::OwnedFunction, table::OwnedTable, userdata::OwnedAnyUserData};

impl<'lua> IntoLua<'lua> for Value<'lua> {
    #[inline]
    fn into_lua(self, _: &'lua Lua) -> Result<Value<'lua>> {
        Ok(self)
    }
}

impl<'lua> FromLua<'lua> for Value<'lua> {
    #[inline]
    fn from_lua(lua_value: Value<'lua>, _: &'lua Lua) -> Result<Self> {
        Ok(lua_value)
    }
}

impl<'lua> IntoLua<'lua> for String<'lua> {
    #[inline]
    fn into_lua(self, _: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::String(self))
    }
}

impl<'lua> FromLua<'lua> for String<'lua> {
    #[inline]
    fn from_lua(value: Value<'lua>, lua: &'lua Lua) -> Result<String<'lua>> {
        let ty = value.type_name();
        lua.coerce_string(value)?
            .ok_or_else(|| Error::FromLuaConversionError {
                from: ty,
                to: "string",
                message: Some("expected string or number".to_string()),
            })
    }
}

impl<'lua> IntoLua<'lua> for Table<'lua> {
    #[inline]
    fn into_lua(self, _: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::Table(self))
    }
}

impl<'lua> FromLua<'lua> for Table<'lua> {
    #[inline]
    fn from_lua(value: Value<'lua>, _: &'lua Lua) -> Result<Table<'lua>> {
        match value {
            Value::Table(table) => Ok(table),
            _ => Err(Error::FromLuaConversionError {
                from: value.type_name(),
                to: "table",
                message: None,
            }),
        }
    }
}

#[cfg(all(feature = "unstable", any(not(feature = "send"), doc)))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "unstable", not(feature = "send")))))]
impl<'lua> IntoLua<'lua> for OwnedTable {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::Table(Table(lua.adopt_owned_ref(self.0))))
    }
}

#[cfg(all(feature = "unstable", any(not(feature = "send"), doc)))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "unstable", not(feature = "send")))))]
impl<'lua> FromLua<'lua> for OwnedTable {
    #[inline]
    fn from_lua(value: Value<'lua>, lua: &'lua Lua) -> Result<OwnedTable> {
        Table::from_lua(value, lua).map(|s| s.into_owned())
    }
}

impl<'lua> IntoLua<'lua> for Function<'lua> {
    #[inline]
    fn into_lua(self, _: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::Function(self))
    }
}

impl<'lua> FromLua<'lua> for Function<'lua> {
    #[inline]
    fn from_lua(value: Value<'lua>, _: &'lua Lua) -> Result<Function<'lua>> {
        match value {
            Value::Function(table) => Ok(table),
            _ => Err(Error::FromLuaConversionError {
                from: value.type_name(),
                to: "function",
                message: None,
            }),
        }
    }
}

#[cfg(all(feature = "unstable", any(not(feature = "send"), doc)))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "unstable", not(feature = "send")))))]
impl<'lua> IntoLua<'lua> for OwnedFunction {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::Function(Function(lua.adopt_owned_ref(self.0))))
    }
}

#[cfg(all(feature = "unstable", any(not(feature = "send"), doc)))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "unstable", not(feature = "send")))))]
impl<'lua> FromLua<'lua> for OwnedFunction {
    #[inline]
    fn from_lua(value: Value<'lua>, lua: &'lua Lua) -> Result<OwnedFunction> {
        Function::from_lua(value, lua).map(|s| s.into_owned())
    }
}

impl<'lua> IntoLua<'lua> for Thread<'lua> {
    #[inline]
    fn into_lua(self, _: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::Thread(self))
    }
}

impl<'lua> FromLua<'lua> for Thread<'lua> {
    #[inline]
    fn from_lua(value: Value<'lua>, _: &'lua Lua) -> Result<Thread<'lua>> {
        match value {
            Value::Thread(t) => Ok(t),
            _ => Err(Error::FromLuaConversionError {
                from: value.type_name(),
                to: "thread",
                message: None,
            }),
        }
    }
}

impl<'lua> IntoLua<'lua> for AnyUserData<'lua> {
    #[inline]
    fn into_lua(self, _: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::UserData(self))
    }
}

impl<'lua> FromLua<'lua> for AnyUserData<'lua> {
    #[inline]
    fn from_lua(value: Value<'lua>, _: &'lua Lua) -> Result<AnyUserData<'lua>> {
        match value {
            Value::UserData(ud) => Ok(ud),
            _ => Err(Error::FromLuaConversionError {
                from: value.type_name(),
                to: "userdata",
                message: None,
            }),
        }
    }
}

#[cfg(all(feature = "unstable", any(not(feature = "send"), doc)))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "unstable", not(feature = "send")))))]
impl<'lua> IntoLua<'lua> for OwnedAnyUserData {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::UserData(AnyUserData(lua.adopt_owned_ref(self.0))))
    }
}

#[cfg(all(feature = "unstable", any(not(feature = "send"), doc)))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "unstable", not(feature = "send")))))]
impl<'lua> FromLua<'lua> for OwnedAnyUserData {
    #[inline]
    fn from_lua(value: Value<'lua>, lua: &'lua Lua) -> Result<OwnedAnyUserData> {
        AnyUserData::from_lua(value, lua).map(|s| s.into_owned())
    }
}

impl<'lua, T: UserData + MaybeSend + 'static> IntoLua<'lua> for T {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::UserData(lua.create_userdata(self)?))
    }
}

impl<'lua, T: 'static> FromLua<'lua> for UserDataRef<'lua, T> {
    #[inline]
    fn from_lua(value: Value<'lua>, _: &'lua Lua) -> Result<Self> {
        Self::from_value(value)
    }
}

impl<'lua, T: 'static> FromLua<'lua> for UserDataRefMut<'lua, T> {
    #[inline]
    fn from_lua(value: Value<'lua>, _: &'lua Lua) -> Result<Self> {
        Self::from_value(value)
    }
}

impl<'lua> IntoLua<'lua> for Error {
    #[inline]
    fn into_lua(self, _: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::Error(self))
    }
}

impl<'lua> FromLua<'lua> for Error {
    #[inline]
    fn from_lua(value: Value<'lua>, lua: &'lua Lua) -> Result<Error> {
        match value {
            Value::Error(err) => Ok(err),
            val => Ok(Error::runtime(
                lua.coerce_string(val)?
                    .and_then(|s| Some(s.to_str().ok()?.to_owned()))
                    .unwrap_or_else(|| "<unprintable error>".to_owned()),
            )),
        }
    }
}

impl<'lua> IntoLua<'lua> for bool {
    #[inline]
    fn into_lua(self, _: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::Boolean(self))
    }
}

impl<'lua> FromLua<'lua> for bool {
    #[inline]
    fn from_lua(v: Value<'lua>, _: &'lua Lua) -> Result<Self> {
        match v {
            Value::Nil => Ok(false),
            Value::Boolean(b) => Ok(b),
            _ => Ok(true),
        }
    }
}

impl<'lua> IntoLua<'lua> for LightUserData {
    #[inline]
    fn into_lua(self, _: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::LightUserData(self))
    }
}

impl<'lua> FromLua<'lua> for LightUserData {
    #[inline]
    fn from_lua(value: Value<'lua>, _: &'lua Lua) -> Result<Self> {
        match value {
            Value::LightUserData(ud) => Ok(ud),
            _ => Err(Error::FromLuaConversionError {
                from: value.type_name(),
                to: "light userdata",
                message: None,
            }),
        }
    }
}

#[cfg(feature = "luau")]
impl<'lua> IntoLua<'lua> for crate::types::Vector {
    #[inline]
    fn into_lua(self, _: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::Vector(self))
    }
}

#[cfg(feature = "luau")]
impl<'lua> FromLua<'lua> for crate::types::Vector {
    #[inline]
    fn from_lua(value: Value<'lua>, _: &'lua Lua) -> Result<Self> {
        match value {
            Value::Vector(v) => Ok(v),
            _ => Err(Error::FromLuaConversionError {
                from: value.type_name(),
                to: "vector",
                message: None,
            }),
        }
    }
}

impl<'lua> IntoLua<'lua> for StdString {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::String(lua.create_string(&self)?))
    }

    #[inline]
    unsafe fn push_into_stack(self, lua: &'lua Lua) -> Result<()> {
        push_bytes_into_stack(self, lua)
    }
}

impl<'lua> FromLua<'lua> for StdString {
    #[inline]
    fn from_lua(value: Value<'lua>, lua: &'lua Lua) -> Result<Self> {
        let ty = value.type_name();
        Ok(lua
            .coerce_string(value)?
            .ok_or_else(|| Error::FromLuaConversionError {
                from: ty,
                to: "String",
                message: Some("expected string or number".to_string()),
            })?
            .to_str()?
            .to_owned())
    }

    #[inline]
    unsafe fn from_stack(idx: c_int, lua: &'lua Lua) -> Result<Self> {
        let state = lua.state();
        if ffi::lua_type(state, idx) == ffi::LUA_TSTRING {
            let mut size = 0;
            let data = ffi::lua_tolstring(state, idx, &mut size);
            let bytes = slice::from_raw_parts(data as *const u8, size);
            return str::from_utf8(bytes).map(|s| s.to_owned()).map_err(|e| {
                Error::FromLuaConversionError {
                    from: "string",
                    to: "String",
                    message: Some(e.to_string()),
                }
            });
        }
        // Fallback to default
        Self::from_lua(lua.stack_value(idx), lua)
    }
}

impl<'lua> IntoLua<'lua> for &str {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::String(lua.create_string(self)?))
    }

    #[inline]
    unsafe fn push_into_stack(self, lua: &'lua Lua) -> Result<()> {
        push_bytes_into_stack(self, lua)
    }
}

impl<'lua> IntoLua<'lua> for Cow<'_, str> {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::String(lua.create_string(self.as_bytes())?))
    }
}

impl<'lua> IntoLua<'lua> for Box<str> {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::String(lua.create_string(&*self)?))
    }
}

impl<'lua> FromLua<'lua> for Box<str> {
    #[inline]
    fn from_lua(value: Value<'lua>, lua: &'lua Lua) -> Result<Self> {
        let ty = value.type_name();
        Ok(lua
            .coerce_string(value)?
            .ok_or_else(|| Error::FromLuaConversionError {
                from: ty,
                to: "Box<str>",
                message: Some("expected string or number".to_string()),
            })?
            .to_str()?
            .to_owned()
            .into_boxed_str())
    }
}

impl<'lua> IntoLua<'lua> for CString {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::String(lua.create_string(self.as_bytes())?))
    }
}

impl<'lua> FromLua<'lua> for CString {
    #[inline]
    fn from_lua(value: Value<'lua>, lua: &'lua Lua) -> Result<Self> {
        let ty = value.type_name();
        let string = lua
            .coerce_string(value)?
            .ok_or_else(|| Error::FromLuaConversionError {
                from: ty,
                to: "CString",
                message: Some("expected string or number".to_string()),
            })?;

        match CStr::from_bytes_with_nul(string.as_bytes_with_nul()) {
            Ok(s) => Ok(s.into()),
            Err(_) => Err(Error::FromLuaConversionError {
                from: ty,
                to: "CString",
                message: Some("invalid C-style string".to_string()),
            }),
        }
    }
}

impl<'lua> IntoLua<'lua> for &CStr {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::String(lua.create_string(self.to_bytes())?))
    }
}

impl<'lua> IntoLua<'lua> for Cow<'_, CStr> {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::String(lua.create_string(self.to_bytes())?))
    }
}

impl<'lua> IntoLua<'lua> for BString {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::String(lua.create_string(&self)?))
    }
}

impl<'lua> FromLua<'lua> for BString {
    #[inline]
    fn from_lua(value: Value<'lua>, lua: &'lua Lua) -> Result<Self> {
        let ty = value.type_name();
        Ok(BString::from(
            lua.coerce_string(value)?
                .ok_or_else(|| Error::FromLuaConversionError {
                    from: ty,
                    to: "BString",
                    message: Some("expected string or number".to_string()),
                })?
                .as_bytes()
                .to_vec(),
        ))
    }
}

impl<'lua> IntoLua<'lua> for &BStr {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::String(lua.create_string(self)?))
    }
}

#[inline]
unsafe fn push_bytes_into_stack<'lua, T>(this: T, lua: &'lua Lua) -> Result<()>
where
    T: IntoLua<'lua> + AsRef<[u8]>,
{
    let bytes = this.as_ref();
    if lua.unlikely_memory_error() && bytes.len() < (1 << 30) {
        // Fast path: push directly into the Lua stack.
        ffi::lua_pushlstring(lua.state(), bytes.as_ptr() as *const _, bytes.len());
        return Ok(());
    }
    // Fallback to default
    lua.push_value(T::into_lua(this, lua)?)
}

macro_rules! lua_convert_int {
    ($x:ty) => {
        impl<'lua> IntoLua<'lua> for $x {
            #[inline]
            fn into_lua(self, _: &'lua Lua) -> Result<Value<'lua>> {
                cast(self)
                    .map(Value::Integer)
                    .or_else(|| cast(self).map(Value::Number))
                    // This is impossible error because conversion to Number never fails
                    .ok_or_else(|| Error::ToLuaConversionError {
                        from: stringify!($x),
                        to: "number",
                        message: Some("out of range".to_owned()),
                    })
            }
        }

        impl<'lua> FromLua<'lua> for $x {
            #[inline]
            fn from_lua(value: Value<'lua>, lua: &'lua Lua) -> Result<Self> {
                let ty = value.type_name();
                (match value {
                    Value::Integer(i) => cast(i),
                    Value::Number(n) => cast(n),
                    _ => {
                        if let Some(i) = lua.coerce_integer(value.clone())? {
                            cast(i)
                        } else {
                            cast(lua.coerce_number(value)?.ok_or_else(|| {
                                Error::FromLuaConversionError {
                                    from: ty,
                                    to: stringify!($x),
                                    message: Some(
                                        "expected number or string coercible to number".to_string(),
                                    ),
                                }
                            })?)
                        }
                    }
                })
                .ok_or_else(|| Error::FromLuaConversionError {
                    from: ty,
                    to: stringify!($x),
                    message: Some("out of range".to_owned()),
                })
            }
        }
    };
}

lua_convert_int!(i8);
lua_convert_int!(u8);
lua_convert_int!(i16);
lua_convert_int!(u16);
lua_convert_int!(i32);
lua_convert_int!(u32);
lua_convert_int!(i64);
lua_convert_int!(u64);
lua_convert_int!(i128);
lua_convert_int!(u128);
lua_convert_int!(isize);
lua_convert_int!(usize);

macro_rules! lua_convert_float {
    ($x:ty) => {
        impl<'lua> IntoLua<'lua> for $x {
            #[inline]
            fn into_lua(self, _: &'lua Lua) -> Result<Value<'lua>> {
                cast(self)
                    .ok_or_else(|| Error::ToLuaConversionError {
                        from: stringify!($x),
                        to: "number",
                        message: Some("out of range".to_string()),
                    })
                    .map(Value::Number)
            }
        }

        impl<'lua> FromLua<'lua> for $x {
            #[inline]
            fn from_lua(value: Value<'lua>, lua: &'lua Lua) -> Result<Self> {
                let ty = value.type_name();
                lua.coerce_number(value)?
                    .ok_or_else(|| Error::FromLuaConversionError {
                        from: ty,
                        to: stringify!($x),
                        message: Some("expected number or string coercible to number".to_string()),
                    })
                    .and_then(|n| {
                        cast(n).ok_or_else(|| Error::FromLuaConversionError {
                            from: ty,
                            to: stringify!($x),
                            message: Some("number out of range".to_string()),
                        })
                    })
            }
        }
    };
}

lua_convert_float!(f32);
lua_convert_float!(f64);

impl<'lua, T> IntoLua<'lua> for &[T]
where
    T: IntoLua<'lua> + Clone,
{
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::Table(
            lua.create_sequence_from(self.iter().cloned())?,
        ))
    }
}

impl<'lua, T, const N: usize> IntoLua<'lua> for [T; N]
where
    T: IntoLua<'lua>,
{
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::Table(lua.create_sequence_from(self)?))
    }
}

impl<'lua, T, const N: usize> FromLua<'lua> for [T; N]
where
    T: FromLua<'lua>,
{
    #[inline]
    fn from_lua(value: Value<'lua>, _lua: &'lua Lua) -> Result<Self> {
        match value {
            #[cfg(feature = "luau")]
            #[rustfmt::skip]
            Value::Vector(v) if N == crate::types::Vector::SIZE => unsafe {
                use std::{mem, ptr};
                let mut arr: [mem::MaybeUninit<T>; N] = mem::MaybeUninit::uninit().assume_init();
                ptr::write(arr[0].as_mut_ptr() , T::from_lua(Value::Number(v.x() as _), _lua)?);
                ptr::write(arr[1].as_mut_ptr(), T::from_lua(Value::Number(v.y() as _), _lua)?);
                ptr::write(arr[2].as_mut_ptr(), T::from_lua(Value::Number(v.z() as _), _lua)?);
                #[cfg(feature = "luau-vector4")]
                ptr::write(arr[3].as_mut_ptr(), T::from_lua(Value::Number(v.w() as _), _lua)?);
                Ok(mem::transmute_copy(&arr))
            },
            Value::Table(table) => {
                let vec = table.sequence_values().collect::<Result<Vec<_>>>()?;
                vec.try_into()
                    .map_err(|vec: Vec<T>| Error::FromLuaConversionError {
                        from: "table",
                        to: "Array",
                        message: Some(format!("expected table of length {}, got {}", N, vec.len())),
                    })
            }
            _ => Err(Error::FromLuaConversionError {
                from: value.type_name(),
                to: "Array",
                message: Some("expected table".to_string()),
            }),
        }
    }
}

impl<'lua, T: IntoLua<'lua>> IntoLua<'lua> for Box<[T]> {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::Table(lua.create_sequence_from(self.into_vec())?))
    }
}

impl<'lua, T: FromLua<'lua>> FromLua<'lua> for Box<[T]> {
    #[inline]
    fn from_lua(value: Value<'lua>, lua: &'lua Lua) -> Result<Self> {
        Ok(Vec::<T>::from_lua(value, lua)?.into_boxed_slice())
    }
}

impl<'lua, T: IntoLua<'lua>> IntoLua<'lua> for Vec<T> {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::Table(lua.create_sequence_from(self)?))
    }
}

impl<'lua, T: FromLua<'lua>> FromLua<'lua> for Vec<T> {
    #[inline]
    fn from_lua(value: Value<'lua>, _lua: &'lua Lua) -> Result<Self> {
        match value {
            Value::Table(table) => table.sequence_values().collect(),
            _ => Err(Error::FromLuaConversionError {
                from: value.type_name(),
                to: "Vec",
                message: Some("expected table".to_string()),
            }),
        }
    }
}

impl<'lua, K: Eq + Hash + IntoLua<'lua>, V: IntoLua<'lua>, S: BuildHasher> IntoLua<'lua>
    for HashMap<K, V, S>
{
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::Table(lua.create_table_from(self)?))
    }
}

impl<'lua, K: Eq + Hash + FromLua<'lua>, V: FromLua<'lua>, S: BuildHasher + Default> FromLua<'lua>
    for HashMap<K, V, S>
{
    #[inline]
    fn from_lua(value: Value<'lua>, _: &'lua Lua) -> Result<Self> {
        if let Value::Table(table) = value {
            table.pairs().collect()
        } else {
            Err(Error::FromLuaConversionError {
                from: value.type_name(),
                to: "HashMap",
                message: Some("expected table".to_string()),
            })
        }
    }
}

impl<'lua, K: Ord + IntoLua<'lua>, V: IntoLua<'lua>> IntoLua<'lua> for BTreeMap<K, V> {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::Table(lua.create_table_from(self)?))
    }
}

impl<'lua, K: Ord + FromLua<'lua>, V: FromLua<'lua>> FromLua<'lua> for BTreeMap<K, V> {
    #[inline]
    fn from_lua(value: Value<'lua>, _: &'lua Lua) -> Result<Self> {
        if let Value::Table(table) = value {
            table.pairs().collect()
        } else {
            Err(Error::FromLuaConversionError {
                from: value.type_name(),
                to: "BTreeMap",
                message: Some("expected table".to_string()),
            })
        }
    }
}

impl<'lua, T: Eq + Hash + IntoLua<'lua>, S: BuildHasher> IntoLua<'lua> for HashSet<T, S> {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::Table(lua.create_table_from(
            self.into_iter().map(|val| (val, true)),
        )?))
    }
}

impl<'lua, T: Eq + Hash + FromLua<'lua>, S: BuildHasher + Default> FromLua<'lua> for HashSet<T, S> {
    #[inline]
    fn from_lua(value: Value<'lua>, _: &'lua Lua) -> Result<Self> {
        match value {
            Value::Table(table) if table.raw_len() > 0 => table.sequence_values().collect(),
            Value::Table(table) => table
                .pairs::<T, Value<'lua>>()
                .map(|res| res.map(|(k, _)| k))
                .collect(),
            _ => Err(Error::FromLuaConversionError {
                from: value.type_name(),
                to: "HashSet",
                message: Some("expected table".to_string()),
            }),
        }
    }
}

impl<'lua, T: Ord + IntoLua<'lua>> IntoLua<'lua> for BTreeSet<T> {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        Ok(Value::Table(lua.create_table_from(
            self.into_iter().map(|val| (val, true)),
        )?))
    }
}

impl<'lua, T: Ord + FromLua<'lua>> FromLua<'lua> for BTreeSet<T> {
    #[inline]
    fn from_lua(value: Value<'lua>, _: &'lua Lua) -> Result<Self> {
        match value {
            Value::Table(table) if table.raw_len() > 0 => table.sequence_values().collect(),
            Value::Table(table) => table
                .pairs::<T, Value<'lua>>()
                .map(|res| res.map(|(k, _)| k))
                .collect(),
            _ => Err(Error::FromLuaConversionError {
                from: value.type_name(),
                to: "BTreeSet",
                message: Some("expected table".to_string()),
            }),
        }
    }
}

impl<'lua, T: IntoLua<'lua>> IntoLua<'lua> for Option<T> {
    #[inline]
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>> {
        match self {
            Some(val) => val.into_lua(lua),
            None => Ok(Nil),
        }
    }
}

impl<'lua, T: FromLua<'lua>> FromLua<'lua> for Option<T> {
    #[inline]
    fn from_lua(value: Value<'lua>, lua: &'lua Lua) -> Result<Self> {
        match value {
            Nil => Ok(None),
            value => Ok(Some(T::from_lua(value, lua)?)),
        }
    }
}
