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
//! By default `mlua` is `!Send`. This can be changed by enabling `feature = "send"` that adds
//! `Send` requirement to Rust functions and [`UserData`] types.
//!
//! In this case [`Lua`] object and their types can be send or used from other threads. Internally
//! access to Lua VM is synchronized using a reentrant mutex that can be locked many times within
//! the same thread.
//!
//! [Lua programming language]: https://www.lua.org/
//! [executing]: crate::Chunk::exec
//! [evaluating]: crate::Chunk::eval
//! [globals]: crate::Lua::globals
//! [`Future`]: std::future::Future
//! [`serde::Serialize`]: https://docs.serde.rs/serde/ser/trait.Serialize.html
//! [`serde::Deserialize`]: https://docs.serde.rs/serde/de/trait.Deserialize.html

// Deny warnings inside doc tests / examples. When this isn't present, rustdoc doesn't show *any*
// warnings at all.
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(send), allow(clippy::arc_with_non_send_sync))]
#![allow(clippy::ptr_eq)]
#![allow(unsafe_op_in_unsafe_fn)]

#[macro_use]
mod macros;

mod buffer;
mod chunk;
mod conversion;
mod debug;
mod error;
mod function;
#[cfg(any(feature = "luau", doc))]
mod luau;
mod memory;
mod multi;
mod scope;
mod state;
mod stdlib;
mod string;
mod table;
mod thread;
mod traits;
mod types;
mod userdata;
mod util;
mod value;
mod vector;

pub mod prelude;

pub use bstr::BString;
pub use ffi::{self, lua_CFunction, lua_State};

pub use crate::chunk::{AsChunk, Chunk, ChunkMode};
pub use crate::debug::{Debug, DebugEvent, DebugNames, DebugSource, DebugStack};
pub use crate::error::{Error, ErrorContext, ExternalError, ExternalResult, Result};
pub use crate::function::{Function, FunctionInfo};
pub use crate::multi::{MultiValue, Variadic};
pub use crate::scope::Scope;
pub use crate::state::{GCMode, Lua, LuaOptions, WeakLua};
pub use crate::stdlib::StdLib;
pub use crate::string::{BorrowedBytes, BorrowedStr, String};
pub use crate::table::{Table, TablePairs, TableSequence};
pub use crate::thread::{Thread, ThreadStatus};
pub use crate::traits::{
    FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, LuaNativeFn, LuaNativeFnMut, ObjectLike,
};
pub use crate::types::{
    AppDataRef, AppDataRefMut, Either, Integer, LightUserData, MaybeSend, Number, RegistryKey, VmState,
};
pub use crate::userdata::{
    AnyUserData, MetaMethod, UserData, UserDataFields, UserDataMetatable, UserDataMethods, UserDataRef,
    UserDataRefMut, UserDataRegistry,
};
pub use crate::value::{Nil, Value};

#[cfg(not(feature = "luau"))]
pub use crate::debug::HookTriggers;

#[cfg(any(feature = "luau", doc))]
#[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
pub use crate::{
    buffer::Buffer,
    chunk::{CompileConstant, Compiler},
    function::CoverageInfo,
    luau::{NavigateError, Require, TextRequirer},
    vector::Vector,
};

#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub use crate::{thread::AsyncThread, traits::LuaNativeAsyncFn};

#[cfg(feature = "serde")]
#[doc(inline)]
pub use crate::serde::{de::Options as DeserializeOptions, ser::Options as SerializeOptions, LuaSerdeExt};

#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
pub mod serde;

#[cfg(feature = "mlua_derive")]
#[allow(unused_imports)]
#[macro_use]
extern crate mlua_derive;

/// Create a type that implements [`AsChunk`] and can capture Rust variables.
///
/// This macro allows to write Lua code directly in Rust code.
///
/// Rust variables can be referenced from Lua using `$` prefix, as shown in the example below.
/// User's Rust types needs to implement [`UserData`] or [`IntoLua`] traits.
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
/// - Certain escape codes in string literals don't work. (Specifically: `\a`, `\b`, `\f`, `\v`,
///   `\123` (octal escape codes), `\u`, and `\U`).
///
///   These are accepted: : `\\`, `\n`, `\t`, `\r`, `\xAB` (hex escape codes), and `\0`.
///
/// - The `//` (floor division) operator is unusable, as its start a comment.
///
/// Everything else should work.
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

/// Registers Lua module entrypoint.
///
/// You can register multiple entrypoints as required.
///
/// ```ignore
/// use mlua::{Lua, Result, Table};
///
/// #[mlua::lua_module]
/// fn my_module(lua: &Lua) -> Result<Table> {
///     let exports = lua.create_table()?;
///     exports.set("hello", "world")?;
///     Ok(exports)
/// }
/// ```
///
/// Internally in the code above the compiler defines C function `luaopen_my_module`.
///
/// You can also pass options to the attribute:
///
/// * name - name of the module, defaults to the name of the function
///
/// ```ignore
/// #[mlua::lua_module(name = "alt_module")]
/// fn my_module(lua: &Lua) -> Result<Table> {
///     ...
/// }
/// ```
///
/// * skip_memory_check - skip memory allocation checks for some operations.
///
/// In module mode, mlua runs in unknown environment and cannot say are there any memory
/// limits or not. As result, some operations that require memory allocation runs in
/// protected mode. Setting this attribute will improve performance of such operations
/// with risk of having uncaught exceptions and memory leaks.
///
/// ```ignore
/// #[mlua::lua_module(skip_memory_check)]
/// fn my_module(lua: &Lua) -> Result<Table> {
///     ...
/// }
/// ```
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
