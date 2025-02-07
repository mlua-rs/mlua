use std::any::Any;
use std::os::raw::c_void;

use crate::types::{Callback, CallbackUpvalue};

#[cfg(feature = "async")]
use crate::types::{AsyncCallback, AsyncCallbackUpvalue, AsyncPollUpvalue};

pub(crate) trait TypeKey: Any {
    fn type_key() -> *const c_void;
}

impl TypeKey for String {
    #[inline(always)]
    fn type_key() -> *const c_void {
        static STRING_TYPE_KEY: u8 = 0;
        &STRING_TYPE_KEY as *const u8 as *const c_void
    }
}

impl TypeKey for Callback {
    #[inline(always)]
    fn type_key() -> *const c_void {
        static CALLBACK_TYPE_KEY: u8 = 0;
        &CALLBACK_TYPE_KEY as *const u8 as *const c_void
    }
}

impl TypeKey for CallbackUpvalue {
    #[inline(always)]
    fn type_key() -> *const c_void {
        static CALLBACK_UPVALUE_TYPE_KEY: u8 = 0;
        &CALLBACK_UPVALUE_TYPE_KEY as *const u8 as *const c_void
    }
}

#[cfg(not(feature = "luau"))]
impl TypeKey for crate::types::HookCallback {
    #[inline(always)]
    fn type_key() -> *const c_void {
        static HOOK_CALLBACK_TYPE_KEY: u8 = 0;
        &HOOK_CALLBACK_TYPE_KEY as *const u8 as *const c_void
    }
}

#[cfg(feature = "async")]
impl TypeKey for AsyncCallback {
    #[inline(always)]
    fn type_key() -> *const c_void {
        static ASYNC_CALLBACK_TYPE_KEY: u8 = 0;
        &ASYNC_CALLBACK_TYPE_KEY as *const u8 as *const c_void
    }
}

#[cfg(feature = "async")]
impl TypeKey for AsyncCallbackUpvalue {
    #[inline(always)]
    fn type_key() -> *const c_void {
        static ASYNC_CALLBACK_UPVALUE_TYPE_KEY: u8 = 0;
        &ASYNC_CALLBACK_UPVALUE_TYPE_KEY as *const u8 as *const c_void
    }
}

#[cfg(feature = "async")]
impl TypeKey for AsyncPollUpvalue {
    #[inline(always)]
    fn type_key() -> *const c_void {
        static ASYNC_POLL_UPVALUE_TYPE_KEY: u8 = 0;
        &ASYNC_POLL_UPVALUE_TYPE_KEY as *const u8 as *const c_void
    }
}

#[cfg(feature = "async")]
impl TypeKey for Option<std::task::Waker> {
    #[inline(always)]
    fn type_key() -> *const c_void {
        static WAKER_TYPE_KEY: u8 = 0;
        &WAKER_TYPE_KEY as *const u8 as *const c_void
    }
}
