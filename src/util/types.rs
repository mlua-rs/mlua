use std::any::Any;
use std::os::raw::c_void;

use crate::types::{Callback, CallbackUpvalue};

#[cfg(feature = "async")]
use crate::types::{AsyncCallback, AsyncCallbackUpvalue, AsyncPollUpvalue};

pub(crate) trait TypeKey: Any {
    fn type_key() -> *const c_void;
}

static STRING_TYPE_KEY: u8 = 0;

impl TypeKey for String {
    #[inline(always)]
    fn type_key() -> *const c_void {
        &STRING_TYPE_KEY as *const u8 as *const c_void
    }
}

static CALLBACK_TYPE_KEY: u8 = 0;

impl TypeKey for Callback {
    #[inline(always)]
    fn type_key() -> *const c_void {
        &CALLBACK_TYPE_KEY as *const u8 as *const c_void
    }
}

static CALLBACK_UPVALUE_TYPE_KEY: u8 = 0;

impl TypeKey for CallbackUpvalue {
    #[inline(always)]
    fn type_key() -> *const c_void {
        &CALLBACK_UPVALUE_TYPE_KEY as *const u8 as *const c_void
    }
}

#[cfg(feature = "async")]
static ASYNC_CALLBACK_TYPE_KEY: u8 = 0;

#[cfg(feature = "async")]
impl TypeKey for AsyncCallback {
    #[inline(always)]
    fn type_key() -> *const c_void {
        &ASYNC_CALLBACK_TYPE_KEY as *const u8 as *const c_void
    }
}

#[cfg(feature = "async")]
static ASYNC_CALLBACK_UPVALUE_TYPE_KEY: u8 = 0;

#[cfg(feature = "async")]
impl TypeKey for AsyncCallbackUpvalue {
    #[inline(always)]
    fn type_key() -> *const c_void {
        &ASYNC_CALLBACK_UPVALUE_TYPE_KEY as *const u8 as *const c_void
    }
}

#[cfg(feature = "async")]
static ASYNC_POLL_UPVALUE_TYPE_KEY: u8 = 0;

#[cfg(feature = "async")]
impl TypeKey for AsyncPollUpvalue {
    #[inline(always)]
    fn type_key() -> *const c_void {
        &ASYNC_POLL_UPVALUE_TYPE_KEY as *const u8 as *const c_void
    }
}

#[cfg(feature = "async")]
static WAKER_TYPE_KEY: u8 = 0;

#[cfg(feature = "async")]
impl TypeKey for Option<std::task::Waker> {
    #[inline(always)]
    fn type_key() -> *const c_void {
        &WAKER_TYPE_KEY as *const u8 as *const c_void
    }
}
