use std::os::raw::c_int;
use std::string::String as StdString;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::multi::MultiValue;
use crate::private::Sealed;
use crate::state::{Lua, RawLua};
use crate::types::MaybeSend;
use crate::util::{check_stack, short_type_name};
use crate::value::Value;

#[cfg(feature = "async")]
use {crate::function::AsyncCallFuture, std::future::Future};

/// Trait for types convertible to [`Value`].
pub trait IntoLua: Sized {
    /// Performs the conversion.
    fn into_lua(self, lua: &Lua) -> Result<Value>;

    /// Pushes the value into the Lua stack.
    ///
    /// # Safety
    /// This method does not check Lua stack space.
    #[doc(hidden)]
    #[inline]
    unsafe fn push_into_stack(self, lua: &RawLua) -> Result<()> {
        lua.push_value(&self.into_lua(lua.lua())?)
    }
}

/// Trait for types convertible from [`Value`].
pub trait FromLua: Sized {
    /// Performs the conversion.
    fn from_lua(value: Value, lua: &Lua) -> Result<Self>;

    /// Performs the conversion for an argument (eg. function argument).
    ///
    /// `i` is the argument index (position),
    /// `to` is a function name that received the argument.
    #[doc(hidden)]
    #[inline]
    fn from_lua_arg(arg: Value, i: usize, to: Option<&str>, lua: &Lua) -> Result<Self> {
        Self::from_lua(arg, lua).map_err(|err| Error::BadArgument {
            to: to.map(|s| s.to_string()),
            pos: i,
            name: None,
            cause: Arc::new(err),
        })
    }

    /// Performs the conversion for a value in the Lua stack at index `idx`.
    #[doc(hidden)]
    #[inline]
    unsafe fn from_stack(idx: c_int, lua: &RawLua) -> Result<Self> {
        Self::from_lua(lua.stack_value(idx, None), lua.lua())
    }

    /// Same as `from_lua_arg` but for a value in the Lua stack at index `idx`.
    #[doc(hidden)]
    #[inline]
    unsafe fn from_stack_arg(idx: c_int, i: usize, to: Option<&str>, lua: &RawLua) -> Result<Self> {
        Self::from_stack(idx, lua).map_err(|err| Error::BadArgument {
            to: to.map(|s| s.to_string()),
            pos: i,
            name: None,
            cause: Arc::new(err),
        })
    }
}

/// Trait for types convertible to any number of Lua values.
///
/// This is a generalization of [`IntoLua`], allowing any number of resulting Lua values instead of
/// just one. Any type that implements [`IntoLua`] will automatically implement this trait.
pub trait IntoLuaMulti: Sized {
    /// Performs the conversion.
    fn into_lua_multi(self, lua: &Lua) -> Result<MultiValue>;

    /// Pushes the values into the Lua stack.
    ///
    /// Returns number of pushed values.
    #[doc(hidden)]
    #[inline]
    unsafe fn push_into_stack_multi(self, lua: &RawLua) -> Result<c_int> {
        let values = self.into_lua_multi(lua.lua())?;
        let len: c_int = values.len().try_into().unwrap();
        unsafe {
            check_stack(lua.state(), len + 1)?;
            for val in &values {
                lua.push_value(val)?;
            }
        }
        Ok(len)
    }
}

/// Trait for types that can be created from an arbitrary number of Lua values.
///
/// This is a generalization of [`FromLua`], allowing an arbitrary number of Lua values to
/// participate in the conversion. Any type that implements [`FromLua`] will automatically
/// implement this trait.
pub trait FromLuaMulti: Sized {
    /// Performs the conversion.
    ///
    /// In case `values` contains more values than needed to perform the conversion, the excess
    /// values should be ignored. This reflects the semantics of Lua when calling a function or
    /// assigning values. Similarly, if not enough values are given, conversions should assume that
    /// any missing values are nil.
    fn from_lua_multi(values: MultiValue, lua: &Lua) -> Result<Self>;

    /// Performs the conversion for a list of arguments.
    ///
    /// `i` is an index (position) of the first argument,
    /// `to` is a function name that received the arguments.
    #[doc(hidden)]
    #[inline]
    fn from_lua_args(args: MultiValue, i: usize, to: Option<&str>, lua: &Lua) -> Result<Self> {
        let _ = (i, to);
        Self::from_lua_multi(args, lua)
    }

    /// Performs the conversion for a number of values in the Lua stack.
    #[doc(hidden)]
    #[inline]
    unsafe fn from_stack_multi(nvals: c_int, lua: &RawLua) -> Result<Self> {
        let mut values = MultiValue::with_capacity(nvals as usize);
        for idx in 0..nvals {
            values.push_back(lua.stack_value(-nvals + idx, None));
        }
        Self::from_lua_multi(values, lua.lua())
    }

    /// Same as `from_lua_args` but for a number of values in the Lua stack.
    #[doc(hidden)]
    #[inline]
    unsafe fn from_stack_args(nargs: c_int, i: usize, to: Option<&str>, lua: &RawLua) -> Result<Self> {
        let _ = (i, to);
        Self::from_stack_multi(nargs, lua)
    }
}

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
    fn call_async<R>(&self, args: impl IntoLuaMulti) -> AsyncCallFuture<R>
    where
        R: FromLuaMulti;

    /// Gets the function associated to key `name` from the object and calls it,
    /// passing the object itself along with `args` as function arguments.
    fn call_method<R>(&self, name: &str, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti;

    /// Gets the function associated to key `name` from the object and asynchronously calls it,
    /// passing the object itself along with `args` as function arguments.
    ///
    /// This might invoke the `__index` metamethod.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn call_async_method<R>(&self, name: &str, args: impl IntoLuaMulti) -> AsyncCallFuture<R>
    where
        R: FromLuaMulti;

    /// Gets the function associated to key `name` from the object and calls it,
    /// passing `args` as function arguments.
    ///
    /// This might invoke the `__index` metamethod.
    fn call_function<R>(&self, name: &str, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti;

    /// Gets the function associated to key `name` from the object and asynchronously calls it,
    /// passing `args` as function arguments.
    ///
    /// This might invoke the `__index` metamethod.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn call_async_function<R>(&self, name: &str, args: impl IntoLuaMulti) -> AsyncCallFuture<R>
    where
        R: FromLuaMulti;

    /// Converts the object to a string in a human-readable format.
    ///
    /// This might invoke the `__tostring` metamethod.
    fn to_string(&self) -> Result<StdString>;
}

/// A trait for types that can be used as Lua functions.
pub trait LuaNativeFn<A: FromLuaMulti> {
    type Output: IntoLuaMulti;

    fn call(&self, args: A) -> Self::Output;
}

/// A trait for types with mutable state that can be used as Lua functions.
pub trait LuaNativeFnMut<A: FromLuaMulti> {
    type Output: IntoLuaMulti;

    fn call(&mut self, args: A) -> Self::Output;
}

/// A trait for types that returns a future and can be used as Lua functions.
#[cfg(feature = "async")]
pub trait LuaNativeAsyncFn<A: FromLuaMulti> {
    type Output: IntoLuaMulti;

    fn call(&self, args: A) -> impl Future<Output = Self::Output> + MaybeSend + 'static;
}

macro_rules! impl_lua_native_fn {
    ($($A:ident),*) => {
        impl<FN, $($A,)* R> LuaNativeFn<($($A,)*)> for FN
        where
            FN: Fn($($A,)*) -> R + MaybeSend + 'static,
            ($($A,)*): FromLuaMulti,
            R: IntoLuaMulti,
        {
            type Output = R;

            #[allow(non_snake_case)]
            fn call(&self, args: ($($A,)*)) -> Self::Output {
                let ($($A,)*) = args;
                self($($A,)*)
            }
        }

        impl<FN, $($A,)* R> LuaNativeFnMut<($($A,)*)> for FN
        where
            FN: FnMut($($A,)*) -> R + MaybeSend + 'static,
            ($($A,)*): FromLuaMulti,
            R: IntoLuaMulti,
        {
            type Output = R;

            #[allow(non_snake_case)]
            fn call(&mut self, args: ($($A,)*)) -> Self::Output {
                let ($($A,)*) = args;
                self($($A,)*)
            }
        }

        #[cfg(feature = "async")]
        impl<FN, $($A,)* Fut, R> LuaNativeAsyncFn<($($A,)*)> for FN
        where
            FN: Fn($($A,)*) -> Fut + MaybeSend + 'static,
            ($($A,)*): FromLuaMulti,
            Fut: Future<Output = R> + MaybeSend + 'static,
            R: IntoLuaMulti,
        {
            type Output = R;

            #[allow(non_snake_case)]
            fn call(&self, args: ($($A,)*)) -> impl Future<Output = Self::Output> + MaybeSend + 'static {
                let ($($A,)*) = args;
                self($($A,)*)
            }
        }
    };
}

impl_lua_native_fn!();
impl_lua_native_fn!(A);
impl_lua_native_fn!(A, B);
impl_lua_native_fn!(A, B, C);
impl_lua_native_fn!(A, B, C, D);
impl_lua_native_fn!(A, B, C, D, E);
impl_lua_native_fn!(A, B, C, D, E, F);
impl_lua_native_fn!(A, B, C, D, E, F, G);
impl_lua_native_fn!(A, B, C, D, E, F, G, H);
impl_lua_native_fn!(A, B, C, D, E, F, G, H, I);
impl_lua_native_fn!(A, B, C, D, E, F, G, H, I, J);
impl_lua_native_fn!(A, B, C, D, E, F, G, H, I, J, K);
impl_lua_native_fn!(A, B, C, D, E, F, G, H, I, J, K, L);
impl_lua_native_fn!(A, B, C, D, E, F, G, H, I, J, K, L, M);
impl_lua_native_fn!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);
impl_lua_native_fn!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);
impl_lua_native_fn!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P);

pub(crate) trait ShortTypeName {
    #[inline(always)]
    fn type_name() -> StdString {
        short_type_name::<Self>()
    }
}

impl<T> ShortTypeName for T {}
