use std::fmt;
use std::os::raw::{c_int, c_void};

use crate::state::{RawLua, WeakLua};

/// A reference to a Lua (complex) value stored in the Lua auxiliary thread.
pub struct ValueRef {
    pub(crate) lua: WeakLua,
    pub(crate) index: c_int,
    pub(crate) drop: bool,
}

impl ValueRef {
    #[inline]
    pub(crate) fn new(lua: &RawLua, index: c_int) -> Self {
        ValueRef {
            lua: lua.weak().clone(),
            index,
            drop: true,
        }
    }

    #[inline]
    pub(crate) fn to_pointer(&self) -> *const c_void {
        let lua = self.lua.lock();
        unsafe { ffi::lua_topointer(lua.ref_thread(), self.index) }
    }

    /// Returns a copy of the value, which is valid as long as the original value is held.
    #[inline]
    pub(crate) fn copy(&self) -> Self {
        ValueRef {
            lua: self.lua.clone(),
            index: self.index,
            drop: false,
        }
    }
}

impl fmt::Debug for ValueRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Ref({:p})", self.to_pointer())
    }
}

impl Clone for ValueRef {
    fn clone(&self) -> Self {
        unsafe { self.lua.lock().clone_ref(self) }
    }
}

impl Drop for ValueRef {
    fn drop(&mut self) {
        if self.drop {
            if let Some(lua) = self.lua.try_lock() {
                unsafe { lua.drop_ref(self) };
            }
        }
    }
}

impl PartialEq for ValueRef {
    fn eq(&self, other: &Self) -> bool {
        assert!(
            self.lua == other.lua,
            "Lua instance passed Value created from a different main Lua state"
        );
        let lua = self.lua.lock();
        unsafe { ffi::lua_rawequal(lua.ref_thread(), self.index, other.index) == 1 }
    }
}
