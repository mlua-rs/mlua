extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::quote_spanned;
use syn::{parse_macro_input, spanned::Spanned, AttributeArgs, Error, ItemFn};

#[proc_macro_attribute]
pub fn lua_module(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as AttributeArgs);
    let item = parse_macro_input!(item as ItemFn);

    if args.len() > 0 {
        let err = Error::new(Span::call_site(), "the number of arguments must be zero")
            .to_compile_error();
        return err.into();
    }

    let span = item.span();
    let item_name = item.sig.ident.clone();
    let ext_entrypoint_name = Ident::new(&format!("luaopen_{}", item.sig.ident), Span::call_site());

    let wrapped = quote_spanned! { span =>
        mlua::require_module_feature!();

        #[no_mangle]
        unsafe extern "C" fn #ext_entrypoint_name(state: *mut mlua::lua_State) -> std::os::raw::c_int {
            #item

            mlua::Lua::init_from_ptr(state)
                .entrypoint1(#item_name)
                .unwrap()
        }
    };

    wrapped.into()
}
