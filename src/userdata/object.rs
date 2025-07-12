use std::string::String as StdString;

use crate::error::{Error, Result};
use crate::table::Table;
use crate::traits::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, ObjectLike};
use crate::userdata::AnyUserData;
use crate::value::Value;
use crate::Function;

#[cfg(feature = "async")]
use crate::function::AsyncCallFuture;

impl ObjectLike for AnyUserData {
    #[inline]
    fn get<V: FromLua>(&self, key: impl IntoLua) -> Result<V> {
        // `lua_gettable` method used under the hood can work with any Lua value
        // that has `__index` metamethod
        Table(self.0.copy()).get_protected(key)
    }

    #[inline]
    fn set(&self, key: impl IntoLua, value: impl IntoLua) -> Result<()> {
        // `lua_settable` method used under the hood can work with any Lua value
        // that has `__newindex` metamethod
        Table(self.0.copy()).set_protected(key, value)
    }

    #[inline]
    fn call<R>(&self, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti,
    {
        Function(self.0.copy()).call(args)
    }

    #[cfg(feature = "async")]
    #[inline]
    fn call_async<R>(&self, args: impl IntoLuaMulti) -> AsyncCallFuture<R>
    where
        R: FromLuaMulti,
    {
        Function(self.0.copy()).call_async(args)
    }

    #[inline]
    fn call_method<R>(&self, name: &str, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti,
    {
        self.call_function(name, (self, args))
    }

    #[cfg(feature = "async")]
    fn call_async_method<R>(&self, name: &str, args: impl IntoLuaMulti) -> AsyncCallFuture<R>
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
                let msg = format!("attempt to call a {} value (function '{name}')", val.type_name());
                Err(Error::RuntimeError(msg))
            }
        }
    }

    #[cfg(feature = "async")]
    fn call_async_function<R>(&self, name: &str, args: impl IntoLuaMulti) -> AsyncCallFuture<R>
    where
        R: FromLuaMulti,
    {
        match self.get(name) {
            Ok(Value::Function(func)) => func.call_async(args),
            Ok(val) => {
                let msg = format!("attempt to call a {} value (function '{name}')", val.type_name());
                AsyncCallFuture::error(Error::RuntimeError(msg))
            }
            Err(err) => AsyncCallFuture::error(err),
        }
    }

    #[inline]
    fn to_string(&self) -> Result<StdString> {
        Value::UserData(AnyUserData(self.0.copy())).to_string()
    }
}
