use std::borrow::{Borrow, Cow};
use std::hash::{Hash, Hasher};
use std::os::raw::c_void;
use std::string::String as StdString;
use std::{slice, str};

#[cfg(feature = "serialize")]
use {
    serde::ser::{Serialize, Serializer},
    std::result::Result as StdResult,
};

use crate::error::{Error, Result};
use crate::ffi;
use crate::types::LuaRef;

/// Handle to an internal Lua string.
///
/// Unlike Rust strings, Lua strings may not be valid UTF-8.
#[derive(Clone, Debug)]
pub struct String<'lua>(pub(crate) LuaRef<'lua>);

impl<'lua> String<'lua> {
    /// Get a `&str` slice if the Lua string is valid UTF-8.
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
    pub fn to_str(&self) -> Result<&str> {
        str::from_utf8(self.as_bytes()).map_err(|e| Error::FromLuaConversionError {
            from: "string",
            to: "&str",
            message: Some(e.to_string()),
        })
    }

    /// Converts this string to a [`Cow<str>`].
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
    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        StdString::from_utf8_lossy(self.as_bytes())
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
    pub fn as_bytes(&self) -> &[u8] {
        let nulled = self.as_bytes_with_nul();
        &nulled[..nulled.len() - 1]
    }

    /// Get the bytes that make up this string, including the trailing nul byte.
    pub fn as_bytes_with_nul(&self) -> &[u8] {
        let ref_thread = self.0.lua.ref_thread();
        unsafe {
            mlua_debug_assert!(
                ffi::lua_type(ref_thread, self.0.index) == ffi::LUA_TSTRING,
                "string ref is not string type"
            );

            let mut size = 0;
            // This will not trigger a 'm' error, because the reference is guaranteed to be of
            // string type
            let data = ffi::lua_tolstring(ref_thread, self.0.index, &mut size);

            slice::from_raw_parts(data as *const u8, size + 1)
        }
    }

    /// Converts the string to a generic C pointer.
    ///
    /// There is no way to convert the pointer back to its original value.
    ///
    /// Typically this function is used only for hashing and debug information.
    #[inline]
    pub fn to_pointer(&self) -> *const c_void {
        let ref_thread = self.0.lua.ref_thread();
        unsafe { ffi::lua_topointer(ref_thread, self.0.index) }
    }
}

impl<'lua> AsRef<[u8]> for String<'lua> {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl<'lua> Borrow<[u8]> for String<'lua> {
    fn borrow(&self) -> &[u8] {
        self.as_bytes()
    }
}

// Lua strings are basically &[u8] slices, so implement PartialEq for anything resembling that.
//
// This makes our `String` comparable with `Vec<u8>`, `[u8]`, `&str`, `String` and `mlua::String`
// itself.
//
// The only downside is that this disallows a comparison with `Cow<str>`, as that only implements
// `AsRef<str>`, which collides with this impl. Requiring `AsRef<str>` would fix that, but limit us
// in other ways.
impl<'lua, T> PartialEq<T> for String<'lua>
where
    T: AsRef<[u8]>,
{
    fn eq(&self, other: &T) -> bool {
        self.as_bytes() == other.as_ref()
    }
}

impl<'lua> Eq for String<'lua> {}

impl<'lua> Hash for String<'lua> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

#[cfg(feature = "serialize")]
impl<'lua> Serialize for String<'lua> {
    fn serialize<S>(&self, serializer: S) -> StdResult<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.to_str() {
            Ok(s) => serializer.serialize_str(s),
            Err(_) => serializer.serialize_bytes(self.as_bytes()),
        }
    }
}
