//! (De)Serialization support using serde.

use std::os::raw::c_void;

use serde::de::DeserializeOwned;
use serde::ser::Serialize;

use crate::error::Result;
use crate::private::Sealed;
use crate::state::Lua;
use crate::table::Table;
use crate::util::check_stack;
use crate::value::Value;

/// Trait for serializing/deserializing Lua values using Serde.
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
pub trait LuaSerdeExt: Sealed {
    /// A special value (lightuserdata) to encode/decode optional (none) values.
    ///
    /// # Example
    ///
    /// ```
    /// use std::collections::HashMap;
    /// use mlua::{Lua, Result, LuaSerdeExt};
    ///
    /// fn main() -> Result<()> {
    ///     let lua = Lua::new();
    ///     lua.globals().set("null", lua.null())?;
    ///
    ///     let val = lua.load(r#"{a = null}"#).eval()?;
    ///     let map: HashMap<String, Option<String>> = lua.from_value(val)?;
    ///     assert_eq!(map["a"], None);
    ///
    ///     Ok(())
    /// }
    /// ```
    fn null(&self) -> Value;

    /// A metatable attachable to a Lua table to systematically encode it as Array (instead of Map).
    /// As result, encoded Array will contain only sequence part of the table, with the same length
    /// as the `#` operator on that table.
    ///
    /// # Example
    ///
    /// ```
    /// use mlua::{Lua, Result, LuaSerdeExt};
    /// use serde_json::Value as JsonValue;
    ///
    /// fn main() -> Result<()> {
    ///     let lua = Lua::new();
    ///     lua.globals().set("array_mt", lua.array_metatable())?;
    ///
    ///     // Encode as an empty array (no sequence part in the lua table)
    ///     let val = lua.load("setmetatable({a = 5}, array_mt)").eval()?;
    ///     let j: JsonValue = lua.from_value(val)?;
    ///     assert_eq!(j.to_string(), "[]");
    ///
    ///     // Encode as object
    ///     let val = lua.load("{a = 5}").eval()?;
    ///     let j: JsonValue = lua.from_value(val)?;
    ///     assert_eq!(j.to_string(), r#"{"a":5}"#);
    ///
    ///     Ok(())
    /// }
    /// ```
    fn array_metatable(&self) -> Table;

    /// Converts `T` into a [`Value`] instance.
    ///
    /// [`Value`]: crate::Value
    ///
    /// # Example
    ///
    /// ```
    /// use mlua::{Lua, Result, LuaSerdeExt};
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct User {
    ///     name: String,
    ///     age: u8,
    /// }
    ///
    /// fn main() -> Result<()> {
    ///     let lua = Lua::new();
    ///     let u = User {
    ///         name: "John Smith".into(),
    ///         age: 20,
    ///     };
    ///     lua.globals().set("user", lua.to_value(&u)?)?;
    ///     lua.load(r#"
    ///         assert(user["name"] == "John Smith")
    ///         assert(user["age"] == 20)
    ///     "#).exec()
    /// }
    /// ```
    fn to_value<T: Serialize + ?Sized>(&self, t: &T) -> Result<Value>;

    /// Converts `T` into a [`Value`] instance with options.
    ///
    /// # Example
    ///
    /// ```
    /// use mlua::{Lua, Result, LuaSerdeExt, SerializeOptions};
    ///
    /// fn main() -> Result<()> {
    ///     let lua = Lua::new();
    ///     let v = vec![1, 2, 3];
    ///     let options = SerializeOptions::new().set_array_metatable(false);
    ///     lua.globals().set("v", lua.to_value_with(&v, options)?)?;
    ///
    ///     lua.load(r#"
    ///         assert(#v == 3 and v[1] == 1 and v[2] == 2 and v[3] == 3)
    ///         assert(getmetatable(v) == nil)
    ///     "#).exec()
    /// }
    /// ```
    fn to_value_with<T>(&self, t: &T, options: ser::Options) -> Result<Value>
    where
        T: Serialize + ?Sized;

    /// Deserializes a [`Value`] into any serde deserializable object.
    ///
    /// # Example
    ///
    /// ```
    /// use mlua::{Lua, Result, LuaSerdeExt};
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize, Debug, PartialEq)]
    /// struct User {
    ///     name: String,
    ///     age: u8,
    /// }
    ///
    /// fn main() -> Result<()> {
    ///     let lua = Lua::new();
    ///     let val = lua.load(r#"{name = "John Smith", age = 20}"#).eval()?;
    ///     let u: User = lua.from_value(val)?;
    ///
    ///     assert_eq!(u, User { name: "John Smith".into(), age: 20 });
    ///
    ///     Ok(())
    /// }
    /// ```
    #[allow(clippy::wrong_self_convention)]
    fn from_value<T: DeserializeOwned>(&self, value: Value) -> Result<T>;

    /// Deserializes a [`Value`] into any serde deserializable object with options.
    ///
    /// # Example
    ///
    /// ```
    /// use mlua::{Lua, Result, LuaSerdeExt, DeserializeOptions};
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize, Debug, PartialEq)]
    /// struct User {
    ///     name: String,
    ///     age: u8,
    /// }
    ///
    /// fn main() -> Result<()> {
    ///     let lua = Lua::new();
    ///     let val = lua.load(r#"{name = "John Smith", age = 20, f = function() end}"#).eval()?;
    ///     let options = DeserializeOptions::new().deny_unsupported_types(false);
    ///     let u: User = lua.from_value_with(val, options)?;
    ///
    ///     assert_eq!(u, User { name: "John Smith".into(), age: 20 });
    ///
    ///     Ok(())
    /// }
    /// ```
    #[allow(clippy::wrong_self_convention)]
    fn from_value_with<T: DeserializeOwned>(&self, value: Value, options: de::Options) -> Result<T>;
}

impl LuaSerdeExt for Lua {
    fn null(&self) -> Value {
        Value::NULL
    }

    fn array_metatable(&self) -> Table {
        let lua = self.lock();
        unsafe {
            push_array_metatable(lua.ref_thread());
            Table(lua.pop_ref_thread())
        }
    }

    fn to_value<T>(&self, t: &T) -> Result<Value>
    where
        T: Serialize + ?Sized,
    {
        t.serialize(ser::Serializer::new(self))
    }

    fn to_value_with<T>(&self, t: &T, options: ser::Options) -> Result<Value>
    where
        T: Serialize + ?Sized,
    {
        t.serialize(ser::Serializer::new_with_options(self, options))
    }

    fn from_value<T>(&self, value: Value) -> Result<T>
    where
        T: DeserializeOwned,
    {
        T::deserialize(de::Deserializer::new(value))
    }

    fn from_value_with<T>(&self, value: Value, options: de::Options) -> Result<T>
    where
        T: DeserializeOwned,
    {
        T::deserialize(de::Deserializer::new_with_options(value, options))
    }
}

// Uses 2 stack spaces and calls checkstack.
pub(crate) unsafe fn init_metatables(state: *mut ffi::lua_State) -> Result<()> {
    check_stack(state, 2)?;
    protect_lua!(state, 0, 0, fn(state) {
        ffi::lua_createtable(state, 0, 1);

        ffi::lua_pushstring(state, cstr!("__metatable"));
        ffi::lua_pushboolean(state, 0);
        ffi::lua_rawset(state, -3);

        let array_metatable_key = &ARRAY_METATABLE_REGISTRY_KEY as *const u8 as *const c_void;
        ffi::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, array_metatable_key);
    })
}

pub(crate) unsafe fn push_array_metatable(state: *mut ffi::lua_State) {
    let array_metatable_key = &ARRAY_METATABLE_REGISTRY_KEY as *const u8 as *const c_void;
    ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, array_metatable_key);
}

static ARRAY_METATABLE_REGISTRY_KEY: u8 = 0;

pub mod de;
pub mod ser;

#[doc(inline)]
pub use de::Deserializer;
#[doc(inline)]
pub use ser::Serializer;
