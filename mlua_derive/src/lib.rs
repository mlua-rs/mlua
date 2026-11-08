use proc_macro::TokenStream;

mod module;

#[cfg(feature = "macros")]
use crate::chunk::Chunk;

#[cfg(feature = "macros")]
macro_rules! try_compile {
    ($expr:expr) => {
        match $expr {
            Ok(val) => val,
            Err(err) => return err.to_compile_error().into(),
        }
    };
}

#[proc_macro_attribute]
pub fn lua_module(attr: TokenStream, item: TokenStream) -> TokenStream {
    module::lua_module(attr, item)
}

#[cfg(feature = "macros")]
#[proc_macro]
pub fn chunk(input: TokenStream) -> TokenStream {
    match Chunk::new(input) {
        Ok(chunk) => chunk.expand().into(),
        Err(err) => err.into(),
    }
}

#[cfg(feature = "macros")]
#[proc_macro_derive(FromLua)]
pub fn from_lua(input: TokenStream) -> TokenStream {
    from_lua::from_lua(input)
}

/// Derive macro for implementing `UserData` for a Rust type.
#[cfg(feature = "macros")]
#[proc_macro_derive(UserData, attributes(lua))]
pub fn userdata(item: TokenStream) -> TokenStream {
    userdata::userdata_type(item)
}

/// Attribute macro for exposing impl block methods to Lua userdata.
#[cfg(feature = "macros")]
#[proc_macro_attribute]
pub fn userdata_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    userdata::userdata_impl::userdata_impl(attr, item)
}

#[cfg(feature = "macros")]
mod chunk;
#[cfg(feature = "macros")]
mod from_lua;
#[cfg(feature = "macros")]
mod userdata;
