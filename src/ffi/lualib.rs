//! Contains definitions from `lualib.h`.

#[cfg(feature = "lua54")]
pub use super::lua54::lualib::*;

#[cfg(feature = "lua53")]
pub use super::lua53::lualib::*;

#[cfg(feature = "lua52")]
pub use super::lua52::lualib::*;

#[cfg(any(feature = "lua51", feature = "luajit"))]
pub use super::lua51::lualib::*;
