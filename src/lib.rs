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
//! The [`ToLua`] and [`FromLua`] traits allow conversion from Rust types to Lua values and vice
//! versa. They are implemented for many data structures found in Rust's standard library.
//!
//! For more general conversions, the [`ToLuaMulti`] and [`FromLuaMulti`] traits allow converting
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
//! The [`LuaSerdeExt`] trait implemented for [`Lua`] allows conversion from Rust types to Lua values
//! and vice versa using serde. Any user defined data type that implements [`serde::Serialize`] or
//! [`serde::Deserialize`] can be converted.
//! For convenience, additional functionality to handle `NULL` values and arrays is provided.
//!
//! The [`Value`] enum implements [`serde::Serialize`] trait to support serializing Lua values
//! (including [`UserData`]) into Rust values.
//!
//! Requires `feature = "serialize"`.
//!
//! # Async/await support
//!
//! The [`create_async_function`] allows creating non-blocking functions that returns [`Future`].
//! Lua code with async capabilities can be executed by [`call_async`] family of functions or polling
//! [`AsyncThread`] using any runtime (eg. Tokio).
//!
//! Requires `feature = "async"`.
//!
//! # `Send` requirement
//! By default `mlua` is `!Send`. This can be changed by enabling `feature = "send"` that adds `Send` requirement
//! to [`Function`]s and [`UserData`].
//!
//! [Lua programming language]: https://www.lua.org/
//! [`Lua`]: struct.Lua.html
//! [executing]: struct.Chunk.html#method.exec
//! [evaluating]: struct.Chunk.html#method.eval
//! [globals]: struct.Lua.html#method.globals
//! [`ToLua`]: trait.ToLua.html
//! [`FromLua`]: trait.FromLua.html
//! [`ToLuaMulti`]: trait.ToLuaMulti.html
//! [`FromLuaMulti`]: trait.FromLuaMulti.html
//! [`Function`]: struct.Function.html
//! [`UserData`]: trait.UserData.html
//! [`UserDataFields`]: trait.UserDataFields.html
//! [`UserDataMethods`]: trait.UserDataMethods.html
//! [`LuaSerdeExt`]: serde/trait.LuaSerdeExt.html
//! [`Value`]: enum.Value.html
//! [`create_async_function`]: struct.Lua.html#method.create_async_function
//! [`call_async`]: struct.Function.html#method.call_async
//! [`AsyncThread`]: struct.AsyncThread.html
//! [`Future`]: ../futures_core/future/trait.Future.html
//! [`serde::Serialize`]: https://docs.serde.rs/serde/ser/trait.Serialize.html
//! [`serde::Deserialize`]: https://docs.serde.rs/serde/de/trait.Deserialize.html

// mlua types in rustdoc of other crates get linked to here.
#![doc(html_root_url = "https://docs.rs/mlua/0.6.0")]
// Deny warnings inside doc tests / examples. When this isn't present, rustdoc doesn't show *any*
// warnings at all.
#![doc(test(attr(deny(warnings))))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
mod macros;

mod conversion;
mod error;
mod ffi;
mod function;
mod hook;
mod lua;
mod multi;
mod scope;
mod stdlib;
mod string;
mod table;
mod thread;
mod types;
mod userdata;
mod util;
mod value;

#[doc(hidden)]
pub use crate::ffi::lua_State;

pub use crate::error::{Error, ExternalError, ExternalResult, Result};
pub use crate::function::Function;
pub use crate::hook::{Debug, DebugNames, DebugSource, DebugStack, HookTriggers};
pub use crate::lua::{AsChunk, Chunk, ChunkMode, GCMode, Lua, LuaOptions};
pub use crate::multi::Variadic;
pub use crate::scope::Scope;
pub use crate::stdlib::StdLib;
pub use crate::string::String;
pub use crate::table::{Table, TableExt, TablePairs, TableSequence};
pub use crate::thread::{Thread, ThreadStatus};
pub use crate::types::{Integer, LightUserData, Number, RegistryKey};
pub use crate::userdata::{
    AnyUserData, MetaMethod, UserData, UserDataFields, UserDataMetatable, UserDataMethods,
};
pub use crate::value::{FromLua, FromLuaMulti, MultiValue, Nil, ToLua, ToLuaMulti, Value};

#[cfg(feature = "async")]
pub use crate::thread::AsyncThread;

#[cfg(feature = "serialize")]
#[doc(inline)]
pub use crate::serde::{ser::Options as SerializeOptions, LuaSerdeExt};

pub mod prelude;
#[cfg(feature = "serialize")]
#[cfg_attr(docsrs, doc(cfg(feature = "serialize")))]
pub mod serde;

#[cfg(any(feature = "mlua_derive"))]
#[allow(unused_imports)]
#[macro_use]
extern crate mlua_derive;

/// Create a type that implements [`AsChunk`] and can capture Rust variables.
///
/// This macro allows to write Lua code directly in Rust code.
///
/// Rust variables can be referenced from Lua using `$` prefix, as shown in the example below.
/// User's Rust types needs to implement [`UserData`] or [`ToLua`] traits.
///
/// Captured variables are **moved** into the chunk.
///
/// ```
/// use mlua::{Lua, Result, chunk};
///
/// fn main() -> Result<()> {
///     let lua = Lua::new();
///     let name = "Rustacean";
///     lua.load(chunk! {
///         print("hello, " .. $name)
///     }).exec()
/// }
/// ```
///
/// ## Syntax issues
///
/// Since the Rust tokenizer will tokenize Lua code, this imposes some restrictions.
/// The main thing to remember is:
///
/// - Use double quoted strings (`""`) instead of single quoted strings (`''`).
///
///   (Single quoted strings only work if they contain a single character, since in Rust,
///   `'a'` is a character literal).
///
/// - Using Lua comments `--` is not desirable in **stable** Rust and can have bad side effects.
///
///   This is because procedural macros have Line/Column information available only in
///   **nightly** Rust. Instead, Lua chunks represented as a big single line of code in stable Rust.
///
///   As workaround, Rust comments `//` can be used.
///
/// Other minor limitations:
///
/// - Certain escape codes in string literals don't work.
///   (Specifically: `\a`, `\b`, `\f`, `\v`, `\123` (octal escape codes), `\u`, and `\U`).
///
///   These are accepted: : `\\`, `\n`, `\t`, `\r`, `\xAB` (hex escape codes), and `\0`.
///
/// - The `//` (floor division) operator is unusable, as its start a comment.
///
/// Everything else should work.
///
/// [`AsChunk`]: trait.AsChunk.html
/// [`UserData`]: trait.UserData.html
/// [`ToLua`]: trait.ToLua.html
#[cfg(any(feature = "macros"))]
#[cfg_attr(docsrs, doc(cfg(feature = "macros")))]
pub use mlua_derive::chunk;

#[cfg(any(feature = "module"))]
#[cfg_attr(docsrs, doc(cfg(feature = "module")))]
pub use mlua_derive::lua_module;
