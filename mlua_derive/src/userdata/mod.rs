mod attr;
pub(crate) mod userdata_impl;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, Data, DeriveInput, Error, Fields, FieldsNamed, Meta, parse_macro_input};

use self::attr::LuaAttr;

/// Parse all `#[lua(...)]` attributes on a field, merging them into one `LuaAttr`.
fn parse_field_lua_attr(attrs: &[Attribute]) -> syn::Result<LuaAttr> {
    let mut lua_attr = LuaAttr::default();
    for attr in attrs {
        if attr.path().is_ident("lua")
            && let Meta::List(_) = &attr.meta
        {
            attr.parse_nested_meta(|meta| lua_attr.parse_inner(meta))?;
        }
    }
    Ok(lua_attr)
}

/// Strip `#[lua(...)]` attributes from a field, keeping all others.
fn strip_lua_attrs(attrs: &[Attribute]) -> Vec<Attribute> {
    (attrs.iter())
        .filter(|attr| !attr.path().is_ident("lua"))
        .cloned()
        .collect()
}

pub fn userdata_type(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return Error::new_spanned(
            proc_macro2::TokenStream::from(attr),
            "`#[userdata]` does not accept arguments",
        )
        .to_compile_error()
        .into();
    }

    let mut input = parse_macro_input!(item as DeriveInput);
    let type_name = &input.ident;

    let mut named_fields: Option<&mut FieldsNamed> = match &mut input.data {
        Data::Struct(data) => match &mut data.fields {
            Fields::Named(fields) => Some(fields),
            Fields::Unnamed(_) | Fields::Unit => None,
        },
        Data::Enum(_) => None,
        Data::Union(_) => {
            return Error::new_spanned(&input, "`#[userdata]` cannot be applied to unions")
                .to_compile_error()
                .into();
        }
    };

    // Check for generic type parameters (not supported)
    let has_type_params = input.generics.type_params().next().is_some();
    if has_type_params {
        return Error::new_spanned(
            &input.generics,
            "`#[userdata]` does not support generic type parameters. Wrap the generic type in a concrete newtype instead."
        )
        .to_compile_error()
        .into();
    }

    let mut field_registrations = Vec::new();
    if let Some(fields) = &mut named_fields {
        for field in &fields.named {
            let field_name = field.ident.as_ref().unwrap();

            let lua_attr = try_compile!(parse_field_lua_attr(&field.attrs));
            if lua_attr.skip {
                continue;
            }

            let lua_name = lua_attr.name.unwrap_or_else(|| field_name.to_string());

            // Assume get/set by default (unless explicitly specified)
            let (has_get, has_set) = if lua_attr.get || lua_attr.set {
                (lua_attr.get, lua_attr.set)
            } else {
                (true, true)
            };

            if has_get {
                field_registrations.push(quote! {
                    registry.add_field_method_get(#lua_name, |_lua, this| Ok(this.#field_name.clone()));
                });
            }
            if has_set {
                field_registrations.push(quote! {
                    registry.add_field_method_set(#lua_name, |_lua, this, val| {
                        this.#field_name = val;
                        Ok(())
                    });
                });
            }
        }

        // Strip mlua-specific attributes from fields before re-emitting
        for field in &mut fields.named {
            field.attrs = strip_lua_attrs(&field.attrs);
        }
    }

    let registration_type_name = format_ident!("__MluaUserDataRegistration_{type_name}");
    let register_fields_fn_name = format_ident!("__mlua_register_{type_name}_fields");

    let output = quote! {
        #input

        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        struct #registration_type_name {
            register: fn(&mut ::mlua::userdata::UserDataRegistry<#type_name>),
        }

        ::mlua::__inventory::collect!(#registration_type_name);

        #[allow(non_snake_case)]
        fn #register_fields_fn_name(registry: &mut ::mlua::userdata::UserDataRegistry<#type_name>) {
            use ::mlua::userdata::UserDataFields as _;
            #(#field_registrations)*
        }

        ::mlua::__inventory::submit! {
            #registration_type_name { register: #register_fields_fn_name }
        }

        impl ::mlua::userdata::UserData for #type_name {
            fn register(registry: &mut ::mlua::userdata::UserDataRegistry<Self>) {
                for item in ::mlua::__inventory::iter::<#registration_type_name> {
                    (item.register)(registry);
                }
            }
        }
    };

    output.into()
}
