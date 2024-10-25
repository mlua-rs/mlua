use std::hash::{Hash, Hasher};
use std::os::raw::c_int;
use std::sync::Arc;
use std::{fmt, mem, ptr};

use parking_lot::Mutex;

/// An auto generated key into the Lua registry.
///
/// This is a handle to a value stored inside the Lua registry. It is not automatically
/// garbage collected on Drop, but it can be removed with [`Lua::remove_registry_value`],
/// and instances not manually removed can be garbage collected with
/// [`Lua::expire_registry_values`].
///
/// Be warned, If you place this into Lua via a [`UserData`] type or a Rust callback, it is *easy*
/// to accidentally cause reference cycles that the Lua garbage collector cannot resolve. Instead of
/// placing a [`RegistryKey`] into a [`UserData`] type, consider to use
/// [`AnyUserData::set_user_value`].
///
/// [`UserData`]: crate::UserData
/// [`RegistryKey`]: crate::RegistryKey
/// [`Lua::remove_registry_value`]: crate::Lua::remove_registry_value
/// [`Lua::expire_registry_values`]: crate::Lua::expire_registry_values
/// [`AnyUserData::set_user_value`]: crate::AnyUserData::set_user_value
pub struct RegistryKey {
    pub(crate) registry_id: i32,
    pub(crate) unref_list: Arc<Mutex<Option<Vec<c_int>>>>,
}

impl fmt::Debug for RegistryKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "RegistryKey({})", self.id())
    }
}

impl Hash for RegistryKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id().hash(state)
    }
}

impl PartialEq for RegistryKey {
    fn eq(&self, other: &RegistryKey) -> bool {
        self.id() == other.id() && Arc::ptr_eq(&self.unref_list, &other.unref_list)
    }
}

impl Eq for RegistryKey {}

impl Drop for RegistryKey {
    fn drop(&mut self) {
        let registry_id = self.id();
        // We don't need to collect nil slot
        if registry_id > ffi::LUA_REFNIL {
            let mut unref_list = self.unref_list.lock();
            if let Some(list) = unref_list.as_mut() {
                list.push(registry_id);
            }
        }
    }
}

impl RegistryKey {
    /// Creates a new instance of `RegistryKey`
    pub(crate) const fn new(id: c_int, unref_list: Arc<Mutex<Option<Vec<c_int>>>>) -> Self {
        RegistryKey {
            registry_id: id,
            unref_list,
        }
    }

    /// Returns the underlying Lua reference of this `RegistryKey`
    #[inline(always)]
    pub fn id(&self) -> c_int {
        self.registry_id
    }

    /// Sets the unique Lua reference key of this `RegistryKey`
    #[inline(always)]
    pub(crate) fn set_id(&mut self, id: c_int) {
        self.registry_id = id;
    }

    /// Destroys the `RegistryKey` without adding to the unref list
    pub(crate) fn take(self) -> i32 {
        let registry_id = self.id();
        unsafe {
            ptr::read(&self.unref_list);
            mem::forget(self);
        }
        registry_id
    }
}

#[cfg(test)]
mod assertions {
    use super::*;

    static_assertions::assert_impl_all!(RegistryKey: Send, Sync);
}
