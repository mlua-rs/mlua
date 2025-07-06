//! Serialize a Rust data structure into Lua value.

use serde::{ser, Serialize};

use super::LuaSerdeExt;
use crate::error::{Error, Result};
use crate::state::Lua;
use crate::table::Table;
use crate::traits::IntoLua;
use crate::value::Value;

/// A struct for serializing Rust values into Lua values.
#[derive(Debug)]
pub struct Serializer<'a> {
    lua: &'a Lua,
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

    /// If true, serialize `serde_json::Number` with arbitrary_precision to a Lua number.
    /// Otherwise it will be serialized as an object (what serde does).
    ///
    /// Default: **false**
    pub detect_serde_json_arbitrary_precision: bool,
}

impl Default for Options {
    fn default() -> Self {
        const { Self::new() }
    }
}

impl Options {
    /// Returns a new instance of [`Options`] with default parameters.
    pub const fn new() -> Self {
        Options {
            set_array_metatable: true,
            serialize_none_to_null: true,
            serialize_unit_to_null: true,
            detect_serde_json_arbitrary_precision: false,
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

    /// Sets [`detect_serde_json_arbitrary_precision`] option.
    ///
    /// This option is used to serialize `serde_json::Number` with arbitrary precision to a Lua
    /// number. Otherwise it will be serialized as an object (what serde does).
    ///
    /// This option is disabled by default.
    ///
    /// [`detect_serde_json_arbitrary_precision`]: #structfield.detect_serde_json_arbitrary_precision
    #[must_use]
    pub const fn detect_serde_json_arbitrary_precision(mut self, enabled: bool) -> Self {
        self.detect_serde_json_arbitrary_precision = enabled;
        self
    }
}

impl<'a> Serializer<'a> {
    /// Creates a new Lua Serializer with default options.
    pub fn new(lua: &'a Lua) -> Self {
        Self::new_with_options(lua, Options::default())
    }

    /// Creates a new Lua Serializer with custom options.
    pub fn new_with_options(lua: &'a Lua, options: Options) -> Self {
        Serializer { lua, options }
    }
}

macro_rules! lua_serialize_number {
    ($name:ident, $t:ty) => {
        #[inline]
        fn $name(self, value: $t) -> Result<Value> {
            value.into_lua(self.lua)
        }
    };
}

impl<'a> ser::Serializer for Serializer<'a> {
    type Ok = Value;
    type Error = Error;

    // Associated types for keeping track of additional state while serializing
    // compound data structures like sequences and maps.
    type SerializeSeq = SerializeSeq<'a>;
    type SerializeTuple = SerializeSeq<'a>;
    type SerializeTupleStruct = SerializeSeq<'a>;
    type SerializeTupleVariant = SerializeTupleVariant<'a>;
    type SerializeMap = SerializeMap<'a>;
    type SerializeStruct = SerializeStruct<'a>;
    type SerializeStructVariant = SerializeStructVariant<'a>;

    #[inline]
    fn serialize_bool(self, value: bool) -> Result<Value> {
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
    fn serialize_char(self, value: char) -> Result<Value> {
        self.serialize_str(&value.to_string())
    }

    #[inline]
    fn serialize_str(self, value: &str) -> Result<Value> {
        self.lua.create_string(value).map(Value::String)
    }

    #[inline]
    fn serialize_bytes(self, value: &[u8]) -> Result<Value> {
        self.lua.create_string(value).map(Value::String)
    }

    #[inline]
    fn serialize_none(self) -> Result<Value> {
        if self.options.serialize_none_to_null {
            Ok(self.lua.null())
        } else {
            Ok(Value::Nil)
        }
    }

    #[inline]
    fn serialize_some<T>(self, value: &T) -> Result<Value>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    #[inline]
    fn serialize_unit(self) -> Result<Value> {
        if self.options.serialize_unit_to_null {
            Ok(self.lua.null())
        } else {
            Ok(Value::Nil)
        }
    }

    #[inline]
    fn serialize_unit_struct(self, _name: &'static str) -> Result<Value> {
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
    ) -> Result<Value> {
        self.serialize_str(variant)
    }

    #[inline]
    fn serialize_newtype_struct<T>(self, _name: &'static str, value: &T) -> Result<Value>
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
    ) -> Result<Value>
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
        let table = self.lua.create_table_with_capacity(len.unwrap_or(0), 0)?;
        if self.options.set_array_metatable {
            table.set_metatable(Some(self.lua.array_metatable()))?;
        }
        Ok(SerializeSeq::new(self.lua, table, self.options))
    }

    #[inline]
    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple> {
        self.serialize_seq(Some(len))
    }

    #[inline]
    fn serialize_tuple_struct(self, name: &'static str, len: usize) -> Result<Self::SerializeTupleStruct> {
        #[cfg(feature = "luau")]
        if name == "Vector" && len == crate::Vector::SIZE {
            return Ok(SerializeSeq::new_vector(self.lua, self.options));
        }
        _ = name;
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
            lua: self.lua,
            variant,
            table: self.lua.create_table()?,
            options: self.options,
        })
    }

    #[inline]
    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap> {
        Ok(SerializeMap {
            lua: self.lua,
            key: None,
            table: self.lua.create_table_with_capacity(0, len.unwrap_or(0))?,
            options: self.options,
        })
    }

    #[inline]
    fn serialize_struct(self, name: &'static str, len: usize) -> Result<Self::SerializeStruct> {
        if self.options.detect_serde_json_arbitrary_precision
            && name == "$serde_json::private::Number"
            && len == 1
        {
            return Ok(SerializeStruct {
                lua: self.lua,
                inner: None,
                options: self.options,
            });
        }

        Ok(SerializeStruct {
            lua: self.lua,
            inner: Some(Value::Table(self.lua.create_table_with_capacity(0, len)?)),
            options: self.options,
        })
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
            lua: self.lua,
            variant,
            table: self.lua.create_table_with_capacity(0, len)?,
            options: self.options,
        })
    }
}

#[doc(hidden)]
pub struct SerializeSeq<'a> {
    lua: &'a Lua,
    #[cfg(feature = "luau")]
    vector: Option<crate::Vector>,
    table: Option<Table>,
    next: usize,
    options: Options,
}

impl<'a> SerializeSeq<'a> {
    fn new(lua: &'a Lua, table: Table, options: Options) -> Self {
        Self {
            lua,
            #[cfg(feature = "luau")]
            vector: None,
            table: Some(table),
            next: 0,
            options,
        }
    }

    #[cfg(feature = "luau")]
    const fn new_vector(lua: &'a Lua, options: Options) -> Self {
        Self {
            lua,
            vector: Some(crate::Vector::zero()),
            table: None,
            next: 0,
            options,
        }
    }
}

impl ser::SerializeSeq for SerializeSeq<'_> {
    type Ok = Value;
    type Error = Error;

    fn serialize_element<T>(&mut self, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        let value = self.lua.to_value_with(value, self.options)?;
        let table = self.table.as_ref().unwrap();
        table.raw_seti(self.next + 1, value)?;
        self.next += 1;
        Ok(())
    }

    fn end(self) -> Result<Value> {
        Ok(Value::Table(self.table.unwrap()))
    }
}

impl ser::SerializeTuple for SerializeSeq<'_> {
    type Ok = Value;
    type Error = Error;

    fn serialize_element<T>(&mut self, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Value> {
        ser::SerializeSeq::end(self)
    }
}

impl ser::SerializeTupleStruct for SerializeSeq<'_> {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T>(&mut self, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        #[cfg(feature = "luau")]
        if let Some(vector) = self.vector.as_mut() {
            let value = self.lua.to_value_with(value, self.options)?;
            let value = self.lua.unpack(value)?;
            vector.0[self.next] = value;
            self.next += 1;
            return Ok(());
        }
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Value> {
        #[cfg(feature = "luau")]
        if let Some(vector) = self.vector {
            return Ok(Value::Vector(vector));
        }
        ser::SerializeSeq::end(self)
    }
}

#[doc(hidden)]
pub struct SerializeTupleVariant<'a> {
    lua: &'a Lua,
    variant: &'static str,
    table: Table,
    options: Options,
}

impl ser::SerializeTupleVariant for SerializeTupleVariant<'_> {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T>(&mut self, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        self.table.raw_push(self.lua.to_value_with(value, self.options)?)
    }

    fn end(self) -> Result<Value> {
        let table = self.lua.create_table()?;
        table.raw_set(self.variant, self.table)?;
        Ok(Value::Table(table))
    }
}

#[doc(hidden)]
pub struct SerializeMap<'a> {
    lua: &'a Lua,
    table: Table,
    key: Option<Value>,
    options: Options,
}

impl ser::SerializeMap for SerializeMap<'_> {
    type Ok = Value;
    type Error = Error;

    fn serialize_key<T>(&mut self, key: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        self.key = Some(self.lua.to_value_with(key, self.options)?);
        Ok(())
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        let key = mlua_expect!(self.key.take(), "serialize_value called before serialize_key");
        let value = self.lua.to_value_with(value, self.options)?;
        self.table.raw_set(key, value)
    }

    fn end(self) -> Result<Value> {
        Ok(Value::Table(self.table))
    }
}

#[doc(hidden)]
pub struct SerializeStruct<'a> {
    lua: &'a Lua,
    inner: Option<Value>,
    options: Options,
}

impl ser::SerializeStruct for SerializeStruct<'_> {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        match self.inner {
            Some(Value::Table(ref table)) => {
                table.raw_set(key, self.lua.to_value_with(value, self.options)?)?;
            }
            None if self.options.detect_serde_json_arbitrary_precision => {
                // A special case for `serde_json::Number` with arbitrary precision.
                assert_eq!(key, "$serde_json::private::Number");
                self.inner = Some(self.lua.to_value_with(value, self.options)?);
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    fn end(self) -> Result<Value> {
        match self.inner {
            Some(table @ Value::Table(_)) => Ok(table),
            Some(value @ Value::String(_)) if self.options.detect_serde_json_arbitrary_precision => {
                let number_s = value.to_string()?;
                if number_s.contains(['.', 'e', 'E']) {
                    if let Ok(number) = number_s.parse().map(Value::Number) {
                        return Ok(number);
                    }
                }
                Ok(number_s
                    .parse()
                    .map(Value::Integer)
                    .or_else(|_| number_s.parse().map(Value::Number))
                    .unwrap_or(value))
            }
            _ => unreachable!(),
        }
    }
}

#[doc(hidden)]
pub struct SerializeStructVariant<'a> {
    lua: &'a Lua,
    variant: &'static str,
    table: Table,
    options: Options,
}

impl ser::SerializeStructVariant for SerializeStructVariant<'_> {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        self.table
            .raw_set(key, self.lua.to_value_with(value, self.options)?)?;
        Ok(())
    }

    fn end(self) -> Result<Value> {
        let table = self.lua.create_table_with_capacity(0, 1)?;
        table.raw_set(self.variant, self.table)?;
        Ok(Value::Table(table))
    }
}
