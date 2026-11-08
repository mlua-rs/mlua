//! # High-level bindings to Lua
//!
//! The `mlua` crate provides safe high-level bindings to the [Lua programming language].
//!
//! # The `Lua` object
//!
//! The main type exported by this library is the [`Lua`] struct. In addition to methods for
//! [executing] Lua chunks or [evaluating] Lua expressions, it provides methods for creating Lua
//! values and accessing the table of [globals].
//!
//! # Converting data
//!
//! The [`IntoLua`] and [`FromLua`] traits allow conversion from Rust types to Lua values and vice
//! versa. They are implemented for many data structures found in Rust's standard library.
//!
//! For more general conversions, the [`IntoLuaMulti`] and [`FromLuaMulti`] traits allow converting
//! between Rust types and *any number* of Lua values.
//!
//! Most code in `mlua` is generic over implementors of those traits, so in most places the normal
//! Rust data structures are accepted without having to write any boilerplate.
//!
//! # Custom Userdata
//!
//! The [`UserData`] trait can be implemented by user-defined types to make them available to Lua.
//! Methods and operators to be used from Lua can be added using the [`UserDataMethods`] API.
//! Fields are supported using the [`UserDataFields`] API.
//!
//! # Serde support
//!
//! The [`LuaSerdeExt`] trait implemented for [`Lua`] allows conversion from Rust types to Lua
//! values and vice versa using serde. Any user defined data type that implements
//! [`serde::Serialize`] or [`serde::Deserialize`] can be converted.
//! For convenience, additional functionality to handle `NULL` values and arrays is provided.
//!
//! The [`Value`] enum and other types implement [`serde::Serialize`] trait to support serializing
//! Lua values into Rust values.
//!
//! Requires `feature = "serde"`.
//!
//! # Async/await support
//!
//! The [`Lua::create_async_function`] allows creating non-blocking functions that returns
//! [`Future`]. Lua code with async capabilities can be executed by [`Function::call_async`] family
//! of functions or polling [`AsyncThread`] using any runtime (eg. Tokio).
//!
//! Requires `feature = "async"`.
//!
//! # `Send` and `Sync` support
//!
//! By default `mlua` is `!Send`. This can be changed by enabling `feature = "send"` that adds a
//! `Send` requirement to Rust functions and a `Send + Sync` requirement to [`UserData`] types.
//! A `Send`-only userdata types must therefore be wrapped (e.g. in a `Mutex`) or created through a
//! [`Scope`] to be used with the `send` feature.
//!
//! In this case [`Lua`] object and their types can be send or used from other threads. Internally
//! access to Lua VM is synchronized using a reentrant mutex that can be locked many times within
//! the same thread.
//!
//! [Lua programming language]: https://www.lua.org/
//! [executing]: crate::chunk::Chunk::exec
//! [evaluating]: crate::chunk::Chunk::eval
//! [globals]: crate::Lua::globals
//! [`Future`]: std::future::Future
//! [`serde::Serialize`]: https://docs.serde.rs/serde/ser/trait.Serialize.html
//! [`serde::Deserialize`]: https://docs.serde.rs/serde/de/trait.Deserialize.html
//! [`AsyncThread`]: crate::thread::AsyncThread

// Deny warnings inside doc tests / examples. When this isn't present, rustdoc doesn't show *any*
// warnings at all.
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "send"), allow(clippy::arc_with_non_send_sync))]
#![allow(unsafe_op_in_unsafe_fn)]

#[macro_use]
mod macros;

mod buffer;
mod conversion;
mod memory;
mod multi;
mod scope;
mod stdlib;
mod traits;
mod types;
mod util;
mod value;
mod vector;

pub mod chunk;
pub mod debug;
pub mod error;
pub mod function;
#[cfg(any(feature = "luau", doc))]
#[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
pub mod luau;
pub mod prelude;
pub mod state;
pub mod string;
pub mod table;
pub mod thread;
pub mod userdata;

pub use bstr::BString;
pub use ffi::{self, lua_CFunction, lua_State};
#[cfg(feature = "macros")]
#[doc(hidden)]
pub use inventory as __inventory;

#[doc(inline)]
pub use crate::error::{Error, Result};
pub use crate::error::{ErrorContext, ExternalError, ExternalResult};
#[doc(inline)]
pub use crate::function::Function;
pub use crate::multi::{MultiValue, Variadic};
pub use crate::scope::Scope;
#[doc(inline)]
pub use crate::state::{Lua, LuaOptions, WeakLua};
pub use crate::stdlib::StdLib;
#[doc(inline)]
pub use crate::string::LuaString;
pub use crate::string::{BorrowedBytes, BorrowedStr};
#[doc(inline)]
pub use crate::table::Table;
#[doc(inline)]
pub use crate::thread::Thread;
#[doc(inline)]
pub use crate::traits::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, ObjectLike};
pub use crate::types::{
    AppDataRef, AppDataRefMut, Either, Integer, LightUserData, MaybeSend, MaybeSync, Number, RegistryKey,
    VmState,
};
#[doc(inline)]
pub use crate::userdata::{AnyUserData, UserData};
pub use crate::userdata::{
    MetaMethod, UserDataFields, UserDataMethods, UserDataOwned, UserDataRef, UserDataRefMut, UserDataRegistry,
};
pub use crate::value::{Nil, Value};

/// Deprecated alias to [`LuaString`].
#[deprecated(since = "0.12.0", note = "use `mlua::LuaString` instead")]
#[doc(hidden)]
pub type String = crate::string::LuaString;

#[cfg(not(feature = "luau"))]
pub use crate::debug::HookTriggers;

#[cfg(any(feature = "luau", doc))]
#[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
pub use crate::{buffer::Buffer, vector::Vector};

#[cfg(feature = "serde")]
#[doc(inline)]
pub use crate::{serde::LuaSerdeExt, value::SerializableValue};

#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
pub mod serde;

#[cfg(feature = "mlua_derive")]
#[allow(unused_imports)]
#[macro_use]
extern crate mlua_derive;

#[doc = include_str!("../docs/chunk.md")]
#[cfg(feature = "macros")]
#[cfg_attr(docsrs, doc(cfg(feature = "macros")))]
pub use mlua_derive::chunk;

/// Derive [`FromLua`] for a Rust type.
///
/// Current implementation generate code that takes [`UserData`] value, borrow it (of the Rust type)
/// and clone.
#[cfg(feature = "macros")]
#[cfg_attr(docsrs, doc(cfg(feature = "macros")))]
pub use mlua_derive::FromLua;

#[doc = include_str!("../docs/UserData.md")]
#[cfg(feature = "macros")]
#[cfg_attr(docsrs, doc(cfg(feature = "macros")))]
pub use mlua_derive::UserData;

/// Registers items in an `impl` block as methods/fields of a [`UserData`](trait@UserData) type.
///
/// See the [`UserData`](derive@UserData) derive macro documentation for usage details.
#[cfg(feature = "macros")]
#[cfg_attr(docsrs, doc(cfg(feature = "macros")))]
pub use mlua_derive::userdata_impl;

#[doc = include_str!("../docs/lua_module.md")]
#[cfg(all(feature = "mlua_derive", any(feature = "module", doc)))]
#[cfg_attr(docsrs, doc(cfg(feature = "module")))]
pub use mlua_derive::lua_module;

#[cfg(all(feature = "module", feature = "send"))]
compile_error!("`send` feature is not supported in module mode");

pub(crate) mod private {
    use super::*;

    pub trait Sealed {}

    impl Sealed for Error {}
    impl<T> Sealed for std::result::Result<T, Error> {}
    impl Sealed for Lua {}
    impl Sealed for Table {}
    impl Sealed for AnyUserData {}
}
