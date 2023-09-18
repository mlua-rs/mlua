use std::prelude::v1::*;

use std::ffi::c_int;
use std::iter::FromIterator;
use std::ops::{Deref, DerefMut};
use std::result::Result as StdResult;

use crate::error::Result;
use crate::lua::Lua;
use crate::util::check_stack;
use crate::value::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, MultiValue, Nil};

/// Result is convertible to `MultiValue` following the common Lua idiom of returning the result
/// on success, or in the case of an error, returning `nil` and an error message.
impl<'lua, T: IntoLua<'lua>, E: IntoLua<'lua>> IntoLuaMulti<'lua> for StdResult<T, E> {
    #[inline]
    fn into_lua_multi(self, lua: &'lua Lua) -> Result<MultiValue<'lua>> {
        let mut result = MultiValue::with_lua_and_capacity(lua, 2);
        match self {
            Ok(v) => result.push_front(v.into_lua(lua)?),
            Err(e) => {
                result.push_front(e.into_lua(lua)?);
                result.push_front(Nil);
            }
        }
        Ok(result)
    }
}

impl<'lua, E: IntoLua<'lua>> IntoLuaMulti<'lua> for StdResult<(), E> {
    #[inline]
    fn into_lua_multi(self, lua: &'lua Lua) -> Result<MultiValue<'lua>> {
        match self {
            Ok(_) => return Ok(MultiValue::new()),
            Err(e) => {
                let mut result = MultiValue::with_lua_and_capacity(lua, 2);
                result.push_front(e.into_lua(lua)?);
                result.push_front(Nil);
                Ok(result)
            }
        }
    }
}

impl<'lua, T: IntoLua<'lua>> IntoLuaMulti<'lua> for T {
    #[inline]
    fn into_lua_multi(self, lua: &'lua Lua) -> Result<MultiValue<'lua>> {
        let mut v = MultiValue::with_lua_and_capacity(lua, 1);
        v.push_front(self.into_lua(lua)?);
        Ok(v)
    }

    #[inline]
    unsafe fn push_into_stack_multi(self, lua: &'lua Lua) -> Result<c_int> {
        self.push_into_stack(lua)?;
        Ok(1)
    }
}

impl<'lua, T: FromLua<'lua>> FromLuaMulti<'lua> for T {
    #[inline]
    fn from_lua_multi(mut values: MultiValue<'lua>, lua: &'lua Lua) -> Result<Self> {
        T::from_lua(values.pop_front().unwrap_or(Nil), lua)
    }

    #[inline]
    fn from_lua_args(
        mut args: MultiValue<'lua>,
        i: usize,
        to: Option<&str>,
        lua: &'lua Lua,
    ) -> Result<Self> {
        T::from_lua_arg(args.pop_front().unwrap_or(Nil), i, to, lua)
    }

    #[inline]
    unsafe fn from_stack_multi(nvals: c_int, lua: &'lua Lua) -> Result<Self> {
        if nvals == 0 {
            return T::from_lua(Nil, lua);
        }
        T::from_stack(-nvals, lua)
    }

    #[inline]
    unsafe fn from_stack_args(
        nargs: c_int,
        i: usize,
        to: Option<&str>,
        lua: &'lua Lua,
    ) -> Result<Self> {
        if nargs == 0 {
            return T::from_lua_arg(Nil, i, to, lua);
        }
        T::from_stack_arg(-nargs, i, to, lua)
    }
}

impl<'lua> IntoLuaMulti<'lua> for MultiValue<'lua> {
    #[inline]
    fn into_lua_multi(self, _: &'lua Lua) -> Result<MultiValue<'lua>> {
        Ok(self)
    }
}

impl<'lua> FromLuaMulti<'lua> for MultiValue<'lua> {
    #[inline]
    fn from_lua_multi(values: MultiValue<'lua>, _: &'lua Lua) -> Result<Self> {
        Ok(values)
    }
}

/// Wraps a variable number of `T`s.
///
/// Can be used to work with variadic functions more easily. Using this type as the last argument of
/// a Rust callback will accept any number of arguments from Lua and convert them to the type `T`
/// using [`FromLua`]. `Variadic<T>` can also be returned from a callback, returning a variable
/// number of values to Lua.
///
/// The [`MultiValue`] type is equivalent to `Variadic<Value>`.
///
/// # Examples
///
/// ```
/// # use mlua::{Lua, Result, Variadic};
/// # fn main() -> Result<()> {
/// # let lua = Lua::new();
/// let add = lua.create_function(|_, vals: Variadic<f64>| -> Result<f64> {
///     Ok(vals.iter().sum())
/// })?;
/// lua.globals().set("add", add)?;
/// assert_eq!(lua.load("add(3, 2, 5)").eval::<f32>()?, 10.0);
/// # Ok(())
/// # }
/// ```
///
/// [`FromLua`]: crate::FromLua
/// [`MultiValue`]: crate::MultiValue
#[derive(Debug, Clone)]
pub struct Variadic<T>(Vec<T>);

impl<T> Variadic<T> {
    /// Creates an empty `Variadic` wrapper containing no values.
    pub const fn new() -> Variadic<T> {
        Variadic(Vec::new())
    }
}

impl<T> Default for Variadic<T> {
    fn default() -> Variadic<T> {
        Variadic::new()
    }
}

impl<T> FromIterator<T> for Variadic<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Variadic(Vec::from_iter(iter))
    }
}

impl<T> IntoIterator for Variadic<T> {
    type Item = T;
    type IntoIter = <Vec<T> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<T> Deref for Variadic<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Variadic<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<'lua, T: IntoLua<'lua>> IntoLuaMulti<'lua> for Variadic<T> {
    #[inline]
    fn into_lua_multi(self, lua: &'lua Lua) -> Result<MultiValue<'lua>> {
        let mut values = MultiValue::with_lua_and_capacity(lua, self.0.len());
        values.refill(self.0.into_iter().map(|e| e.into_lua(lua)))?;
        Ok(values)
    }
}

impl<'lua, T: FromLua<'lua>> FromLuaMulti<'lua> for Variadic<T> {
    #[inline]
    fn from_lua_multi(mut values: MultiValue<'lua>, lua: &'lua Lua) -> Result<Self> {
        values
            .drain_all()
            .map(|e| T::from_lua(e, lua))
            .collect::<Result<Vec<T>>>()
            .map(Variadic)
    }
}

macro_rules! impl_tuple {
    () => (
        impl<'lua> IntoLuaMulti<'lua> for () {
            #[inline]
            fn into_lua_multi(self, lua: &'lua Lua) -> Result<MultiValue<'lua>> {
                Ok(MultiValue::with_lua_and_capacity(lua, 0))
            }

            #[inline]
            unsafe fn push_into_stack_multi(self, _lua: &'lua Lua) -> Result<c_int> {
                Ok(0)
            }
        }

        impl<'lua> FromLuaMulti<'lua> for () {
            #[inline]
            fn from_lua_multi(_values: MultiValue<'lua>, _lua: &'lua Lua) -> Result<Self> {
                Ok(())
            }

            #[inline]
            unsafe fn from_stack_multi(nvals: c_int, lua: &'lua Lua) -> Result<Self> {
                if nvals > 0 {
                    ffi::lua_pop(lua.state(), nvals);
                }
                Ok(())
            }
        }
    );

    ($last:ident $($name:ident)*) => (
        impl<'lua, $($name,)* $last> IntoLuaMulti<'lua> for ($($name,)* $last,)
            where $($name: IntoLua<'lua>,)*
                  $last: IntoLuaMulti<'lua>
        {
            #[allow(unused_mut, non_snake_case)]
            #[inline]
            fn into_lua_multi(self, lua: &'lua Lua) -> Result<MultiValue<'lua>> {
                let ($($name,)* $last,) = self;

                let mut results = $last.into_lua_multi(lua)?;
                push_reverse!(results, $($name.into_lua(lua)?,)*);
                Ok(results)
            }

            #[allow(non_snake_case)]
            #[inline]
            unsafe fn push_into_stack_multi(self, lua: &'lua Lua) -> Result<c_int> {
                let ($($name,)* $last,) = self;
                let mut nresults = 0;
                $(
                    _ = $name;
                    nresults += 1;
                )*
                check_stack(lua.state(), nresults + 1)?;
                $(
                    $name.push_into_stack(lua)?;
                )*
                nresults += $last.push_into_stack_multi(lua)?;
                Ok(nresults)
            }
        }

        impl<'lua, $($name,)* $last> FromLuaMulti<'lua> for ($($name,)* $last,)
            where $($name: FromLua<'lua>,)*
                  $last: FromLuaMulti<'lua>
        {
            #[allow(unused_mut, non_snake_case)]
            #[inline]
            fn from_lua_multi(mut values: MultiValue<'lua>, lua: &'lua Lua) -> Result<Self> {
                $(let $name = FromLua::from_lua(values.pop_front().unwrap_or(Nil), lua)?;)*
                let $last = FromLuaMulti::from_lua_multi(values, lua)?;
                Ok(($($name,)* $last,))
            }

            #[allow(unused_mut, non_snake_case)]
            #[inline]
            fn from_lua_args(mut args: MultiValue<'lua>, mut i: usize, to: Option<&str>, lua: &'lua Lua) -> Result<Self> {
                $(
                    let $name = FromLua::from_lua_arg(args.pop_front().unwrap_or(Nil), i, to, lua)?;
                    i += 1;
                )*
                let $last = FromLuaMulti::from_lua_args(args, i, to, lua)?;
                Ok(($($name,)* $last,))
            }

            #[allow(unused_mut, non_snake_case)]
            #[inline]
            unsafe fn from_stack_multi(mut nvals: c_int, lua: &'lua Lua) -> Result<Self> {
                $(
                    let $name = if nvals > 0 {
                        nvals -= 1;
                        FromLua::from_stack(-(nvals + 1), lua)
                    } else {
                        FromLua::from_lua(Nil, lua)
                    }?;
                )*
                let $last = FromLuaMulti::from_stack_multi(nvals, lua)?;
                Ok(($($name,)* $last,))
            }

            #[allow(unused_mut, non_snake_case)]
            #[inline]
            unsafe fn from_stack_args(mut nargs: c_int, mut i: usize, to: Option<&str>, lua: &'lua Lua) -> Result<Self> {
                $(
                    let $name = if nargs > 0 {
                        nargs -= 1;
                        FromLua::from_stack_arg(-(nargs + 1), i, to, lua)
                    } else {
                        FromLua::from_lua_arg(Nil, i, to, lua)
                    }?;
                    i += 1;
                )*
                let $last = FromLuaMulti::from_stack_args(nargs, i, to, lua)?;
                Ok(($($name,)* $last,))
            }
        }
    );
}

macro_rules! push_reverse {
    ($multi_value:expr, $first:expr, $($rest:expr,)*) => (
        push_reverse!($multi_value, $($rest,)*);
        $multi_value.push_front($first);
    );

    ($multi_value:expr, $first:expr) => (
        $multi_value.push_front($first);
    );

    ($multi_value:expr,) => ();
}

impl_tuple!();
impl_tuple!(A);
impl_tuple!(A B);
impl_tuple!(A B C);
impl_tuple!(A B C D);
impl_tuple!(A B C D E);
impl_tuple!(A B C D E F);
impl_tuple!(A B C D E F G);
impl_tuple!(A B C D E F G H);
impl_tuple!(A B C D E F G H I);
impl_tuple!(A B C D E F G H I J);
impl_tuple!(A B C D E F G H I J K);
impl_tuple!(A B C D E F G H I J K L);
impl_tuple!(A B C D E F G H I J K L M);
impl_tuple!(A B C D E F G H I J K L M N);
impl_tuple!(A B C D E F G H I J K L M N O);
impl_tuple!(A B C D E F G H I J K L M N O P);
