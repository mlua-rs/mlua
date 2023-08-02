use num_traits::FromPrimitive;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::iter::{self, FromIterator};
use std::ops::Index;
use std::os::raw::{c_int, c_void};
use std::string::String as StdString;
use std::sync::Arc;
use std::{fmt, mem, ptr, slice, str, vec};

#[cfg(feature = "serialize")]
use {
    serde::ser::{self, Serialize, Serializer},
    std::convert::TryInto,
    std::result::Result as StdResult,
};

use crate::error::{Error, Result};
use crate::function::Function;
use crate::lua::Lua;
use crate::string::String;
use crate::table::Table;
use crate::thread::Thread;
use crate::types::{Integer, LightUserData, Number};
use crate::userdata::AnyUserData;
use crate::util::{check_stack, StackGuard};

/// A dynamically typed Lua value. The `String`, `Table`, `Function`, `Thread`, and `UserData`
/// variants contain handle types into the internal Lua state. It is a logic error to mix handle
/// types between separate `Lua` instances, and doing so will result in a panic.
#[derive(Clone)]
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
    Vector(crate::types::Vector),
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
    /// A special value (lightuserdata) to represent null value.
    ///
    /// It can be used in Lua tables without downsides of `nil`.
    pub const NULL: Value<'static> = Value::LightUserData(LightUserData(ptr::null_mut()));

    /// Returns type name of this value.
    pub const fn type_name(&self) -> &'static str {
        match *self {
            Value::Nil => "nil",
            Value::Boolean(_) => "boolean",
            Value::LightUserData(_) => "lightuserdata",
            Value::Integer(_) => "integer",
            Value::Number(_) => "number",
            #[cfg(feature = "luau")]
            Value::Vector(_) => "vector",
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
            (a, b) => Ok(a == b),
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
        match self {
            Value::LightUserData(ud) => ud.0,
            Value::String(String(r))
            | Value::Table(Table(r))
            | Value::Function(Function(r))
            | Value::Thread(Thread(r))
            | Value::UserData(AnyUserData(r)) => r.to_pointer(),
            _ => ptr::null(),
        }
    }

    /// Converts the value to a string.
    ///
    /// If the value has a metatable with a `__tostring` method, then it will be called to get the result.
    pub fn to_string(&self) -> Result<StdString> {
        match self {
            Value::Nil => Ok("nil".to_string()),
            Value::Boolean(b) => Ok(b.to_string()),
            Value::LightUserData(ud) if ud.0.is_null() => Ok("null".to_string()),
            Value::LightUserData(ud) => Ok(format!("lightuserdata: {:p}", ud.0)),
            Value::Integer(i) => Ok(i.to_string()),
            Value::Number(n) => Ok(n.to_string()),
            #[cfg(feature = "luau")]
            Value::Vector(v) => Ok(v.to_string()),
            Value::String(s) => Ok(s.to_str()?.to_string()),
            Value::Table(Table(r))
            | Value::Function(Function(r))
            | Value::Thread(Thread(r))
            | Value::UserData(AnyUserData(r)) => unsafe {
                let state = r.lua.state();
                let _guard = StackGuard::new(state);
                check_stack(state, 3)?;

                r.lua.push_ref(r);
                protect_lua!(state, 1, 1, fn(state) {
                    ffi::luaL_tolstring(state, -1, ptr::null_mut());
                })?;
                Ok(String(r.lua.pop_ref()).to_str()?.to_string())
            },
            Value::Error(err) => Ok(err.to_string()),
        }
    }

    /// Returns true if the value is a nil. Returns false otherwise.
    pub fn is_nil(&self) -> bool {
        self == &Nil
    }

    /// Returns true if the value is a [`NULL`]. Returns false otherwise.
    pub fn is_null(&self) -> bool {
        self == &Self::NULL
    }

    /// Returns true if the value is a boolean. Returns false otherwise.
    pub fn is_boolean(&self) -> bool {
        self.as_bool().is_some()
    }

    /// Cast the value to boolean.
    ///
    /// If the value is a Boolean, returns the associated bool. Returns None otherwise.
    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Value::Boolean(b) => Some(b),
            _ => None,
        }
    }

    /// Returns true if the value is a lightuserdata. Returns false otherwise.
    pub fn is_light_userdata(&self) -> bool {
        self.as_light_userdata().is_some()
    }

    /// Cast the value to lightuserdata.
    ///
    /// If the value is a LightUserData, returns the associated pointer. Returns None otherwise.
    pub fn as_light_userdata(&self) -> Option<LightUserData> {
        match *self {
            Value::LightUserData(l) => Some(l),
            _ => None,
        }
    }

    /// Returns true if the value is an integer. Returns false otherwise.
    pub fn is_integer(&self) -> bool {
        self.as_integer().is_some()
    }

    /// Cast the value to integer.
    ///
    /// If the value is an Integer, returns the associated Integer. Returns None otherwise.
    pub fn as_integer(&self) -> Option<Integer> {
        match *self {
            Value::Integer(i) => Some(i),
            _ => None,
        }
    }

    /// Cast the value to i32.
    ///
    /// If the value is an Integer, try to convert it to i32. Returns None otherwise.
    pub fn as_i32(&self) -> Option<i32> {
        self.as_integer().and_then(|i| i32::try_from(i).ok())
    }

    /// Cast the value to u32.
    ///
    /// If the value is an Integer, try to convert it to u32. Returns None otherwise.
    pub fn as_u32(&self) -> Option<u32> {
        self.as_integer().and_then(|i| u32::try_from(i).ok())
    }

    /// Cast the value to i64.
    ///
    /// If the value is an Integer, try to convert it to i64. Returns None otherwise.
    #[cfg(any(
        feature = "luau",
        all(any(feature = "lua51", feature = "lua52"), target_pointer_width = "32")
    ))]
    pub fn as_i64(&self) -> Option<i64> {
        self.as_integer().and_then(|i| i64::try_from(i).ok())
    }

    /// Cast the value to i64.
    ///
    /// If the value is an Integer, try to convert it to i64. Returns None otherwise.
    #[cfg(not(any(
        feature = "luau",
        all(any(feature = "lua51", feature = "lua52"), target_pointer_width = "32")
    )))]
    pub fn as_i64(&self) -> Option<i64> {
        self.as_integer()
    }

    /// Cast the value to u64.
    ///
    /// If the value is an Integer, try to convert it to u64. Returns None otherwise.
    pub fn as_u64(&self) -> Option<u64> {
        match *self {
            Value::Integer(i) => u64::try_from(i).ok(),
            _ => None,
        }
    }

    /// Cast the value to isize.
    ///
    /// If the value is an Integer, try to convert it to isize. Returns None otherwise.
    pub fn as_isize(&self) -> Option<isize> {
        match *self {
            Value::Integer(i) => isize::try_from(i).ok(),
            _ => None,
        }
    }

    /// Cast the value to usize.
    ///
    /// If the value is an Integer, try to convert it to usize. Returns None otherwise.
    pub fn as_usize(&self) -> Option<usize> {
        match *self {
            Value::Integer(i) => usize::try_from(i).ok(),
            _ => None,
        }
    }

    /// Returns true if the value is a number. Returns false otherwise.
    pub fn is_number(&self) -> bool {
        self.as_number().is_some()
    }

    /// Cast the value to number.
    ///
    /// If the value is a Number, returns the associated Number. Returns None otherwise.
    pub fn as_number(&self) -> Option<Number> {
        match *self {
            Value::Number(n) => Some(n),
            _ => None,
        }
    }

    /// Cast the value to f32.
    ///
    /// If the value is a Number, try to convert it to f32. Returns None otherwise.
    pub fn as_f32(&self) -> Option<f32> {
        match *self {
            Value::Number(n) => f32::from_f64(n),
            _ => None,
        }
    }

    /// Cast the value to f64.
    ///
    /// If the value is a Number, try to convert it to f64. Returns None otherwise.
    pub fn as_f64(&self) -> Option<f64> {
        self.as_number()
    }

    /// Returns true if the value is a string. Returns false otherwise.
    pub fn is_string(&self) -> bool {
        self.as_string().is_some()
    }

    /// Cast the value to lua string.
    ///
    /// If the value is a String, returns the associated String. Returns None otherwise.
    pub fn as_string(&self) -> Option<&String> {
        match *self {
            Value::String(ref s) => Some(s),
            _ => None,
        }
    }

    /// Cast the value to [`str`]
    ///
    /// If the value is a String, try to convert it to [`str`]. Returns None otherwise.
    pub fn as_str(&self) -> Option<&str> {
        match *self {
            Value::String(ref s) => s.to_str().ok(),
            _ => None,
        }
    }

    /// Cast the value to [`Cow<str>`].
    ///
    /// If the value is a String, convert it to [`Cow<str>`]. Returns None otherwise.
    pub fn as_string_lossy(&self) -> Option<Cow<str>> {
        match *self {
            Value::String(ref s) => Some(s.to_string_lossy()),
            _ => None,
        }
    }

    /// Returns true if the value is a table. Returns false otherwise.
    pub fn is_table(&self) -> bool {
        self.as_table().is_some()
    }

    /// Cast the value to table.
    ///
    /// If the value is a Table, returns the associated Table. Returns None otherwise.
    pub fn as_table(&self) -> Option<&Table> {
        match *self {
            Value::Table(ref t) => Some(t),
            _ => None,
        }
    }

    /// Returns true if the value is a thread. Returns false otherwise.
    pub fn is_thread(&self) -> bool {
        self.as_thread().is_some()
    }

    /// Cast the value to thread.
    ///
    /// If the value is a Thread, returns the associated Thread. Returns None otherwise.
    pub fn as_thread(&self) -> Option<&Thread> {
        match *self {
            Value::Thread(ref t) => Some(t),
            _ => None,
        }
    }

    /// Returns true if the value is a function. Returns false otherwise.
    pub fn is_function(&self) -> bool {
        self.as_function().is_some()
    }

    /// Cast the value to function.
    ///
    /// If the value is a Function, returns the associated Function. Returns None otherwise.
    pub fn as_function(&self) -> Option<&Function> {
        match *self {
            Value::Function(ref f) => Some(f),
            _ => None,
        }
    }

    /// Returns true if the value is a userdata. Returns false otherwise.
    pub fn is_userdata(&self) -> bool {
        self.as_userdata().is_some()
    }

    /// Cast the value to userdata.
    ///
    /// If the value is a UserData, returns the associated UserData. Returns None otherwise.
    pub fn as_userdata(&self) -> Option<&AnyUserData> {
        match *self {
            Value::UserData(ref u) => Some(u),
            _ => None,
        }
    }

    // Compares two values.
    // Used to sort values for Debug printing.
    pub(crate) fn cmp(&self, other: &Self) -> Ordering {
        fn cmp_num(a: Number, b: Number) -> Ordering {
            match (a, b) {
                _ if a < b => Ordering::Less,
                _ if a > b => Ordering::Greater,
                _ => Ordering::Equal,
            }
        }

        match (self, other) {
            // Nil
            (Value::Nil, Value::Nil) => Ordering::Equal,
            (Value::Nil, _) => Ordering::Less,
            (_, Value::Nil) => Ordering::Greater,
            // Null (a special case)
            (Value::LightUserData(ud1), Value::LightUserData(ud2)) if ud1 == ud2 => Ordering::Equal,
            (Value::LightUserData(ud1), _) if ud1.0.is_null() => Ordering::Less,
            (_, Value::LightUserData(ud2)) if ud2.0.is_null() => Ordering::Greater,
            // Boolean
            (Value::Boolean(a), Value::Boolean(b)) => a.cmp(b),
            (Value::Boolean(_), _) => Ordering::Less,
            (_, Value::Boolean(_)) => Ordering::Greater,
            // Integer && Number
            (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
            (&Value::Integer(a), &Value::Number(b)) => cmp_num(a as Number, b),
            (&Value::Number(a), &Value::Integer(b)) => cmp_num(a, b as Number),
            (&Value::Number(a), &Value::Number(b)) => cmp_num(a, b),
            (Value::Integer(_) | Value::Number(_), _) => Ordering::Less,
            (_, Value::Integer(_) | Value::Number(_)) => Ordering::Greater,
            // String
            (Value::String(a), Value::String(b)) => a.as_bytes().cmp(b.as_bytes()),
            (Value::String(_), _) => Ordering::Less,
            (_, Value::String(_)) => Ordering::Greater,
            // Other variants can be randomly ordered
            (a, b) => a.to_pointer().cmp(&b.to_pointer()),
        }
    }

    pub(crate) fn fmt_pretty(
        &self,
        fmt: &mut fmt::Formatter,
        recursive: bool,
        ident: usize,
        visited: &mut HashSet<*const c_void>,
    ) -> fmt::Result {
        match self {
            Value::Nil => write!(fmt, "nil"),
            Value::Boolean(b) => write!(fmt, "{b}"),
            Value::LightUserData(ud) if ud.0.is_null() => write!(fmt, "null"),
            Value::LightUserData(ud) => write!(fmt, "lightuserdata: {:?}", ud.0),
            Value::Integer(i) => write!(fmt, "{i}"),
            Value::Number(n) => write!(fmt, "{n}"),
            #[cfg(feature = "luau")]
            Value::Vector(v) => write!(fmt, "{v}"),
            Value::String(s) => write!(fmt, "{s:?}"),
            Value::Table(t) if recursive && !visited.contains(&t.to_pointer()) => {
                t.fmt_pretty(fmt, ident, visited)
            }
            t @ Value::Table(_) => write!(fmt, "table: {:?}", t.to_pointer()),
            f @ Value::Function(_) => write!(fmt, "function: {:?}", f.to_pointer()),
            t @ Value::Thread(_) => write!(fmt, "thread: {:?}", t.to_pointer()),
            u @ Value::UserData(ud) => {
                // Try `__name/__type` first then `__tostring`
                let name = ud.type_name().ok().flatten();
                let s = name
                    .map(|name| format!("{name}: {:?}", u.to_pointer()))
                    .or_else(|| u.to_string().ok())
                    .unwrap_or_else(|| format!("userdata: {:?}", u.to_pointer()));
                write!(fmt, "{s}")
            }
            Value::Error(e) if recursive => write!(fmt, "{e:?}"),
            Value::Error(_) => write!(fmt, "error"),
        }
    }
}

impl fmt::Debug for Value<'_> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        if fmt.alternate() {
            return self.fmt_pretty(fmt, true, 0, &mut HashSet::new());
        }
        match self {
            Value::Nil => write!(fmt, "Nil"),
            Value::Boolean(b) => write!(fmt, "Boolean({b})"),
            Value::LightUserData(ud) => write!(fmt, "{ud:?}"),
            Value::Integer(i) => write!(fmt, "Integer({i})"),
            Value::Number(n) => write!(fmt, "Number({n})"),
            #[cfg(feature = "luau")]
            Value::Vector(v) => write!(fmt, "{v:?}"),
            Value::String(s) => write!(fmt, "String({s:?})"),
            Value::Table(t) => write!(fmt, "{t:?}"),
            Value::Function(f) => write!(fmt, "{f:?}"),
            Value::Thread(t) => write!(fmt, "{t:?}"),
            Value::UserData(ud) => write!(fmt, "{ud:?}"),
            Value::Error(e) => write!(fmt, "Error({e:?})"),
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
            (Value::Vector(v1), Value::Vector(v2)) => v1 == v2,
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
                .serialize_i64((*i).try_into().expect("cannot convert Lua Integer to i64")),
            Value::Number(n) => serializer.serialize_f64(*n),
            #[cfg(feature = "luau")]
            Value::Vector(v) => v.serialize(serializer),
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
pub trait IntoLua<'lua>: Sized {
    /// Performs the conversion.
    fn into_lua(self, lua: &'lua Lua) -> Result<Value<'lua>>;

    /// Pushes the value into the Lua stack.
    ///
    /// # Safety
    /// This method does not check Lua stack space.
    #[doc(hidden)]
    #[inline]
    unsafe fn push_into_stack(self, lua: &'lua Lua) -> Result<()> {
        lua.push_value(self.into_lua(lua)?)
    }
}

/// Trait for types convertible from `Value`.
pub trait FromLua<'lua>: Sized {
    /// Performs the conversion.
    fn from_lua(value: Value<'lua>, lua: &'lua Lua) -> Result<Self>;

    /// Performs the conversion for an argument (eg. function argument).
    ///
    /// `i` is the argument index (position),
    /// `to` is a function name that received the argument.
    #[doc(hidden)]
    #[inline]
    fn from_lua_arg(arg: Value<'lua>, i: usize, to: Option<&str>, lua: &'lua Lua) -> Result<Self> {
        Self::from_lua(arg, lua).map_err(|err| Error::BadArgument {
            to: to.map(|s| s.to_string()),
            pos: i,
            name: None,
            cause: Arc::new(err),
        })
    }

    /// Performs the conversion for a value in the Lua stack at index `idx`.
    #[doc(hidden)]
    #[inline]
    unsafe fn from_stack(idx: c_int, lua: &'lua Lua) -> Result<Self> {
        Self::from_lua(lua.stack_value(idx), lua)
    }

    /// Same as `from_lua_arg` but for a value in the Lua stack at index `idx`.
    #[doc(hidden)]
    #[inline]
    unsafe fn from_stack_arg(
        idx: c_int,
        i: usize,
        to: Option<&str>,
        lua: &'lua Lua,
    ) -> Result<Self> {
        Self::from_stack(idx, lua).map_err(|err| Error::BadArgument {
            to: to.map(|s| s.to_string()),
            pos: i,
            name: None,
            cause: Arc::new(err),
        })
    }
}

/// Multiple Lua values used for both argument passing and also for multiple return values.
#[derive(Debug, Clone)]
pub struct MultiValue<'lua> {
    vec: Vec<Value<'lua>>,
    lua: Option<&'lua Lua>,
}

impl Drop for MultiValue<'_> {
    fn drop(&mut self) {
        if let Some(lua) = self.lua {
            let vec = mem::replace(&mut self.vec, Vec::new());
            lua.push_multivalue_to_pool(vec);
        }
    }
}

impl<'lua> MultiValue<'lua> {
    /// Creates an empty `MultiValue` containing no values.
    pub const fn new() -> MultiValue<'lua> {
        MultiValue {
            vec: Vec::new(),
            lua: None,
        }
    }

    /// Similar to `new` but can reuse previously used container with allocated capacity.
    #[inline]
    pub(crate) fn with_lua_and_capacity(lua: &'lua Lua, capacity: usize) -> MultiValue<'lua> {
        let vec = lua
            .pop_multivalue_from_pool()
            .map(|mut vec| {
                vec.reserve(capacity);
                vec
            })
            .unwrap_or_else(|| Vec::with_capacity(capacity));
        MultiValue {
            vec,
            lua: Some(lua),
        }
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
    fn into_iter(mut self) -> Self::IntoIter {
        let vec = mem::replace(&mut self.vec, Vec::new());
        mem::forget(self);
        vec.into_iter().rev()
    }
}

impl<'a, 'lua> IntoIterator for &'a MultiValue<'lua> {
    type Item = &'a Value<'lua>;
    type IntoIter = iter::Rev<slice::Iter<'a, Value<'lua>>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.vec.iter().rev()
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
    pub fn from_vec(mut vec: Vec<Value<'lua>>) -> MultiValue<'lua> {
        vec.reverse();
        MultiValue { vec, lua: None }
    }

    #[inline]
    pub fn into_vec(mut self) -> Vec<Value<'lua>> {
        let mut vec = mem::replace(&mut self.vec, Vec::new());
        mem::forget(self);
        vec.reverse();
        vec
    }

    #[inline]
    pub fn get(&self, index: usize) -> Option<&Value<'lua>> {
        if index < self.vec.len() {
            return self.vec.get(self.vec.len() - index - 1);
        }
        None
    }

    #[inline]
    pub fn pop_front(&mut self) -> Option<Value<'lua>> {
        self.vec.pop()
    }

    #[inline]
    pub fn push_front(&mut self, value: Value<'lua>) {
        self.vec.push(value);
    }

    #[inline]
    pub fn clear(&mut self) {
        self.vec.clear();
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.vec.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.vec.is_empty()
    }

    #[inline]
    pub fn iter(&self) -> iter::Rev<slice::Iter<Value<'lua>>> {
        self.vec.iter().rev()
    }

    #[inline]
    pub(crate) fn drain_all(&mut self) -> iter::Rev<vec::Drain<Value<'lua>>> {
        self.vec.drain(..).rev()
    }

    #[inline]
    pub(crate) fn refill(
        &mut self,
        iter: impl IntoIterator<Item = Result<Value<'lua>>>,
    ) -> Result<()> {
        self.vec.clear();
        for value in iter {
            self.vec.push(value?);
        }
        self.vec.reverse();
        Ok(())
    }
}

/// Trait for types convertible to any number of Lua values.
///
/// This is a generalization of `IntoLua`, allowing any number of resulting Lua values instead of just
/// one. Any type that implements `IntoLua` will automatically implement this trait.
pub trait IntoLuaMulti<'lua>: Sized {
    /// Performs the conversion.
    fn into_lua_multi(self, lua: &'lua Lua) -> Result<MultiValue<'lua>>;

    /// Pushes the values into the Lua stack.
    ///
    /// Returns number of pushed values.
    #[doc(hidden)]
    #[inline]
    unsafe fn push_into_stack_multi(self, lua: &'lua Lua) -> Result<c_int> {
        let mut values = self.into_lua_multi(lua)?;
        let len: c_int = values.len().try_into().unwrap();
        unsafe {
            check_stack(lua.state(), len + 1)?;
            for v in values.drain_all() {
                lua.push_value(v)?;
            }
        }
        Ok(len)
    }
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

    /// Performs the conversion for a list of arguments.
    ///
    /// `i` is an index (position) of the first argument,
    /// `to` is a function name that received the arguments.
    #[doc(hidden)]
    #[inline]
    fn from_lua_args(
        args: MultiValue<'lua>,
        i: usize,
        to: Option<&str>,
        lua: &'lua Lua,
    ) -> Result<Self> {
        let _ = (i, to);
        Self::from_lua_multi(args, lua)
    }

    /// Performs the conversion for a number of values in the Lua stack.
    #[doc(hidden)]
    #[inline]
    unsafe fn from_stack_multi(nvals: c_int, lua: &'lua Lua) -> Result<Self> {
        let mut values = MultiValue::with_lua_and_capacity(lua, nvals as usize);
        for idx in 1..=nvals {
            values.push_front(lua.stack_value(-idx));
        }
        if nvals > 0 {
            // It's safe to clear the stack as all references moved to ref thread
            ffi::lua_pop(lua.state(), nvals);
        }
        Self::from_lua_multi(values, lua)
    }

    /// Same as `from_lua_args` but for a number of values in the Lua stack.
    #[doc(hidden)]
    #[inline]
    unsafe fn from_stack_args(
        nargs: c_int,
        i: usize,
        to: Option<&str>,
        lua: &'lua Lua,
    ) -> Result<Self> {
        let _ = (i, to);
        Self::from_stack_multi(nargs, lua)
    }
}

#[cfg(test)]
mod assertions {
    use super::*;

    static_assertions::assert_not_impl_any!(Value: Send);
    static_assertions::assert_not_impl_any!(MultiValue: Send);
}
