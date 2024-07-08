use std::any::{type_name, TypeId};
use std::cell::{Cell, UnsafeCell};
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::os::raw::c_int;
use std::rc::Rc;

#[cfg(feature = "serialize")]
use serde::ser::{Serialize, Serializer};

use crate::error::{Error, Result};
use crate::state::RawLua;
use crate::state::{Lua, LuaGuard};
use crate::userdata::AnyUserData;
use crate::util::get_userdata;
use crate::value::{FromLua, Value};

// A enum for storing userdata values.
// It's stored inside a Lua VM and protected by the outer `ReentrantMutex`.
pub(crate) enum UserDataVariant<T> {
    Default(Rc<InnerRefCell<T>>),
    #[cfg(feature = "serialize")]
    Serializable(Rc<InnerRefCell<Box<dyn erased_serde::Serialize>>>),
}

impl<T> Clone for UserDataVariant<T> {
    #[inline]
    fn clone(&self) -> Self {
        match self {
            Self::Default(inner) => Self::Default(Rc::clone(inner)),
            #[cfg(feature = "serialize")]
            Self::Serializable(inner) => UserDataVariant::Serializable(Rc::clone(inner)),
        }
    }
}

impl<T> UserDataVariant<T> {
    #[inline(always)]
    pub(crate) fn new(data: T) -> Self {
        Self::Default(Rc::new(InnerRefCell::new(data)))
    }

    // Immutably borrows the wrapped value in-place.
    #[inline(always)]
    pub(crate) unsafe fn try_borrow(&self) -> Result<UserDataBorrowRef<T>> {
        UserDataBorrowRef::try_from(self)
    }

    // Immutably borrows the wrapped value and returns an owned reference.
    #[inline(always)]
    pub(crate) fn try_make_ref(&self, guard: LuaGuard) -> Result<UserDataRef<T>> {
        UserDataRef::try_from(self.clone(), guard)
    }

    // Mutably borrows the wrapped value in-place.
    #[inline(always)]
    pub(crate) unsafe fn try_borrow_mut(&self) -> Result<UserDataBorrowMut<T>> {
        UserDataBorrowMut::try_from(self)
    }

    // Mutably borrows the wrapped value and returns an owned reference.
    #[inline(always)]
    pub(crate) fn try_make_mut_ref(&self, guard: LuaGuard) -> Result<UserDataRefMut<T>> {
        UserDataRefMut::try_from(self.clone(), guard)
    }

    // Returns the wrapped value.
    //
    // This method checks that we have exclusive access to the value.
    pub(crate) fn into_inner(self) -> Result<T> {
        set_writing(self.flag())?;
        Ok(match self {
            Self::Default(inner) => Rc::into_inner(inner).unwrap().value.into_inner(),
            #[cfg(feature = "serialize")]
            Self::Serializable(inner) => unsafe {
                let raw = Box::into_raw(Rc::into_inner(inner).unwrap().value.into_inner());
                *Box::from_raw(raw as *mut T)
            },
        })
    }

    #[inline(always)]
    fn flag(&self) -> &Cell<BorrowFlag> {
        match self {
            Self::Default(inner) => &inner.borrow,
            #[cfg(feature = "serialize")]
            Self::Serializable(inner) => &inner.borrow,
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
impl<T: Serialize + 'static> UserDataVariant<T> {
    #[inline(always)]
    pub(crate) fn new_ser(data: T) -> Self {
        let data = Box::new(data) as Box<dyn erased_serde::Serialize>;
        Self::Serializable(Rc::new(InnerRefCell::new(data)))
    }
}

#[cfg(feature = "serialize")]
impl Serialize for UserDataVariant<()> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            UserDataVariant::Default(_) => {
                Err(serde::ser::Error::custom("cannot serialize <userdata>"))
            }
            UserDataVariant::Serializable(inner) => unsafe {
                let _ = self.try_borrow().map_err(serde::ser::Error::custom)?;
                (*inner.value.get()).serialize(serializer)
            },
        }
    }
}

//
// Inspired by `std::cell::RefCell`` implementation
//

pub(crate) struct InnerRefCell<T> {
    borrow: Cell<BorrowFlag>,
    value: UnsafeCell<T>,
}

impl<T> InnerRefCell<T> {
    #[inline(always)]
    pub fn new(value: T) -> Self {
        InnerRefCell {
            borrow: Cell::new(UNUSED),
            value: UnsafeCell::new(value),
        }
    }
}

/// A wrapper type for a [`UserData`] value that provides read access.
///
/// It implements [`FromLua`] and can be used to receive a typed userdata from Lua.
pub struct UserDataRef<T> {
    variant: UserDataVariant<T>,
    #[allow(unused)]
    guard: LuaGuard,
}

impl<T> Deref for UserDataRef<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.variant.as_ptr() }
    }
}

impl<T> Drop for UserDataRef<T> {
    #[inline]
    fn drop(&mut self) {
        unset_reading(self.variant.flag());
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

impl<T> UserDataRef<T> {
    #[inline]
    fn try_from(variant: UserDataVariant<T>, guard: LuaGuard) -> Result<Self> {
        set_reading(variant.flag())?;
        Ok(UserDataRef { variant, guard })
    }
}

impl<T: 'static> FromLua for UserDataRef<T> {
    fn from_lua(value: Value, _: &Lua) -> Result<Self> {
        try_value_to_userdata::<T>(value)?.borrow()
    }

    unsafe fn from_stack(idx: c_int, lua: &RawLua) -> Result<Self> {
        let type_id = lua.get_userdata_type_id(idx)?;
        match type_id {
            Some(type_id) if type_id == TypeId::of::<T>() => {
                let guard = lua.lua().lock_arc();
                (*get_userdata::<UserDataVariant<T>>(lua.state(), idx)).try_make_ref(guard)
            }
            _ => Err(Error::UserDataTypeMismatch),
        }
    }
}

/// A wrapper type for a mutably borrowed value from a `AnyUserData`.
///
/// It implements [`FromLua`] and can be used to receive a typed userdata from Lua.
pub struct UserDataRefMut<T> {
    variant: UserDataVariant<T>,
    #[allow(unused)]
    guard: LuaGuard,
}

impl<T> Deref for UserDataRefMut<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.variant.as_ptr() }
    }
}

impl<T> DerefMut for UserDataRefMut<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.variant.as_ptr() }
    }
}

impl<T> Drop for UserDataRefMut<T> {
    #[inline]
    fn drop(&mut self) {
        unset_writing(self.variant.flag());
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

impl<T> UserDataRefMut<T> {
    fn try_from(variant: UserDataVariant<T>, guard: LuaGuard) -> Result<Self> {
        // There must currently be no existing references
        set_writing(variant.flag())?;
        Ok(UserDataRefMut { variant, guard })
    }
}

impl<T: 'static> FromLua for UserDataRefMut<T> {
    fn from_lua(value: Value, _: &Lua) -> Result<Self> {
        try_value_to_userdata::<T>(value)?.borrow_mut()
    }

    unsafe fn from_stack(idx: c_int, lua: &RawLua) -> Result<Self> {
        let type_id = lua.get_userdata_type_id(idx)?;
        match type_id {
            Some(type_id) if type_id == TypeId::of::<T>() => {
                let guard = lua.lua().lock_arc();
                (*get_userdata::<UserDataVariant<T>>(lua.state(), idx)).try_make_mut_ref(guard)
            }
            _ => Err(Error::UserDataTypeMismatch),
        }
    }
}

// Positive values represent the number of `Ref` active. Negative values
// represent the number of `RefMut` active. Multiple `RefMut`s can only be
// active at a time if they refer to distinct, nonoverlapping components of a
// `RefCell` (e.g., different ranges of a slice).
type BorrowFlag = isize;
const UNUSED: BorrowFlag = 0;

#[inline(always)]
fn is_writing(x: BorrowFlag) -> bool {
    x < UNUSED
}

#[inline(always)]
fn is_reading(x: BorrowFlag) -> bool {
    x > UNUSED
}

#[inline(always)]
fn set_writing(borrow: &Cell<BorrowFlag>) -> Result<()> {
    let flag = borrow.get();
    if flag != UNUSED {
        return Err(Error::UserDataBorrowMutError);
    }
    borrow.set(UNUSED - 1);
    Ok(())
}

#[inline(always)]
fn set_reading(borrow: &Cell<BorrowFlag>) -> Result<()> {
    let flag = borrow.get().wrapping_add(1);
    if !is_reading(flag) {
        return Err(Error::UserDataBorrowError);
    }
    borrow.set(flag);
    Ok(())
}

#[inline(always)]
#[track_caller]
fn unset_writing(borrow: &Cell<BorrowFlag>) {
    let flag = borrow.get();
    debug_assert!(is_writing(flag));
    borrow.set(flag + 1);
}

#[inline(always)]
#[track_caller]
fn unset_reading(borrow: &Cell<BorrowFlag>) {
    let flag = borrow.get();
    debug_assert!(is_reading(flag));
    borrow.set(flag - 1);
}

pub(crate) struct UserDataBorrowRef<'a, T>(&'a UserDataVariant<T>);

impl<'a, T> Drop for UserDataBorrowRef<'a, T> {
    #[inline]
    fn drop(&mut self) {
        unset_reading(self.0.flag());
    }
}

impl<'a, T> Deref for UserDataBorrowRef<'a, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.0.as_ptr() }
    }
}

impl<'a, T> TryFrom<&'a UserDataVariant<T>> for UserDataBorrowRef<'a, T> {
    type Error = Error;

    #[inline(always)]
    fn try_from(variant: &'a UserDataVariant<T>) -> Result<Self> {
        set_reading(variant.flag())?;
        Ok(UserDataBorrowRef(variant))
    }
}

impl<'a, T> UserDataBorrowRef<'a, T> {
    #[inline(always)]
    pub(crate) fn get_ref(&self) -> &'a T {
        // SAFETY: `UserDataBorrowRef` is only created when the borrow flag is set to reading.
        unsafe { &*self.0.as_ptr() }
    }
}

pub(crate) struct UserDataBorrowMut<'a, T>(&'a UserDataVariant<T>);

impl<'a, T> Drop for UserDataBorrowMut<'a, T> {
    #[inline]
    fn drop(&mut self) {
        unset_writing(self.0.flag());
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
        set_writing(variant.flag())?;
        Ok(UserDataBorrowMut(variant))
    }
}

impl<'a, T> UserDataBorrowMut<'a, T> {
    #[inline(always)]
    pub(crate) fn get_mut(&mut self) -> &'a mut T {
        // SAFETY: `UserDataBorrowMut` is only created when the borrow flag is set to writing.
        unsafe { &mut *self.0.as_ptr() }
    }
}

#[inline]
fn try_value_to_userdata<T>(value: Value) -> Result<AnyUserData> {
    match value {
        Value::UserData(ud) => Ok(ud),
        _ => Err(Error::FromLuaConversionError {
            from: value.type_name(),
            to: "userdata",
            message: Some(format!("expected userdata of type {}", type_name::<T>())),
        }),
    }
}

#[cfg(test)]
mod assertions {
    use super::*;

    static_assertions::assert_not_impl_all!(UserDataRef<()>: Sync, Send);
    static_assertions::assert_not_impl_all!(UserDataRefMut<()>: Sync, Send);
}
