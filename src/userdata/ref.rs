use std::any::{TypeId, type_name};
use std::ops::{Deref, DerefMut};
use std::os::raw::c_int;
use std::{fmt, mem};

use maybe_dangling::MaybeDangling;

use crate::error::{Error, Result};
use crate::state::{Lua, RawLua};
use crate::traits::FromLua;
use crate::userdata::AnyUserData;
use crate::util::{check_stack, get_userdata, take_userdata};
use crate::value::Value;

use super::cell::{UserDataStorage, UserDataVariant};
use super::lock::{LockGuard, RawLock, UserDataLock};

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

#[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
type RcRefCellRef<T> = MaybeDangling<Ref<'static, T>>;
#[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
type RcRefCellRefMut<T> = MaybeDangling<RefMut<'static, T>>;

struct UserDataGuard<T: 'static> {
    // Keep the guard before its owner so the lock is released before the allocation is freed.
    _guard: MaybeDangling<LockGuard<'static, RawLock>>,
    variant: UserDataVariant<T>,
}

impl<T> UserDataGuard<T> {
    #[inline]
    fn try_read(variant: UserDataVariant<T>) -> Result<Self> {
        let guard = variant
            .raw_lock()
            .try_lock_shared_guarded()
            .map_err(|_| Error::UserDataBorrowError)?;
        let guard = unsafe { mem::transmute::<LockGuard<'_, _>, LockGuard<'static, _>>(guard) };
        Ok(Self::new(variant, guard))
    }

    #[inline]
    fn try_write(variant: UserDataVariant<T>) -> Result<Self> {
        let guard = variant
            .raw_lock()
            .try_lock_exclusive_guarded()
            .map_err(|_| Error::UserDataBorrowMutError)?;
        let guard = unsafe { mem::transmute::<LockGuard<'_, _>, LockGuard<'static, _>>(guard) };
        Ok(Self::new(variant, guard))
    }

    #[inline(always)]
    fn new(variant: UserDataVariant<T>, guard: LockGuard<'static, RawLock>) -> Self {
        Self {
            _guard: MaybeDangling::new(guard),
            variant,
        }
    }

    #[inline(always)]
    fn as_ptr(&self) -> *mut T {
        self.variant.as_ptr()
    }

    #[inline(always)]
    fn get(&self) -> &T {
        // SAFETY: `variant` owns the value and `_guard` keeps it locked.
        unsafe { &*self.as_ptr() }
    }
}

/// A wrapper type for a userdata value that provides read access.
///
/// It implements [`FromLua`] and can be used to receive a typed userdata from Lua.
pub struct UserDataRef<T: 'static> {
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
        // Shared (read) lock is always correct:
        // - with `send` feature, `T: Sync` is guaranteed by the `MaybeSync` bound on userdata
        //   creation
        // - without `send` feature, single-threaded access makes shared lock safe for any `T`
        Ok(UserDataRef {
            inner: UserDataRefInner::Default(UserDataGuard::try_read(variant)?),
        })
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
    #[cfg(feature = "userdata-wrappers")]
    fn remap<U>(
        self,
        f: impl FnOnce(UserDataGuard<T>) -> Result<UserDataRefInner<U>>,
    ) -> Result<UserDataRef<U>> {
        match self.inner {
            UserDataRefInner::Default(guard) => Ok(UserDataRef { inner: f(guard)? }),
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
        self.remap(|owner| Ok(UserDataRefInner::Rc(owner)))
    }
}

#[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
impl<T> UserDataRef<Rc<RefCell<T>>> {
    fn transform_rc_refcell(self) -> Result<UserDataRef<T>> {
        self.remap(|owner| {
            let obj = owner.get();
            let borrow = obj.try_borrow().map_err(|_| Error::UserDataBorrowError)?;
            let borrow = unsafe { mem::transmute::<Ref<T>, Ref<'static, T>>(borrow) };
            let borrow = MaybeDangling::new(borrow);
            Ok(UserDataRefInner::RcRefCell(borrow, owner))
        })
    }
}

#[cfg(feature = "userdata-wrappers")]
impl<T> UserDataRef<Arc<T>> {
    fn transform_arc(self) -> Result<UserDataRef<T>> {
        self.remap(|owner| Ok(UserDataRefInner::Arc(owner)))
    }
}

#[cfg(feature = "userdata-wrappers")]
impl<T> UserDataRef<Arc<MutexPL<T>>> {
    fn transform_arc_mutex_pl(self) -> Result<UserDataRef<T>> {
        self.remap(|owner| {
            let obj = owner.get();
            let borrow = obj.try_lock().ok_or(Error::UserDataBorrowError)?;
            let borrow = unsafe { mem::transmute::<MutexGuardPL<'_, T>, MutexGuardPL<'static, T>>(borrow) };
            let borrow = MaybeDangling::new(borrow);
            Ok(UserDataRefInner::ArcMutexPL(borrow, owner))
        })
    }
}

#[cfg(feature = "userdata-wrappers")]
impl<T> UserDataRef<Arc<RwLockPL<T>>> {
    fn transform_arc_rwlock_pl(self) -> Result<UserDataRef<T>> {
        self.remap(|owner| {
            let obj = owner.get();
            let borrow = obj.try_read().ok_or(Error::UserDataBorrowError)?;
            let borrow =
                unsafe { mem::transmute::<RwLockReadGuardPL<'_, T>, RwLockReadGuardPL<'static, T>>(borrow) };
            let borrow = MaybeDangling::new(borrow);
            Ok(UserDataRefInner::ArcRwLockPL(borrow, owner))
        })
    }
}

#[allow(unused)]
// In variants with a separate borrowing guard, the guard must remain before its owner.
enum UserDataRefInner<T: 'static> {
    Default(UserDataGuard<T>),

    #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
    Rc(UserDataGuard<Rc<T>>),
    #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
    RcRefCell(RcRefCellRef<T>, UserDataGuard<Rc<RefCell<T>>>),

    #[cfg(feature = "userdata-wrappers")]
    Arc(UserDataGuard<Arc<T>>),
    #[cfg(feature = "userdata-wrappers")]
    ArcMutexPL(
        MaybeDangling<MutexGuardPL<'static, T>>,
        UserDataGuard<Arc<MutexPL<T>>>,
    ),
    #[cfg(feature = "userdata-wrappers")]
    ArcRwLockPL(
        MaybeDangling<RwLockReadGuardPL<'static, T>>,
        UserDataGuard<Arc<RwLockPL<T>>>,
    ),
}

impl<T> Deref for UserDataRefInner<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        match self {
            Self::Default(inner) => inner.get(),

            #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
            Self::Rc(inner) => inner.get(),
            #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
            Self::RcRefCell(x, ..) => x,

            #[cfg(feature = "userdata-wrappers")]
            Self::Arc(inner) => inner.get(),
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
        Ok(UserDataRefMut {
            inner: UserDataRefMutInner::Default(UserDataGuard::try_write(variant)?),
        })
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
    #[cfg(feature = "userdata-wrappers")]
    fn remap<U>(
        self,
        f: impl FnOnce(UserDataGuard<T>) -> Result<UserDataRefMutInner<U>>,
    ) -> Result<UserDataRefMut<U>> {
        match self.inner {
            UserDataRefMutInner::Default(guard) => Ok(UserDataRefMut { inner: f(guard)? }),
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
        self.remap(|owner| {
            let obj = owner.get();
            let borrow = obj.try_borrow_mut().map_err(|_| Error::UserDataBorrowMutError)?;
            let borrow = unsafe { mem::transmute::<RefMut<T>, RefMut<'static, T>>(borrow) };
            let borrow = MaybeDangling::new(borrow);
            Ok(UserDataRefMutInner::RcRefCell(borrow, owner))
        })
    }
}

#[cfg(feature = "userdata-wrappers")]
impl<T> UserDataRefMut<Arc<MutexPL<T>>> {
    fn transform_arc_mutex_pl(self) -> Result<UserDataRefMut<T>> {
        self.remap(|owner| {
            let obj = owner.get();
            let borrow = obj.try_lock().ok_or(Error::UserDataBorrowMutError)?;
            let borrow = unsafe { mem::transmute::<MutexGuardPL<'_, T>, MutexGuardPL<'static, T>>(borrow) };
            let borrow = MaybeDangling::new(borrow);
            Ok(UserDataRefMutInner::ArcMutexPL(borrow, owner))
        })
    }
}

#[cfg(feature = "userdata-wrappers")]
impl<T> UserDataRefMut<Arc<RwLockPL<T>>> {
    fn transform_arc_rwlock_pl(self) -> Result<UserDataRefMut<T>> {
        self.remap(|owner| {
            let obj = owner.get();
            let borrow = obj.try_write().ok_or(Error::UserDataBorrowMutError)?;
            let borrow = unsafe {
                mem::transmute::<RwLockWriteGuardPL<'_, T>, RwLockWriteGuardPL<'static, T>>(borrow)
            };
            let borrow = MaybeDangling::new(borrow);
            Ok(UserDataRefMutInner::ArcRwLockPL(borrow, owner))
        })
    }
}

#[allow(unused)]
// In variants with a separate borrowing guard, the guard must remain before its owner.
enum UserDataRefMutInner<T: 'static> {
    Default(UserDataGuard<T>),

    #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
    RcRefCell(RcRefCellRefMut<T>, UserDataGuard<Rc<RefCell<T>>>),

    #[cfg(feature = "userdata-wrappers")]
    ArcMutexPL(
        MaybeDangling<MutexGuardPL<'static, T>>,
        UserDataGuard<Arc<MutexPL<T>>>,
    ),
    #[cfg(feature = "userdata-wrappers")]
    ArcRwLockPL(
        MaybeDangling<RwLockWriteGuardPL<'static, T>>,
        UserDataGuard<Arc<RwLockPL<T>>>,
    ),
}

impl<T> Deref for UserDataRefMutInner<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        match self {
            Self::Default(inner) => inner.get(),

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

/// A wrapper type that takes ownership of a userdata value.
///
/// It implements [`FromLua`] and can be used to receive a typed userdata from Lua by taking
/// ownership of it.
/// The original Lua userdata is marked as destructed and cannot be used further.
pub struct UserDataOwned<T>(pub T);

impl<T> Deref for UserDataOwned<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for UserDataOwned<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T: fmt::Debug> fmt::Debug for UserDataOwned<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: fmt::Display> fmt::Display for UserDataOwned<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: 'static> FromLua for UserDataOwned<T> {
    fn from_lua(value: Value, _: &Lua) -> Result<Self> {
        try_value_to_userdata::<T>(value)?.take().map(UserDataOwned)
    }

    unsafe fn from_stack(idx: c_int, lua: &RawLua) -> Result<Self> {
        let state = lua.state();
        let type_id = lua.get_userdata_type_id::<T>(state, idx)?;
        match type_id {
            Some(type_id) if type_id == TypeId::of::<T>() => {
                let ud = get_userdata::<UserDataStorage<T>>(state, idx);
                if (*ud).has_exclusive_access() {
                    check_stack(state, 1)?;
                    take_userdata::<UserDataStorage<T>>(state, idx)
                        .into_inner()
                        .map(UserDataOwned)
                } else {
                    Err(Error::UserDataBorrowMutError)
                }
            }
            _ => Err(Error::UserDataTypeMismatch),
        }
    }
}

#[inline]
fn try_value_to_userdata<T>(value: Value) -> Result<AnyUserData> {
    match value {
        Value::UserData(ud) => Ok(ud),
        _ => Err(Error::from_lua_conversion(
            value.type_name(),
            "userdata",
            format!("expected userdata of type {}", type_name::<T>()),
        )),
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
    static_assertions::assert_impl_all!(UserDataOwned<()>: Send, Sync);
    #[cfg(feature = "send")]
    static_assertions::assert_not_impl_all!(UserDataOwned<std::rc::Rc<()>>: Send, Sync);

    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_all!(UserDataRef<()>: Send, Sync);
    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_all!(UserDataRefMut<()>: Send, Sync);
}
