#[cfg(feature = "serde")]
use serde::ser::{Serialize, Serializer};

use crate::state::RawLua;
use crate::types::ValueRef;

/// A Luau buffer type.
///
/// See the buffer [documentation] for more information.
///
/// [documentation]: https://luau.org/library#buffer-library
#[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
#[derive(Clone, Debug, PartialEq)]
pub struct Buffer(pub(crate) ValueRef);

#[cfg_attr(not(feature = "luau"), allow(unused))]
impl Buffer {
    /// Copies the buffer data into a new `Vec<u8>`.
    pub fn to_vec(&self) -> Vec<u8> {
        let lua = self.0.lua.lock();
        self.as_slice(&lua).to_vec()
    }

    /// Returns the length of the buffer.
    pub fn len(&self) -> usize {
        let lua = self.0.lua.lock();
        self.as_slice(&lua).len()
    }

    /// Returns `true` if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reads given number of bytes from the buffer at the given offset.
    ///
    /// Offset is 0-based.
    #[track_caller]
    pub fn read_bytes<const N: usize>(&self, offset: usize) -> [u8; N] {
        let lua = self.0.lua.lock();
        let data = self.as_slice(&lua);
        let mut bytes = [0u8; N];
        bytes.copy_from_slice(&data[offset..offset + N]);
        bytes
    }

    /// Writes given bytes to the buffer at the given offset.
    ///
    /// Offset is 0-based.
    #[track_caller]
    pub fn write_bytes(&self, offset: usize, bytes: &[u8]) {
        let lua = self.0.lua.lock();
        let data = unsafe {
            let (buf, size) = self.as_raw_parts(&lua);
            std::slice::from_raw_parts_mut(buf, size)
        };
        data[offset..offset + bytes.len()].copy_from_slice(bytes);
    }

    pub(crate) fn as_slice(&self, lua: &RawLua) -> &[u8] {
        unsafe {
            let (buf, size) = self.as_raw_parts(lua);
            std::slice::from_raw_parts(buf, size)
        }
    }

    #[cfg(feature = "luau")]
    unsafe fn as_raw_parts(&self, lua: &RawLua) -> (*mut u8, usize) {
        let mut size = 0usize;
        let buf = ffi::lua_tobuffer(lua.ref_thread(), self.0.index, &mut size);
        mlua_assert!(!buf.is_null(), "invalid Luau buffer");
        (buf as *mut u8, size)
    }

    #[cfg(not(feature = "luau"))]
    unsafe fn as_raw_parts(&self, lua: &RawLua) -> (*mut u8, usize) {
        unreachable!()
    }
}

#[cfg(feature = "serde")]
impl Serialize for Buffer {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let lua = self.0.lua.lock();
        serializer.serialize_bytes(self.as_slice(&lua))
    }
}

#[cfg(feature = "luau")]
impl crate::types::LuaType for Buffer {
    const TYPE_ID: std::os::raw::c_int = ffi::LUA_TBUFFER;
}
