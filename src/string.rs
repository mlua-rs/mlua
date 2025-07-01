use std::borrow::{Borrow, Cow};
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::os::raw::{c_int, c_void};
use std::string::String as StdString;
use std::{cmp, fmt, slice, str};

use crate::error::{Error, Result};
use crate::state::Lua;
use crate::traits::IntoLua;
use crate::types::{LuaType, ValueRef};
use crate::value::Value;

#[cfg(feature = "serde")]
use {
    serde::ser::{Serialize, Serializer},
    std::result::Result as StdResult,
};

/// Handle to an internal Lua string.
///
/// Unlike Rust strings, Lua strings may not be valid UTF-8.
#[derive(Clone)]
pub struct String(pub(crate) ValueRef);

impl String {
    /// Get a [`BorrowedStr`] if the Lua string is valid UTF-8.
    ///
    /// # Examples
    ///
    /// ```
    /// # use mlua::{Lua, Result, String};
    /// # fn main() -> Result<()> {
    /// # let lua = Lua::new();
    /// let globals = lua.globals();
    ///
    /// let version: String = globals.get("_VERSION")?;
    /// assert!(version.to_str()?.contains("Lua"));
    ///
    /// let non_utf8: String = lua.load(r#"  "test\255"  "#).eval()?;
    /// assert!(non_utf8.to_str().is_err());
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn to_str(&self) -> Result<BorrowedStr<'_>> {
        BorrowedStr::try_from(self)
    }

    /// Converts this string to a [`StdString`].
    ///
    /// Any non-Unicode sequences are replaced with [`U+FFFD REPLACEMENT CHARACTER`][U+FFFD].
    ///
    /// This method returns [`StdString`] instead of [`Cow<'_, str>`] because lifetime cannot be
    /// bound to a weak Lua object.
    ///
    /// [U+FFFD]: std::char::REPLACEMENT_CHARACTER
    /// [`Cow<'_, str>`]: std::borrow::Cow
    ///
    /// # Examples
    ///
    /// ```
    /// # use mlua::{Lua, Result};
    /// # fn main() -> Result<()> {
    /// let lua = Lua::new();
    ///
    /// let s = lua.create_string(b"test\xff")?;
    /// assert_eq!(s.to_string_lossy(), "test\u{fffd}");
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn to_string_lossy(&self) -> StdString {
        StdString::from_utf8_lossy(&self.as_bytes()).into_owned()
    }

    /// Returns an object that implements [`Display`] for safely printing a Lua [`String`] that may
    /// contain non-Unicode data.
    ///
    /// This may perform lossy conversion.
    ///
    /// [`Display`]: fmt::Display
    pub fn display(&self) -> impl fmt::Display + '_ {
        Display(self)
    }

    /// Get the bytes that make up this string.
    ///
    /// The returned slice will not contain the terminating null byte, but will contain any null
    /// bytes embedded into the Lua string.
    ///
    /// # Examples
    ///
    /// ```
    /// # use mlua::{Lua, Result, String};
    /// # fn main() -> Result<()> {
    /// # let lua = Lua::new();
    /// let non_utf8: String = lua.load(r#"  "test\255"  "#).eval()?;
    /// assert!(non_utf8.to_str().is_err());    // oh no :(
    /// assert_eq!(non_utf8.as_bytes(), &b"test\xff"[..]);
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn as_bytes(&self) -> BorrowedBytes<'_> {
        BorrowedBytes::from(self)
    }

    /// Get the bytes that make up this string, including the trailing null byte.
    pub fn as_bytes_with_nul(&self) -> BorrowedBytes<'_> {
        let BorrowedBytes { buf, borrow, _lua } = BorrowedBytes::from(self);
        // Include the trailing null byte (it's always present but excluded by default)
        let buf = unsafe { slice::from_raw_parts((*buf).as_ptr(), (*buf).len() + 1) };
        BorrowedBytes { buf, borrow, _lua }
    }

    // Does not return the terminating null byte
    unsafe fn to_slice(&self) -> (&[u8], Lua) {
        let lua = self.0.lua.upgrade();
        let slice = {
            let rawlua = lua.lock();
            let ref_thread = rawlua.ref_thread();

            mlua_debug_assert!(
                ffi::lua_type(ref_thread, self.0.index) == ffi::LUA_TSTRING,
                "string ref is not string type"
            );

            // This will not trigger a 'm' error, because the reference is guaranteed to be of
            // string type
            let mut size = 0;
            let data = ffi::lua_tolstring(ref_thread, self.0.index, &mut size);
            slice::from_raw_parts(data as *const u8, size)
        };
        (slice, lua)
    }

    /// Converts this string to a generic C pointer.
    ///
    /// There is no way to convert the pointer back to its original value.
    ///
    /// Typically this function is used only for hashing and debug information.
    #[inline]
    pub fn to_pointer(&self) -> *const c_void {
        self.0.to_pointer()
    }
}

impl fmt::Debug for String {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let bytes = self.as_bytes();
        // Check if the string is valid utf8
        if let Ok(s) = str::from_utf8(&bytes) {
            return s.fmt(f);
        }

        // Format as bytes
        write!(f, "b")?;
        <bstr::BStr as fmt::Debug>::fmt(bstr::BStr::new(&bytes), f)
    }
}

// Lua strings are basically `&[u8]` slices, so implement `PartialEq` for anything resembling that.
//
// This makes our `String` comparable with `Vec<u8>`, `[u8]`, `&str` and `String`.
//
// The only downside is that this disallows a comparison with `Cow<str>`, as that only implements
// `AsRef<str>`, which collides with this impl. Requiring `AsRef<str>` would fix that, but limit us
// in other ways.
impl<T> PartialEq<T> for String
where
    T: AsRef<[u8]> + ?Sized,
{
    fn eq(&self, other: &T) -> bool {
        self.as_bytes() == other.as_ref()
    }
}

impl PartialEq for String {
    fn eq(&self, other: &String) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for String {}

impl<T> PartialOrd<T> for String
where
    T: AsRef<[u8]> + ?Sized,
{
    fn partial_cmp(&self, other: &T) -> Option<cmp::Ordering> {
        self.as_bytes().partial_cmp(&other.as_ref())
    }
}

impl PartialOrd for String {
    fn partial_cmp(&self, other: &String) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for String {
    fn cmp(&self, other: &String) -> cmp::Ordering {
        self.as_bytes().cmp(&other.as_bytes())
    }
}

impl Hash for String {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

#[cfg(feature = "serde")]
impl Serialize for String {
    fn serialize<S>(&self, serializer: S) -> StdResult<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.to_str() {
            Ok(s) => serializer.serialize_str(&s),
            Err(_) => serializer.serialize_bytes(&self.as_bytes()),
        }
    }
}

struct Display<'a>(&'a String);

impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let bytes = self.0.as_bytes();
        <bstr::BStr as fmt::Display>::fmt(bstr::BStr::new(&bytes), f)
    }
}

/// A borrowed string (`&str`) that holds a strong reference to the Lua state.
pub struct BorrowedStr<'a> {
    // `buf` points to a readonly memory managed by Lua
    pub(crate) buf: &'a str,
    pub(crate) borrow: Cow<'a, String>,
    pub(crate) _lua: Lua,
}

impl Deref for BorrowedStr<'_> {
    type Target = str;

    #[inline(always)]
    fn deref(&self) -> &str {
        self.buf
    }
}

impl Borrow<str> for BorrowedStr<'_> {
    #[inline(always)]
    fn borrow(&self) -> &str {
        self.buf
    }
}

impl AsRef<str> for BorrowedStr<'_> {
    #[inline(always)]
    fn as_ref(&self) -> &str {
        self.buf
    }
}

impl fmt::Display for BorrowedStr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.buf.fmt(f)
    }
}

impl fmt::Debug for BorrowedStr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.buf.fmt(f)
    }
}

impl<T> PartialEq<T> for BorrowedStr<'_>
where
    T: AsRef<str>,
{
    fn eq(&self, other: &T) -> bool {
        self.buf == other.as_ref()
    }
}

impl Eq for BorrowedStr<'_> {}

impl<T> PartialOrd<T> for BorrowedStr<'_>
where
    T: AsRef<str>,
{
    fn partial_cmp(&self, other: &T) -> Option<cmp::Ordering> {
        self.buf.partial_cmp(other.as_ref())
    }
}

impl Ord for BorrowedStr<'_> {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.buf.cmp(other.buf)
    }
}

impl<'a> TryFrom<&'a String> for BorrowedStr<'a> {
    type Error = Error;

    #[inline]
    fn try_from(value: &'a String) -> Result<Self> {
        let BorrowedBytes { buf, borrow, _lua } = BorrowedBytes::from(value);
        let buf = str::from_utf8(buf).map_err(|e| Error::FromLuaConversionError {
            from: "string",
            to: "&str".to_string(),
            message: Some(e.to_string()),
        })?;
        Ok(Self { buf, borrow, _lua })
    }
}

/// A borrowed byte slice (`&[u8]`) that holds a strong reference to the Lua state.
pub struct BorrowedBytes<'a> {
    // `buf` points to a readonly memory managed by Lua
    pub(crate) buf: &'a [u8],
    pub(crate) borrow: Cow<'a, String>,
    pub(crate) _lua: Lua,
}

impl Deref for BorrowedBytes<'_> {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &[u8] {
        self.buf
    }
}

impl Borrow<[u8]> for BorrowedBytes<'_> {
    #[inline(always)]
    fn borrow(&self) -> &[u8] {
        self.buf
    }
}

impl AsRef<[u8]> for BorrowedBytes<'_> {
    #[inline(always)]
    fn as_ref(&self) -> &[u8] {
        self.buf
    }
}

impl fmt::Debug for BorrowedBytes<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.buf.fmt(f)
    }
}

impl<T> PartialEq<T> for BorrowedBytes<'_>
where
    T: AsRef<[u8]>,
{
    fn eq(&self, other: &T) -> bool {
        self.buf == other.as_ref()
    }
}

impl Eq for BorrowedBytes<'_> {}

impl<T> PartialOrd<T> for BorrowedBytes<'_>
where
    T: AsRef<[u8]>,
{
    fn partial_cmp(&self, other: &T) -> Option<cmp::Ordering> {
        self.buf.partial_cmp(other.as_ref())
    }
}

impl Ord for BorrowedBytes<'_> {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.buf.cmp(other.buf)
    }
}

impl<'a> IntoIterator for &'a BorrowedBytes<'_> {
    type Item = &'a u8;
    type IntoIter = slice::Iter<'a, u8>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> From<&'a String> for BorrowedBytes<'a> {
    #[inline]
    fn from(value: &'a String) -> Self {
        let (buf, _lua) = unsafe { value.to_slice() };
        let borrow = Cow::Borrowed(value);
        Self { buf, borrow, _lua }
    }
}

struct WrappedString<T: AsRef<[u8]>>(T);

impl String {
    /// Wraps bytes, returning an opaque type that implements [`IntoLua`] trait.
    ///
    /// This function uses [`Lua::create_string`] under the hood.
    pub fn wrap(data: impl AsRef<[u8]>) -> impl IntoLua {
        WrappedString(data)
    }
}

impl<T: AsRef<[u8]>> IntoLua for WrappedString<T> {
    fn into_lua(self, lua: &Lua) -> Result<Value> {
        lua.create_string(self.0).map(Value::String)
    }
}

impl LuaType for String {
    const TYPE_ID: c_int = ffi::LUA_TSTRING;
}

#[cfg(test)]
mod assertions {
    use super::*;

    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_any!(String: Send);
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(String: Send, Sync);
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(BorrowedBytes: Send, Sync);
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(BorrowedStr: Send, Sync);
}
