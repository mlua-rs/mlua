use std::iter::{self, FromIterator};
use std::ops::Index;
use std::os::raw::c_void;
use std::{ptr, slice, str, vec};

#[cfg(feature = "serialize")]
use {
    serde::ser::{self, Serialize, Serializer},
    std::convert::TryInto,
    std::result::Result as StdResult,
};

use crate::error::{Error, Result};
use crate::ffi;
use crate::function::Function;
use crate::lua::Lua;
use crate::string::String;
use crate::table::Table;
use crate::thread::Thread;
use crate::types::{Integer, LightUserData, Number};
use crate::userdata::AnyUserData;

/// A dynamically typed Lua value. The `String`, `Table`, `Function`, `Thread`, and `UserData`
/// variants contain handle types into the internal Lua state. It is a logic error to mix handle
/// types between separate `Lua` instances, and doing so will result in a panic.
#[derive(Debug, Clone)]
pub enum Value<'lua> {
    /// The Lua value `nil`.
    Nil,
    /// The Lua value `true` or `false`.
    Boolean(bool),
    /// A "light userdata" object, equivalent to a raw pointer.
    LightUserData(LightUserData),
    /// An integer number.
    ///
    /// Any Lua number convertible to a `Integer` will be represented as this variant.
    Integer(Integer),
    /// A floating point number.
    Number(Number),
    /// A Luau vector.
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    Vector(f32, f32, f32),
    /// An interned string, managed by Lua.
    ///
    /// Unlike Rust strings, Lua strings may not be valid UTF-8.
    String(String<'lua>),
    /// Reference to a Lua table.
    Table(Table<'lua>),
    /// Reference to a Lua function (or closure).
    Function(Function<'lua>),
    /// Reference to a Lua thread (or coroutine).
    Thread(Thread<'lua>),
    /// Reference to a userdata object that holds a custom type which implements `UserData`.
    /// Special builtin userdata types will be represented as other `Value` variants.
    UserData(AnyUserData<'lua>),
    /// `Error` is a special builtin userdata type. When received from Lua it is implicitly cloned.
    Error(Error),
}

pub use self::Value::Nil;

impl<'lua> Value<'lua> {
    pub const fn type_name(&self) -> &'static str {
        match *self {
            Value::Nil => "nil",
            Value::Boolean(_) => "boolean",
            Value::LightUserData(_) => "lightuserdata",
            Value::Integer(_) => "integer",
            Value::Number(_) => "number",
            #[cfg(feature = "luau")]
            Value::Vector(_, _, _) => "vector",
            Value::String(_) => "string",
            Value::Table(_) => "table",
            Value::Function(_) => "function",
            Value::Thread(_) => "thread",
            Value::UserData(_) => "userdata",
            Value::Error(_) => "error",
        }
    }

    /// Compares two values for equality.
    ///
    /// Equality comparisons do not convert strings to numbers or vice versa.
    /// Tables, Functions, Threads, and Userdata are compared by reference:
    /// two objects are considered equal only if they are the same object.
    ///
    /// If Tables or Userdata have `__eq` metamethod then mlua will try to invoke it.
    /// The first value is checked first. If that value does not define a metamethod
    /// for `__eq`, then mlua will check the second value.
    /// Then mlua calls the metamethod with the two values as arguments, if found.
    pub fn equals<T: AsRef<Self>>(&self, other: T) -> Result<bool> {
        match (self, other.as_ref()) {
            (Value::Table(a), Value::Table(b)) => a.equals(b),
            (Value::UserData(a), Value::UserData(b)) => a.equals(b),
            _ => Ok(self == other.as_ref()),
        }
    }

    /// Converts the value to a generic C pointer.
    ///
    /// The value can be a userdata, a table, a thread, a string, or a function; otherwise it returns NULL.
    /// Different objects will give different pointers.
    /// There is no way to convert the pointer back to its original value.
    ///
    /// Typically this function is used only for hashing and debug information.
    #[inline]
    pub fn to_pointer(&self) -> *const c_void {
        unsafe {
            match self {
                Value::LightUserData(ud) => ud.0,
                Value::Table(t) => t.to_pointer(),
                Value::String(s) => s.to_pointer(),
                Value::Function(Function(r))
                | Value::Thread(Thread(r))
                | Value::UserData(AnyUserData(r)) => {
                    ffi::lua_topointer(r.lua.ref_thread(), r.index)
                }
                _ => ptr::null(),
            }
        }
    }
}

impl<'lua> PartialEq for Value<'lua> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Boolean(a), Value::Boolean(b)) => a == b,
            (Value::LightUserData(a), Value::LightUserData(b)) => a == b,
            (Value::Integer(a), Value::Integer(b)) => *a == *b,
            (Value::Integer(a), Value::Number(b)) => *a as Number == *b,
            (Value::Number(a), Value::Integer(b)) => *a == *b as Number,
            (Value::Number(a), Value::Number(b)) => *a == *b,
            #[cfg(feature = "luau")]
            (Value::Vector(x1, y1, z1), Value::Vector(x2, y2, z2)) => (x1, y1, z1) == (x2, y2, z2),
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Table(a), Value::Table(b)) => a == b,
            (Value::Function(a), Value::Function(b)) => a == b,
            (Value::Thread(a), Value::Thread(b)) => a == b,
            (Value::UserData(a), Value::UserData(b)) => a == b,
            _ => false,
        }
    }
}

impl<'lua> AsRef<Value<'lua>> for Value<'lua> {
    #[inline]
    fn as_ref(&self) -> &Self {
        self
    }
}

#[cfg(feature = "serialize")]
impl<'lua> Serialize for Value<'lua> {
    fn serialize<S>(&self, serializer: S) -> StdResult<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Value::Nil => serializer.serialize_unit(),
            Value::Boolean(b) => serializer.serialize_bool(*b),
            #[allow(clippy::useless_conversion)]
            Value::Integer(i) => serializer
                .serialize_i64((*i).try_into().expect("cannot convert lua_Integer to i64")),
            #[allow(clippy::useless_conversion)]
            Value::Number(n) => serializer.serialize_f64(*n),
            #[cfg(feature = "luau")]
            Value::Vector(x, y, z) => (x, y, z).serialize(serializer),
            Value::String(s) => s.serialize(serializer),
            Value::Table(t) => t.serialize(serializer),
            Value::UserData(ud) => ud.serialize(serializer),
            Value::LightUserData(ud) if ud.0.is_null() => serializer.serialize_none(),
            Value::Error(_) | Value::LightUserData(_) | Value::Function(_) | Value::Thread(_) => {
                let msg = format!("cannot serialize <{}>", self.type_name());
                Err(ser::Error::custom(msg))
            }
        }
    }
}

/// Trait for types convertible to `Value`.
pub trait ToLua<'lua> {
    /// Performs the conversion.
    fn to_lua(self, lua: &'lua Lua) -> Result<Value<'lua>>;
}

/// Trait for types convertible from `Value`.
pub trait FromLua<'lua>: Sized {
    /// Performs the conversion.
    fn from_lua(lua_value: Value<'lua>, lua: &'lua Lua) -> Result<Self>;
}

/// Multiple Lua values used for both argument passing and also for multiple return values.
#[derive(Debug, Clone)]
pub struct MultiValue<'lua>(Vec<Value<'lua>>);

impl<'lua> MultiValue<'lua> {
    /// Creates an empty `MultiValue` containing no values.
    #[inline]
    pub const fn new() -> MultiValue<'lua> {
        MultiValue(Vec::new())
    }

    /// Similar to `new` but can return previously used container with allocated capacity.
    #[inline]
    pub(crate) fn new_or_cached(lua: &'lua Lua) -> MultiValue<'lua> {
        lua.new_or_cached_multivalue()
    }
}

impl<'lua> Default for MultiValue<'lua> {
    #[inline]
    fn default() -> MultiValue<'lua> {
        MultiValue::new()
    }
}

impl<'lua> FromIterator<Value<'lua>> for MultiValue<'lua> {
    #[inline]
    fn from_iter<I: IntoIterator<Item = Value<'lua>>>(iter: I) -> Self {
        MultiValue::from_vec(Vec::from_iter(iter))
    }
}

impl<'lua> IntoIterator for MultiValue<'lua> {
    type Item = Value<'lua>;
    type IntoIter = iter::Rev<vec::IntoIter<Value<'lua>>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter().rev()
    }
}

impl<'a, 'lua> IntoIterator for &'a MultiValue<'lua> {
    type Item = &'a Value<'lua>;
    type IntoIter = iter::Rev<slice::Iter<'a, Value<'lua>>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().rev()
    }
}

impl<'lua> Index<usize> for MultiValue<'lua> {
    type Output = Value<'lua>;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        if let Some(result) = self.get(index) {
            result
        } else {
            panic!(
                "index out of bounds: the len is {} but the index is {}",
                self.len(),
                index
            )
        }
    }
}

impl<'lua> MultiValue<'lua> {
    #[inline]
    pub fn from_vec(mut v: Vec<Value<'lua>>) -> MultiValue<'lua> {
        v.reverse();
        MultiValue(v)
    }

    #[inline]
    pub fn into_vec(self) -> Vec<Value<'lua>> {
        let mut v = self.0;
        v.reverse();
        v
    }

    #[inline]
    pub fn get(&self, index: usize) -> Option<&Value<'lua>> {
        self.0.get(self.0.len() - index - 1)
    }

    #[inline]
    pub(crate) fn reserve(&mut self, size: usize) {
        self.0.reserve(size);
    }

    #[inline]
    pub fn pop_front(&mut self) -> Option<Value<'lua>> {
        self.0.pop()
    }

    #[inline]
    pub fn push_front(&mut self, value: Value<'lua>) {
        self.0.push(value);
    }

    #[inline]
    pub fn clear(&mut self) {
        self.0.clear();
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[inline]
    pub fn iter(&self) -> iter::Rev<slice::Iter<Value<'lua>>> {
        self.0.iter().rev()
    }

    #[inline]
    pub(crate) fn drain_all(&mut self) -> iter::Rev<vec::Drain<Value<'lua>>> {
        self.0.drain(..).rev()
    }

    #[inline]
    pub(crate) fn refill(
        &mut self,
        iter: impl IntoIterator<Item = Result<Value<'lua>>>,
    ) -> Result<()> {
        self.0.clear();
        for value in iter {
            self.0.push(value?);
        }
        self.0.reverse();
        Ok(())
    }
}

/// Trait for types convertible to any number of Lua values.
///
/// This is a generalization of `ToLua`, allowing any number of resulting Lua values instead of just
/// one. Any type that implements `ToLua` will automatically implement this trait.
pub trait ToLuaMulti<'lua> {
    /// Performs the conversion.
    fn to_lua_multi(self, lua: &'lua Lua) -> Result<MultiValue<'lua>>;
}

/// Trait for types that can be created from an arbitrary number of Lua values.
///
/// This is a generalization of `FromLua`, allowing an arbitrary number of Lua values to participate
/// in the conversion. Any type that implements `FromLua` will automatically implement this trait.
pub trait FromLuaMulti<'lua>: Sized {
    /// Performs the conversion.
    ///
    /// In case `values` contains more values than needed to perform the conversion, the excess
    /// values should be ignored. This reflects the semantics of Lua when calling a function or
    /// assigning values. Similarly, if not enough values are given, conversions should assume that
    /// any missing values are nil.
    fn from_lua_multi(values: MultiValue<'lua>, lua: &'lua Lua) -> Result<Self>;
}
