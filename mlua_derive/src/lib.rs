use proc_macro::TokenStream;

mod module;

#[cfg(feature = "macros")]
use {crate::chunk::Chunk, proc_macro_error2::proc_macro_error};

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
#[proc_macro_error]
pub fn chunk(input: TokenStream) -> TokenStream {
    Chunk::new(input).expand().into()
}

#[cfg(feature = "macros")]
#[proc_macro_derive(FromLua)]
pub fn from_lua(input: TokenStream) -> TokenStream {
    from_lua::from_lua(input)
}

/// Attribute macro for exposing a Rust type as a Lua userdata.
#[cfg(feature = "macros")]
#[proc_macro_attribute]
pub fn userdata(attr: TokenStream, item: TokenStream) -> TokenStream {
    userdata::userdata_type(attr, item)
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
