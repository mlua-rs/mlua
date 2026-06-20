use std::os::raw::{c_int, c_void};
use std::{fmt, ptr};

use super::XRc;
use crate::state::{RawLua, WeakLua};

use self::ref_count::RefCount;

/// A reference to a Lua (complex) value stored in the Lua auxiliary thread.
pub struct ValueRef {
    pub(crate) lua: WeakLua,
    pub(crate) index: c_int,
    count: RefCount,
}

impl ValueRef {
    #[inline]
    pub(crate) fn new(lua: &RawLua, index: c_int) -> Self {
        ValueRef {
            lua: lua.weak().clone(),
            index,
            count: RefCount::unique(),
        }
    }

    #[cfg(feature = "async")]
    #[inline]
    pub(crate) fn take_index(&mut self) -> Option<c_int> {
        self.count.take(self.index)
    }

    #[inline]
    pub(crate) fn to_pointer(&self) -> *const c_void {
        let lua = self.lua.lock();
        unsafe { ffi::lua_topointer(lua.ref_thread(), self.index) }
    }
}

impl Clone for ValueRef {
    #[inline]
    fn clone(&self) -> Self {
        ValueRef {
            lua: self.lua.clone(),
            index: self.index,
            count: self.count.clone_shared(),
        }
    }
}

impl fmt::Debug for ValueRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Ref({:p})", self.to_pointer())
    }
}

impl Drop for ValueRef {
    fn drop(&mut self) {
        if self.count.drop_is_last()
            && let Some(lua) = self.lua.try_lock()
        {
            unsafe { lua.drop_ref(self) }
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

// The counter is a pure refcount token.
type Unit = ();
const UNIQUE: *mut Unit = ptr::without_provenance_mut(1);
const NONE: *mut Unit = ptr::null_mut();

impl RefCount {
    #[inline]
    fn unique() -> Self {
        Self::from_raw(UNIQUE)
    }

    #[inline]
    fn clone_shared(&self) -> RefCount {
        let mut current = self.load();
        loop {
            if current != UNIQUE {
                if current != NONE {
                    unsafe { XRc::increment_strong_count(current as *const Unit) };
                }
                return RefCount::from_raw(current);
            }
            // Lazily allocate the shared counter
            let shared = XRc::into_raw(XRc::new(())) as *mut Unit;
            match self.promote(shared) {
                Ok(()) => {
                    unsafe { XRc::increment_strong_count(shared as *const Unit) };
                    return RefCount::from_raw(shared);
                }
                Err(actual) => {
                    unsafe { drop(XRc::from_raw(shared as *const Unit)) };
                    current = actual;
                }
            }
        }
    }

    /// Takes the slot if it's solely owned by `self`.
    ///
    /// Returns `None` if the slot is still shared or already non-owning.
    #[cfg(feature = "async")]
    fn take(&mut self, index: c_int) -> Option<c_int> {
        match self.load() {
            current if current == UNIQUE => {
                self.swap_none();
                Some(index)
            }
            current if current == NONE => None,
            current => {
                // Shared: recyclable only if no other owner remains
                let rc = unsafe { XRc::from_raw(current as *const Unit) };
                if XRc::strong_count(&rc) == 1 {
                    drop(rc);
                    self.swap_none();
                    Some(index)
                } else {
                    let _ = XRc::into_raw(rc); // still shared
                    None
                }
            }
        }
    }

    /// Drops the reference and returns `true` if it was the last owner of the slot
    /// (so the slot must be freed).
    #[inline]
    fn drop_is_last(&mut self) -> bool {
        let current = self.load();
        if current == UNIQUE {
            true
        } else if current == NONE {
            false
        } else {
            unsafe { XRc::into_inner(XRc::from_raw(current as *const Unit)).is_some() }
        }
    }
}

#[cfg(feature = "send")]
mod ref_count {
    use std::sync::atomic::{AtomicPtr, Ordering};

    use super::Unit;

    pub(super) struct RefCount(AtomicPtr<Unit>);

    impl RefCount {
        #[inline]
        pub(super) fn from_raw(ptr: *mut Unit) -> Self {
            RefCount(AtomicPtr::new(ptr))
        }

        #[inline]
        pub(super) fn load(&self) -> *mut Unit {
            self.0.load(Ordering::Acquire)
        }

        /// Replaces the `UNIQUE` tag with the freshly allocated shared counter (`new`).
        ///
        /// Returns `Err(current)` if another thread promoted first.
        #[inline]
        pub(super) fn promote(&self, new: *mut Unit) -> Result<(), *mut Unit> {
            self.0
                .compare_exchange(super::UNIQUE, new, Ordering::AcqRel, Ordering::Acquire)
                .map(|_| ())
        }

        #[cfg(feature = "async")]
        #[inline]
        pub(super) fn swap_none(&self) -> *mut Unit {
            self.0.swap(super::NONE, Ordering::AcqRel)
        }
    }
}

#[cfg(not(feature = "send"))]
mod ref_count {
    use std::cell::Cell;

    use super::Unit;

    pub(super) struct RefCount(Cell<*mut Unit>);

    impl RefCount {
        #[inline]
        pub(super) fn from_raw(ptr: *mut Unit) -> Self {
            RefCount(Cell::new(ptr))
        }

        #[inline]
        pub(super) fn load(&self) -> *mut Unit {
            self.0.get()
        }

        /// Replaces the `UNIQUE` tag with the freshly allocated shared counter `new`.
        ///
        /// Never fails.
        #[inline]
        pub(super) fn promote(&self, new: *mut Unit) -> Result<(), *mut Unit> {
            debug_assert_eq!(self.0.get(), super::UNIQUE);
            self.0.set(new);
            Ok(())
        }

        #[cfg(feature = "async")]
        #[inline]
        pub(super) fn swap_none(&self) -> *mut Unit {
            self.0.replace(super::NONE)
        }
    }
}
