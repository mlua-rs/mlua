use std::ffi::CStr;
use std::fmt;
use std::hash::Hash;
use std::os::raw::c_int;

use crate::error::{Error, Result};
use crate::state::{Lua, RawLua};
use crate::traits::ShortTypeName as _;
use crate::value::{FromLua, IntoLua, Value};

/// Combination of two types into a single one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Either<L, R> {
    Left(L),
    Right(R),
}

impl<L, R> Either<L, R> {
    /// Return true if the value is the Left variant.
    #[inline]
    pub fn is_left(&self) -> bool {
        matches!(self, Either::Left(_))
    }

    /// Return true if the value is the Right variant.
    #[inline]
    pub fn is_right(&self) -> bool {
        matches!(self, Either::Right(_))
    }

    /// Convert the left side of `Either<L, R>` to an `Option<L>`.
    #[inline]
    pub fn left(self) -> Option<L> {
        match self {
            Either::Left(l) => Some(l),
            _ => None,
        }
    }

    /// Convert the right side of `Either<L, R>` to an `Option<R>`.
    #[inline]
    pub fn right(self) -> Option<R> {
        match self {
            Either::Right(r) => Some(r),
            _ => None,
        }
    }

    /// Convert `&Either<L, R>` to `Either<&L, &R>`.
    #[inline]
    pub fn as_ref(&self) -> Either<&L, &R> {
        match self {
            Either::Left(l) => Either::Left(l),
            Either::Right(r) => Either::Right(r),
        }
    }

    /// Convert `&mut Either<L, R>` to `Either<&mut L, &mut R>`.
    #[inline]
    pub fn as_mut(&mut self) -> Either<&mut L, &mut R> {
        match self {
            Either::Left(l) => Either::Left(l),
            Either::Right(r) => Either::Right(r),
        }
    }
}

impl<L, R> fmt::Display for Either<L, R>
where
    L: fmt::Display,
    R: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Either::Left(a) => a.fmt(f),
            Either::Right(b) => b.fmt(f),
        }
    }
}

impl<L: IntoLua, R: IntoLua> IntoLua for Either<L, R> {
    #[inline]
    fn into_lua(self, lua: &Lua) -> Result<Value> {
        match self {
            Either::Left(l) => l.into_lua(lua),
            Either::Right(r) => r.into_lua(lua),
        }
    }

    #[inline]
    unsafe fn push_into_stack(self, lua: &RawLua) -> Result<()> {
        match self {
            Either::Left(l) => l.push_into_stack(lua),
            Either::Right(r) => r.push_into_stack(lua),
        }
    }
}

impl<L: FromLua, R: FromLua> FromLua for Either<L, R> {
    #[inline]
    fn from_lua(value: Value, lua: &Lua) -> Result<Self> {
        let value_type_name = value.type_name();
        // Try the left type first
        match L::from_lua(value.clone(), lua) {
            Ok(l) => Ok(Either::Left(l)),
            // Try the right type
            Err(_) => match R::from_lua(value, lua).map(Either::Right) {
                Ok(r) => Ok(r),
                Err(_) => Err(Error::FromLuaConversionError {
                    from: value_type_name,
                    to: Self::type_name(),
                    message: None,
                }),
            },
        }
    }

    #[inline]
    unsafe fn from_stack(idx: c_int, lua: &RawLua) -> Result<Self> {
        match L::from_stack(idx, lua) {
            Ok(l) => Ok(Either::Left(l)),
            Err(_) => match R::from_stack(idx, lua).map(Either::Right) {
                Ok(r) => Ok(r),
                Err(_) => {
                    let value_type_name = CStr::from_ptr(ffi::luaL_typename(lua.state(), idx));
                    Err(Error::FromLuaConversionError {
                        from: value_type_name.to_str().unwrap(),
                        to: Self::type_name(),
                        message: None,
                    })
                }
            },
        }
    }
}
