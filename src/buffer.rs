use std::io;

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
        let data = self.as_slice_mut(&lua);
        data[offset..offset + bytes.len()].copy_from_slice(bytes);
    }

    /// Returns an adaptor implementing [`io::Read`], [`io::Write`] and [`io::Seek`] over the
    /// buffer.
    ///
    /// Buffer operations are infallible, none of the read/write functions will return a Err.
    pub fn cursor(self) -> impl io::Read + io::Write + io::Seek {
        BufferCursor(self, 0)
    }

    pub(crate) fn as_slice(&self, lua: &RawLua) -> &[u8] {
        unsafe {
            let (buf, size) = self.as_raw_parts(lua);
            std::slice::from_raw_parts(buf, size)
        }
    }

    #[allow(clippy::mut_from_ref)]
    fn as_slice_mut(&self, lua: &RawLua) -> &mut [u8] {
        unsafe {
            let (buf, size) = self.as_raw_parts(lua);
            std::slice::from_raw_parts_mut(buf, size)
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

struct BufferCursor(Buffer, usize);

impl io::Read for BufferCursor {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let lua = self.0 .0.lua.lock();
        let data = self.0.as_slice(&lua);
        if self.1 == data.len() {
            return Ok(0);
        }
        let len = buf.len().min(data.len() - self.1);
        buf[..len].copy_from_slice(&data[self.1..self.1 + len]);
        self.1 += len;
        Ok(len)
    }
}

impl io::Write for BufferCursor {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let lua = self.0 .0.lua.lock();
        let data = self.0.as_slice_mut(&lua);
        if self.1 == data.len() {
            return Ok(0);
        }
        let len = buf.len().min(data.len() - self.1);
        data[self.1..self.1 + len].copy_from_slice(&buf[..len]);
        self.1 += len;
        Ok(len)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl io::Seek for BufferCursor {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        let lua = self.0 .0.lua.lock();
        let data = self.0.as_slice(&lua);
        let new_offset = match pos {
            io::SeekFrom::Start(offset) => offset as i64,
            io::SeekFrom::End(offset) => data.len() as i64 + offset,
            io::SeekFrom::Current(offset) => self.1 as i64 + offset,
        };
        if new_offset < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid seek to a negative position",
            ));
        }
        if new_offset as usize > data.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid seek to a position beyond the end of the buffer",
            ));
        }
        self.1 = new_offset as usize;
        Ok(self.1 as u64)
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
