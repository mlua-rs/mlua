use std::iter::FromIterator;
use std::ops::{Deref, DerefMut};
use std::result::Result as StdResult;

use crate::error::Result;
use crate::lua::Lua;
use crate::value::{FromLua, FromLuaMulti, MultiValue, Nil, ToLua, ToLuaMulti};

/// Result is convertible to `MultiValue` following the common Lua idiom of returning the result
/// on success, or in the case of an error, returning `nil` and an error message.
impl<'lua, T: ToLua<'lua>, E: ToLua<'lua>> ToLuaMulti<'lua> for StdResult<T, E> {
    fn to_lua_multi(self, lua: &'lua Lua) -> Result<MultiValue<'lua>> {
        let mut result = MultiValue::new();

        match self {
            Ok(v) => result.push_front(v.to_lua(lua)?),
            Err(e) => {
                result.push_front(e.to_lua(lua)?);
                result.push_front(Nil);
            }
        }

        Ok(result)
    }
}

impl<'lua, T: ToLua<'lua>> ToLuaMulti<'lua> for T {
    fn to_lua_multi(self, lua: &'lua Lua) -> Result<MultiValue<'lua>> {
        let mut v = MultiValue::new();
        v.push_front(self.to_lua(lua)?);
        Ok(v)
    }
}

impl<'lua, T: FromLua<'lua>> FromLuaMulti<'lua> for T {
    fn from_lua_multi(mut values: MultiValue<'lua>, lua: &'lua Lua) -> Result<Self> {
        T::from_lua(values.pop_front().unwrap_or(Nil), lua)
    }
}

impl<'lua> ToLuaMulti<'lua> for MultiValue<'lua> {
    fn to_lua_multi(self, _: &'lua Lua) -> Result<MultiValue<'lua>> {
        Ok(self)
    }
}

impl<'lua> FromLuaMulti<'lua> for MultiValue<'lua> {
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
/// [`FromLua`]: trait.FromLua.html
/// [`MultiValue`]: struct.MultiValue.html
#[derive(Debug, Clone)]
pub struct Variadic<T>(Vec<T>);

impl<T> Variadic<T> {
    /// Creates an empty `Variadic` wrapper containing no values.
    pub fn new() -> Variadic<T> {
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

impl<'lua, T: ToLua<'lua>> ToLuaMulti<'lua> for Variadic<T> {
    fn to_lua_multi(self, lua: &'lua Lua) -> Result<MultiValue<'lua>> {
        self.0.into_iter().map(|e| e.to_lua(lua)).collect()
    }
}

impl<'lua, T: FromLua<'lua>> FromLuaMulti<'lua> for Variadic<T> {
    fn from_lua_multi(values: MultiValue<'lua>, lua: &'lua Lua) -> Result<Self> {
        values
            .into_iter()
            .map(|e| T::from_lua(e, lua))
            .collect::<Result<Vec<T>>>()
            .map(Variadic)
    }
}

macro_rules! impl_tuple {
    () => (
        impl<'lua> ToLuaMulti<'lua> for () {
            fn to_lua_multi(self, _: &'lua Lua) -> Result<MultiValue<'lua>> {
                Ok(MultiValue::new())
            }
        }

        impl<'lua> FromLuaMulti<'lua> for () {
            fn from_lua_multi(_: MultiValue<'lua>, _: &'lua Lua) -> Result<Self> {
                Ok(())
            }
        }
    );

    ($last:ident $($name:ident)*) => (
        impl<'lua, $($name,)* $last> ToLuaMulti<'lua> for ($($name,)* $last,)
            where $($name: ToLua<'lua>,)*
                  $last: ToLuaMulti<'lua>
        {
            #[allow(unused_mut)]
            #[allow(non_snake_case)]
            fn to_lua_multi(self, lua: &'lua Lua) -> Result<MultiValue<'lua>> {
                let ($($name,)* $last,) = self;

                let mut results = $last.to_lua_multi(lua)?;
                push_reverse!(results, $($name.to_lua(lua)?,)*);
                Ok(results)
            }
        }

        impl<'lua, $($name,)* $last> FromLuaMulti<'lua> for ($($name,)* $last,)
            where $($name: FromLua<'lua>,)*
                  $last: FromLuaMulti<'lua>
        {
            #[allow(unused_mut)]
            #[allow(non_snake_case)]
            fn from_lua_multi(mut values: MultiValue<'lua>, lua: &'lua Lua) -> Result<Self> {
                $(let $name = values.pop_front().unwrap_or(Nil);)*
                let $last = FromLuaMulti::from_lua_multi(values, lua)?;
                Ok(($(FromLua::from_lua($name, lua)?,)* $last,))
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
