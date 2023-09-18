use std::prelude::v1::*;

use crate::error::{Error, Result};
use crate::private::Sealed;
use crate::userdata::{AnyUserData, MetaMethod};
use crate::value::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, Value};

#[cfg(feature = "async")]
use futures_util::future::{self, LocalBoxFuture};

/// An extension trait for [`AnyUserData`] that provides a variety of convenient functionality.
pub trait AnyUserDataExt<'lua>: Sealed {
    /// Gets the value associated to `key` from the userdata, assuming it has `__index` metamethod.
    fn get<K: IntoLua<'lua>, V: FromLua<'lua>>(&self, key: K) -> Result<V>;

    /// Sets the value associated to `key` in the userdata, assuming it has `__newindex` metamethod.
    fn set<K: IntoLua<'lua>, V: IntoLua<'lua>>(&self, key: K, value: V) -> Result<()>;

    /// Calls the userdata as a function assuming it has `__call` metamethod.
    ///
    /// The metamethod is called with the userdata as its first argument, followed by the passed arguments.
    fn call<A, R>(&self, args: A) -> Result<R>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua>;

    /// Asynchronously calls the userdata as a function assuming it has `__call` metamethod.
    ///
    /// The metamethod is called with the userdata as its first argument, followed by the passed arguments.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn call_async<A, R>(&self, args: A) -> LocalBoxFuture<'lua, Result<R>>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'lua;

    /// Calls the userdata method, assuming it has `__index` metamethod
    /// and a function associated to `name`.
    fn call_method<A, R>(&self, name: &str, args: A) -> Result<R>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua>;

    /// Gets the function associated to `key` from the table and asynchronously executes it,
    /// passing the table itself along with `args` as function arguments and returning Future.
    ///
    /// Requires `feature = "async"`
    ///
    /// This might invoke the `__index` metamethod.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn call_async_method<A, R>(&self, name: &str, args: A) -> LocalBoxFuture<'lua, Result<R>>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'lua;

    /// Gets the function associated to `key` from the table and executes it,
    /// passing `args` as function arguments.
    ///
    /// This is a shortcut for
    /// `table.get::<_, Function>(key)?.call(args)`
    ///
    /// This might invoke the `__index` metamethod.
    fn call_function<A, R>(&self, name: &str, args: A) -> Result<R>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua>;

    /// Gets the function associated to `key` from the table and asynchronously executes it,
    /// passing `args` as function arguments and returning Future.
    ///
    /// Requires `feature = "async"`
    ///
    /// This might invoke the `__index` metamethod.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn call_async_function<A, R>(&self, name: &str, args: A) -> LocalBoxFuture<'lua, Result<R>>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'lua;
}

impl<'lua> AnyUserDataExt<'lua> for AnyUserData<'lua> {
    fn get<K: IntoLua<'lua>, V: FromLua<'lua>>(&self, key: K) -> Result<V> {
        let metatable = self.get_metatable()?;
        match metatable.get::<Value>(MetaMethod::Index)? {
            Value::Table(table) => table.raw_get(key),
            Value::Function(func) => func.call((self.clone(), key)),
            _ => Err(Error::runtime("attempt to index a userdata value")),
        }
    }

    fn set<K: IntoLua<'lua>, V: IntoLua<'lua>>(&self, key: K, value: V) -> Result<()> {
        let metatable = self.get_metatable()?;
        match metatable.get::<Value>(MetaMethod::NewIndex)? {
            Value::Table(table) => table.raw_set(key, value),
            Value::Function(func) => func.call((self.clone(), key, value)),
            _ => Err(Error::runtime("attempt to index a userdata value")),
        }
    }

    fn call<A, R>(&self, args: A) -> Result<R>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua>,
    {
        let metatable = self.get_metatable()?;
        match metatable.get::<Value>(MetaMethod::Call)? {
            Value::Function(func) => func.call((self.clone(), args)),
            _ => Err(Error::runtime("attempt to call a userdata value")),
        }
    }

    #[cfg(feature = "async")]
    fn call_async<A, R>(&self, args: A) -> LocalBoxFuture<'lua, Result<R>>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'lua,
    {
        let metatable = match self.get_metatable() {
            Ok(metatable) => metatable,
            Err(err) => return Box::pin(future::err(err)),
        };
        match metatable.get::<Value>(MetaMethod::Call) {
            Ok(Value::Function(func)) => {
                let mut args = match args.into_lua_multi(self.0.lua) {
                    Ok(args) => args,
                    Err(e) => return Box::pin(future::err(e)),
                };
                args.push_front(Value::UserData(self.clone()));
                Box::pin(async move { func.call_async(args).await })
            }
            Ok(_) => Box::pin(future::err(Error::runtime(
                "attempt to call a userdata value",
            ))),
            Err(err) => Box::pin(future::err(err)),
        }
    }

    fn call_method<A, R>(&self, name: &str, args: A) -> Result<R>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua>,
    {
        self.call_function(name, (self.clone(), args))
    }

    #[cfg(feature = "async")]
    fn call_async_method<A, R>(&self, name: &str, args: A) -> LocalBoxFuture<'lua, Result<R>>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'lua,
    {
        self.call_async_function(name, (self.clone(), args))
    }

    fn call_function<A, R>(&self, name: &str, args: A) -> Result<R>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua>,
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
    fn call_async_function<A, R>(&self, name: &str, args: A) -> LocalBoxFuture<'lua, Result<R>>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'lua,
    {
        match self.get(name) {
            Ok(Value::Function(func)) => {
                let args = match args.into_lua_multi(self.0.lua) {
                    Ok(args) => args,
                    Err(e) => return Box::pin(future::err(e)),
                };
                Box::pin(async move { func.call_async(args).await })
            }
            Ok(val) => {
                let msg = format!("attempt to call a {} value", val.type_name());
                Box::pin(future::err(Error::runtime(msg)))
            }
            Err(err) => Box::pin(future::err(err)),
        }
    }
}
