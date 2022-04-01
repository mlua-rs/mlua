use std::os::raw::c_int;

use serde::{ser, Serialize};

use super::LuaSerdeExt;
use crate::error::{Error, Result};
use crate::ffi;
use crate::lua::Lua;
use crate::string::String;
use crate::table::Table;
use crate::types::Integer;
use crate::util::{check_stack, StackGuard};
use crate::value::{ToLua, Value};

/// A struct for serializing Rust values into Lua values.
#[derive(Debug)]
pub struct Serializer<'lua> {
    lua: &'lua Lua,
    options: Options,
}

/// A struct with options to change default serializer behavior.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Options {
    /// If true, sequence serialization to a Lua table will create table
    /// with the [`array_metatable`] attached.
    ///
    /// Default: **true**
    ///
    /// [`array_metatable`]: crate::LuaSerdeExt::array_metatable
    pub set_array_metatable: bool,

    /// If true, serialize `None` (part of the `Option` type) to [`null`].
    /// Otherwise it will be set to Lua [`Nil`].
    ///
    /// Default: **true**
    ///
    /// [`null`]: crate::LuaSerdeExt::null
    /// [`Nil`]: crate::Value::Nil
    pub serialize_none_to_null: bool,

    /// If true, serialize `Unit` (type of `()` in Rust) and Unit structs to [`null`].
    /// Otherwise it will be set to Lua [`Nil`].
    ///
    /// Default: **true**
    ///
    /// [`null`]: crate::LuaSerdeExt::null
    /// [`Nil`]: crate::Value::Nil
    pub serialize_unit_to_null: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self::new()
    }
}

impl Options {
    /// Returns a new instance of [`Options`] with default parameters.
    pub const fn new() -> Self {
        Options {
            set_array_metatable: true,
            serialize_none_to_null: true,
            serialize_unit_to_null: true,
        }
    }

    /// Sets [`set_array_metatable`] option.
    ///
    /// [`set_array_metatable`]: #structfield.set_array_metatable
    #[must_use]
    pub const fn set_array_metatable(mut self, enabled: bool) -> Self {
        self.set_array_metatable = enabled;
        self
    }

    /// Sets [`serialize_none_to_null`] option.
    ///
    /// [`serialize_none_to_null`]: #structfield.serialize_none_to_null
    #[must_use]
    pub const fn serialize_none_to_null(mut self, enabled: bool) -> Self {
        self.serialize_none_to_null = enabled;
        self
    }

    /// Sets [`serialize_unit_to_null`] option.
    ///
    /// [`serialize_unit_to_null`]: #structfield.serialize_unit_to_null
    #[must_use]
    pub const fn serialize_unit_to_null(mut self, enabled: bool) -> Self {
        self.serialize_unit_to_null = enabled;
        self
    }
}

impl<'lua> Serializer<'lua> {
    /// Creates a new Lua Serializer with default options.
    pub fn new(lua: &'lua Lua) -> Self {
        Self::new_with_options(lua, Options::default())
    }

    /// Creates a new Lua Serializer with custom options.
    pub fn new_with_options(lua: &'lua Lua, options: Options) -> Self {
        Serializer { lua, options }
    }
}

macro_rules! lua_serialize_number {
    ($name:ident, $t:ty) => {
        #[inline]
        fn $name(self, value: $t) -> Result<Value<'lua>> {
            value.to_lua(self.lua)
        }
    };
}

impl<'lua> ser::Serializer for Serializer<'lua> {
    type Ok = Value<'lua>;
    type Error = Error;

    // Associated types for keeping track of additional state while serializing
    // compound data structures like sequences and maps.
    type SerializeSeq = SerializeVec<'lua>;
    type SerializeTuple = SerializeVec<'lua>;
    type SerializeTupleStruct = SerializeVec<'lua>;
    type SerializeTupleVariant = SerializeTupleVariant<'lua>;
    type SerializeMap = SerializeMap<'lua>;
    type SerializeStruct = SerializeMap<'lua>;
    type SerializeStructVariant = SerializeStructVariant<'lua>;

    #[inline]
    fn serialize_bool(self, value: bool) -> Result<Value<'lua>> {
        Ok(Value::Boolean(value))
    }

    lua_serialize_number!(serialize_i8, i8);
    lua_serialize_number!(serialize_u8, u8);
    lua_serialize_number!(serialize_i16, i16);
    lua_serialize_number!(serialize_u16, u16);
    lua_serialize_number!(serialize_i32, i32);
    lua_serialize_number!(serialize_u32, u32);
    lua_serialize_number!(serialize_i64, i64);
    lua_serialize_number!(serialize_u64, u64);
    lua_serialize_number!(serialize_i128, i128);
    lua_serialize_number!(serialize_u128, u128);

    lua_serialize_number!(serialize_f32, f32);
    lua_serialize_number!(serialize_f64, f64);

    #[inline]
    fn serialize_char(self, value: char) -> Result<Value<'lua>> {
        self.serialize_str(&value.to_string())
    }

    #[inline]
    fn serialize_str(self, value: &str) -> Result<Value<'lua>> {
        self.lua.create_string(value).map(Value::String)
    }

    #[inline]
    fn serialize_bytes(self, value: &[u8]) -> Result<Value<'lua>> {
        self.lua.create_string(value).map(Value::String)
    }

    #[inline]
    fn serialize_none(self) -> Result<Value<'lua>> {
        if self.options.serialize_none_to_null {
            Ok(self.lua.null())
        } else {
            Ok(Value::Nil)
        }
    }

    #[inline]
    fn serialize_some<T>(self, value: &T) -> Result<Value<'lua>>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    #[inline]
    fn serialize_unit(self) -> Result<Value<'lua>> {
        if self.options.serialize_unit_to_null {
            Ok(self.lua.null())
        } else {
            Ok(Value::Nil)
        }
    }

    #[inline]
    fn serialize_unit_struct(self, _name: &'static str) -> Result<Value<'lua>> {
        if self.options.serialize_unit_to_null {
            Ok(self.lua.null())
        } else {
            Ok(Value::Nil)
        }
    }

    #[inline]
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Value<'lua>> {
        self.serialize_str(variant)
    }

    #[inline]
    fn serialize_newtype_struct<T>(self, _name: &'static str, value: &T) -> Result<Value<'lua>>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    #[inline]
    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Value<'lua>>
    where
        T: Serialize + ?Sized,
    {
        let table = self.lua.create_table()?;
        let variant = self.lua.create_string(variant)?;
        let value = self.lua.to_value_with(value, self.options)?;
        table.raw_set(variant, value)?;
        Ok(Value::Table(table))
    }

    #[inline]
    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq> {
        let len = len.unwrap_or(0) as c_int;
        let table = self.lua.create_table_with_capacity(len, 0)?;
        if self.options.set_array_metatable {
            table.set_metatable(Some(self.lua.array_metatable()));
        }
        let options = self.options;
        Ok(SerializeVec { table, options })
    }

    #[inline]
    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple> {
        self.serialize_seq(Some(len))
    }

    #[inline]
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct> {
        self.serialize_seq(Some(len))
    }

    #[inline]
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant> {
        Ok(SerializeTupleVariant {
            name: self.lua.create_string(variant)?,
            table: self.lua.create_table()?,
            options: self.options,
        })
    }

    #[inline]
    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap> {
        let len = len.unwrap_or(0) as c_int;
        Ok(SerializeMap {
            key: None,
            table: self.lua.create_table_with_capacity(0, len)?,
            options: self.options,
        })
    }

    #[inline]
    fn serialize_struct(self, _name: &'static str, len: usize) -> Result<Self::SerializeStruct> {
        self.serialize_map(Some(len))
    }

    #[inline]
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant> {
        Ok(SerializeStructVariant {
            name: self.lua.create_string(variant)?,
            table: self.lua.create_table_with_capacity(0, len as c_int)?,
            options: self.options,
        })
    }
}

#[doc(hidden)]
pub struct SerializeVec<'lua> {
    table: Table<'lua>,
    options: Options,
}

impl<'lua> ser::SerializeSeq for SerializeVec<'lua> {
    type Ok = Value<'lua>;
    type Error = Error;

    fn serialize_element<T>(&mut self, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        let lua = self.table.0.lua;
        let value = lua.to_value_with(value, self.options)?;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, 4)?;

            lua.push_ref(&self.table.0);
            lua.push_value(value)?;
            protect_lua!(lua.state, 2, 0, fn(state) {
                let len = ffi::lua_rawlen(state, -2) as Integer;
                ffi::lua_rawseti(state, -2, len + 1);
            })
        }
    }

    fn end(self) -> Result<Value<'lua>> {
        Ok(Value::Table(self.table))
    }
}

impl<'lua> ser::SerializeTuple for SerializeVec<'lua> {
    type Ok = Value<'lua>;
    type Error = Error;

    fn serialize_element<T>(&mut self, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Value<'lua>> {
        ser::SerializeSeq::end(self)
    }
}

impl<'lua> ser::SerializeTupleStruct for SerializeVec<'lua> {
    type Ok = Value<'lua>;
    type Error = Error;

    fn serialize_field<T>(&mut self, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Value<'lua>> {
        ser::SerializeSeq::end(self)
    }
}

#[doc(hidden)]
pub struct SerializeTupleVariant<'lua> {
    name: String<'lua>,
    table: Table<'lua>,
    options: Options,
}

impl<'lua> ser::SerializeTupleVariant for SerializeTupleVariant<'lua> {
    type Ok = Value<'lua>;
    type Error = Error;

    fn serialize_field<T>(&mut self, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        let lua = self.table.0.lua;
        let idx = self.table.raw_len() + 1;
        self.table
            .raw_insert(idx, lua.to_value_with(value, self.options)?)
    }

    fn end(self) -> Result<Value<'lua>> {
        let lua = self.table.0.lua;
        let table = lua.create_table()?;
        table.raw_set(self.name, self.table)?;
        Ok(Value::Table(table))
    }
}

#[doc(hidden)]
pub struct SerializeMap<'lua> {
    table: Table<'lua>,
    key: Option<Value<'lua>>,
    options: Options,
}

impl<'lua> ser::SerializeMap for SerializeMap<'lua> {
    type Ok = Value<'lua>;
    type Error = Error;

    fn serialize_key<T>(&mut self, key: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        let lua = self.table.0.lua;
        self.key = Some(lua.to_value_with(key, self.options)?);
        Ok(())
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        let lua = self.table.0.lua;
        let key = mlua_expect!(
            self.key.take(),
            "serialize_value called before serialize_key"
        );
        let value = lua.to_value_with(value, self.options)?;
        self.table.raw_set(key, value)
    }

    fn end(self) -> Result<Value<'lua>> {
        Ok(Value::Table(self.table))
    }
}

impl<'lua> ser::SerializeStruct for SerializeMap<'lua> {
    type Ok = Value<'lua>;
    type Error = Error;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        ser::SerializeMap::serialize_key(self, key)?;
        ser::SerializeMap::serialize_value(self, value)
    }

    fn end(self) -> Result<Value<'lua>> {
        ser::SerializeMap::end(self)
    }
}

#[doc(hidden)]
pub struct SerializeStructVariant<'lua> {
    name: String<'lua>,
    table: Table<'lua>,
    options: Options,
}

impl<'lua> ser::SerializeStructVariant for SerializeStructVariant<'lua> {
    type Ok = Value<'lua>;
    type Error = Error;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        let lua = self.table.0.lua;
        self.table
            .raw_set(key, lua.to_value_with(value, self.options)?)?;
        Ok(())
    }

    fn end(self) -> Result<Value<'lua>> {
        let lua = self.table.0.lua;
        let table = lua.create_table()?;
        table.raw_set(self.name, self.table)?;
        Ok(Value::Table(table))
    }
}
