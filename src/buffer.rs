#[cfg(feature = "serde")]
use serde::ser::{Serialize, Serializer};

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
        unsafe { self.as_slice().to_vec() }
    }

    /// Returns the length of the buffer.
    pub fn len(&self) -> usize {
        unsafe { self.as_slice().len() }
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
        let data = unsafe { self.as_slice() };
        let mut bytes = [0u8; N];
        bytes.copy_from_slice(&data[offset..offset + N]);
        bytes
    }

    /// Writes given bytes to the buffer at the given offset.
    ///
    /// Offset is 0-based.
    #[track_caller]
    pub fn write_bytes(&self, offset: usize, bytes: &[u8]) {
        let data = unsafe {
            let (buf, size) = self.as_raw_parts();
            std::slice::from_raw_parts_mut(buf, size)
        };
        data[offset..offset + bytes.len()].copy_from_slice(bytes);
    }

    pub(crate) unsafe fn as_slice(&self) -> &[u8] {
        let (buf, size) = self.as_raw_parts();
        std::slice::from_raw_parts(buf, size)
    }

    #[cfg(feature = "luau")]
    unsafe fn as_raw_parts(&self) -> (*mut u8, usize) {
        let lua = self.0.lua.lock();
        let mut size = 0usize;
        let buf = ffi::lua_tobuffer(lua.ref_thread(self.0.aux_thread), self.0.index, &mut size);
        mlua_assert!(!buf.is_null(), "invalid Luau buffer");
        (buf as *mut u8, size)
    }

    #[cfg(not(feature = "luau"))]
    unsafe fn as_raw_parts(&self) -> (*mut u8, usize) {
        unreachable!()
    }
}

#[cfg(feature = "serde")]
impl Serialize for Buffer {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_bytes(unsafe { self.as_slice() })
    }
}

#[cfg(feature = "luau")]
impl crate::types::LuaType for Buffer {
    const TYPE_ID: std::os::raw::c_int = ffi::LUA_TBUFFER;
}
