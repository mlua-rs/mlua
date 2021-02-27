use std::string::String as StdString;

use serde::de::{self, IntoDeserializer};

use crate::error::{Error, Result};
use crate::table::{TablePairs, TableSequence};
use crate::value::Value;

/// A struct for deserializing Lua values into Rust values.
pub struct Deserializer<'lua>(pub Value<'lua>);

impl<'lua, 'de> serde::Deserializer<'de> for Deserializer<'lua> {
    type Error = Error;

    #[inline]
    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.0 {
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
            | Value::Error(_) => Err(de::Error::custom("invalid value type")),
        }
    }

    #[inline]
    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.0 {
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
        let (variant, value) = match self.0 {
            Value::Table(value) => {
                let mut iter = value.pairs::<StdString, Value>();
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
                (variant, Some(value))
            }
            Value::String(variant) => (variant.to_str()?.to_owned(), None),
            _ => return Err(de::Error::custom("bad enum value")),
        };

        visitor.visit_enum(EnumDeserializer { variant, value })
    }

    #[inline]
    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.0 {
            Value::Table(t) => {
                let len = t.raw_len() as usize;
                let mut deserializer = SeqDeserializer(t.raw_sequence_values());
                let seq = visitor.visit_seq(&mut deserializer)?;
                if deserializer.0.count() == 0 {
                    Ok(seq)
                } else {
                    Err(de::Error::invalid_length(
                        len,
                        &"fewer elements in the table",
                    ))
                }
            }
            _ => Err(de::Error::custom("invalid value type")),
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
        match self.0 {
            Value::Table(t) => {
                let mut deserializer = MapDeserializer::new(t.pairs());
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
            _ => Err(de::Error::custom("invalid value type")),
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
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes
        byte_buf unit unit_struct newtype_struct
        identifier ignored_any
    }
}

struct SeqDeserializer<'lua>(TableSequence<'lua, Value<'lua>>);

impl<'lua, 'de> de::SeqAccess<'de> for SeqDeserializer<'lua> {
    type Error = Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>>
    where
        T: de::DeserializeSeed<'de>,
    {
        match self.0.next() {
            Some(value) => seed.deserialize(Deserializer(value?)).map(Some),
            None => Ok(None),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        match self.0.size_hint() {
            (lower, Some(upper)) if lower == upper => Some(upper),
            _ => None,
        }
    }
}

struct MapDeserializer<'lua> {
    pairs: TablePairs<'lua, Value<'lua>, Value<'lua>>,
    value: Option<Value<'lua>>,
    processed: usize,
}

impl<'lua> MapDeserializer<'lua> {
    fn new(pairs: TablePairs<'lua, Value<'lua>, Value<'lua>>) -> Self {
        MapDeserializer {
            pairs,
            value: None,
            processed: 0,
        }
    }
}

impl<'lua, 'de> de::MapAccess<'de> for MapDeserializer<'lua> {
    type Error = Error;

    fn next_key_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>>
    where
        T: de::DeserializeSeed<'de>,
    {
        match self.pairs.next() {
            Some(item) => {
                let (key, value) = item?;
                self.processed += 1;
                self.value = Some(value);
                let key_de = Deserializer(key);
                seed.deserialize(key_de).map(Some)
            }
            None => Ok(None),
        }
    }

    fn next_value_seed<T>(&mut self, seed: T) -> Result<T::Value>
    where
        T: de::DeserializeSeed<'de>,
    {
        match self.value.take() {
            Some(value) => seed.deserialize(Deserializer(value)),
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
}

impl<'lua, 'de> de::EnumAccess<'de> for EnumDeserializer<'lua> {
    type Error = Error;
    type Variant = VariantDeserializer<'lua>;

    fn variant_seed<T>(self, seed: T) -> Result<(T::Value, Self::Variant)>
    where
        T: de::DeserializeSeed<'de>,
    {
        let variant = self.variant.into_deserializer();
        let variant_access = VariantDeserializer { value: self.value };
        seed.deserialize(variant).map(|v| (v, variant_access))
    }
}

struct VariantDeserializer<'lua> {
    value: Option<Value<'lua>>,
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
            Some(value) => seed.deserialize(Deserializer(value)),
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
            Some(value) => serde::Deserializer::deserialize_seq(Deserializer(value), visitor),
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
            Some(value) => serde::Deserializer::deserialize_map(Deserializer(value), visitor),
            None => Err(de::Error::invalid_type(
                de::Unexpected::UnitVariant,
                &"struct variant",
            )),
        }
    }
}
