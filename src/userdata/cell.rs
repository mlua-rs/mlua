use std::cell::RefCell;
use std::mem::ManuallyDrop;

#[cfg(feature = "serde")]
use serde::ser::{Serialize, Serializer};

use crate::error::{Error, Result};
use crate::types::XRc;
#[cfg(feature = "serde")]
use crate::types::XWeak;

use super::lock::{RawLock, RwLock, UserDataLock};
use super::r#ref::{UserDataRef, UserDataRefMut};

#[cfg(all(feature = "serde", not(feature = "send")))]
type DynSerialize = dyn erased_serde::Serialize;

#[cfg(all(feature = "serde", feature = "send"))]
type DynSerialize = dyn erased_serde::Serialize + Send + Sync;

#[cfg_attr(feature = "serde", repr(C))]
pub(crate) struct UserDataStorage<T> {
    #[cfg(feature = "serde")]
    header: SerializeHeader, // Must remain first for erased access.
    inner: UserDataStorageInner<T>,
}

enum UserDataStorageInner<T> {
    Owned(UserDataVariant<T>),
    Scoped(ScopedUserDataVariant<T>),
}

// A enum for storing userdata values.
// It's stored inside a Lua VM and protected by the outer `ReentrantMutex`.
pub(crate) enum UserDataVariant<T> {
    Default(XRc<RwLock<T>>),
    #[cfg(feature = "serde")]
    Serializable(XRc<RwLock<Box<DynSerialize>>>),
}

impl<T> Clone for UserDataVariant<T> {
    #[inline]
    fn clone(&self) -> Self {
        match self {
            Self::Default(inner) => Self::Default(XRc::clone(inner)),
            #[cfg(feature = "serde")]
            Self::Serializable(inner) => Self::Serializable(XRc::clone(inner)),
        }
    }
}

impl<T> UserDataVariant<T> {
    #[inline(always)]
    pub(super) fn try_borrow_scoped<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R> {
        // Shared (read) lock is always correct for in-place borrows:
        // - this method is called internally while the Lua mutex is held, ensuring exclusive
        //   Lua-level access per call frame
        // - with `send` feature, all owned userdata satisfies `T: Sync`, so simultaneous shared
        //   references from multiple threads are sound
        // - without `send` feature, single-threaded execution makes shared lock safe for any `T`
        let _guard = (self.raw_lock().try_lock_shared_guarded()).map_err(|_| Error::UserDataBorrowError)?;
        Ok(f(unsafe { &*self.as_ptr() }))
    }

    // Mutably borrows the wrapped value in-place.
    #[inline(always)]
    fn try_borrow_scoped_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> Result<R> {
        let _guard =
            (self.raw_lock().try_lock_exclusive_guarded()).map_err(|_| Error::UserDataBorrowMutError)?;
        Ok(f(unsafe { &mut *self.as_mut_ptr() }))
    }

    // Immutably borrows the wrapped value and returns an owned reference.
    #[inline(always)]
    fn try_borrow_owned(&self) -> Result<UserDataRef<T>> {
        UserDataRef::try_from(self.clone())
    }

    // Mutably borrows the wrapped value and returns an owned reference.
    #[inline(always)]
    fn try_borrow_owned_mut(&self) -> Result<UserDataRefMut<T>> {
        UserDataRefMut::try_from(self.clone())
    }

    // Returns the wrapped value.
    //
    // This method checks that we have exclusive access to the value.
    fn into_inner(self) -> Result<T> {
        debug_assert_eq!(self.strong_count(), 1, "userdata must have a single owner");
        if !self.raw_lock().try_lock_exclusive() {
            return Err(Error::UserDataBorrowMutError);
        }
        Ok(match self {
            Self::Default(inner) => XRc::into_inner(inner)
                .expect("userdata ownership checked before extraction")
                .into_inner(),
            #[cfg(feature = "serde")]
            Self::Serializable(inner) => unsafe {
                // The serde variant erases `T` to `Box<DynSerialize>`, so we
                // must cast the raw pointer back to recover the concrete type.
                let raw = Box::into_raw(XRc::into_inner(inner).unwrap().into_inner());
                *Box::from_raw(raw as *mut T)
            },
        })
    }

    #[inline(always)]
    fn strong_count(&self) -> usize {
        match self {
            Self::Default(inner) => XRc::strong_count(inner),
            #[cfg(feature = "serde")]
            Self::Serializable(inner) => XRc::strong_count(inner),
        }
    }

    #[inline(always)]
    pub(super) fn raw_lock(&self) -> &RawLock {
        match self {
            Self::Default(inner) => unsafe { inner.raw() },
            #[cfg(feature = "serde")]
            Self::Serializable(inner) => unsafe { inner.raw() },
        }
    }

    #[inline(always)]
    pub(super) unsafe fn as_ptr(&self) -> *const T {
        match self {
            Self::Default(inner) => inner.data_ptr(),
            #[cfg(feature = "serde")]
            Self::Serializable(inner) => unsafe { (&**inner.data_ptr()) as *const DynSerialize as *const T },
        }
    }

    #[inline(always)]
    pub(super) unsafe fn as_mut_ptr(&self) -> *mut T {
        match self {
            Self::Default(inner) => inner.data_ptr(),
            #[cfg(feature = "serde")]
            Self::Serializable(inner) => unsafe { (&mut **inner.data_ptr()) as *mut DynSerialize as *mut T },
        }
    }
}

#[cfg(feature = "serde")]
#[derive(Default)]
pub(super) struct SerializeHeader {
    inner: Option<XWeak<RwLock<Box<DynSerialize>>>>,
}

#[cfg(feature = "serde")]
impl SerializeHeader {
    #[inline]
    fn new(inner: &XRc<RwLock<Box<DynSerialize>>>) -> Self {
        Self {
            inner: Some(XRc::downgrade(inner)),
        }
    }

    #[inline]
    pub(super) fn is_serializable(&self) -> bool {
        self.inner.is_some()
    }
}

#[cfg(feature = "serde")]
impl Serialize for SerializeHeader {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let inner = (self.inner)
            .as_ref()
            .and_then(XWeak::upgrade)
            .ok_or_else(|| serde::ser::Error::custom("cannot serialize <userdata>"))?;
        let _guard = (unsafe { inner.raw() }.try_lock_shared_guarded())
            .map_err(|_| serde::ser::Error::custom(Error::UserDataBorrowError))?;
        unsafe { (*inner.data_ptr()).serialize(serializer) }
    }
}

pub(crate) enum ScopedUserDataVariant<T> {
    Ref(*const T),
    RefMut(RefCell<*mut T>),
    Boxed(RefCell<ManuallyDrop<Box<T>>>),
}

impl<T> Drop for ScopedUserDataVariant<T> {
    #[inline]
    fn drop(&mut self) {
        if let Self::Boxed(value) = self
            && let Ok(mut value) = value.try_borrow_mut()
        {
            unsafe { ManuallyDrop::drop(&mut value) }
        }
    }
}

impl<T: 'static> UserDataStorage<T> {
    #[inline(always)]
    pub(crate) fn new(data: T) -> Self {
        Self::from_inner(UserDataStorageInner::Owned(UserDataVariant::Default(XRc::new(
            RwLock::new(data),
        ))))
    }

    #[inline(always)]
    pub(crate) fn new_ref(data: &T) -> Self {
        Self::from_inner(UserDataStorageInner::Scoped(ScopedUserDataVariant::Ref(data)))
    }

    #[inline(always)]
    pub(crate) fn new_ref_mut(data: &mut T) -> Self {
        Self::from_inner(UserDataStorageInner::Scoped(ScopedUserDataVariant::RefMut(
            RefCell::new(data),
        )))
    }

    #[cfg(feature = "serde")]
    #[inline(always)]
    pub(crate) fn new_ser(data: T) -> Self
    where
        T: Serialize + crate::types::MaybeSend + crate::types::MaybeSync,
    {
        let data = Box::new(data) as Box<DynSerialize>;
        let inner = XRc::new(RwLock::new(data));
        let header = SerializeHeader::new(&inner);
        Self {
            header,
            inner: UserDataStorageInner::Owned(UserDataVariant::Serializable(inner)),
        }
    }

    // Immutably borrows the wrapped value and returns an owned reference.
    #[inline(always)]
    pub(crate) fn try_borrow_owned(&self) -> Result<UserDataRef<T>> {
        match &self.inner {
            UserDataStorageInner::Owned(data) => data.try_borrow_owned(),
            UserDataStorageInner::Scoped(_) => Err(Error::UserDataTypeMismatch),
        }
    }

    // Mutably borrows the wrapped value and returns an owned reference.
    #[inline(always)]
    pub(crate) fn try_borrow_owned_mut(&self) -> Result<UserDataRefMut<T>> {
        match &self.inner {
            UserDataStorageInner::Owned(data) => data.try_borrow_owned_mut(),
            UserDataStorageInner::Scoped(_) => Err(Error::UserDataTypeMismatch),
        }
    }

    #[inline(always)]
    pub(crate) fn into_inner(self) -> Result<T> {
        match self.inner {
            UserDataStorageInner::Owned(data) => data.into_inner(),
            UserDataStorageInner::Scoped(_) => Err(Error::UserDataTypeMismatch),
        }
    }
}

impl<T> UserDataStorage<T> {
    #[inline(always)]
    fn from_inner(inner: UserDataStorageInner<T>) -> Self {
        Self {
            #[cfg(feature = "serde")]
            header: SerializeHeader::default(),
            inner,
        }
    }

    #[inline(always)]
    pub(crate) fn new_scoped(data: T) -> Self {
        let data = ManuallyDrop::new(Box::new(data));
        Self::from_inner(UserDataStorageInner::Scoped(ScopedUserDataVariant::Boxed(
            RefCell::new(data),
        )))
    }

    /// Returns `true` if it's safe to destroy the container.
    ///
    /// It's safe to destroy the container if the lock is not acquired.
    #[inline(always)]
    pub(crate) fn is_safe_to_destroy(&self) -> bool {
        match &self.inner {
            UserDataStorageInner::Owned(variant) => !variant.raw_lock().is_locked(),
            UserDataStorageInner::Scoped(_) => false,
        }
    }

    /// Returns `true` if it's safe for Lua to collect the container.
    #[inline(always)]
    pub(crate) fn is_safe_to_collect(&self) -> bool {
        match &self.inner {
            UserDataStorageInner::Owned(variant) => {
                variant.strong_count() > 1 || !variant.raw_lock().is_locked()
            }
            UserDataStorageInner::Scoped(_) => false,
        }
    }

    /// Returns `true` if the container has exclusive access to the value.
    #[inline(always)]
    pub(crate) fn has_exclusive_access(&self) -> bool {
        match &self.inner {
            UserDataStorageInner::Owned(variant) => {
                variant.strong_count() == 1 && !variant.raw_lock().is_locked()
            }
            UserDataStorageInner::Scoped(_) => false,
        }
    }

    #[inline]
    pub(crate) fn try_borrow_scoped<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R> {
        match &self.inner {
            UserDataStorageInner::Owned(data) => data.try_borrow_scoped(f),
            UserDataStorageInner::Scoped(ScopedUserDataVariant::Ref(value)) => Ok(f(unsafe { &**value })),
            UserDataStorageInner::Scoped(ScopedUserDataVariant::RefMut(value)) => {
                let t = value.try_borrow().map_err(|_| Error::UserDataBorrowError)?;
                Ok(f(unsafe { &**t }))
            }
            UserDataStorageInner::Scoped(ScopedUserDataVariant::Boxed(value)) => {
                let t = value.try_borrow().map_err(|_| Error::UserDataBorrowError)?;
                Ok(f(&t))
            }
        }
    }

    #[inline]
    pub(crate) fn try_borrow_scoped_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> Result<R> {
        match &self.inner {
            UserDataStorageInner::Owned(data) => data.try_borrow_scoped_mut(f),
            UserDataStorageInner::Scoped(ScopedUserDataVariant::Ref(_)) => Err(Error::UserDataBorrowMutError),
            UserDataStorageInner::Scoped(ScopedUserDataVariant::RefMut(value)) => {
                let mut t = value
                    .try_borrow_mut()
                    .map_err(|_| Error::UserDataBorrowMutError)?;
                Ok(f(unsafe { &mut **t }))
            }
            UserDataStorageInner::Scoped(ScopedUserDataVariant::Boxed(value)) => {
                let mut t = value
                    .try_borrow_mut()
                    .map_err(|_| Error::UserDataBorrowMutError)?;
                Ok(f(&mut t))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{UserDataStorage, UserDataStorageInner};

    #[cfg(feature = "serde")]
    use {super::SerializeHeader, serde::Serialize as _};

    #[test]
    fn test_exclusive_access_requires_sole_owner() {
        let storage = UserDataStorage::new(String::from("value"));
        let owner = match &storage.inner {
            UserDataStorageInner::Owned(variant) => variant.clone(),
            UserDataStorageInner::Scoped(_) => unreachable!(),
        };

        assert!(!storage.has_exclusive_access());
        drop(owner);
        assert!(storage.has_exclusive_access());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_erased_serializable_storage() {
        let storage = UserDataStorage::new_ser(String::from("value"));

        let first = storage.try_borrow_owned().unwrap();
        let second = storage.try_borrow_owned().unwrap();
        assert_eq!(&*first, "value");
        assert_eq!(&*second, "value");
        drop((first, second));

        let header = unsafe { &*(&storage as *const UserDataStorage<String>).cast::<SerializeHeader>() };
        let borrowed = storage.try_borrow_owned_mut().unwrap();
        assert!(header.serialize(serde_json::value::Serializer).is_err());
        drop(borrowed);

        storage
            .try_borrow_scoped_mut(|value| value.push_str(" scoped"))
            .unwrap();
        storage.try_borrow_owned_mut().unwrap().push_str(" owned");

        assert!(header.is_serializable());
        let json = header.serialize(serde_json::value::Serializer);
        assert_eq!(json.unwrap(), "value scoped owned");

        let storage = UserDataStorage::new(String::from("value"));
        let header = unsafe { &*(&storage as *const UserDataStorage<String>).cast::<SerializeHeader>() };
        assert!(!header.is_serializable());
    }
}
