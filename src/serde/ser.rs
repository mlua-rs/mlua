use std::os::raw::c_int;

use serde::{ser, Serialize};

use super::LuaSerdeExt;
use crate::error::{Error, Result};
use crate::ffi;
use crate::lua::Lua;
use crate::string::String;
use crate::table::Table;
use crate::types::Integer;
use crate::util::{assert_stack, protect_lua, StackGuard};
use crate::value::{ToLua, Value};

/// A struct for serializing Rust values into Lua values.
pub struct Serializer<'lua>(pub &'lua Lua);

macro_rules! lua_serialize_number {
    ($name:ident, $t:ty) => {
        #[inline]
        fn $name(self, value: $t) -> Result<Value<'lua>> {
            value.to_lua(self.0)
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

    lua_serialize_number!(serialize_f32, f32);
    lua_serialize_number!(serialize_f64, f64);

    #[inline]
    fn serialize_char(self, value: char) -> Result<Value<'lua>> {
        self.serialize_str(&value.to_string())
    }

    #[inline]
    fn serialize_str(self, value: &str) -> Result<Value<'lua>> {
        self.0.create_string(value).map(Value::String)
    }

    #[inline]
    fn serialize_bytes(self, value: &[u8]) -> Result<Value<'lua>> {
        self.0.create_string(value).map(Value::String)
    }

    #[inline]
    fn serialize_none(self) -> Result<Value<'lua>> {
        self.0.null()
    }

    #[inline]
    fn serialize_some<T>(self, value: &T) -> Result<Value<'lua>>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    #[inline]
    fn serialize_unit(self) -> Result<Value<'lua>> {
        self.0.null()
    }

    #[inline]
    fn serialize_unit_struct(self, _name: &'static str) -> Result<Value<'lua>> {
        self.0.null()
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
        T: ?Sized + Serialize,
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
        T: ?Sized + Serialize,
    {
        let table = self.0.create_table()?;
        let variant = self.0.create_string(variant)?;
        let value = self.0.to_value(value)?;
        table.raw_set(variant, value)?;
        Ok(Value::Table(table))
    }

    #[inline]
    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq> {
        let len = len.unwrap_or(0) as c_int;
        let table = self.0.create_table_with_capacity(len, 0)?;
        table.set_metatable(Some(self.0.array_metatable()?));
        Ok(SerializeVec { table })
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
        let name = self.0.create_string(variant)?;
        let table = self.0.create_table()?;
        Ok(SerializeTupleVariant { name, table })
    }

    #[inline]
    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap> {
        let len = len.unwrap_or(0) as c_int;
        Ok(SerializeMap {
            key: None,
            table: self.0.create_table_with_capacity(0, len)?,
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
        let name = self.0.create_string(variant)?;
        let table = self.0.create_table_with_capacity(0, len as c_int)?;
        Ok(SerializeStructVariant { name, table })
    }
}

pub struct SerializeVec<'lua> {
    table: Table<'lua>,
}

impl<'lua> ser::SerializeSeq for SerializeVec<'lua> {
    type Ok = Value<'lua>;
    type Error = Error;

    fn serialize_element<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        let lua = self.table.0.lua;
        let value = lua.to_value(value)?;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 4);

            lua.push_ref(&self.table.0);
            lua.push_value(value)?;

            unsafe extern "C" fn push_to_table(state: *mut ffi::lua_State) -> c_int {
                let len = ffi::lua_rawlen(state, -2) as Integer;
                ffi::lua_rawseti(state, -2, len + 1);
                1
            }

            protect_lua(lua.state, 2, push_to_table)
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
        T: ?Sized + Serialize,
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
        T: ?Sized + Serialize,
    {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Value<'lua>> {
        ser::SerializeSeq::end(self)
    }
}

pub struct SerializeTupleVariant<'lua> {
    name: String<'lua>,
    table: Table<'lua>,
}

impl<'lua> ser::SerializeTupleVariant for SerializeTupleVariant<'lua> {
    type Ok = Value<'lua>;
    type Error = Error;

    fn serialize_field<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        let lua = self.table.0.lua;
        let idx = self.table.raw_len() + 1;
        self.table.raw_insert(idx, lua.to_value(value)?)
    }

    fn end(self) -> Result<Value<'lua>> {
        let lua = self.table.0.lua;
        let table = lua.create_table()?;
        table.raw_set(self.name, self.table)?;
        Ok(Value::Table(table))
    }
}

pub struct SerializeMap<'lua> {
    table: Table<'lua>,
    key: Option<Value<'lua>>,
}

impl<'lua> ser::SerializeMap for SerializeMap<'lua> {
    type Ok = Value<'lua>;
    type Error = Error;

    fn serialize_key<T>(&mut self, key: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        let lua = self.table.0.lua;
        self.key = Some(lua.to_value(key)?);
        Ok(())
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        let lua = self.table.0.lua;
        let key = mlua_expect!(
            self.key.take(),
            "serialize_value called before serialize_key"
        );
        let value = lua.to_value(value)?;
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
        T: ?Sized + Serialize,
    {
        ser::SerializeMap::serialize_key(self, key)?;
        ser::SerializeMap::serialize_value(self, value)
    }

    fn end(self) -> Result<Value<'lua>> {
        ser::SerializeMap::end(self)
    }
}

pub struct SerializeStructVariant<'lua> {
    name: String<'lua>,
    table: Table<'lua>,
}

impl<'lua> ser::SerializeStructVariant for SerializeStructVariant<'lua> {
    type Ok = Value<'lua>;
    type Error = Error;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        let lua = self.table.0.lua;
        self.table.raw_set(key, lua.to_value(value)?)?;
        Ok(())
    }

    fn end(self) -> Result<Value<'lua>> {
        let lua = self.table.0.lua;
        let table = lua.create_table()?;
        table.raw_set(self.name, self.table)?;
        Ok(Value::Table(table))
    }
}
