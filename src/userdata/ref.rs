use std::any::{type_name, TypeId};
use std::ops::{Deref, DerefMut};
use std::os::raw::c_int;
use std::{fmt, mem};

use crate::error::{Error, Result};
use crate::state::{Lua, RawLua};
use crate::traits::FromLua;
use crate::userdata::AnyUserData;
use crate::util::get_userdata;
use crate::value::Value;

use super::cell::{UserDataStorage, UserDataVariant};
use super::lock::{LockGuard, RawLock, UserDataLock};
use super::util::is_sync;

#[cfg(feature = "userdata-wrappers")]
use {
    parking_lot::{
        Mutex as MutexPL, MutexGuard as MutexGuardPL, RwLock as RwLockPL,
        RwLockReadGuard as RwLockReadGuardPL, RwLockWriteGuard as RwLockWriteGuardPL,
    },
    std::sync::Arc,
};
#[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
use {
    std::cell::{Ref, RefCell, RefMut},
    std::rc::Rc,
};

/// A wrapper type for a userdata value that provides read access.
///
/// It implements [`FromLua`] and can be used to receive a typed userdata from Lua.
pub struct UserDataRef<T: 'static> {
    // It's important to drop the guard first, as it refers to the `inner` data.
    _guard: LockGuard<'static, RawLock>,
    inner: UserDataRefInner<T>,
}

impl<T> Deref for UserDataRef<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.inner
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
        let guard = if cfg!(not(feature = "send")) || is_sync::<T>() {
            variant.raw_lock().try_lock_shared_guarded()
        } else {
            variant.raw_lock().try_lock_exclusive_guarded()
        };
        let guard = guard.map_err(|_| Error::UserDataBorrowError)?;
        let guard = unsafe { mem::transmute::<LockGuard<_>, LockGuard<'static, _>>(guard) };
        Ok(UserDataRef::from_parts(UserDataRefInner::Default(variant), guard))
    }
}

impl<T: 'static> FromLua for UserDataRef<T> {
    fn from_lua(value: Value, _: &Lua) -> Result<Self> {
        try_value_to_userdata::<T>(value)?.borrow()
    }

    #[inline]
    unsafe fn from_stack(idx: c_int, lua: &RawLua) -> Result<Self> {
        Self::borrow_from_stack(lua, lua.state(), idx)
    }
}

impl<T: 'static> UserDataRef<T> {
    #[inline(always)]
    fn from_parts(inner: UserDataRefInner<T>, guard: LockGuard<'static, RawLock>) -> Self {
        Self { _guard: guard, inner }
    }

    #[cfg(feature = "userdata-wrappers")]
    fn remap<U>(
        self,
        f: impl FnOnce(UserDataVariant<T>) -> Result<UserDataRefInner<U>>,
    ) -> Result<UserDataRef<U>> {
        match &self.inner {
            UserDataRefInner::Default(variant) => {
                let inner = f(variant.clone())?;
                Ok(UserDataRef::from_parts(inner, self._guard))
            }
            _ => Err(Error::UserDataTypeMismatch),
        }
    }

    pub(crate) unsafe fn borrow_from_stack(
        lua: &RawLua,
        state: *mut ffi::lua_State,
        idx: c_int,
    ) -> Result<Self> {
        let type_id = lua.get_userdata_type_id::<T>(state, idx)?;
        match type_id {
            Some(type_id) if type_id == TypeId::of::<T>() => {
                let ud = get_userdata::<UserDataStorage<T>>(state, idx);
                (*ud).try_borrow_owned()
            }

            #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
            Some(type_id) if type_id == TypeId::of::<Rc<T>>() => {
                let ud = get_userdata::<UserDataStorage<Rc<T>>>(state, idx);
                ((*ud).try_borrow_owned()).and_then(|ud| ud.transform_rc())
            }
            #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
            Some(type_id) if type_id == TypeId::of::<Rc<RefCell<T>>>() => {
                let ud = get_userdata::<UserDataStorage<Rc<RefCell<T>>>>(state, idx);
                ((*ud).try_borrow_owned()).and_then(|ud| ud.transform_rc_refcell())
            }

            #[cfg(feature = "userdata-wrappers")]
            Some(type_id) if type_id == TypeId::of::<Arc<T>>() => {
                let ud = get_userdata::<UserDataStorage<Arc<T>>>(state, idx);
                ((*ud).try_borrow_owned()).and_then(|ud| ud.transform_arc())
            }
            #[cfg(feature = "userdata-wrappers")]
            Some(type_id) if type_id == TypeId::of::<Arc<MutexPL<T>>>() => {
                let ud = get_userdata::<UserDataStorage<Arc<MutexPL<T>>>>(state, idx);
                ((*ud).try_borrow_owned()).and_then(|ud| ud.transform_arc_mutex_pl())
            }
            #[cfg(feature = "userdata-wrappers")]
            Some(type_id) if type_id == TypeId::of::<Arc<RwLockPL<T>>>() => {
                let ud = get_userdata::<UserDataStorage<Arc<RwLockPL<T>>>>(state, idx);
                ((*ud).try_borrow_owned()).and_then(|ud| ud.transform_arc_rwlock_pl())
            }
            _ => Err(Error::UserDataTypeMismatch),
        }
    }
}

#[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
impl<T> UserDataRef<Rc<T>> {
    fn transform_rc(self) -> Result<UserDataRef<T>> {
        self.remap(|variant| Ok(UserDataRefInner::Rc(variant)))
    }
}

#[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
impl<T> UserDataRef<Rc<RefCell<T>>> {
    fn transform_rc_refcell(self) -> Result<UserDataRef<T>> {
        self.remap(|variant| unsafe {
            let obj = &*variant.as_ptr();
            let r#ref = obj.try_borrow().map_err(|_| Error::UserDataBorrowError)?;
            let borrow = std::mem::transmute::<Ref<T>, Ref<'static, T>>(r#ref);
            Ok(UserDataRefInner::RcRefCell(borrow, variant))
        })
    }
}

#[cfg(feature = "userdata-wrappers")]
impl<T> UserDataRef<Arc<T>> {
    fn transform_arc(self) -> Result<UserDataRef<T>> {
        self.remap(|variant| Ok(UserDataRefInner::Arc(variant)))
    }
}

#[cfg(feature = "userdata-wrappers")]
impl<T> UserDataRef<Arc<MutexPL<T>>> {
    fn transform_arc_mutex_pl(self) -> Result<UserDataRef<T>> {
        self.remap(|variant| unsafe {
            let obj = &*variant.as_ptr();
            let guard = obj.try_lock().ok_or(Error::UserDataBorrowError)?;
            let borrow = std::mem::transmute::<MutexGuardPL<T>, MutexGuardPL<'static, T>>(guard);
            Ok(UserDataRefInner::ArcMutexPL(borrow, variant))
        })
    }
}

#[cfg(feature = "userdata-wrappers")]
impl<T> UserDataRef<Arc<RwLockPL<T>>> {
    fn transform_arc_rwlock_pl(self) -> Result<UserDataRef<T>> {
        self.remap(|variant| unsafe {
            let obj = &*variant.as_ptr();
            let guard = obj.try_read().ok_or(Error::UserDataBorrowError)?;
            let borrow = std::mem::transmute::<RwLockReadGuardPL<T>, RwLockReadGuardPL<'static, T>>(guard);
            Ok(UserDataRefInner::ArcRwLockPL(borrow, variant))
        })
    }
}

#[allow(unused)]
enum UserDataRefInner<T: 'static> {
    Default(UserDataVariant<T>),

    #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
    Rc(UserDataVariant<Rc<T>>),
    #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
    RcRefCell(Ref<'static, T>, UserDataVariant<Rc<RefCell<T>>>),

    #[cfg(feature = "userdata-wrappers")]
    Arc(UserDataVariant<Arc<T>>),
    #[cfg(feature = "userdata-wrappers")]
    ArcMutexPL(MutexGuardPL<'static, T>, UserDataVariant<Arc<MutexPL<T>>>),
    #[cfg(feature = "userdata-wrappers")]
    ArcRwLockPL(RwLockReadGuardPL<'static, T>, UserDataVariant<Arc<RwLockPL<T>>>),
}

impl<T> Deref for UserDataRefInner<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        match self {
            Self::Default(inner) => unsafe { &*inner.as_ptr() },

            #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
            Self::Rc(inner) => unsafe { &*Rc::as_ptr(&*inner.as_ptr()) },
            #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
            Self::RcRefCell(x, ..) => x,

            #[cfg(feature = "userdata-wrappers")]
            Self::Arc(inner) => unsafe { &*Arc::as_ptr(&*inner.as_ptr()) },
            #[cfg(feature = "userdata-wrappers")]
            Self::ArcMutexPL(x, ..) => x,
            #[cfg(feature = "userdata-wrappers")]
            Self::ArcRwLockPL(x, ..) => x,
        }
    }
}

/// A wrapper type for a userdata value that provides read and write access.
///
/// It implements [`FromLua`] and can be used to receive a typed userdata from Lua.
pub struct UserDataRefMut<T: 'static> {
    // It's important to drop the guard first, as it refers to the `inner` data.
    _guard: LockGuard<'static, RawLock>,
    inner: UserDataRefMutInner<T>,
}

impl<T> Deref for UserDataRefMut<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> DerefMut for UserDataRefMut<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
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
        let guard = variant.raw_lock().try_lock_exclusive_guarded();
        let guard = guard.map_err(|_| Error::UserDataBorrowMutError)?;
        let guard = unsafe { mem::transmute::<LockGuard<_>, LockGuard<'static, _>>(guard) };
        Ok(UserDataRefMut::from_parts(
            UserDataRefMutInner::Default(variant),
            guard,
        ))
    }
}

impl<T: 'static> FromLua for UserDataRefMut<T> {
    fn from_lua(value: Value, _: &Lua) -> Result<Self> {
        try_value_to_userdata::<T>(value)?.borrow_mut()
    }

    unsafe fn from_stack(idx: c_int, lua: &RawLua) -> Result<Self> {
        Self::borrow_from_stack(lua, lua.state(), idx)
    }
}

impl<T: 'static> UserDataRefMut<T> {
    #[inline(always)]
    fn from_parts(inner: UserDataRefMutInner<T>, guard: LockGuard<'static, RawLock>) -> Self {
        Self { _guard: guard, inner }
    }

    #[cfg(feature = "userdata-wrappers")]
    fn remap<U>(
        self,
        f: impl FnOnce(UserDataVariant<T>) -> Result<UserDataRefMutInner<U>>,
    ) -> Result<UserDataRefMut<U>> {
        match &self.inner {
            UserDataRefMutInner::Default(variant) => {
                let inner = f(variant.clone())?;
                Ok(UserDataRefMut::from_parts(inner, self._guard))
            }
            _ => Err(Error::UserDataTypeMismatch),
        }
    }

    pub(crate) unsafe fn borrow_from_stack(
        lua: &RawLua,
        state: *mut ffi::lua_State,
        idx: c_int,
    ) -> Result<Self> {
        let type_id = lua.get_userdata_type_id::<T>(state, idx)?;
        match type_id {
            Some(type_id) if type_id == TypeId::of::<T>() => {
                let ud = get_userdata::<UserDataStorage<T>>(state, idx);
                (*ud).try_borrow_owned_mut()
            }

            #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
            Some(type_id) if type_id == TypeId::of::<Rc<T>>() => Err(Error::UserDataBorrowMutError),
            #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
            Some(type_id) if type_id == TypeId::of::<Rc<RefCell<T>>>() => {
                let ud = get_userdata::<UserDataStorage<Rc<RefCell<T>>>>(state, idx);
                ((*ud).try_borrow_owned_mut()).and_then(|ud| ud.transform_rc_refcell())
            }

            #[cfg(feature = "userdata-wrappers")]
            Some(type_id) if type_id == TypeId::of::<Arc<T>>() => Err(Error::UserDataBorrowMutError),
            #[cfg(feature = "userdata-wrappers")]
            Some(type_id) if type_id == TypeId::of::<Arc<MutexPL<T>>>() => {
                let ud = get_userdata::<UserDataStorage<Arc<MutexPL<T>>>>(state, idx);
                ((*ud).try_borrow_owned_mut()).and_then(|ud| ud.transform_arc_mutex_pl())
            }
            #[cfg(feature = "userdata-wrappers")]
            Some(type_id) if type_id == TypeId::of::<Arc<RwLockPL<T>>>() => {
                let ud = get_userdata::<UserDataStorage<Arc<RwLockPL<T>>>>(state, idx);
                ((*ud).try_borrow_owned_mut()).and_then(|ud| ud.transform_arc_rwlock_pl())
            }
            _ => Err(Error::UserDataTypeMismatch),
        }
    }
}

#[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
impl<T> UserDataRefMut<Rc<RefCell<T>>> {
    fn transform_rc_refcell(self) -> Result<UserDataRefMut<T>> {
        self.remap(|variant| unsafe {
            let obj = &*variant.as_ptr();
            let refmut = obj.try_borrow_mut().map_err(|_| Error::UserDataBorrowMutError)?;
            let borrow = std::mem::transmute::<RefMut<T>, RefMut<'static, T>>(refmut);
            Ok(UserDataRefMutInner::RcRefCell(borrow, variant))
        })
    }
}

#[cfg(feature = "userdata-wrappers")]
impl<T> UserDataRefMut<Arc<MutexPL<T>>> {
    fn transform_arc_mutex_pl(self) -> Result<UserDataRefMut<T>> {
        self.remap(|variant| unsafe {
            let obj = &*variant.as_ptr();
            let guard = obj.try_lock().ok_or(Error::UserDataBorrowMutError)?;
            let borrow = std::mem::transmute::<MutexGuardPL<T>, MutexGuardPL<'static, T>>(guard);
            Ok(UserDataRefMutInner::ArcMutexPL(borrow, variant))
        })
    }
}

#[cfg(feature = "userdata-wrappers")]
impl<T> UserDataRefMut<Arc<RwLockPL<T>>> {
    fn transform_arc_rwlock_pl(self) -> Result<UserDataRefMut<T>> {
        self.remap(|variant| unsafe {
            let obj = &*variant.as_ptr();
            let guard = obj.try_write().ok_or(Error::UserDataBorrowMutError)?;
            let borrow = std::mem::transmute::<RwLockWriteGuardPL<T>, RwLockWriteGuardPL<'static, T>>(guard);
            Ok(UserDataRefMutInner::ArcRwLockPL(borrow, variant))
        })
    }
}

#[allow(unused)]
enum UserDataRefMutInner<T: 'static> {
    Default(UserDataVariant<T>),

    #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
    RcRefCell(RefMut<'static, T>, UserDataVariant<Rc<RefCell<T>>>),

    #[cfg(feature = "userdata-wrappers")]
    ArcMutexPL(MutexGuardPL<'static, T>, UserDataVariant<Arc<MutexPL<T>>>),
    #[cfg(feature = "userdata-wrappers")]
    ArcRwLockPL(RwLockWriteGuardPL<'static, T>, UserDataVariant<Arc<RwLockPL<T>>>),
}

impl<T> Deref for UserDataRefMutInner<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        match self {
            Self::Default(inner) => unsafe { &*inner.as_ptr() },

            #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
            Self::RcRefCell(x, ..) => x,

            #[cfg(feature = "userdata-wrappers")]
            Self::ArcMutexPL(x, ..) => x,
            #[cfg(feature = "userdata-wrappers")]
            Self::ArcRwLockPL(x, ..) => x,
        }
    }
}

impl<T> DerefMut for UserDataRefMutInner<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        match self {
            Self::Default(inner) => unsafe { &mut *inner.as_ptr() },

            #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
            Self::RcRefCell(x, ..) => x,

            #[cfg(feature = "userdata-wrappers")]
            Self::ArcMutexPL(x, ..) => x,
            #[cfg(feature = "userdata-wrappers")]
            Self::ArcRwLockPL(x, ..) => x,
        }
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

    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_all!(UserDataRef<()>: Send, Sync);
    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_all!(UserDataRefMut<()>: Send, Sync);
}
