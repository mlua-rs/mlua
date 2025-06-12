use std::cell::{RefCell, UnsafeCell};

#[cfg(feature = "serde")]
use serde::ser::{Serialize, Serializer};

use crate::error::{Error, Result};
use crate::types::XRc;

use super::lock::{RawLock, UserDataLock};
use super::r#ref::{UserDataRef, UserDataRefMut};

#[cfg(all(feature = "serde", not(feature = "send")))]
type DynSerialize = dyn erased_serde::Serialize;

#[cfg(all(feature = "serde", feature = "send"))]
type DynSerialize = dyn erased_serde::Serialize + Send;

pub(crate) enum UserDataStorage<T> {
    Owned(UserDataVariant<T>),
    Scoped(ScopedUserDataVariant<T>),
}

// A enum for storing userdata values.
// It's stored inside a Lua VM and protected by the outer `ReentrantMutex`.
pub(crate) enum UserDataVariant<T> {
    Default(XRc<UserDataCell<T>>),
    #[cfg(feature = "serde")]
    Serializable(XRc<UserDataCell<Box<DynSerialize>>>, bool), // bool is `is_sync`
}

impl<T> Clone for UserDataVariant<T> {
    #[inline]
    fn clone(&self) -> Self {
        match self {
            Self::Default(inner) => Self::Default(XRc::clone(inner)),
            #[cfg(feature = "serde")]
            Self::Serializable(inner, is_sync) => Self::Serializable(XRc::clone(inner), *is_sync),
        }
    }
}

impl<T> UserDataVariant<T> {
    #[inline(always)]
    pub(super) fn try_borrow_scoped<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R> {
        // We don't need to check for `T: Sync` because when this method is used (internally),
        // Lua mutex is already locked.
        // If non-`Sync` userdata is already borrowed by another thread (via `UserDataRef`), it will be
        // exclusively locked.
        let _guard = (self.raw_lock().try_lock_shared_guarded()).map_err(|_| Error::UserDataBorrowError)?;
        Ok(f(unsafe { &*self.as_ptr() }))
    }

    // Mutably borrows the wrapped value in-place.
    #[inline(always)]
    fn try_borrow_scoped_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> Result<R> {
        let _guard =
            (self.raw_lock().try_lock_exclusive_guarded()).map_err(|_| Error::UserDataBorrowMutError)?;
        Ok(f(unsafe { &mut *self.as_ptr() }))
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
        if !self.raw_lock().try_lock_exclusive() {
            return Err(Error::UserDataBorrowMutError);
        }
        Ok(match self {
            Self::Default(inner) => XRc::into_inner(inner).unwrap().value.into_inner(),
            #[cfg(feature = "serde")]
            Self::Serializable(inner, _) => unsafe {
                let raw = Box::into_raw(XRc::into_inner(inner).unwrap().value.into_inner());
                *Box::from_raw(raw as *mut T)
            },
        })
    }

    #[inline(always)]
    fn strong_count(&self) -> usize {
        match self {
            Self::Default(inner) => XRc::strong_count(inner),
            #[cfg(feature = "serde")]
            Self::Serializable(inner, _) => XRc::strong_count(inner),
        }
    }

    #[inline(always)]
    pub(super) fn raw_lock(&self) -> &RawLock {
        match self {
            Self::Default(inner) => &inner.raw_lock,
            #[cfg(feature = "serde")]
            Self::Serializable(inner, _) => &inner.raw_lock,
        }
    }

    #[inline(always)]
    pub(super) fn as_ptr(&self) -> *mut T {
        match self {
            Self::Default(inner) => inner.value.get(),
            #[cfg(feature = "serde")]
            Self::Serializable(inner, _) => unsafe { &mut **(inner.value.get() as *mut Box<T>) },
        }
    }
}

#[cfg(feature = "serde")]
impl Serialize for UserDataStorage<()> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::Owned(variant @ UserDataVariant::Serializable(inner, is_sync)) => unsafe {
                #[cfg(feature = "send")]
                if *is_sync {
                    let _guard = (variant.raw_lock().try_lock_shared_guarded())
                        .map_err(|_| serde::ser::Error::custom(Error::UserDataBorrowError))?;
                    (*inner.value.get()).serialize(serializer)
                } else {
                    let _guard = (variant.raw_lock().try_lock_exclusive_guarded())
                        .map_err(|_| serde::ser::Error::custom(Error::UserDataBorrowError))?;
                    (*inner.value.get()).serialize(serializer)
                }
                #[cfg(not(feature = "send"))]
                {
                    let _ = is_sync;
                    let _guard = (variant.raw_lock().try_lock_shared_guarded())
                        .map_err(|_| serde::ser::Error::custom(Error::UserDataBorrowError))?;
                    (*inner.value.get()).serialize(serializer)
                }
            },
            _ => Err(serde::ser::Error::custom("cannot serialize <userdata>")),
        }
    }
}

/// A type that provides interior mutability for a userdata value (thread-safe).
pub(crate) struct UserDataCell<T> {
    raw_lock: RawLock,
    value: UnsafeCell<T>,
}

#[cfg(feature = "send")]
unsafe impl<T: Send> Send for UserDataCell<T> {}
#[cfg(feature = "send")]
unsafe impl<T: Send> Sync for UserDataCell<T> {}

impl<T> UserDataCell<T> {
    #[inline(always)]
    fn new(value: T) -> Self {
        UserDataCell {
            raw_lock: RawLock::INIT,
            value: UnsafeCell::new(value),
        }
    }
}

pub(crate) enum ScopedUserDataVariant<T> {
    Ref(*const T),
    RefMut(RefCell<*mut T>),
    Boxed(RefCell<*mut T>),
}

impl<T> Drop for ScopedUserDataVariant<T> {
    #[inline]
    fn drop(&mut self) {
        if let Self::Boxed(value) = self {
            if let Ok(value) = value.try_borrow_mut() {
                unsafe { drop(Box::from_raw(*value)) };
            }
        }
    }
}

impl<T: 'static> UserDataStorage<T> {
    #[inline(always)]
    pub(crate) fn new(data: T) -> Self {
        Self::Owned(UserDataVariant::Default(XRc::new(UserDataCell::new(data))))
    }

    #[inline(always)]
    pub(crate) fn new_ref(data: &T) -> Self {
        Self::Scoped(ScopedUserDataVariant::Ref(data))
    }

    #[inline(always)]
    pub(crate) fn new_ref_mut(data: &mut T) -> Self {
        Self::Scoped(ScopedUserDataVariant::RefMut(RefCell::new(data)))
    }

    #[cfg(feature = "serde")]
    #[inline(always)]
    pub(crate) fn new_ser(data: T) -> Self
    where
        T: Serialize + crate::types::MaybeSend,
    {
        let data = Box::new(data) as Box<DynSerialize>;
        let is_sync = super::util::is_sync::<T>();
        let variant = UserDataVariant::Serializable(XRc::new(UserDataCell::new(data)), is_sync);
        Self::Owned(variant)
    }

    #[cfg(feature = "serde")]
    #[inline(always)]
    pub(crate) fn is_serializable(&self) -> bool {
        matches!(self, Self::Owned(UserDataVariant::Serializable(..)))
    }

    // Immutably borrows the wrapped value and returns an owned reference.
    #[inline(always)]
    pub(crate) fn try_borrow_owned(&self) -> Result<UserDataRef<T>> {
        match self {
            Self::Owned(data) => data.try_borrow_owned(),
            Self::Scoped(_) => Err(Error::UserDataTypeMismatch),
        }
    }

    // Mutably borrows the wrapped value and returns an owned reference.
    #[inline(always)]
    pub(crate) fn try_borrow_owned_mut(&self) -> Result<UserDataRefMut<T>> {
        match self {
            Self::Owned(data) => data.try_borrow_owned_mut(),
            Self::Scoped(_) => Err(Error::UserDataTypeMismatch),
        }
    }

    #[inline(always)]
    pub(crate) fn into_inner(self) -> Result<T> {
        match self {
            Self::Owned(data) => data.into_inner(),
            Self::Scoped(_) => Err(Error::UserDataTypeMismatch),
        }
    }
}

impl<T> UserDataStorage<T> {
    #[inline(always)]
    pub(crate) fn new_scoped(data: T) -> Self {
        let data = Box::into_raw(Box::new(data));
        Self::Scoped(ScopedUserDataVariant::Boxed(RefCell::new(data)))
    }

    /// Returns `true` if it's safe to destroy the container.
    ///
    /// It's safe to destroy the container if the reference count is greater than 1 or the lock is
    /// not acquired.
    #[inline(always)]
    pub(crate) fn is_safe_to_destroy(&self) -> bool {
        match self {
            Self::Owned(variant) => variant.strong_count() > 1 || !variant.raw_lock().is_locked(),
            Self::Scoped(_) => false,
        }
    }

    /// Returns `true` if the container has exclusive access to the value.
    #[inline(always)]
    pub(crate) fn has_exclusive_access(&self) -> bool {
        match self {
            Self::Owned(variant) => !variant.raw_lock().is_locked(),
            Self::Scoped(_) => false,
        }
    }

    #[inline]
    pub(crate) fn try_borrow_scoped<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R> {
        match self {
            Self::Owned(data) => data.try_borrow_scoped(f),
            Self::Scoped(ScopedUserDataVariant::Ref(value)) => Ok(f(unsafe { &**value })),
            Self::Scoped(ScopedUserDataVariant::RefMut(value) | ScopedUserDataVariant::Boxed(value)) => {
                let t = value.try_borrow().map_err(|_| Error::UserDataBorrowError)?;
                Ok(f(unsafe { &**t }))
            }
        }
    }

    #[inline]
    pub(crate) fn try_borrow_scoped_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> Result<R> {
        match self {
            Self::Owned(data) => data.try_borrow_scoped_mut(f),
            Self::Scoped(ScopedUserDataVariant::Ref(_)) => Err(Error::UserDataBorrowMutError),
            Self::Scoped(ScopedUserDataVariant::RefMut(value) | ScopedUserDataVariant::Boxed(value)) => {
                let mut t = value
                    .try_borrow_mut()
                    .map_err(|_| Error::UserDataBorrowMutError)?;
                Ok(f(unsafe { &mut **t }))
            }
        }
    }
}
