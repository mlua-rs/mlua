use proc_macro::TokenStream;
use quote::{quote, format_ident};
use syn::{parse_macro_input, DeriveInput, Data, Fields, Generics};

pub fn to_lua_table(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let ident = input.ident;
    let generics = add_trait_bounds(input.generics);
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let fields = if let Data::Struct(data_struct) = input.data {
        match data_struct.fields {
            Fields::Named(fields) => fields,
            _ => panic!("ToLua can only be derived for structs with named fields"),
        }
    } else {
        panic!("ToLua can only be derived for structs");
    };

    let set_fields = fields.named.iter().map(|field| {
        let name = &field.ident;
        let name_str = name.as_ref().unwrap().to_string();
        quote! {
            table.set(#name_str, self.#name)?;
        }
    });

    let gen = quote! {
        impl #impl_generics mlua::IntoLua for #ident #ty_generics #where_clause {
            fn into_lua(self, lua: &mlua::Lua) -> ::mlua::Result<::mlua::Value> {
                let table = lua.create_table()?;
                #(#set_fields)*
                Ok(::mlua::Value::Table(table))
            }
        }
    };

    gen.into()
}

fn add_trait_bounds(mut generics: Generics) -> Generics {
    for param in &mut generics.params {
        if let syn::GenericParam::Type(type_param) = param {
            type_param.bounds.push(syn::parse_quote!(mlua::IntoLua));
        }
    }
    generics
}
