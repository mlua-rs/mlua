use std::borrow::Borrow;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::os::raw::{c_int, c_void};
use std::string::String as StdString;
use std::{cmp, fmt, slice, str};

#[cfg(feature = "serialize")]
use {
    serde::ser::{Serialize, Serializer},
    std::result::Result as StdResult,
};

use crate::error::{Error, Result};
use crate::state::Lua;
use crate::types::{LuaType, ValueRef};

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
    pub fn to_str(&self) -> Result<BorrowedStr> {
        let BorrowedBytes(bytes, guard) = self.as_bytes();
        let s = str::from_utf8(bytes).map_err(|e| Error::FromLuaConversionError {
            from: "string",
            to: "&str".to_string(),
            message: Some(e.to_string()),
        })?;
        Ok(BorrowedStr(s, guard))
    }

    /// Converts this string to a [`StdString`].
    ///
    /// Any non-Unicode sequences are replaced with [`U+FFFD REPLACEMENT CHARACTER`][U+FFFD].
    ///
    /// [U+FFFD]: std::char::REPLACEMENT_CHARACTER
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

    /// Get the bytes that make up this string.
    ///
    /// The returned slice will not contain the terminating nul byte, but will contain any nul
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
    pub fn as_bytes(&self) -> BorrowedBytes {
        let (bytes, guard) = unsafe { self.to_slice() };
        BorrowedBytes(&bytes[..bytes.len() - 1], guard)
    }

    /// Get the bytes that make up this string, including the trailing nul byte.
    pub fn as_bytes_with_nul(&self) -> BorrowedBytes {
        let (bytes, guard) = unsafe { self.to_slice() };
        BorrowedBytes(bytes, guard)
    }

    unsafe fn to_slice(&self) -> (&[u8], Lua) {
        let lua = self.0.lua.upgrade();
        let slice = unsafe {
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
            slice::from_raw_parts(data as *const u8, size + 1)
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

// Lua strings are basically &[u8] slices, so implement PartialEq for anything resembling that.
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

impl PartialEq<String> for String {
    fn eq(&self, other: &String) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl PartialEq<&String> for String {
    fn eq(&self, other: &&String) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for String {}

impl Hash for String {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

#[cfg(feature = "serialize")]
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

/// A borrowed string (`&str`) that holds a strong reference to the Lua state.
pub struct BorrowedStr<'a>(&'a str, #[allow(unused)] Lua);

impl Deref for BorrowedStr<'_> {
    type Target = str;

    #[inline(always)]
    fn deref(&self) -> &str {
        self.0
    }
}

impl Borrow<str> for BorrowedStr<'_> {
    #[inline(always)]
    fn borrow(&self) -> &str {
        self.0
    }
}

impl AsRef<str> for BorrowedStr<'_> {
    #[inline(always)]
    fn as_ref(&self) -> &str {
        self.0
    }
}

impl fmt::Display for BorrowedStr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Debug for BorrowedStr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<T> PartialEq<T> for BorrowedStr<'_>
where
    T: AsRef<str>,
{
    fn eq(&self, other: &T) -> bool {
        self.0 == other.as_ref()
    }
}

impl<T> PartialOrd<T> for BorrowedStr<'_>
where
    T: AsRef<str>,
{
    fn partial_cmp(&self, other: &T) -> Option<cmp::Ordering> {
        self.0.partial_cmp(other.as_ref())
    }
}

/// A borrowed byte slice (`&[u8]`) that holds a strong reference to the Lua state.
pub struct BorrowedBytes<'a>(&'a [u8], #[allow(unused)] Lua);

impl Deref for BorrowedBytes<'_> {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &[u8] {
        self.0
    }
}

impl Borrow<[u8]> for BorrowedBytes<'_> {
    #[inline(always)]
    fn borrow(&self) -> &[u8] {
        self.0
    }
}

impl AsRef<[u8]> for BorrowedBytes<'_> {
    #[inline(always)]
    fn as_ref(&self) -> &[u8] {
        self.0
    }
}

impl fmt::Debug for BorrowedBytes<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<T> PartialEq<T> for BorrowedBytes<'_>
where
    T: AsRef<[u8]>,
{
    fn eq(&self, other: &T) -> bool {
        self.0 == other.as_ref()
    }
}

impl<T> PartialOrd<T> for BorrowedBytes<'_>
where
    T: AsRef<[u8]>,
{
    fn partial_cmp(&self, other: &T) -> Option<cmp::Ordering> {
        self.0.partial_cmp(other.as_ref())
    }
}

impl<'a> IntoIterator for BorrowedBytes<'a> {
    type Item = &'a u8;
    type IntoIter = slice::Iter<'a, u8>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
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
