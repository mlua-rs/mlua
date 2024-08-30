use std::string::String as StdString;

use crate::error::Result;
use crate::private::Sealed;
use crate::value::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti};

#[cfg(feature = "async")]
use std::future::Future;

/// A trait for types that can be used as Lua objects (usually table and userdata).
pub trait ObjectLike: Sealed {
    /// Gets the value associated to `key` from the object, assuming it has `__index` metamethod.
    fn get<V: FromLua>(&self, key: impl IntoLua) -> Result<V>;

    /// Sets the value associated to `key` in the object, assuming it has `__newindex` metamethod.
    fn set(&self, key: impl IntoLua, value: impl IntoLua) -> Result<()>;

    /// Calls the object as a function assuming it has `__call` metamethod.
    ///
    /// The metamethod is called with the object as its first argument, followed by the passed
    /// arguments.
    fn call<R>(&self, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti;

    /// Asynchronously calls the object as a function assuming it has `__call` metamethod.
    ///
    /// The metamethod is called with the object as its first argument, followed by the passed
    /// arguments.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn call_async<R>(&self, args: impl IntoLuaMulti) -> impl Future<Output = Result<R>>
    where
        R: FromLuaMulti;

    /// Gets the function associated to `key` from the object and calls it,
    /// passing the object itself along with `args` as function arguments.
    fn call_method<R>(&self, name: &str, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti;

    /// Gets the function associated to `key` from the object and asynchronously calls it,
    /// passing the object itself along with `args` as function arguments.
    ///
    /// Requires `feature = "async"`
    ///
    /// This might invoke the `__index` metamethod.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn call_async_method<R>(&self, name: &str, args: impl IntoLuaMulti) -> impl Future<Output = Result<R>>
    where
        R: FromLuaMulti;

    /// Gets the function associated to `key` from the object and calls it,
    /// passing `args` as function arguments.
    ///
    /// This might invoke the `__index` metamethod.
    fn call_function<R>(&self, name: &str, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti;

    /// Gets the function associated to `key` from the object and asynchronously calls it,
    /// passing `args` as function arguments.
    ///
    /// Requires `feature = "async"`
    ///
    /// This might invoke the `__index` metamethod.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn call_async_function<R>(&self, name: &str, args: impl IntoLuaMulti) -> impl Future<Output = Result<R>>
    where
        R: FromLuaMulti;

    /// Converts the object to a string in a human-readable format.
    ///
    /// This might invoke the `__tostring` metamethod.
    fn to_string(&self) -> Result<StdString>;
}
