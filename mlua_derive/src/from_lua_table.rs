use proc_macro::TokenStream;
use quote::{quote, format_ident};
use syn::{parse_macro_input, DeriveInput, Data, Fields};

pub fn from_lua_table(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let ident = input.ident;

    let fields = if let Data::Struct(data_struct) = input.data {
        match data_struct.fields {
            Fields::Named(fields) => fields,
            _ => panic!("FromLuaTable can only be derived for structs with named fields"),
        }
    } else {
        panic!("FromLuaTable can only be derived for structs");
    };

    let get_fields = fields.named.iter().map(|field| {
        let name = &field.ident;
        let name_str = name.as_ref().unwrap().to_string();
        quote! {
            #name: table.get(#name_str)?,
        }
    });

    let gen = quote! {
        impl<'lua> ::mlua::FromLua<'lua> for #ident {
            fn from_lua(lua_value: ::mlua::Value<'lua>, lua: &'lua ::mlua::Lua) -> ::mlua::Result<Self> {
                if let ::mlua::Value::Table(table) = lua_value {
                    Ok(Self {
                        #(#get_fields)*
                    })
                } else {
                    Err(::mlua::Error::FromLuaConversionError {
                        from: lua_value.type_name(),
                        to: stringify!(#ident),
                        message: Some(String::from("expected a Lua table")),
                    })
                }
            }
        }
    };

    gen.into()
}
