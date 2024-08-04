use crate::error::{Error, Result};
use crate::private::Sealed;
use crate::userdata::{AnyUserData, MetaMethod};
use crate::value::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, Value};

#[cfg(feature = "async")]
use std::future::Future;

/// An extension trait for [`AnyUserData`] that provides a variety of convenient functionality.
pub trait AnyUserDataExt: Sealed {
    /// Gets the value associated to `key` from the userdata, assuming it has `__index` metamethod.
    fn get<V: FromLua>(&self, key: impl IntoLua) -> Result<V>;

    /// Sets the value associated to `key` in the userdata, assuming it has `__newindex` metamethod.
    fn set(&self, key: impl IntoLua, value: impl IntoLua) -> Result<()>;

    /// Calls the userdata as a function assuming it has `__call` metamethod.
    ///
    /// The metamethod is called with the userdata as its first argument, followed by the passed
    /// arguments.
    fn call<R>(&self, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti;

    /// Asynchronously calls the userdata as a function assuming it has `__call` metamethod.
    ///
    /// The metamethod is called with the userdata as its first argument, followed by the passed
    /// arguments.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn call_async<R>(&self, args: impl IntoLuaMulti) -> impl Future<Output = Result<R>>
    where
        R: FromLuaMulti;

    /// Calls the userdata method, assuming it has `__index` metamethod
    /// and a function associated to `name`.
    fn call_method<R>(&self, name: &str, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti;

    /// Gets the function associated to `key` from the table and asynchronously executes it,
    /// passing the table itself along with `args` as function arguments and returning Future.
    ///
    /// Requires `feature = "async"`
    ///
    /// This might invoke the `__index` metamethod.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn call_async_method<R>(&self, name: &str, args: impl IntoLuaMulti) -> impl Future<Output = Result<R>>
    where
        R: FromLuaMulti;

    /// Gets the function associated to `key` from the table and executes it,
    /// passing `args` as function arguments.
    ///
    /// This is a shortcut for
    /// `table.get::<Function>(key)?.call(args)`
    ///
    /// This might invoke the `__index` metamethod.
    fn call_function<R>(&self, name: &str, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti;

    /// Gets the function associated to `key` from the table and asynchronously executes it,
    /// passing `args` as function arguments and returning Future.
    ///
    /// Requires `feature = "async"`
    ///
    /// This might invoke the `__index` metamethod.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn call_async_function<R>(&self, name: &str, args: impl IntoLuaMulti) -> impl Future<Output = Result<R>>
    where
        R: FromLuaMulti;
}

impl AnyUserDataExt for AnyUserData {
    fn get<V: FromLua>(&self, key: impl IntoLua) -> Result<V> {
        let metatable = self.get_metatable()?;
        match metatable.get::<Value>(MetaMethod::Index)? {
            Value::Table(table) => table.raw_get(key),
            Value::Function(func) => func.call((self, key)),
            _ => Err(Error::runtime("attempt to index a userdata value")),
        }
    }

    fn set(&self, key: impl IntoLua, value: impl IntoLua) -> Result<()> {
        let metatable = self.get_metatable()?;
        match metatable.get::<Value>(MetaMethod::NewIndex)? {
            Value::Table(table) => table.raw_set(key, value),
            Value::Function(func) => func.call((self, key, value)),
            _ => Err(Error::runtime("attempt to index a userdata value")),
        }
    }

    fn call<R>(&self, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti,
    {
        let metatable = self.get_metatable()?;
        match metatable.get::<Value>(MetaMethod::Call)? {
            Value::Function(func) => func.call((self, args)),
            _ => Err(Error::runtime("attempt to call a userdata value")),
        }
    }

    #[cfg(feature = "async")]
    fn call_async<R>(&self, args: impl IntoLuaMulti) -> impl Future<Output = Result<R>>
    where
        R: FromLuaMulti,
    {
        let lua = self.0.lua.lock();
        let args = (self, args).into_lua_multi(lua.lua());
        async move {
            let metatable = self.get_metatable()?;
            match metatable.get::<Value>(MetaMethod::Call)? {
                Value::Function(func) => func.call_async(args?).await,
                _ => Err(Error::runtime("attempt to call a userdata value")),
            }
        }
    }

    fn call_method<R>(&self, name: &str, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti,
    {
        self.call_function(name, (self, args))
    }

    #[cfg(feature = "async")]
    fn call_async_method<R>(&self, name: &str, args: impl IntoLuaMulti) -> impl Future<Output = Result<R>>
    where
        R: FromLuaMulti,
    {
        self.call_async_function(name, (self, args))
    }

    fn call_function<R>(&self, name: &str, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti,
    {
        match self.get(name)? {
            Value::Function(func) => func.call(args),
            val => {
                let msg = format!("attempt to call a {} value", val.type_name());
                Err(Error::runtime(msg))
            }
        }
    }

    #[cfg(feature = "async")]
    fn call_async_function<R>(&self, name: &str, args: impl IntoLuaMulti) -> impl Future<Output = Result<R>>
    where
        R: FromLuaMulti,
    {
        let lua = self.0.lua.lock();
        let args = args.into_lua_multi(lua.lua());
        async move {
            match self.get::<Value>(name)? {
                Value::Function(func) => func.call_async(args?).await,
                val => {
                    let msg = format!("attempt to call a {} value", val.type_name());
                    Err(Error::runtime(msg))
                }
            }
        }
    }
}
