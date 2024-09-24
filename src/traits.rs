use std::string::String as StdString;

use crate::error::Result;
use crate::private::Sealed;
use crate::types::MaybeSend;
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

    /// Gets the function associated to key `name` from the object and calls it,
    /// passing the object itself along with `args` as function arguments.
    fn call_method<R>(&self, name: &str, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti;

    /// Gets the function associated to key `name` from the object and asynchronously calls it,
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
