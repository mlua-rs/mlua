use std::cell::RefCell;
use std::os::raw::c_void;
use std::rc::Rc;
use std::string::String as StdString;

use rustc_hash::FxHashSet;
use serde::de::{self, IntoDeserializer};

use crate::error::{Error, Result};
use crate::ffi;
use crate::table::{Table, TablePairs, TableSequence};
use crate::value::Value;

/// A struct for deserializing Lua values into Rust values.
#[derive(Debug)]
pub struct Deserializer<'lua> {
    value: Value<'lua>,
    options: Options,
    visited: Rc<RefCell<FxHashSet<*const c_void>>>,
}

/// A struct with options to change default deserializer behavior.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Options {
    /// If true, an attempt to serialize types such as [`Thread`], [`UserData`], [`LightUserData`]
    /// and [`Error`] will cause an error.
    /// Otherwise these types skipped when iterating or serialized as unit type.
    ///
    /// Default: **true**
    ///
    /// [`Thread`]: crate::Thread
    /// [`UserData`]: crate::UserData
    /// [`LightUserData`]: crate::LightUserData
    /// [`Error`]: crate::Error
    pub deny_unsupported_types: bool,

    /// If true, an attempt to serialize a recursive table (table that refers to itself)
    /// will cause an error.
    /// Otherwise subsequent attempts to serialize the same table will be ignored.
    ///
    /// Default: **true**
    pub deny_recursive_tables: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self::new()
    }
}

impl Options {
    /// Returns a new instance of `Options` with default parameters.
    pub const fn new() -> Self {
        Options {
            deny_unsupported_types: true,
            deny_recursive_tables: true,
        }
    }

    /// Sets [`deny_unsupported_types`] option.
    ///
    /// [`deny_unsupported_types`]: #structfield.deny_unsupported_types
    #[must_use]
    pub const fn deny_unsupported_types(mut self, enabled: bool) -> Self {
        self.deny_unsupported_types = enabled;
        self
    }

    /// Sets [`deny_recursive_tables`] option.
    ///
    /// [`deny_recursive_tables`]: #structfield.deny_recursive_tables
    #[must_use]
    pub fn deny_recursive_tables(mut self, enabled: bool) -> Self {
        self.deny_recursive_tables = enabled;
        self
    }
}

impl<'lua> Deserializer<'lua> {
    /// Creates a new Lua Deserializer for the `Value`.
    pub fn new(value: Value<'lua>) -> Self {
        Self::new_with_options(value, Options::default())
    }

    /// Creates a new Lua Deserializer for the `Value` with custom options.
    pub fn new_with_options(value: Value<'lua>, options: Options) -> Self {
        Deserializer {
            value,
            options,
            visited: Rc::new(RefCell::new(FxHashSet::default())),
        }
    }

    fn from_parts(
        value: Value<'lua>,
        options: Options,
        visited: Rc<RefCell<FxHashSet<*const c_void>>>,
    ) -> Self {
        Deserializer {
            value,
            options,
            visited,
        }
    }
}

impl<'lua, 'de> serde::Deserializer<'de> for Deserializer<'lua> {
    type Error = Error;

    #[inline]
    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.value {
            Value::Nil => visitor.visit_unit(),
            Value::Boolean(b) => visitor.visit_bool(b),
            #[allow(clippy::useless_conversion)]
            Value::Integer(i) => visitor.visit_i64(i.into()),
            #[allow(clippy::useless_conversion)]
            Value::Number(n) => visitor.visit_f64(n.into()),
            Value::String(s) => match s.to_str() {
                Ok(s) => visitor.visit_str(s),
                Err(_) => visitor.visit_bytes(s.as_bytes()),
            },
            Value::Table(ref t) if t.raw_len() > 0 || t.is_array() => self.deserialize_seq(visitor),
            Value::Table(_) => self.deserialize_map(visitor),
            Value::LightUserData(ud) if ud.0.is_null() => visitor.visit_none(),
            Value::Function(_)
            | Value::Thread(_)
            | Value::UserData(_)
            | Value::LightUserData(_)
            | Value::Error(_) => {
                if self.options.deny_unsupported_types {
                    Err(de::Error::custom(format!(
                        "unsupported value type `{}`",
                        self.value.type_name()
                    )))
                } else {
                    visitor.visit_unit()
                }
            }
        }
    }

    #[inline]
    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.value {
            Value::Nil => visitor.visit_none(),
            Value::LightUserData(ud) if ud.0.is_null() => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    #[inline]
    fn deserialize_enum<V>(
        self,
        _name: &str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        let (variant, value, _guard) = match self.value {
            Value::Table(table) => {
                let _guard = RecursionGuard::new(&table, &self.visited);

                let mut iter = table.pairs::<StdString, Value>();
                let (variant, value) = match iter.next() {
                    Some(v) => v?,
                    None => {
                        return Err(de::Error::invalid_value(
                            de::Unexpected::Map,
                            &"map with a single key",
                        ))
                    }
                };

                if iter.next().is_some() {
                    return Err(de::Error::invalid_value(
                        de::Unexpected::Map,
                        &"map with a single key",
                    ));
                }
                if check_value_if_skip(&value, self.options, &self.visited)? {
                    return Err(de::Error::custom("bad enum value"));
                }

                (variant, Some(value), Some(_guard))
            }
            Value::String(variant) => (variant.to_str()?.to_owned(), None, None),
            _ => return Err(de::Error::custom("bad enum value")),
        };

        visitor.visit_enum(EnumDeserializer {
            variant,
            value,
            options: self.options,
            visited: self.visited,
        })
    }

    #[inline]
    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.value {
            Value::Table(t) => {
                let _guard = RecursionGuard::new(&t, &self.visited);

                let len = t.raw_len() as usize;
                let mut deserializer = SeqDeserializer {
                    seq: t.raw_sequence_values(),
                    options: self.options,
                    visited: self.visited,
                };
                let seq = visitor.visit_seq(&mut deserializer)?;
                if deserializer.seq.count() == 0 {
                    Ok(seq)
                } else {
                    Err(de::Error::invalid_length(
                        len,
                        &"fewer elements in the table",
                    ))
                }
            }
            value => Err(de::Error::invalid_type(
                de::Unexpected::Other(value.type_name()),
                &"table",
            )),
        }
    }

    #[inline]
    fn deserialize_tuple<V>(self, _len: usize, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_seq(visitor)
    }

    #[inline]
    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_seq(visitor)
    }

    #[inline]
    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.value {
            Value::Table(t) => {
                let _guard = RecursionGuard::new(&t, &self.visited);

                let mut deserializer = MapDeserializer {
                    pairs: t.pairs(),
                    value: None,
                    options: self.options,
                    visited: self.visited,
                    processed: 0,
                };
                let map = visitor.visit_map(&mut deserializer)?;
                let count = deserializer.pairs.count();
                if count == 0 {
                    Ok(map)
                } else {
                    Err(de::Error::invalid_length(
                        deserializer.processed + count,
                        &"fewer elements in the table",
                    ))
                }
            }
            value => Err(de::Error::invalid_type(
                de::Unexpected::Other(value.type_name()),
                &"table",
            )),
        }
    }

    #[inline]
    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_map(visitor)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes
        byte_buf unit unit_struct newtype_struct
        identifier ignored_any
    }
}

struct SeqDeserializer<'lua> {
    seq: TableSequence<'lua, Value<'lua>>,
    options: Options,
    visited: Rc<RefCell<FxHashSet<*const c_void>>>,
}

impl<'lua, 'de> de::SeqAccess<'de> for SeqDeserializer<'lua> {
    type Error = Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>>
    where
        T: de::DeserializeSeed<'de>,
    {
        loop {
            match self.seq.next() {
                Some(value) => {
                    let value = value?;
                    if check_value_if_skip(&value, self.options, &self.visited)? {
                        continue;
                    }
                    let visited = Rc::clone(&self.visited);
                    let deserializer = Deserializer::from_parts(value, self.options, visited);
                    return seed.deserialize(deserializer).map(Some);
                }
                None => return Ok(None),
            }
        }
    }

    fn size_hint(&self) -> Option<usize> {
        match self.seq.size_hint() {
            (lower, Some(upper)) if lower == upper => Some(upper),
            _ => None,
        }
    }
}

struct MapDeserializer<'lua> {
    pairs: TablePairs<'lua, Value<'lua>, Value<'lua>>,
    value: Option<Value<'lua>>,
    options: Options,
    visited: Rc<RefCell<FxHashSet<*const c_void>>>,
    processed: usize,
}

impl<'lua, 'de> de::MapAccess<'de> for MapDeserializer<'lua> {
    type Error = Error;

    fn next_key_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>>
    where
        T: de::DeserializeSeed<'de>,
    {
        loop {
            match self.pairs.next() {
                Some(item) => {
                    let (key, value) = item?;
                    if check_value_if_skip(&key, self.options, &self.visited)?
                        || check_value_if_skip(&value, self.options, &self.visited)?
                    {
                        continue;
                    }
                    self.processed += 1;
                    self.value = Some(value);
                    let visited = Rc::clone(&self.visited);
                    let key_de = Deserializer::from_parts(key, self.options, visited);
                    return seed.deserialize(key_de).map(Some);
                }
                None => return Ok(None),
            }
        }
    }

    fn next_value_seed<T>(&mut self, seed: T) -> Result<T::Value>
    where
        T: de::DeserializeSeed<'de>,
    {
        match self.value.take() {
            Some(value) => {
                let visited = Rc::clone(&self.visited);
                seed.deserialize(Deserializer::from_parts(value, self.options, visited))
            }
            None => Err(de::Error::custom("value is missing")),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        match self.pairs.size_hint() {
            (lower, Some(upper)) if lower == upper => Some(upper),
            _ => None,
        }
    }
}

struct EnumDeserializer<'lua> {
    variant: StdString,
    value: Option<Value<'lua>>,
    options: Options,
    visited: Rc<RefCell<FxHashSet<*const c_void>>>,
}

impl<'lua, 'de> de::EnumAccess<'de> for EnumDeserializer<'lua> {
    type Error = Error;
    type Variant = VariantDeserializer<'lua>;

    fn variant_seed<T>(self, seed: T) -> Result<(T::Value, Self::Variant)>
    where
        T: de::DeserializeSeed<'de>,
    {
        let variant = self.variant.into_deserializer();
        let variant_access = VariantDeserializer {
            value: self.value,
            options: self.options,
            visited: self.visited,
        };
        seed.deserialize(variant).map(|v| (v, variant_access))
    }
}

struct VariantDeserializer<'lua> {
    value: Option<Value<'lua>>,
    options: Options,
    visited: Rc<RefCell<FxHashSet<*const c_void>>>,
}

impl<'lua, 'de> de::VariantAccess<'de> for VariantDeserializer<'lua> {
    type Error = Error;

    fn unit_variant(self) -> Result<()> {
        match self.value {
            Some(_) => Err(de::Error::invalid_type(
                de::Unexpected::NewtypeVariant,
                &"unit variant",
            )),
            None => Ok(()),
        }
    }

    fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value>
    where
        T: de::DeserializeSeed<'de>,
    {
        match self.value {
            Some(value) => {
                seed.deserialize(Deserializer::from_parts(value, self.options, self.visited))
            }
            None => Err(de::Error::invalid_type(
                de::Unexpected::UnitVariant,
                &"newtype variant",
            )),
        }
    }

    fn tuple_variant<V>(self, _len: usize, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.value {
            Some(value) => serde::Deserializer::deserialize_seq(
                Deserializer::from_parts(value, self.options, self.visited),
                visitor,
            ),
            None => Err(de::Error::invalid_type(
                de::Unexpected::UnitVariant,
                &"tuple variant",
            )),
        }
    }

    fn struct_variant<V>(self, _fields: &'static [&'static str], visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.value {
            Some(value) => serde::Deserializer::deserialize_map(
                Deserializer::from_parts(value, self.options, self.visited),
                visitor,
            ),
            None => Err(de::Error::invalid_type(
                de::Unexpected::UnitVariant,
                &"struct variant",
            )),
        }
    }
}

// Adds `ptr` to the `visited` map and removes on drop
// Used to track recursive tables but allow to traverse same tables multiple times
struct RecursionGuard {
    ptr: *const c_void,
    visited: Rc<RefCell<FxHashSet<*const c_void>>>,
}

impl RecursionGuard {
    #[inline]
    fn new(table: &Table, visited: &Rc<RefCell<FxHashSet<*const c_void>>>) -> Self {
        let visited = Rc::clone(visited);
        let lua = table.0.lua;
        let ptr =
            unsafe { lua.ref_thread_exec(|refthr| ffi::lua_topointer(refthr, table.0.index)) };
        visited.borrow_mut().insert(ptr);
        RecursionGuard { ptr, visited }
    }
}

impl Drop for RecursionGuard {
    fn drop(&mut self) {
        self.visited.borrow_mut().remove(&self.ptr);
    }
}

// Checks `options` and decides should we emit an error or skip next element
fn check_value_if_skip(
    value: &Value,
    options: Options,
    visited: &RefCell<FxHashSet<*const c_void>>,
) -> Result<bool> {
    match value {
        Value::Table(table) => {
            let lua = table.0.lua;
            let ptr =
                unsafe { lua.ref_thread_exec(|refthr| ffi::lua_topointer(refthr, table.0.index)) };
            if visited.borrow().contains(&ptr) {
                if options.deny_recursive_tables {
                    return Err(de::Error::custom("recursive table detected"));
                }
                return Ok(true); // skip
            }
        }
        Value::Function(_)
        | Value::Thread(_)
        | Value::UserData(_)
        | Value::LightUserData(_)
        | Value::Error(_)
            if !options.deny_unsupported_types =>
        {
            return Ok(true); // skip
        }
        _ => {}
    }
    Ok(false) // do not skip
}
