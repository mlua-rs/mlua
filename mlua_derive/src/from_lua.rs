use proc_macro::TokenStream;
use quote::quote;
use syn::ext::IdentExt;
use syn::{DeriveInput, parse_macro_input, parse_quote};

pub fn from_lua(input: TokenStream) -> TokenStream {
    let DeriveInput {
        ident, mut generics, ..
    } = parse_macro_input!(input as DeriveInput);

    let ident_str = ident.unraw().to_string();
    generics
        .make_where_clause()
        .predicates
        .push(parse_quote!(Self: 'static + Clone));
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    quote! {
        impl #impl_generics ::mlua::FromLua for #ident #ty_generics #where_clause {
            #[inline]
            fn from_lua(value: ::mlua::Value, _: &::mlua::Lua) -> ::mlua::Result<Self> {
                match value {
                    ::mlua::Value::UserData(ud) => Ok(ud.borrow::<Self>()?.clone()),
                    _ => Err(::mlua::Error::FromLuaConversionError {
                        from: value.type_name(),
                        to: #ident_str.to_string(),
                        message: None,
                    }),
                }
            }
        }
    }
    .into()
}
