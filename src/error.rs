use std::error::Error as StdError;
use std::fmt;
use std::io::Error as IoError;
use std::net::AddrParseError;
use std::result::Result as StdResult;
use std::str::Utf8Error;
use std::string::String as StdString;
use std::sync::Arc;

use crate::private::Sealed;

#[cfg(feature = "error-send")]
type DynStdError = dyn StdError + Send + Sync;

#[cfg(not(feature = "error-send"))]
type DynStdError = dyn StdError;

/// Error type returned by `mlua` methods.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Error {
    /// Syntax error while parsing Lua source code.
    SyntaxError {
        /// The error message as returned by Lua.
        message: StdString,
        /// `true` if the error can likely be fixed by appending more input to the source code.
        ///
        /// This is useful for implementing REPLs as they can query the user for more input if this
        /// is set.
        incomplete_input: bool,
    },
    /// Lua runtime error, aka `LUA_ERRRUN`.
    ///
    /// The Lua VM returns this error when a builtin operation is performed on incompatible types.
    /// Among other things, this includes invoking operators on wrong types (such as calling or
    /// indexing a `nil` value).
    RuntimeError(StdString),
    /// Lua memory error, aka `LUA_ERRMEM`
    ///
    /// The Lua VM returns this error when the allocator does not return the requested memory, aka
    /// it is an out-of-memory error.
    MemoryError(StdString),
    /// Lua garbage collector error, aka `LUA_ERRGCMM`.
    ///
    /// The Lua VM returns this error when there is an error running a `__gc` metamethod.
    #[cfg(any(feature = "lua53", feature = "lua52", doc))]
    #[cfg_attr(docsrs, doc(cfg(any(feature = "lua53", feature = "lua52"))))]
    GarbageCollectorError(StdString),
    /// Potentially unsafe action in safe mode.
    SafetyError(StdString),
    /// Memory control is not available.
    ///
    /// This error can only happen when Lua state was not created by us and does not have the
    /// custom allocator attached.
    MemoryControlNotAvailable,
    /// A mutable callback has triggered Lua code that has called the same mutable callback again.
    ///
    /// This is an error because a mutable callback can only be borrowed mutably once.
    RecursiveMutCallback,
    /// Either a callback or a userdata method has been called, but the callback or userdata has
    /// been destructed.
    ///
    /// This can happen either due to to being destructed in a previous __gc, or due to being
    /// destructed from exiting a `Lua::scope` call.
    CallbackDestructed,
    /// Not enough stack space to place arguments to Lua functions or return values from callbacks.
    ///
    /// Due to the way `mlua` works, it should not be directly possible to run out of stack space
    /// during normal use. The only way that this error can be triggered is if a `Function` is
    /// called with a huge number of arguments, or a Rust callback returns a huge number of return
    /// values.
    StackError,
    /// Too many arguments to [`Function::bind`].
    ///
    /// [`Function::bind`]: crate::Function::bind
    BindError,
    /// Bad argument received from Lua (usually when calling a function).
    ///
    /// This error can help to identify the argument that caused the error
    /// (which is stored in the corresponding field).
    BadArgument {
        /// Function that was called.
        to: Option<StdString>,
        /// Argument position (usually starts from 1).
        pos: usize,
        /// Argument name.
        name: Option<StdString>,
        /// Underlying error returned when converting argument to a Lua value.
        cause: Arc<Error>,
    },
    /// A Rust value could not be converted to a Lua value.
    ToLuaConversionError {
        /// Name of the Rust type that could not be converted.
        from: String,
        /// Name of the Lua type that could not be created.
        to: &'static str,
        /// A message indicating why the conversion failed in more detail.
        message: Option<StdString>,
    },
    /// A Lua value could not be converted to the expected Rust type.
    FromLuaConversionError {
        /// Name of the Lua type that could not be converted.
        from: &'static str,
        /// Name of the Rust type that could not be created.
        to: String,
        /// A string containing more detailed error information.
        message: Option<StdString>,
    },
    /// [`Thread::resume`] was called on an unresumable coroutine.
    ///
    /// A coroutine is unresumable if its main function has returned or if an error has occurred
    /// inside the coroutine. Already running coroutines are also marked as unresumable.
    ///
    /// [`Thread::status`] can be used to check if the coroutine can be resumed without causing this
    /// error.
    ///
    /// [`Thread::resume`]: crate::Thread::resume
    /// [`Thread::status`]: crate::Thread::status
    CoroutineUnresumable,
    /// An [`AnyUserData`] is not the expected type in a borrow.
    ///
    /// This error can only happen when manually using [`AnyUserData`], or when implementing
    /// metamethods for binary operators. Refer to the documentation of [`UserDataMethods`] for
    /// details.
    ///
    /// [`AnyUserData`]: crate::AnyUserData
    /// [`UserDataMethods`]: crate::UserDataMethods
    UserDataTypeMismatch,
    /// An [`AnyUserData`] borrow failed because it has been destructed.
    ///
    /// This error can happen either due to to being destructed in a previous __gc, or due to being
    /// destructed from exiting a `Lua::scope` call.
    ///
    /// [`AnyUserData`]: crate::AnyUserData
    UserDataDestructed,
    /// An [`AnyUserData`] immutable borrow failed.
    ///
    /// This error can occur when a method on a [`UserData`] type calls back into Lua, which then
    /// tries to call a method on the same [`UserData`] type. Consider restructuring your API to
    /// prevent these errors.
    ///
    /// [`AnyUserData`]: crate::AnyUserData
    /// [`UserData`]: crate::UserData
    UserDataBorrowError,
    /// An [`AnyUserData`] mutable borrow failed.
    ///
    /// This error can occur when a method on a [`UserData`] type calls back into Lua, which then
    /// tries to call a method on the same [`UserData`] type. Consider restructuring your API to
    /// prevent these errors.
    ///
    /// [`AnyUserData`]: crate::AnyUserData
    /// [`UserData`]: crate::UserData
    UserDataBorrowMutError,
    /// A [`MetaMethod`] operation is restricted (typically for `__gc` or `__metatable`).
    ///
    /// [`MetaMethod`]: crate::MetaMethod
    MetaMethodRestricted(StdString),
    /// A [`MetaMethod`] (eg. `__index` or `__newindex`) has invalid type.
    ///
    /// [`MetaMethod`]: crate::MetaMethod
    MetaMethodTypeError {
        /// Name of the metamethod.
        method: StdString,
        /// Passed value type.
        type_name: &'static str,
        /// A string containing more detailed error information.
        message: Option<StdString>,
    },
    /// A [`RegistryKey`] produced from a different Lua state was used.
    ///
    /// [`RegistryKey`]: crate::RegistryKey
    MismatchedRegistryKey,
    /// A Rust callback returned `Err`, raising the contained `Error` as a Lua error.
    CallbackError {
        /// Lua call stack backtrace.
        traceback: StdString,
        /// Original error returned by the Rust code.
        cause: Arc<Error>,
    },
    /// A Rust panic that was previously resumed, returned again.
    ///
    /// This error can occur only when a Rust panic resumed previously was recovered
    /// and returned again.
    PreviouslyResumedPanic,
    /// Serialization error.
    #[cfg(feature = "serde")]
    #[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
    SerializeError(StdString),
    /// Deserialization error.
    #[cfg(feature = "serde")]
    #[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
    DeserializeError(StdString),
    /// A custom error.
    ///
    /// This can be used for returning user-defined errors from callbacks.
    ///
    /// Returning `Err(ExternalError(...))` from a Rust callback will raise the error as a Lua
    /// error. The Rust code that originally invoked the Lua code then receives a `CallbackError`,
    /// from which the original error (and a stack traceback) can be recovered.
    ExternalError(Arc<DynStdError>),
    /// An error with additional context.
    WithContext {
        /// A string containing additional context.
        context: StdString,
        /// Underlying error.
        cause: Arc<Error>,
    },
}

/// A specialized `Result` type used by `mlua`'s API.
pub type Result<T> = StdResult<T, Error>;

#[cfg(not(tarpaulin_include))]
impl fmt::Display for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::SyntaxError { message, .. } => write!(fmt, "syntax error: {message}"),
            Error::RuntimeError(msg) => write!(fmt, "runtime error: {msg}"),
            Error::MemoryError(msg) => {
                write!(fmt, "memory error: {msg}")
            }
            #[cfg(any(feature = "lua53", feature = "lua52"))]
            Error::GarbageCollectorError(msg) => {
                write!(fmt, "garbage collector error: {msg}")
            }
            Error::SafetyError(msg) => {
                write!(fmt, "safety error: {msg}")
            },
            Error::MemoryControlNotAvailable => {
                write!(fmt, "memory control is not available")
            }
            Error::RecursiveMutCallback => write!(fmt, "mutable callback called recursively"),
            Error::CallbackDestructed => write!(
                fmt,
                "a destructed callback or destructed userdata method was called"
            ),
            Error::StackError => write!(
                fmt,
                "out of Lua stack, too many arguments to a Lua function or too many return values from a callback"
            ),
            Error::BindError => write!(
                fmt,
                "too many arguments to Function::bind"
            ),
            Error::BadArgument { to, pos, name, cause } => {
                if let Some(name) = name {
                    write!(fmt, "bad argument `{name}`")?;
                } else {
                    write!(fmt, "bad argument #{pos}")?;
                }
                if let Some(to) = to {
                    write!(fmt, " to `{to}`")?;
                }
                write!(fmt, ": {cause}")
            },
            Error::ToLuaConversionError { from, to, message } => {
                write!(fmt, "error converting {from} to Lua {to}")?;
                match message {
                    None => Ok(()),
                    Some(message) => write!(fmt, " ({message})"),
                }
            }
            Error::FromLuaConversionError { from, to, message } => {
                write!(fmt, "error converting Lua {from} to {to}")?;
                match message {
                    None => Ok(()),
                    Some(message) => write!(fmt, " ({message})"),
                }
            }
            Error::CoroutineUnresumable => write!(fmt, "coroutine is non-resumable"),
            Error::UserDataTypeMismatch => write!(fmt, "userdata is not expected type"),
            Error::UserDataDestructed => write!(fmt, "userdata has been destructed"),
            Error::UserDataBorrowError => write!(fmt, "error borrowing userdata"),
            Error::UserDataBorrowMutError => write!(fmt, "error mutably borrowing userdata"),
            Error::MetaMethodRestricted(method) => write!(fmt, "metamethod {method} is restricted"),
            Error::MetaMethodTypeError { method, type_name, message } => {
                write!(fmt, "metamethod {method} has unsupported type {type_name}")?;
                match message {
                    None => Ok(()),
                    Some(message) => write!(fmt, " ({message})"),
                }
            }
            Error::MismatchedRegistryKey => {
                write!(fmt, "RegistryKey used from different Lua state")
            }
            Error::CallbackError { cause, traceback } => {
                // Trace errors down to the root
                let (mut cause, mut full_traceback) = (cause, None);
                while let Error::CallbackError { cause: cause2, traceback: traceback2 } = &**cause {
                    cause = cause2;
                    full_traceback = Some(traceback2);
                }
                writeln!(fmt, "{cause}")?;
                if let Some(full_traceback) = full_traceback {
                    let traceback = traceback.trim_start_matches("stack traceback:");
                    let traceback = traceback.trim_start().trim_end();
                    // Try to find local traceback within the full traceback
                    if let Some(pos) = full_traceback.find(traceback) {
                        write!(fmt, "{}", &full_traceback[..pos])?;
                        writeln!(fmt, ">{}", &full_traceback[pos..].trim_end())?;
                    } else {
                        writeln!(fmt, "{}", full_traceback.trim_end())?;
                    }
                } else {
                    writeln!(fmt, "{}", traceback.trim_end())?;
                }
                Ok(())
            }
            Error::PreviouslyResumedPanic => {
                write!(fmt, "previously resumed panic returned again")
            }
            #[cfg(feature = "serde")]
            Error::SerializeError(err) => {
                write!(fmt, "serialize error: {err}")
            },
            #[cfg(feature = "serde")]
            Error::DeserializeError(err) => {
                write!(fmt, "deserialize error: {err}")
            },
            Error::ExternalError(err) => err.fmt(fmt),
            Error::WithContext { context, cause } => {
                writeln!(fmt, "{context}")?;
                write!(fmt, "{cause}")
            }
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            // An error type with a source error should either return that error via source or
            // include that source's error message in its own Display output, but never both.
            // https://blog.rust-lang.org/inside-rust/2021/07/01/What-the-error-handling-project-group-is-working-towards.html
            // Given that we include source to fmt::Display implementation for `CallbackError`, this call
            // returns nothing.
            Error::CallbackError { .. } => None,
            Error::ExternalError(err) => err.source(),
            Error::WithContext { cause, .. } => Self::source(cause),
            _ => None,
        }
    }
}

impl Error {
    /// Creates a new `RuntimeError` with the given message.
    #[inline]
    pub fn runtime<S: fmt::Display>(message: S) -> Self {
        Error::RuntimeError(message.to_string())
    }

    /// Wraps an external error object.
    #[inline]
    pub fn external<T: Into<Box<DynStdError>>>(err: T) -> Self {
        Error::ExternalError(err.into().into())
    }

    /// Attempts to downcast the external error object to a concrete type by reference.
    pub fn downcast_ref<T>(&self) -> Option<&T>
    where
        T: StdError + 'static,
    {
        match self {
            Error::ExternalError(err) => err.downcast_ref(),
            Error::WithContext { cause, .. } => Self::downcast_ref(cause),
            _ => None,
        }
    }

    /// An iterator over the chain of nested errors wrapped by this Error.
    pub fn chain(&self) -> impl Iterator<Item = &(dyn StdError + 'static)> {
        Chain {
            root: self,
            current: None,
        }
    }

    /// Returns the parent of this error.
    #[doc(hidden)]
    pub fn parent(&self) -> Option<&Error> {
        match self {
            Error::CallbackError { cause, .. } => Some(cause.as_ref()),
            Error::WithContext { cause, .. } => Some(cause.as_ref()),
            _ => None,
        }
    }

    pub(crate) fn bad_self_argument(to: &str, cause: Error) -> Self {
        Error::BadArgument {
            to: Some(to.to_string()),
            pos: 1,
            name: Some("self".to_string()),
            cause: Arc::new(cause),
        }
    }

    pub(crate) fn from_lua_conversion(
        from: &'static str,
        to: impl ToString,
        message: impl Into<Option<String>>,
    ) -> Self {
        Error::FromLuaConversionError {
            from,
            to: to.to_string(),
            message: message.into(),
        }
    }
}

/// Trait for converting [`std::error::Error`] into Lua [`Error`].
pub trait ExternalError {
    fn into_lua_err(self) -> Error;
}

impl<E: Into<Box<DynStdError>>> ExternalError for E {
    fn into_lua_err(self) -> Error {
        Error::external(self)
    }
}

/// Trait for converting [`std::result::Result`] into Lua [`Result`].
pub trait ExternalResult<T> {
    fn into_lua_err(self) -> Result<T>;
}

impl<T, E> ExternalResult<T> for StdResult<T, E>
where
    E: ExternalError,
{
    fn into_lua_err(self) -> Result<T> {
        self.map_err(|e| e.into_lua_err())
    }
}

/// Provides the `context` method for [`Error`] and `Result<T, Error>`.
pub trait ErrorContext: Sealed {
    /// Wraps the error value with additional context.
    fn context<C: fmt::Display>(self, context: C) -> Self;

    /// Wrap the error value with additional context that is evaluated lazily
    /// only once an error does occur.
    fn with_context<C: fmt::Display>(self, f: impl FnOnce(&Error) -> C) -> Self;
}

impl ErrorContext for Error {
    fn context<C: fmt::Display>(self, context: C) -> Self {
        let context = context.to_string();
        match self {
            Error::WithContext { cause, .. } => Error::WithContext { context, cause },
            _ => Error::WithContext {
                context,
                cause: Arc::new(self),
            },
        }
    }

    fn with_context<C: fmt::Display>(self, f: impl FnOnce(&Error) -> C) -> Self {
        let context = f(&self).to_string();
        match self {
            Error::WithContext { cause, .. } => Error::WithContext { context, cause },
            _ => Error::WithContext {
                context,
                cause: Arc::new(self),
            },
        }
    }
}

impl<T> ErrorContext for Result<T> {
    fn context<C: fmt::Display>(self, context: C) -> Self {
        self.map_err(|err| err.context(context))
    }

    fn with_context<C: fmt::Display>(self, f: impl FnOnce(&Error) -> C) -> Self {
        self.map_err(|err| err.with_context(f))
    }
}

impl From<AddrParseError> for Error {
    fn from(err: AddrParseError) -> Self {
        Error::external(err)
    }
}

impl From<IoError> for Error {
    fn from(err: IoError) -> Self {
        Error::external(err)
    }
}

impl From<Utf8Error> for Error {
    fn from(err: Utf8Error) -> Self {
        Error::external(err)
    }
}

#[cfg(feature = "serde")]
impl serde::ser::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::SerializeError(msg.to_string())
    }
}

#[cfg(feature = "serde")]
impl serde::de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::DeserializeError(msg.to_string())
    }
}

#[cfg(feature = "anyhow")]
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        match err.downcast::<Self>() {
            Ok(err) => err,
            Err(err) => Error::external(err),
        }
    }
}

struct Chain<'a> {
    root: &'a Error,
    current: Option<&'a (dyn StdError + 'static)>,
}

impl<'a> Iterator for Chain<'a> {
    type Item = &'a (dyn StdError + 'static);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let error: Option<&dyn StdError> = match self.current {
                None => {
                    self.current = Some(self.root);
                    self.current
                }
                Some(current) => match current.downcast_ref::<Error>()? {
                    Error::BadArgument { cause, .. }
                    | Error::CallbackError { cause, .. }
                    | Error::WithContext { cause, .. } => {
                        self.current = Some(&**cause);
                        self.current
                    }
                    Error::ExternalError(err) => {
                        self.current = Some(&**err);
                        self.current
                    }
                    _ => None,
                },
            };

            // Skip `ExternalError` as it only wraps the underlying error
            // without meaningful context
            if let Some(Error::ExternalError(_)) = error?.downcast_ref::<Error>() {
                continue;
            }

            return self.current;
        }
    }
}

#[cfg(test)]
mod assertions {
    use super::*;

    #[cfg(not(feature = "error-send"))]
    static_assertions::assert_not_impl_any!(Error: Send, Sync);
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(Error: Send, Sync);
}
