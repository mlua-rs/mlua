//! (De)Serialization support using serde.

use std::os::raw::{c_int, c_void};
use std::ptr;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::ffi;
use crate::lua::Lua;
use crate::table::Table;
use crate::util::{assert_stack, protect_lua, StackGuard};
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
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 3);

            unsafe extern "C" fn push_null(state: *mut ffi::lua_State) -> c_int {
                ffi::lua_pushlightuserdata(state, ptr::null_mut());
                1
            }
            protect_lua(self.state, 0, push_null)?;
            Ok(self.pop_value())
        }
    }

    fn array_metatable(&'lua self) -> Result<Table<'lua>> {
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 3);

            unsafe extern "C" fn get_array_mt(state: *mut ffi::lua_State) -> c_int {
                push_array_metatable(state);
                1
            }
            protect_lua(self.state, 0, get_array_mt)?;
            Ok(Table(self.pop_ref()))
        }
    }

    fn to_value<T>(&'lua self, t: &T) -> Result<Value<'lua>>
    where
        T: Serialize + ?Sized,
    {
        t.serialize(ser::Serializer(self))
    }

    fn from_value<T>(&'lua self, value: Value<'lua>) -> Result<T>
    where
        T: Deserialize<'lua>,
    {
        T::deserialize(de::Deserializer(value))
    }
}

pub(crate) unsafe fn init_metatables(state: *mut ffi::lua_State) {
    ffi::lua_pushlightuserdata(
        state,
        &ARRAY_METATABLE_REGISTRY_KEY as *const u8 as *mut c_void,
    );
    ffi::lua_newtable(state);

    ffi::lua_pushstring(state, cstr!("__metatable"));
    ffi::lua_pushboolean(state, 0);
    ffi::lua_rawset(state, -3);

    ffi::lua_rawset(state, ffi::LUA_REGISTRYINDEX);
}

pub(crate) unsafe fn push_array_metatable(state: *mut ffi::lua_State) {
    let key = &ARRAY_METATABLE_REGISTRY_KEY as *const u8 as *mut c_void;
    ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, key);
}

static ARRAY_METATABLE_REGISTRY_KEY: u8 = 0;

pub mod de;
pub mod ser;
