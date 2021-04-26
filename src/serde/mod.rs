//! (De)Serialization support using serde.

use std::os::raw::c_void;
use std::ptr;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::ffi;
use crate::lua::Lua;
use crate::table::Table;
use crate::types::LightUserData;
use crate::util::{assert_stack, StackGuard};
use crate::value::Value;

pub trait LuaSerdeExt<'lua> {
    /// A special value (lightuserdata) to encode/decode optional (none) values.
    ///
    /// Requires `feature = "serialize"`
    ///
    /// # Example
    ///
    /// ```
    /// use std::collections::HashMap;
    /// use mlua::{Lua, Result, LuaSerdeExt};
    ///
    /// fn main() -> Result<()> {
    ///     let lua = Lua::new();
    ///     lua.globals().set("null", lua.null()?)?;
    ///
    ///     let val = lua.load(r#"{a = null}"#).eval()?;
    ///     let map: HashMap<String, Option<String>> = lua.from_value(val)?;
    ///     assert_eq!(map["a"], None);
    ///
    ///     Ok(())
    /// }
    /// ```
    fn null(&'lua self) -> Result<Value<'lua>>;

    /// A metatable attachable to a Lua table to systematically encode it as Array (instead of Map).
    /// As result, encoded Array will contain only sequence part of the table, with the same length
    /// as the `#` operator on that table.
    ///
    /// Requires `feature = "serialize"`
    ///
    /// # Example
    ///
    /// ```
    /// use mlua::{Lua, Result, LuaSerdeExt};
    /// use serde_json::Value as JsonValue;
    ///
    /// fn main() -> Result<()> {
    ///     let lua = Lua::new();
    ///     lua.globals().set("array_mt", lua.array_metatable()?)?;
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
    fn array_metatable(&'lua self) -> Result<Table<'lua>>;

    /// Converts `T` into a `Value` instance.
    ///
    /// Requires `feature = "serialize"`
    ///
    /// [`Value`]: enum.Value.html
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
    fn to_value<T: Serialize + ?Sized>(&'lua self, t: &T) -> Result<Value<'lua>>;

    /// Converts `T` into a `Value` instance with options.
    ///
    /// Requires `feature = "serialize"`
    ///
    /// [`Value`]: enum.Value.html
    ///
    /// # Example
    ///
    /// ```
    /// use mlua::{Lua, Result, LuaSerdeExt, SerializeOptions};
    ///
    /// fn main() -> Result<()> {
    ///     let lua = Lua::new();
    ///     let v = vec![1, 2, 3];
    ///     lua.globals().set("v", lua.to_value_with(&v, SerializeOptions {
    ///         set_array_metatable: false,
    ///         ..SerializeOptions::default()
    ///     })?)?;
    ///
    ///     lua.load(r#"
    ///         assert(#v == 3 and v[1] == 1 and v[2] == 2 and v[3] == 3)
    ///         assert(getmetatable(v) == nil)
    ///     "#).exec()
    /// }
    /// ```
    fn to_value_with<T>(&'lua self, t: &T, options: ser::Options) -> Result<Value<'lua>>
    where
        T: Serialize + ?Sized;

    /// Deserializes a `Value` into any serde deserializable object.
    ///
    /// Requires `feature = "serialize"`
    ///
    /// [`Value`]: enum.Value.html
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
    fn from_value<T: Deserialize<'lua>>(&'lua self, value: Value<'lua>) -> Result<T>;
}

impl<'lua> LuaSerdeExt<'lua> for Lua {
    fn null(&'lua self) -> Result<Value<'lua>> {
        // TODO: Remove Result?
        Ok(Value::LightUserData(LightUserData(ptr::null_mut())))
    }

    fn array_metatable(&'lua self) -> Result<Table<'lua>> {
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 1);

            push_array_metatable(self.state);

            Ok(Table(self.pop_ref()))
        }
    }

    fn to_value<T>(&'lua self, t: &T) -> Result<Value<'lua>>
    where
        T: Serialize + ?Sized,
    {
        t.serialize(ser::Serializer::new(self))
    }

    fn to_value_with<T>(&'lua self, t: &T, options: ser::Options) -> Result<Value<'lua>>
    where
        T: Serialize + ?Sized,
    {
        t.serialize(ser::Serializer::new_with_options(self, options))
    }

    fn from_value<T>(&'lua self, value: Value<'lua>) -> Result<T>
    where
        T: Deserialize<'lua>,
    {
        T::deserialize(de::Deserializer(value))
    }
}

pub(crate) unsafe fn init_metatables(state: *mut ffi::lua_State) -> Result<()> {
    ffi::lua_pushlightuserdata(
        state,
        &ARRAY_METATABLE_REGISTRY_KEY as *const u8 as *mut c_void,
    );
    ffi::safe::lua_createtable(state, 0, 1)?;

    ffi::lua_pushboolean(state, 0);
    ffi::safe::lua_rawsetfield(state, -2, "__metatable")?;

    ffi::safe::lua_rawset(state, ffi::LUA_REGISTRYINDEX)
}

pub(crate) unsafe fn push_array_metatable(state: *mut ffi::lua_State) {
    let key = &ARRAY_METATABLE_REGISTRY_KEY as *const u8 as *mut c_void;
    ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, key);
}

static ARRAY_METATABLE_REGISTRY_KEY: u8 = 0;

pub mod de;
pub mod ser;
