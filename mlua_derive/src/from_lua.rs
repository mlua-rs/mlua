use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

pub fn from_lua(input: TokenStream) -> TokenStream {
    let DeriveInput {
        ident, generics, ..
    } = parse_macro_input!(input as DeriveInput);

    let where_clause = match &generics.where_clause {
        Some(where_clause) => quote! { #where_clause, Self: 'static + Clone },
        None => quote! { where Self: 'static + Clone },
    };
    let ident_str = ident.to_string();

    quote! {
      impl #generics ::mlua::FromLua<'_> for #ident #generics #where_clause {
        #[inline]
        fn from_lua(value: ::mlua::Value<'_>, lua: &'_ ::mlua::Lua) -> ::mlua::Result<Self> {
          match value {
            ::mlua::Value::UserData(ud) => Ok(ud.borrow::<Self>()?.clone()),
            _ => Err(::mlua::Error::FromLuaConversionError {
                from: value.type_name(),
                to: #ident_str,
                message: None,
            }),
          }
        }
      }
    }
    .into()
}
