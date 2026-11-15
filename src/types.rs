use std::cell::UnsafeCell;
use std::mem::ManuallyDrop;
use std::os::raw::{c_int, c_void};

#[cfg(not(feature = "luau"))]
use crate::debug::{Debug, HookTriggers};
use crate::error::Result;
use crate::state::{ExtraData, Lua, RawLua};

// Re-export mutex wrappers
pub(crate) use sync::{ArcReentrantMutexGuard, ReentrantMutex, ReentrantMutexGuard, XRc, XWeak};

#[cfg(all(feature = "async", feature = "send"))]
pub(crate) type BoxFuture<'a, T> = futures_util::future::BoxFuture<'a, T>;

#[cfg(all(feature = "async", not(feature = "send")))]
pub(crate) type BoxFuture<'a, T> = futures_util::future::LocalBoxFuture<'a, T>;

pub use app_data::{AppData, AppDataRef, AppDataRefMut};
pub use either::Either;
pub use registry_key::RegistryKey;
pub(crate) use value_ref::ValueRef;

/// Type of Lua integer numbers.
pub type Integer = ffi::lua_Integer;
/// Type of Lua floating point numbers.
pub type Number = ffi::lua_Number;

/// A "light" userdata value. Equivalent to an unmanaged raw pointer.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct LightUserData(pub *mut c_void);

#[cfg(feature = "send")]
unsafe impl Send for LightUserData {}
#[cfg(feature = "send")]
unsafe impl Sync for LightUserData {}

#[cfg(feature = "send")]
type CallbackFn<'a> = dyn Fn(&RawLua, c_int) -> Result<c_int> + Send + 'a;

#[cfg(not(feature = "send"))]
type CallbackFn<'a> = dyn Fn(&RawLua, c_int) -> Result<c_int> + 'a;

pub(crate) type Callback = Box<CallbackFn<'static>>;
pub(crate) type CallbackPtr = *const CallbackFn<'static>;

pub(crate) type ScopedCallback<'s> = Box<dyn Fn(&RawLua, c_int) -> Result<c_int> + 's>;

pub(crate) struct CallbackUpvalueStorage<T> {
    pub(crate) data: ManuallyDrop<T>,
    // Lua (not Luau) can resurrect and call finalized objects, so we need the pointer to remain
    // readable after its XRc ownership is released
    #[cfg(not(feature = "luau"))]
    extra: *const UnsafeCell<ExtraData>,
    #[cfg(feature = "luau")]
    extra: XRc<UnsafeCell<ExtraData>>,
}

impl<T> CallbackUpvalueStorage<T> {
    fn from_parts(data: T, extra: XRc<UnsafeCell<ExtraData>>) -> Self {
        Self {
            data: ManuallyDrop::new(data),
            #[cfg(not(feature = "luau"))]
            extra: XRc::into_raw(extra),
            #[cfg(feature = "luau")]
            extra,
        }
    }

    #[inline(always)]
    pub(crate) fn extra(&self) -> *mut ExtraData {
        #[cfg(not(feature = "luau"))]
        return unsafe { (*self.extra).get() };
        #[cfg(feature = "luau")]
        return self.extra.get();
    }
}

impl<T> Drop for CallbackUpvalueStorage<T> {
    fn drop(&mut self) {
        unsafe {
            ManuallyDrop::drop(&mut self.data);
            #[cfg(not(feature = "luau"))]
            XRc::decrement_strong_count(self.extra);
        }
    }
}

#[cfg(not(feature = "luau"))]
impl<T> CallbackUpvalueStorage<Option<T>> {
    pub(crate) unsafe fn drop_userdata(this: *mut Self) {
        // Leave a tombstone before dropping captures, which can reenter Lua.
        let data = (&mut *(*this).data).take();
        drop(data);
        XRc::decrement_strong_count((*this).extra);
    }
}

pub(crate) type CallbackUpvalue = CallbackUpvalueStorage<Option<Callback>>;

impl CallbackUpvalueStorage<Option<Callback>> {
    pub(crate) fn new(extra: XRc<UnsafeCell<ExtraData>>) -> Self {
        Self::from_parts(None, extra)
    }
}

#[cfg(all(feature = "async", feature = "send"))]
type AsyncCallbackFn = dyn for<'a> Fn(&'a RawLua, c_int) -> BoxFuture<'a, Result<c_int>> + Send + 'static;

#[cfg(all(feature = "async", not(feature = "send")))]
type AsyncCallbackFn = dyn for<'a> Fn(&'a RawLua, c_int) -> BoxFuture<'a, Result<c_int>> + 'static;

#[cfg(feature = "async")]
pub(crate) type AsyncCallback = Box<AsyncCallbackFn>;

#[cfg(all(feature = "async", not(feature = "luau")))]
#[repr(transparent)]
pub(crate) struct AsyncCallbackPtr(*mut AsyncCallbackFn);

#[cfg(all(feature = "async", not(feature = "luau")))]
impl std::ops::Deref for AsyncCallbackPtr {
    type Target = AsyncCallbackFn;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.0 }
    }
}

#[cfg(all(feature = "async", not(feature = "luau")))]
impl Drop for AsyncCallbackPtr {
    fn drop(&mut self) {
        unsafe { drop(Box::from_raw(self.0)) }
    }
}

#[cfg(all(feature = "async", not(feature = "luau")))]
pub(crate) type AsyncCallbackUpvalue = CallbackUpvalueStorage<AsyncCallbackPtr>;

#[cfg(all(feature = "async", feature = "luau"))]
pub(crate) type AsyncCallbackUpvalue = CallbackUpvalueStorage<AsyncCallback>;

#[cfg(all(feature = "async", not(feature = "luau")))]
impl CallbackUpvalueStorage<AsyncCallbackPtr> {
    pub(crate) fn new(func: AsyncCallback, extra: XRc<UnsafeCell<ExtraData>>) -> Self {
        Self::from_parts(AsyncCallbackPtr(Box::into_raw(func)), extra)
    }

    pub(crate) unsafe fn drop_userdata(this: *mut Self) {
        static DESTRUCTED: for<'a> fn(&'a RawLua, c_int) -> BoxFuture<'a, Result<c_int>> = destructed;
        fn destructed<'a>(_: &'a RawLua, _: c_int) -> BoxFuture<'a, Result<c_int>> {
            Box::pin(async { Err(crate::Error::CallbackDestructed) })
        }

        let destructed = &DESTRUCTED as &AsyncCallbackFn as *const AsyncCallbackFn as *mut AsyncCallbackFn;
        let data = std::mem::replace(&mut *(*this).data, AsyncCallbackPtr(destructed));
        drop(data);
        XRc::decrement_strong_count((*this).extra);
    }
}

#[cfg(all(feature = "async", feature = "luau"))]
impl CallbackUpvalueStorage<AsyncCallback> {
    pub(crate) fn new(func: AsyncCallback, extra: XRc<UnsafeCell<ExtraData>>) -> Self {
        Self::from_parts(func, extra)
    }
}

#[cfg(feature = "async")]
pub(crate) struct AsyncPollUpvalue {
    pub(crate) data: Option<BoxFuture<'static, Result<c_int>>>,
    pub(crate) extra: XRc<UnsafeCell<ExtraData>>,
}

/// Type to set next Lua VM action after executing interrupt or hook function.
#[non_exhaustive]
pub enum VmState {
    Continue,
    /// Yield the current thread.
    ///
    /// Supported by Lua 5.3+ and Luau.
    Yield,
}

#[cfg(not(feature = "luau"))]
pub(crate) enum HookKind {
    Global,
    Thread(HookTriggers, HookCallback),
}

#[cfg(all(feature = "send", not(feature = "luau")))]
pub(crate) type HookCallback = XRc<dyn Fn(&Lua, &Debug) -> Result<VmState> + Send>;

#[cfg(all(not(feature = "send"), not(feature = "luau")))]
pub(crate) type HookCallback = XRc<dyn Fn(&Lua, &Debug) -> Result<VmState>>;

#[cfg(all(feature = "send", feature = "luau"))]
pub(crate) type InterruptCallback = XRc<dyn Fn(&Lua) -> Result<VmState> + Send>;

#[cfg(all(not(feature = "send"), feature = "luau"))]
pub(crate) type InterruptCallback = XRc<dyn Fn(&Lua) -> Result<VmState>>;

#[cfg(feature = "send")]
pub(crate) type ThreadEventCallback = XRc<dyn Fn(&Lua, crate::thread::ThreadEvent) -> Result<()> + Send>;

#[cfg(not(feature = "send"))]
pub(crate) type ThreadEventCallback = XRc<dyn Fn(&Lua, crate::thread::ThreadEvent) -> Result<()>>;

#[cfg(feature = "send")]
#[cfg(any(feature = "lua55", feature = "lua54"))]
pub(crate) type WarnCallback = XRc<dyn Fn(&Lua, &str, bool) -> Result<()> + Send>;

#[cfg(not(feature = "send"))]
#[cfg(any(feature = "lua55", feature = "lua54"))]
pub(crate) type WarnCallback = XRc<dyn Fn(&Lua, &str, bool) -> Result<()>>;

/// A trait that adds `Send` requirement if `send` feature is enabled.
#[cfg(feature = "send")]
pub trait MaybeSend: Send {}
#[cfg(feature = "send")]
impl<T: Send> MaybeSend for T {}

/// A trait that adds `Send` requirement if `send` feature is enabled.
#[cfg(not(feature = "send"))]
pub trait MaybeSend {}
#[cfg(not(feature = "send"))]
impl<T> MaybeSend for T {}

/// Adds a `Sync` requirement to userdata types when the `send` feature is enabled.
///
/// It is automatically implemented for all applicable types.
#[cfg(feature = "send")]
pub trait MaybeSync: Sync {}
#[cfg(feature = "send")]
impl<T: Sync> MaybeSync for T {}

/// Adds a `Sync` requirement to userdata types when the `send` feature is enabled.
///
/// It is automatically implemented for all applicable types.
#[cfg(not(feature = "send"))]
pub trait MaybeSync {}
#[cfg(not(feature = "send"))]
impl<T> MaybeSync for T {}

pub(crate) struct DestructedUserdata;

pub(crate) trait LuaType {
    const TYPE_ID: c_int;
}

impl LuaType for bool {
    const TYPE_ID: c_int = ffi::LUA_TBOOLEAN;
}

impl LuaType for Number {
    const TYPE_ID: c_int = ffi::LUA_TNUMBER;
}

impl LuaType for LightUserData {
    const TYPE_ID: c_int = ffi::LUA_TLIGHTUSERDATA;
}

mod app_data;
mod registry_key;
mod sync;
mod value_ref;

#[cfg(test)]
mod assertions {
    use super::*;

    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_any!(ValueRef: Send);
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(ValueRef: Send, Sync);
}
