mod attr;
pub(crate) mod userdata_impl;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{Attribute, Data, DeriveInput, Error, Fields, FieldsNamed, Meta, parse_macro_input};

use self::attr::LuaAttr;

/// Wrap registration tokens with any `#[cfg]`/`#[cfg_attr]` attributes from the original item.
pub(crate) fn with_cfg(tokens: proc_macro2::TokenStream, attrs: &[Attribute]) -> proc_macro2::TokenStream {
    let cfgs: Vec<_> = (attrs.iter())
        .filter(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
        .collect();
    if cfgs.is_empty() {
        return tokens;
    }
    quote! {
        #(#cfgs)*
        #tokens
    }
}

/// Parse all `#[lua(...)]` attributes on a field, merging them into one `LuaAttr`.
fn parse_field_lua_attr(attrs: &[Attribute]) -> syn::Result<LuaAttr> {
    let mut lua_attr = LuaAttr::default();
    for attr in attrs {
        if !attr.path().is_ident("lua") {
            continue;
        }
        match &attr.meta {
            Meta::List(_) => {
                lua_attr.span = Some(attr.span());
                attr.parse_nested_meta(|meta| lua_attr.parse_inner(meta))?;
                validate_field_lua_attr(&lua_attr)?;
            }
            Meta::Path(_) => {}
            Meta::NameValue(_) => {
                return Err(syn::Error::new_spanned(
                    attr,
                    "`#[lua = \"...\"]` is not supported: use `#[lua(attr = \"...\")]`",
                ));
            }
        }
    }
    Ok(lua_attr)
}

fn validate_field_lua_attr(attr: &LuaAttr) -> syn::Result<()> {
    for (set, name) in [
        (attr.getter, "getter"),
        (attr.setter, "setter"),
        (attr.field, "field"),
        (attr.meta, "meta"),
        (attr.infallible, "infallible"),
    ] {
        if set {
            return Err(syn::Error::new(
                attr.span(),
                format!("`{name}` is not valid for struct fields"),
            ));
        }
    }
    Ok(())
}

pub fn userdata_type(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    let type_name = &input.ident;

    let named_fields: Option<&FieldsNamed> = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => Some(fields),
            Fields::Unnamed(_) | Fields::Unit => None,
        },
        Data::Enum(_) => None,
        Data::Union(_) => {
            return Error::new_spanned(&input, "`#[derive(UserData)]` cannot be applied to unions")
                .to_compile_error()
                .into();
        }
    };

    // Check for generic parameters (not supported)
    let has_generics = !input.generics.params.is_empty();
    if has_generics {
        return Error::new_spanned(
            &input.generics,
            "`#[derive(UserData)]` does not support generic type parameters. Wrap the generic type in a concrete newtype instead."
        )
        .to_compile_error()
        .into();
    }

    let mut field_registrations = Vec::new();
    if let Some(fields) = &named_fields {
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
                let tokens = quote! {
                    registry.add_field_method_get(#lua_name, |_lua, this| Ok(this.#field_name.clone()));
                };
                field_registrations.push(with_cfg(tokens, &field.attrs));
            }
            if has_set {
                let tokens = quote! {
                    registry.add_field_method_set(#lua_name, |_lua, this, val| {
                        this.#field_name = val;
                        Ok(())
                    });
                };
                field_registrations.push(with_cfg(tokens, &field.attrs));
            }
        }
    }

    let registration_type_name = format_ident!("__MluaUserDataRegistration_{type_name}");
    let register_fields_fn_name = format_ident!("__mlua_register_{type_name}_fields");

    let output = quote! {
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
