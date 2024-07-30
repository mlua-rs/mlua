use proc_macro::TokenStream;
use quote::{quote, format_ident};
use syn::{parse_macro_input, DeriveInput, Data, Fields, Type};
use syn::spanned::Spanned;
use syn::punctuated::Punctuated;
use proc_macro2::TokenStream as TokenStream2;

pub fn to_lua(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let ident = input.ident;

    let fields = if let Data::Struct(data_struct) = input.data {
        match data_struct.fields {
            Fields::Named(fields) => fields,
            _ => panic!("ToLua can only be derived for structs with named fields"),
        }
    } else {
        panic!("ToLua can only be derived for structs");
    };

    let add_field_methods = fields.named.iter().map(|field| {
        let name = &field.ident;
        let name_str = name.as_ref().unwrap().to_string();
        let ty = &field.ty;

        let get_method = if is_copy_type(ty) {
            quote! {
                fields.add_field_method_get(#name_str, |_, this| Ok(this.#name));
            }
        } else {
            quote! {
                fields.add_field_method_get(#name_str, |_, this| Ok(this.#name.clone()));
            }
        };

        let set_method = quote! {
            fields.add_field_method_set(#name_str, |_, this, val| {
                this.#name = val;
                Ok(())
            });
        };

        quote! {
            #get_method
            #set_method
        }
    });

    let gen = quote! {
        impl ::mlua::UserData for #ident {
            fn add_fields<'lua, F: ::mlua::prelude::LuaUserDataFields<'lua, Self>>(fields: &mut F) {
                #(#add_field_methods)*
            }
        }
    };

    gen.into()
}

// I don't know how to determine whether or not something implements copy, so for now everything
// will be cloned that isn't one of these copyable primitives.
fn is_copy_type(ty: &Type) -> bool {
    match ty {
        Type::Path(type_path) => {
            let segments = &type_path.path.segments;
            let segment = segments.last().unwrap();
            match segment.ident.to_string().as_str() {
                "u8" | "u16" | "u32" | "u64" | "u128" |
                "i8" | "i16" | "i32" | "i64" | "i128" |
                "f32" | "f64" |
                "bool" | "char" | "usize" | "isize" => true,
                _ => false,
            }
        }
        _ => false,
    }
}
