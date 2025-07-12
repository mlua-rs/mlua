//! Deserialize Lua values to a Rust data structure.

use std::cell::RefCell;
use std::os::raw::c_void;
use std::rc::Rc;
use std::result::Result as StdResult;
use std::string::String as StdString;

use rustc_hash::FxHashSet;
use serde::de::{self, IntoDeserializer};

use crate::error::{Error, Result};
use crate::table::{Table, TablePairs, TableSequence};
use crate::userdata::AnyUserData;
use crate::value::Value;

/// A struct for deserializing Lua values into Rust values.
#[derive(Debug)]
pub struct Deserializer {
    value: Value,
    options: Options,
    visited: Rc<RefCell<FxHashSet<*const c_void>>>,
}

/// A struct with options to change default deserializer behavior.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Options {
    /// If true, an attempt to serialize types such as [`Function`], [`Thread`], [`LightUserData`]
    /// and [`Error`] will cause an error.
    /// Otherwise these types skipped when iterating or serialized as unit type.
    ///
    /// Default: **true**
    ///
    /// [`Function`]: crate::Function
    /// [`Thread`]: crate::Thread
    /// [`LightUserData`]: crate::LightUserData
    /// [`Error`]: crate::Error
    pub deny_unsupported_types: bool,

    /// If true, an attempt to serialize a recursive table (table that refers to itself)
    /// will cause an error.
    /// Otherwise subsequent attempts to serialize the same table will be ignored.
    ///
    /// Default: **true**
    pub deny_recursive_tables: bool,

    /// If true, keys in tables will be iterated in sorted order.
    ///
    /// Default: **false**
    pub sort_keys: bool,

    /// If true, empty Lua tables will be encoded as array, instead of map.
    ///
    /// Default: **false**
    pub encode_empty_tables_as_array: bool,
}

impl Default for Options {
    fn default() -> Self {
        const { Self::new() }
    }
}

impl Options {
    /// Returns a new instance of `Options` with default parameters.
    pub const fn new() -> Self {
        Options {
            deny_unsupported_types: true,
            deny_recursive_tables: true,
            sort_keys: false,
            encode_empty_tables_as_array: false,
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
    pub const fn deny_recursive_tables(mut self, enabled: bool) -> Self {
        self.deny_recursive_tables = enabled;
        self
    }

    /// Sets [`sort_keys`] option.
    ///
    /// [`sort_keys`]: #structfield.sort_keys
    #[must_use]
    pub const fn sort_keys(mut self, enabled: bool) -> Self {
        self.sort_keys = enabled;
        self
    }

    /// Sets [`encode_empty_tables_as_array`] option.
    ///
    /// [`encode_empty_tables_as_array`]: #structfield.encode_empty_tables_as_array
    #[must_use]
    pub const fn encode_empty_tables_as_array(mut self, enabled: bool) -> Self {
        self.encode_empty_tables_as_array = enabled;
        self
    }
}

impl Deserializer {
    /// Creates a new Lua Deserializer for the [`Value`].
    pub fn new(value: Value) -> Self {
        Self::new_with_options(value, Options::default())
    }

    /// Creates a new Lua Deserializer for the [`Value`] with custom options.
    pub fn new_with_options(value: Value, options: Options) -> Self {
        Deserializer {
            value,
            options,
            visited: Rc::new(RefCell::new(FxHashSet::default())),
        }
    }

    fn from_parts(value: Value, options: Options, visited: Rc<RefCell<FxHashSet<*const c_void>>>) -> Self {
        Deserializer {
            value,
            options,
            visited,
        }
    }
}

impl<'de> serde::Deserializer<'de> for Deserializer {
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
            #[cfg(feature = "luau")]
            Value::Vector(_) => self.deserialize_seq(visitor),
            Value::String(s) => match s.to_str() {
                Ok(s) => visitor.visit_str(&s),
                Err(_) => visitor.visit_bytes(&s.as_bytes()),
            },
            Value::Table(ref t) if t.raw_len() > 0 || t.is_array() => self.deserialize_seq(visitor),
            Value::Table(ref t) if self.options.encode_empty_tables_as_array && t.is_empty() => {
                self.deserialize_seq(visitor)
            }
            Value::Table(_) => self.deserialize_map(visitor),
            Value::LightUserData(ud) if ud.0.is_null() => visitor.visit_none(),
            Value::UserData(ud) if ud.is_serializable() => {
                serde_userdata(ud, |value| value.deserialize_any(visitor))
            }
            #[cfg(feature = "luau")]
            Value::Buffer(buf) => {
                let lua = buf.0.lua.lock();
                visitor.visit_bytes(buf.as_slice(&lua))
            }
            Value::Function(_)
            | Value::Thread(_)
            | Value::UserData(_)
            | Value::LightUserData(_)
            | Value::Error(_)
            | Value::Other(_) => {
                if self.options.deny_unsupported_types {
                    let msg = format!("unsupported value type `{}`", self.value.type_name());
                    Err(de::Error::custom(msg))
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
        name: &'static str,
        variants: &'static [&'static str],
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
                let skip = check_value_for_skip(&value, self.options, &self.visited)
                    .map_err(|err| Error::DeserializeError(err.to_string()))?;
                if skip {
                    return Err(de::Error::custom("bad enum value"));
                }

                (variant, Some(value), Some(_guard))
            }
            Value::String(variant) => (variant.to_str()?.to_owned(), None, None),
            Value::UserData(ud) if ud.is_serializable() => {
                return serde_userdata(ud, |value| value.deserialize_enum(name, variants, visitor));
            }
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
            #[cfg(feature = "luau")]
            Value::Vector(vec) => {
                let mut deserializer = VecDeserializer {
                    vec,
                    next: 0,
                    options: self.options,
                    visited: self.visited,
                };
                visitor.visit_seq(&mut deserializer)
            }
            Value::Table(t) => {
                let _guard = RecursionGuard::new(&t, &self.visited);

                let len = t.raw_len();
                let mut deserializer = SeqDeserializer {
                    seq: t.sequence_values(),
                    options: self.options,
                    visited: self.visited,
                };
                let seq = visitor.visit_seq(&mut deserializer)?;
                if deserializer.seq.count() == 0 {
                    Ok(seq)
                } else {
                    Err(de::Error::invalid_length(len, &"fewer elements in the table"))
                }
            }
            Value::UserData(ud) if ud.is_serializable() => {
                serde_userdata(ud, |value| value.deserialize_seq(visitor))
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
    fn deserialize_tuple_struct<V>(self, _name: &'static str, _len: usize, visitor: V) -> Result<V::Value>
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
                    pairs: MapPairs::new(&t, self.options.sort_keys)?,
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
            Value::UserData(ud) if ud.is_serializable() => {
                serde_userdata(ud, |value| value.deserialize_map(visitor))
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

    #[inline]
    fn deserialize_newtype_struct<V>(self, name: &'static str, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.value {
            Value::UserData(ud) if ud.is_serializable() => {
                serde_userdata(ud, |value| value.deserialize_newtype_struct(name, visitor))
            }
            _ => visitor.visit_newtype_struct(self),
        }
    }

    #[inline]
    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.value {
            Value::LightUserData(ud) if ud.0.is_null() => visitor.visit_unit(),
            _ => self.deserialize_any(visitor),
        }
    }

    #[inline]
    fn deserialize_unit_struct<V>(self, _name: &'static str, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.value {
            Value::LightUserData(ud) if ud.0.is_null() => visitor.visit_unit(),
            _ => self.deserialize_any(visitor),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes
        byte_buf identifier ignored_any
    }
}

struct SeqDeserializer<'a> {
    seq: TableSequence<'a, Value>,
    options: Options,
    visited: Rc<RefCell<FxHashSet<*const c_void>>>,
}

impl<'de> de::SeqAccess<'de> for SeqDeserializer<'_> {
    type Error = Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>>
    where
        T: de::DeserializeSeed<'de>,
    {
        loop {
            match self.seq.next() {
                Some(value) => {
                    let value = value?;
                    let skip = check_value_for_skip(&value, self.options, &self.visited)
                        .map_err(|err| Error::DeserializeError(err.to_string()))?;
                    if skip {
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

#[cfg(feature = "luau")]
struct VecDeserializer {
    vec: crate::Vector,
    next: usize,
    options: Options,
    visited: Rc<RefCell<FxHashSet<*const c_void>>>,
}

#[cfg(feature = "luau")]
impl<'de> de::SeqAccess<'de> for VecDeserializer {
    type Error = Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>>
    where
        T: de::DeserializeSeed<'de>,
    {
        match self.vec.0.get(self.next) {
            Some(&n) => {
                self.next += 1;
                let visited = Rc::clone(&self.visited);
                let deserializer = Deserializer::from_parts(Value::Number(n as _), self.options, visited);
                seed.deserialize(deserializer).map(Some)
            }
            None => Ok(None),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        Some(crate::Vector::SIZE)
    }
}

pub(crate) enum MapPairs<'a> {
    Iter(TablePairs<'a, Value, Value>),
    Vec(Vec<(Value, Value)>),
}

impl<'a> MapPairs<'a> {
    pub(crate) fn new(t: &'a Table, sort_keys: bool) -> Result<Self> {
        if sort_keys {
            let mut pairs = t.pairs::<Value, Value>().collect::<Result<Vec<_>>>()?;
            pairs.sort_by(|(a, _), (b, _)| b.sort_cmp(a)); // reverse order as we pop values from the end
            Ok(MapPairs::Vec(pairs))
        } else {
            Ok(MapPairs::Iter(t.pairs::<Value, Value>()))
        }
    }

    pub(crate) fn count(self) -> usize {
        match self {
            MapPairs::Iter(iter) => iter.count(),
            MapPairs::Vec(vec) => vec.len(),
        }
    }

    pub(crate) fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            MapPairs::Iter(iter) => iter.size_hint(),
            MapPairs::Vec(vec) => (vec.len(), Some(vec.len())),
        }
    }
}

impl Iterator for MapPairs<'_> {
    type Item = Result<(Value, Value)>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            MapPairs::Iter(iter) => iter.next(),
            MapPairs::Vec(vec) => vec.pop().map(Ok),
        }
    }
}

struct MapDeserializer<'a> {
    pairs: MapPairs<'a>,
    value: Option<Value>,
    options: Options,
    visited: Rc<RefCell<FxHashSet<*const c_void>>>,
    processed: usize,
}

impl MapDeserializer<'_> {
    fn next_key_deserializer(&mut self) -> Result<Option<Deserializer>> {
        loop {
            match self.pairs.next() {
                Some(item) => {
                    let (key, value) = item?;
                    let skip_key = check_value_for_skip(&key, self.options, &self.visited)
                        .map_err(|err| Error::DeserializeError(err.to_string()))?;
                    let skip_value = check_value_for_skip(&value, self.options, &self.visited)
                        .map_err(|err| Error::DeserializeError(err.to_string()))?;
                    if skip_key || skip_value {
                        continue;
                    }
                    self.processed += 1;
                    self.value = Some(value);
                    let visited = Rc::clone(&self.visited);
                    let key_de = Deserializer::from_parts(key, self.options, visited);
                    return Ok(Some(key_de));
                }
                None => return Ok(None),
            }
        }
    }

    fn next_value_deserializer(&mut self) -> Result<Deserializer> {
        match self.value.take() {
            Some(value) => {
                let visited = Rc::clone(&self.visited);
                Ok(Deserializer::from_parts(value, self.options, visited))
            }
            None => Err(de::Error::custom("value is missing")),
        }
    }
}

impl<'de> de::MapAccess<'de> for MapDeserializer<'_> {
    type Error = Error;

    fn next_key_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>>
    where
        T: de::DeserializeSeed<'de>,
    {
        match self.next_key_deserializer() {
            Ok(Some(key_de)) => seed.deserialize(key_de).map(Some),
            Ok(None) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn next_value_seed<T>(&mut self, seed: T) -> Result<T::Value>
    where
        T: de::DeserializeSeed<'de>,
    {
        match self.next_value_deserializer() {
            Ok(value_de) => seed.deserialize(value_de),
            Err(error) => Err(error),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        match self.pairs.size_hint() {
            (lower, Some(upper)) if lower == upper => Some(upper),
            _ => None,
        }
    }
}

struct EnumDeserializer {
    variant: StdString,
    value: Option<Value>,
    options: Options,
    visited: Rc<RefCell<FxHashSet<*const c_void>>>,
}

impl<'de> de::EnumAccess<'de> for EnumDeserializer {
    type Error = Error;
    type Variant = VariantDeserializer;

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

struct VariantDeserializer {
    value: Option<Value>,
    options: Options,
    visited: Rc<RefCell<FxHashSet<*const c_void>>>,
}

impl<'de> de::VariantAccess<'de> for VariantDeserializer {
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
            Some(value) => seed.deserialize(Deserializer::from_parts(value, self.options, self.visited)),
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
pub(crate) struct RecursionGuard {
    ptr: *const c_void,
    visited: Rc<RefCell<FxHashSet<*const c_void>>>,
}

impl RecursionGuard {
    #[inline]
    pub(crate) fn new(table: &Table, visited: &Rc<RefCell<FxHashSet<*const c_void>>>) -> Self {
        let visited = Rc::clone(visited);
        let ptr = table.to_pointer();
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
pub(crate) fn check_value_for_skip(
    value: &Value,
    options: Options,
    visited: &RefCell<FxHashSet<*const c_void>>,
) -> StdResult<bool, &'static str> {
    match value {
        Value::Table(table) => {
            let ptr = table.to_pointer();
            if visited.borrow().contains(&ptr) {
                if options.deny_recursive_tables {
                    return Err("recursive table detected");
                }
                return Ok(true); // skip
            }
        }
        Value::UserData(ud) if ud.is_serializable() => {}
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

fn serde_userdata<V>(
    ud: AnyUserData,
    f: impl FnOnce(serde_value::Value) -> std::result::Result<V, serde_value::DeserializerError>,
) -> Result<V> {
    match serde_value::to_value(ud) {
        Ok(value) => match f(value) {
            Ok(r) => Ok(r),
            Err(error) => Err(Error::DeserializeError(error.to_string())),
        },
        Err(error) => Err(Error::SerializeError(error.to_string())),
    }
}
