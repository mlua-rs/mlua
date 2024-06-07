use std::any::{Any, TypeId};
use std::cell::{Cell, Ref, RefCell, RefMut, UnsafeCell};
use std::hash::{Hash, Hasher};
use std::ops::{Deref, DerefMut};
use std::os::raw::{c_int, c_void};
use std::result::Result as StdResult;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::{fmt, mem, ptr};

use rustc_hash::FxHashMap;

use crate::error::Result;
#[cfg(not(feature = "luau"))]
use crate::hook::Debug;
use crate::lua::{ExtraData, Lua};

#[cfg(feature = "async")]
use {crate::value::MultiValue, futures_util::future::LocalBoxFuture};

#[cfg(feature = "unstable")]
use {crate::lua::LuaInner, std::marker::PhantomData};

#[cfg(all(feature = "luau", feature = "serialize"))]
use serde::ser::{Serialize, SerializeTupleStruct, Serializer};

/// Type of Lua integer numbers.
pub type Integer = ffi::lua_Integer;
/// Type of Lua floating point numbers.
pub type Number = ffi::lua_Number;

// Represents different subtypes wrapped to AnyUserData
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) enum SubtypeId {
    None,
    #[cfg(feature = "luau")]
    Buffer,
    #[cfg(feature = "luajit")]
    CData,
}

/// A "light" userdata value. Equivalent to an unmanaged raw pointer.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct LightUserData(usize);

impl LightUserData {
    pub fn new(ptr: *mut c_void) -> Self {
        LightUserData(ptr as usize)
    }

    pub const fn null_ptr() -> Self {
        LightUserData(0)
    }

    pub fn as_ptr(&self) -> *mut c_void {
        self.0 as *mut c_void
    }

    pub fn is_null(&self) -> bool {
        self.as_ptr().is_null()
    }
}

pub(crate) type Callback<'lua, 'a> = Box<dyn Fn(&'lua Lua, c_int) -> Result<c_int> + 'a>;

pub(crate) struct Upvalue<T> {
    pub(crate) data: T,
    pub(crate) extra: Arc<UnsafeCell<ExtraData>>,
}

pub(crate) type CallbackUpvalue = Upvalue<Callback<'static, 'static>>;

#[cfg(feature = "async")]
pub(crate) type AsyncCallback<'lua, 'a> =
    Box<dyn Fn(&'lua Lua, MultiValue<'lua>) -> LocalBoxFuture<'lua, Result<c_int>> + 'a>;

#[cfg(feature = "async")]
pub(crate) type AsyncCallbackUpvalue = Upvalue<AsyncCallback<'static, 'static>>;

#[cfg(feature = "async")]
pub(crate) type AsyncPollUpvalue = Upvalue<LocalBoxFuture<'static, Result<c_int>>>;

/// Type to set next Luau VM action after executing interrupt function.
#[cfg(any(feature = "luau", doc))]
#[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
pub enum VmState {
    Continue,
    Yield,
}

#[cfg(all(feature = "send", not(feature = "luau")))]
pub(crate) type HookCallback = Arc<dyn Fn(&Lua, Debug) -> Result<()> + Send>;

#[cfg(all(not(feature = "send"), not(feature = "luau")))]
pub(crate) type HookCallback = Arc<dyn Fn(&Lua, Debug) -> Result<()>>;

#[cfg(all(feature = "luau", feature = "send"))]
pub(crate) type InterruptCallback = Arc<dyn Fn(&Lua) -> Result<VmState> + Send>;

#[cfg(all(feature = "luau", not(feature = "send")))]
pub(crate) type InterruptCallback = Arc<dyn Fn(&Lua) -> Result<VmState>>;

#[cfg(all(feature = "send", feature = "lua54"))]
pub(crate) type WarnCallback = Box<dyn Fn(&Lua, &str, bool) -> Result<()> + Send>;

#[cfg(all(not(feature = "send"), feature = "lua54"))]
pub(crate) type WarnCallback = Box<dyn Fn(&Lua, &str, bool) -> Result<()>>;

#[cfg(feature = "send")]
pub trait MaybeSend: Send {}
#[cfg(feature = "send")]
impl<T: Send> MaybeSend for T {}

#[cfg(not(feature = "send"))]
pub trait MaybeSend {}
#[cfg(not(feature = "send"))]
impl<T> MaybeSend for T {}

/// A Luau vector type.
///
/// By default vectors are 3-dimensional, but can be 4-dimensional
/// if the `luau-vector4` feature is enabled.
#[cfg(any(feature = "luau", doc))]
#[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Vector(pub(crate) [f32; Self::SIZE]);

#[cfg(any(feature = "luau", doc))]
impl fmt::Display for Vector {
    #[rustfmt::skip]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[cfg(not(feature = "luau-vector4"))]
        return write!(f, "vector({}, {}, {})", self.x(), self.y(), self.z());
        #[cfg(feature = "luau-vector4")]
        return write!(f, "vector({}, {}, {}, {})", self.x(), self.y(), self.z(), self.w());
    }
}

#[cfg(any(feature = "luau", doc))]
impl Vector {
    pub(crate) const SIZE: usize = if cfg!(feature = "luau-vector4") { 4 } else { 3 };

    /// Creates a new vector.
    #[cfg(not(feature = "luau-vector4"))]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self([x, y, z])
    }

    /// Creates a new vector.
    #[cfg(feature = "luau-vector4")]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self([x, y, z, w])
    }

    /// Creates a new vector with all components set to `0.0`.
    #[doc(hidden)]
    pub const fn zero() -> Self {
        Self([0.0; Self::SIZE])
    }

    /// Returns 1st component of the vector.
    pub const fn x(&self) -> f32 {
        self.0[0]
    }

    /// Returns 2nd component of the vector.
    pub const fn y(&self) -> f32 {
        self.0[1]
    }

    /// Returns 3rd component of the vector.
    pub const fn z(&self) -> f32 {
        self.0[2]
    }

    /// Returns 4th component of the vector.
    #[cfg(any(feature = "luau-vector4", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau-vector4")))]
    pub const fn w(&self) -> f32 {
        self.0[3]
    }
}

#[cfg(all(feature = "luau", feature = "serialize"))]
impl Serialize for Vector {
    fn serialize<S: Serializer>(&self, serializer: S) -> StdResult<S::Ok, S::Error> {
        let mut ts = serializer.serialize_tuple_struct("Vector", Self::SIZE)?;
        ts.serialize_field(&self.x())?;
        ts.serialize_field(&self.y())?;
        ts.serialize_field(&self.z())?;
        #[cfg(feature = "luau-vector4")]
        ts.serialize_field(&self.w())?;
        ts.end()
    }
}

#[cfg(any(feature = "luau", doc))]
impl PartialEq<[f32; Self::SIZE]> for Vector {
    #[inline]
    fn eq(&self, other: &[f32; Self::SIZE]) -> bool {
        self.0 == *other
    }
}

pub(crate) struct DestructedUserdata;

/// An auto generated key into the Lua registry.
///
/// This is a handle to a value stored inside the Lua registry. It is not automatically
/// garbage collected on Drop, but it can be removed with [`Lua::remove_registry_value`],
/// and instances not manually removed can be garbage collected with [`Lua::expire_registry_values`].
///
/// Be warned, If you place this into Lua via a [`UserData`] type or a rust callback, it is *very
/// easy* to accidentally cause reference cycles that the Lua garbage collector cannot resolve.
/// Instead of placing a [`RegistryKey`] into a [`UserData`] type, prefer instead to use
/// [`AnyUserData::set_user_value`] / [`AnyUserData::user_value`].
///
/// [`UserData`]: crate::UserData
/// [`RegistryKey`]: crate::RegistryKey
/// [`Lua::remove_registry_value`]: crate::Lua::remove_registry_value
/// [`Lua::expire_registry_values`]: crate::Lua::expire_registry_values
/// [`AnyUserData::set_user_value`]: crate::AnyUserData::set_user_value
/// [`AnyUserData::user_value`]: crate::AnyUserData::user_value
pub struct RegistryKey {
    pub(crate) registry_id: c_int,
    pub(crate) is_nil: AtomicBool,
    pub(crate) unref_list: Arc<Mutex<Option<Vec<c_int>>>>,
}

impl fmt::Debug for RegistryKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "RegistryKey({})", self.registry_id)
    }
}

impl Hash for RegistryKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.registry_id.hash(state)
    }
}

impl PartialEq for RegistryKey {
    fn eq(&self, other: &RegistryKey) -> bool {
        self.registry_id == other.registry_id && Arc::ptr_eq(&self.unref_list, &other.unref_list)
    }
}

impl Eq for RegistryKey {}

impl Drop for RegistryKey {
    fn drop(&mut self) {
        // We don't need to collect nil slot
        if self.registry_id > ffi::LUA_REFNIL {
            let mut unref_list = mlua_expect!(self.unref_list.lock(), "unref list poisoned");
            if let Some(list) = unref_list.as_mut() {
                list.push(self.registry_id);
            }
        }
    }
}

impl RegistryKey {
    // Creates a new instance of `RegistryKey`
    pub(crate) const fn new(id: c_int, unref_list: Arc<Mutex<Option<Vec<c_int>>>>) -> Self {
        RegistryKey {
            registry_id: id,
            is_nil: AtomicBool::new(id == ffi::LUA_REFNIL),
            unref_list,
        }
    }

    // Destroys the `RegistryKey` without adding to the unref list
    pub(crate) fn take(self) -> c_int {
        let registry_id = self.registry_id;
        unsafe {
            ptr::read(&self.unref_list);
            mem::forget(self);
        }
        registry_id
    }

    // Returns true if this `RegistryKey` holds a nil value
    #[inline(always)]
    pub(crate) fn is_nil(&self) -> bool {
        self.is_nil.load(Ordering::Relaxed)
    }

    // Marks value of this `RegistryKey` as `Nil`
    #[inline(always)]
    pub(crate) fn set_nil(&self, enabled: bool) {
        // We cannot replace previous value with nil in as this will break
        // Lua mechanism to find free keys.
        // Instead, we set a special flag to mark value as nil.
        self.is_nil.store(enabled, Ordering::Relaxed);
    }
}

pub(crate) struct LuaRef<'lua> {
    pub(crate) lua: &'lua Lua,
    pub(crate) index: c_int,
    pub(crate) drop: bool,
}

impl<'lua> LuaRef<'lua> {
    pub(crate) const fn new(lua: &'lua Lua, index: c_int) -> Self {
        LuaRef {
            lua,
            index,
            drop: true,
        }
    }

    #[inline]
    pub(crate) fn to_pointer(&self) -> *const c_void {
        unsafe { ffi::lua_topointer(self.lua.ref_thread(), self.index) }
    }

    #[cfg(feature = "unstable")]
    #[inline]
    pub(crate) fn into_owned(self) -> LuaOwnedRef {
        assert!(self.drop, "Cannot turn non-drop reference into owned");
        let owned_ref = LuaOwnedRef::new(self.lua.clone(), self.index);
        mem::forget(self);
        owned_ref
    }
}

impl<'lua> fmt::Debug for LuaRef<'lua> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Ref({:p})", self.to_pointer())
    }
}

impl<'lua> Clone for LuaRef<'lua> {
    fn clone(&self) -> Self {
        self.lua.clone_ref(self)
    }
}

impl<'lua> Drop for LuaRef<'lua> {
    fn drop(&mut self) {
        if self.drop {
            self.lua.drop_ref_index(self.index);
        }
    }
}

impl<'lua> PartialEq for LuaRef<'lua> {
    fn eq(&self, other: &Self) -> bool {
        let ref_thread = self.lua.ref_thread();
        assert!(
            ref_thread == other.lua.ref_thread(),
            "Lua instance passed Value created from a different main Lua state"
        );
        unsafe { ffi::lua_rawequal(ref_thread, self.index, other.index) == 1 }
    }
}

#[cfg(feature = "unstable")]
pub(crate) struct LuaOwnedRef {
    pub(crate) inner: Arc<LuaInner>,
    pub(crate) index: c_int,
    _non_send: PhantomData<*const ()>,
}

#[cfg(feature = "unstable")]
impl fmt::Debug for LuaOwnedRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "OwnedRef({:p})", self.to_ref().to_pointer())
    }
}

#[cfg(feature = "unstable")]
impl Clone for LuaOwnedRef {
    fn clone(&self) -> Self {
        self.to_ref().clone().into_owned()
    }
}

#[cfg(feature = "unstable")]
impl Drop for LuaOwnedRef {
    fn drop(&mut self) {
        let lua: &Lua = unsafe { mem::transmute(&self.inner) };
        lua.drop_ref_index(self.index);
    }
}

#[cfg(feature = "unstable")]
impl LuaOwnedRef {
    pub(crate) const fn new(inner: Arc<LuaInner>, index: c_int) -> Self {
        LuaOwnedRef {
            inner,
            index,
            _non_send: PhantomData,
        }
    }

    pub(crate) const fn to_ref(&self) -> LuaRef {
        LuaRef {
            lua: unsafe { mem::transmute(&self.inner) },
            index: self.index,
            drop: false,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct AppData {
    #[cfg(not(feature = "send"))]
    container: UnsafeCell<FxHashMap<TypeId, RefCell<Box<dyn Any>>>>,
    #[cfg(feature = "send")]
    container: UnsafeCell<FxHashMap<TypeId, RefCell<Box<dyn Any + Send>>>>,
    borrow: Cell<usize>,
}

impl AppData {
    #[track_caller]
    pub(crate) fn insert<T: MaybeSend + 'static>(&self, data: T) -> Option<T> {
        match self.try_insert(data) {
            Ok(data) => data,
            Err(_) => panic!("cannot mutably borrow app data container"),
        }
    }

    pub(crate) fn try_insert<T: MaybeSend + 'static>(&self, data: T) -> StdResult<Option<T>, T> {
        if self.borrow.get() != 0 {
            return Err(data);
        }
        // SAFETY: we checked that there are no other references to the container
        Ok(unsafe { &mut *self.container.get() }
            .insert(TypeId::of::<T>(), RefCell::new(Box::new(data)))
            .and_then(|data| data.into_inner().downcast::<T>().ok().map(|data| *data)))
    }

    #[track_caller]
    pub(crate) fn borrow<T: 'static>(&self) -> Option<AppDataRef<T>> {
        let data = unsafe { &*self.container.get() }
            .get(&TypeId::of::<T>())?
            .borrow();
        self.borrow.set(self.borrow.get() + 1);
        Some(AppDataRef {
            data: Ref::filter_map(data, |data| data.downcast_ref()).ok()?,
            borrow: &self.borrow,
        })
    }

    #[track_caller]
    pub(crate) fn borrow_mut<T: 'static>(&self) -> Option<AppDataRefMut<T>> {
        let data = unsafe { &*self.container.get() }
            .get(&TypeId::of::<T>())?
            .borrow_mut();
        self.borrow.set(self.borrow.get() + 1);
        Some(AppDataRefMut {
            data: RefMut::filter_map(data, |data| data.downcast_mut()).ok()?,
            borrow: &self.borrow,
        })
    }

    #[track_caller]
    pub(crate) fn remove<T: 'static>(&self) -> Option<T> {
        if self.borrow.get() != 0 {
            panic!("cannot mutably borrow app data container");
        }
        // SAFETY: we checked that there are no other references to the container
        unsafe { &mut *self.container.get() }
            .remove(&TypeId::of::<T>())?
            .into_inner()
            .downcast::<T>()
            .ok()
            .map(|data| *data)
    }
}

/// A wrapper type for an immutably borrowed value from an app data container.
///
/// This type is similar to [`Ref`].
pub struct AppDataRef<'a, T: ?Sized + 'a> {
    data: Ref<'a, T>,
    borrow: &'a Cell<usize>,
}

impl<T: ?Sized> Drop for AppDataRef<'_, T> {
    fn drop(&mut self) {
        self.borrow.set(self.borrow.get() - 1);
    }
}

impl<T: ?Sized> Deref for AppDataRef<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<T: ?Sized + fmt::Display> fmt::Display for AppDataRef<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for AppDataRef<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

/// A wrapper type for a mutably borrowed value from an app data container.
///
/// This type is similar to [`RefMut`].
pub struct AppDataRefMut<'a, T: ?Sized + 'a> {
    data: RefMut<'a, T>,
    borrow: &'a Cell<usize>,
}

impl<T: ?Sized> Drop for AppDataRefMut<'_, T> {
    fn drop(&mut self) {
        self.borrow.set(self.borrow.get() - 1);
    }
}

impl<T: ?Sized> Deref for AppDataRefMut<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<T: ?Sized> DerefMut for AppDataRefMut<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

impl<T: ?Sized + fmt::Display> fmt::Display for AppDataRefMut<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for AppDataRefMut<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

#[cfg(test)]
mod assertions {
    use super::*;

    static_assertions::assert_impl_all!(RegistryKey: Send, Sync);
    static_assertions::assert_not_impl_any!(LuaRef: Send);

    #[cfg(feature = "unstable")]
    static_assertions::assert_not_impl_any!(LuaOwnedRef: Send);
}
