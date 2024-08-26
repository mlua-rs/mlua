use std::cell::UnsafeCell;
use std::fmt;
use std::os::raw::{c_int, c_void};
use std::rc::Rc;

use crate::error::Result;
#[cfg(not(feature = "luau"))]
use crate::hook::Debug;
use crate::state::{ExtraData, Lua, RawLua, WeakLua};

// Re-export mutex wrappers
pub(crate) use sync::{ArcReentrantMutexGuard, ReentrantMutex, ReentrantMutexGuard, XRc, XWeak};

#[cfg(all(feature = "async", feature = "send"))]
pub(crate) type BoxFuture<'a, T> = futures_util::future::BoxFuture<'a, T>;

#[cfg(all(feature = "async", not(feature = "send")))]
pub(crate) type BoxFuture<'a, T> = futures_util::future::LocalBoxFuture<'a, T>;

pub use app_data::{AppData, AppDataRef, AppDataRefMut};
pub use registry_key::RegistryKey;
#[cfg(any(feature = "luau", doc))]
pub use vector::Vector;

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
pub struct LightUserData(pub *mut c_void);

#[cfg(feature = "send")]
unsafe impl Send for LightUserData {}
#[cfg(feature = "send")]
unsafe impl Sync for LightUserData {}

#[cfg(feature = "send")]
pub(crate) type Callback = Box<dyn Fn(&RawLua, c_int) -> Result<c_int> + Send + 'static>;

#[cfg(not(feature = "send"))]
pub(crate) type Callback = Box<dyn Fn(&RawLua, c_int) -> Result<c_int> + 'static>;

pub(crate) struct Upvalue<T> {
    pub(crate) data: T,
    pub(crate) extra: XRc<UnsafeCell<ExtraData>>,
}

pub(crate) type CallbackUpvalue = Upvalue<Callback>;

#[cfg(all(feature = "async", feature = "send"))]
pub(crate) type AsyncCallback =
    Box<dyn for<'a> Fn(&'a RawLua, c_int) -> BoxFuture<'a, Result<c_int>> + Send + 'static>;

#[cfg(all(feature = "async", not(feature = "send")))]
pub(crate) type AsyncCallback =
    Box<dyn for<'a> Fn(&'a RawLua, c_int) -> BoxFuture<'a, Result<c_int>> + 'static>;

#[cfg(feature = "async")]
pub(crate) type AsyncCallbackUpvalue = Upvalue<AsyncCallback>;

#[cfg(feature = "async")]
pub(crate) type AsyncPollUpvalue = Upvalue<BoxFuture<'static, Result<c_int>>>;

/// Type to set next Luau VM action after executing interrupt function.
#[cfg(any(feature = "luau", doc))]
#[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
pub enum VmState {
    Continue,
    Yield,
}

#[cfg(all(feature = "send", not(feature = "luau")))]
pub(crate) type HookCallback = Rc<dyn Fn(&Lua, Debug) -> Result<()> + Send>;

#[cfg(all(not(feature = "send"), not(feature = "luau")))]
pub(crate) type HookCallback = Rc<dyn Fn(&Lua, Debug) -> Result<()>>;

#[cfg(all(feature = "send", feature = "luau"))]
pub(crate) type InterruptCallback = Rc<dyn Fn(&Lua) -> Result<VmState> + Send>;

#[cfg(all(not(feature = "send"), feature = "luau"))]
pub(crate) type InterruptCallback = Rc<dyn Fn(&Lua) -> Result<VmState>>;

#[cfg(all(feature = "send", feature = "lua54"))]
pub(crate) type WarnCallback = Box<dyn Fn(&Lua, &str, bool) -> Result<()> + Send>;

#[cfg(all(not(feature = "send"), feature = "lua54"))]
pub(crate) type WarnCallback = Box<dyn Fn(&Lua, &str, bool) -> Result<()>>;

/// A trait that adds `Send` requirement if `send` feature is enabled.
#[cfg(feature = "send")]
pub trait MaybeSend: Send {}
#[cfg(feature = "send")]
impl<T: Send> MaybeSend for T {}

#[cfg(not(feature = "send"))]
pub trait MaybeSend {}
#[cfg(not(feature = "send"))]
impl<T> MaybeSend for T {}

pub(crate) struct DestructedUserdata;

pub(crate) struct ValueRef {
    pub(crate) lua: WeakLua,
    pub(crate) index: c_int,
    pub(crate) drop: bool,
}

impl ValueRef {
    #[inline]
    pub(crate) fn new(lua: &RawLua, index: c_int) -> Self {
        ValueRef {
            lua: lua.weak().clone(),
            index,
            drop: true,
        }
    }

    #[inline]
    pub(crate) fn to_pointer(&self) -> *const c_void {
        let lua = self.lua.lock();
        unsafe { ffi::lua_topointer(lua.ref_thread(), self.index) }
    }
}

impl fmt::Debug for ValueRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Ref({:p})", self.to_pointer())
    }
}

impl Clone for ValueRef {
    fn clone(&self) -> Self {
        unsafe { self.lua.lock().clone_ref(self) }
    }
}

impl Drop for ValueRef {
    fn drop(&mut self) {
        if self.drop {
            if let Some(lua) = self.lua.try_lock() {
                unsafe { lua.drop_ref(self) };
            }
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

mod app_data;
mod registry_key;
mod sync;

#[cfg(any(feature = "luau", doc))]
mod vector;

#[cfg(test)]
mod assertions {
    use super::*;

    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_any!(ValueRef: Send);
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(ValueRef: Send, Sync);
}
