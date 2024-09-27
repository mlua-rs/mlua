use std::any::{type_name, TypeId};
use std::cell::{RefCell, UnsafeCell};
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::os::raw::c_int;

#[cfg(feature = "serialize")]
use serde::ser::{Serialize, Serializer};

use crate::error::{Error, Result};
use crate::state::{Lua, RawLua};
use crate::types::XRc;
use crate::userdata::AnyUserData;
use crate::util::get_userdata;
use crate::value::{FromLua, Value};

use super::lock::{RawLock, UserDataLock};

#[cfg(all(feature = "serialize", not(feature = "send")))]
type DynSerialize = dyn erased_serde::Serialize;

#[cfg(all(feature = "serialize", feature = "send"))]
type DynSerialize = dyn erased_serde::Serialize + Send;

pub(crate) enum UserDataStorage<T> {
    Owned(UserDataVariant<T>),
    Scoped(ScopedUserDataVariant<T>),
}

// A enum for storing userdata values.
// It's stored inside a Lua VM and protected by the outer `ReentrantMutex`.
pub(crate) enum UserDataVariant<T> {
    Default(XRc<UserDataCell<T>>),
    #[cfg(feature = "serialize")]
    Serializable(XRc<UserDataCell<Box<DynSerialize>>>),
}

impl<T> Clone for UserDataVariant<T> {
    #[inline]
    fn clone(&self) -> Self {
        match self {
            Self::Default(inner) => Self::Default(XRc::clone(inner)),
            #[cfg(feature = "serialize")]
            Self::Serializable(inner) => Self::Serializable(XRc::clone(inner)),
        }
    }
}

impl<T> UserDataVariant<T> {
    // Immutably borrows the wrapped value in-place.
    #[inline(always)]
    fn try_borrow(&self) -> Result<UserDataBorrowRef<T>> {
        UserDataBorrowRef::try_from(self)
    }

    // Immutably borrows the wrapped value and returns an owned reference.
    #[inline(always)]
    fn try_borrow_owned(&self) -> Result<UserDataRef<T>> {
        UserDataRef::try_from(self.clone())
    }

    // Mutably borrows the wrapped value in-place.
    #[inline(always)]
    fn try_borrow_mut(&self) -> Result<UserDataBorrowMut<T>> {
        UserDataBorrowMut::try_from(self)
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
            #[cfg(feature = "serialize")]
            Self::Serializable(inner) => unsafe {
                let raw = Box::into_raw(XRc::into_inner(inner).unwrap().value.into_inner());
                *Box::from_raw(raw as *mut T)
            },
        })
    }

    #[inline(always)]
    fn raw_lock(&self) -> &RawLock {
        match self {
            Self::Default(inner) => &inner.raw_lock,
            #[cfg(feature = "serialize")]
            Self::Serializable(inner) => &inner.raw_lock,
        }
    }

    #[inline(always)]
    fn as_ptr(&self) -> *mut T {
        match self {
            Self::Default(inner) => inner.value.get(),
            #[cfg(feature = "serialize")]
            Self::Serializable(inner) => unsafe { &mut **(inner.value.get() as *mut Box<T>) },
        }
    }
}

#[cfg(feature = "serialize")]
impl Serialize for UserDataStorage<()> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::Owned(UserDataVariant::Serializable(inner)) => unsafe {
                // We need to borrow the inner value exclusively to serialize it.
                #[cfg(feature = "send")]
                let _guard = self.try_borrow_mut().map_err(serde::ser::Error::custom)?;
                // No need to do this if the `send` feature is disabled.
                #[cfg(not(feature = "send"))]
                let _guard = self.try_borrow().map_err(serde::ser::Error::custom)?;
                (*inner.value.get()).serialize(serializer)
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

unsafe impl<T: Send> Send for UserDataCell<T> {}
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

/// A wrapper type for a [`UserData`] value that provides read access.
///
/// It implements [`FromLua`] and can be used to receive a typed userdata from Lua.
pub struct UserDataRef<T>(UserDataVariant<T>);

impl<T> Deref for UserDataRef<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.0.as_ptr() }
    }
}

impl<T> Drop for UserDataRef<T> {
    #[inline]
    fn drop(&mut self) {
        unsafe { self.0.raw_lock().unlock_shared() };
    }
}

impl<T: fmt::Debug> fmt::Debug for UserDataRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: fmt::Display> fmt::Display for UserDataRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T> TryFrom<UserDataVariant<T>> for UserDataRef<T> {
    type Error = Error;

    #[inline]
    fn try_from(variant: UserDataVariant<T>) -> Result<Self> {
        if !variant.raw_lock().try_lock_shared() {
            return Err(Error::UserDataBorrowError);
        }
        Ok(UserDataRef(variant))
    }
}

impl<T: 'static> FromLua for UserDataRef<T> {
    fn from_lua(value: Value, _: &Lua) -> Result<Self> {
        try_value_to_userdata::<T>(value)?.borrow()
    }

    unsafe fn from_stack(idx: c_int, lua: &RawLua) -> Result<Self> {
        let type_id = lua.get_userdata_type_id::<T>(idx)?;
        match type_id {
            Some(type_id) if type_id == TypeId::of::<T>() => {
                (*get_userdata::<UserDataStorage<T>>(lua.state(), idx)).try_borrow_owned()
            }
            _ => Err(Error::UserDataTypeMismatch),
        }
    }
}

/// A wrapper type for a mutably borrowed value from a `AnyUserData`.
///
/// It implements [`FromLua`] and can be used to receive a typed userdata from Lua.
pub struct UserDataRefMut<T>(UserDataVariant<T>);

impl<T> Deref for UserDataRefMut<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.0.as_ptr() }
    }
}

impl<T> DerefMut for UserDataRefMut<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.0.as_ptr() }
    }
}

impl<T> Drop for UserDataRefMut<T> {
    #[inline]
    fn drop(&mut self) {
        unsafe { self.0.raw_lock().unlock_exclusive() };
    }
}

impl<T: fmt::Debug> fmt::Debug for UserDataRefMut<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: fmt::Display> fmt::Display for UserDataRefMut<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T> TryFrom<UserDataVariant<T>> for UserDataRefMut<T> {
    type Error = Error;

    #[inline]
    fn try_from(variant: UserDataVariant<T>) -> Result<Self> {
        if !variant.raw_lock().try_lock_exclusive() {
            return Err(Error::UserDataBorrowMutError);
        }
        Ok(UserDataRefMut(variant))
    }
}

impl<T: 'static> FromLua for UserDataRefMut<T> {
    fn from_lua(value: Value, _: &Lua) -> Result<Self> {
        try_value_to_userdata::<T>(value)?.borrow_mut()
    }

    unsafe fn from_stack(idx: c_int, lua: &RawLua) -> Result<Self> {
        let type_id = lua.get_userdata_type_id::<T>(idx)?;
        match type_id {
            Some(type_id) if type_id == TypeId::of::<T>() => {
                (*get_userdata::<UserDataStorage<T>>(lua.state(), idx)).try_borrow_owned_mut()
            }
            _ => Err(Error::UserDataTypeMismatch),
        }
    }
}

/// A type that provides read access to a userdata value (borrowing the value).
pub(crate) struct UserDataBorrowRef<'a, T>(&'a UserDataVariant<T>);

impl<'a, T> Drop for UserDataBorrowRef<'a, T> {
    #[inline]
    fn drop(&mut self) {
        unsafe { self.0.raw_lock().unlock_shared() };
    }
}

impl<'a, T> Deref for UserDataBorrowRef<'a, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: `UserDataBorrowRef` is only created with shared access to the value.
        unsafe { &*self.0.as_ptr() }
    }
}

impl<'a, T> TryFrom<&'a UserDataVariant<T>> for UserDataBorrowRef<'a, T> {
    type Error = Error;

    #[inline(always)]
    fn try_from(variant: &'a UserDataVariant<T>) -> Result<Self> {
        if !variant.raw_lock().try_lock_shared() {
            return Err(Error::UserDataBorrowError);
        }
        Ok(UserDataBorrowRef(variant))
    }
}

pub(crate) struct UserDataBorrowMut<'a, T>(&'a UserDataVariant<T>);

impl<'a, T> Drop for UserDataBorrowMut<'a, T> {
    #[inline]
    fn drop(&mut self) {
        unsafe { self.0.raw_lock().unlock_exclusive() };
    }
}

impl<'a, T> Deref for UserDataBorrowMut<'a, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.0.as_ptr() }
    }
}

impl<'a, T> DerefMut for UserDataBorrowMut<'a, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.0.as_ptr() }
    }
}

impl<'a, T> TryFrom<&'a UserDataVariant<T>> for UserDataBorrowMut<'a, T> {
    type Error = Error;

    #[inline(always)]
    fn try_from(variant: &'a UserDataVariant<T>) -> Result<Self> {
        if !variant.raw_lock().try_lock_exclusive() {
            return Err(Error::UserDataBorrowMutError);
        }
        Ok(UserDataBorrowMut(variant))
    }
}

#[inline]
fn try_value_to_userdata<T>(value: Value) -> Result<AnyUserData> {
    match value {
        Value::UserData(ud) => Ok(ud),
        _ => Err(Error::FromLuaConversionError {
            from: value.type_name(),
            to: "userdata".to_string(),
            message: Some(format!("expected userdata of type {}", type_name::<T>())),
        }),
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

    #[cfg(feature = "serialize")]
    #[inline(always)]
    pub(crate) fn new_ser(data: T) -> Self
    where
        T: Serialize + crate::types::MaybeSend,
    {
        let data = Box::new(data) as Box<DynSerialize>;
        Self::Owned(UserDataVariant::Serializable(XRc::new(UserDataCell::new(data))))
    }

    #[cfg(feature = "serialize")]
    #[inline(always)]
    pub(crate) fn is_serializable(&self) -> bool {
        matches!(self, Self::Owned(UserDataVariant::Serializable(_)))
    }

    // Immutably borrows the wrapped value and returns an owned reference.
    #[inline(always)]
    pub(crate) fn try_borrow_owned(&self) -> Result<UserDataRef<T>> {
        match self {
            Self::Owned(data) => data.try_borrow_owned(),
            Self::Scoped(_) => Err(Error::UserDataTypeMismatch),
        }
    }

    #[allow(unused)]
    #[inline(always)]
    pub(crate) fn try_borrow(&self) -> Result<UserDataBorrowRef<T>> {
        match self {
            Self::Owned(data) => data.try_borrow(),
            Self::Scoped(_) => Err(Error::UserDataTypeMismatch),
        }
    }

    #[inline(always)]
    pub(crate) fn try_borrow_mut(&self) -> Result<UserDataBorrowMut<T>> {
        match self {
            Self::Owned(data) => data.try_borrow_mut(),
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

    #[inline]
    pub(crate) fn try_borrow_scoped<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R> {
        match self {
            Self::Owned(data) => Ok(f(&*data.try_borrow()?)),
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
            Self::Owned(data) => Ok(f(&mut *data.try_borrow_mut()?)),
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

#[cfg(test)]
mod assertions {
    use super::*;

    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(UserDataRef<()>: Send, Sync);
    #[cfg(feature = "send")]
    static_assertions::assert_not_impl_all!(UserDataRef<std::rc::Rc<()>>: Send, Sync);
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(UserDataRefMut<()>: Sync, Send);
    #[cfg(feature = "send")]
    static_assertions::assert_not_impl_all!(UserDataRefMut<std::rc::Rc<()>>: Send, Sync);
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(UserDataBorrowRef<'_, ()>: Send, Sync);
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(UserDataBorrowMut<'_, ()>: Send, Sync);

    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_all!(UserDataRef<()>: Send, Sync);
    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_all!(UserDataRefMut<()>: Send, Sync);
    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_all!(UserDataBorrowRef<'_, ()>: Send, Sync);
    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_all!(UserDataBorrowMut<'_, ()>: Send, Sync);
}
