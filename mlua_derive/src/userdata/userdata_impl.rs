use std::sync::atomic::{AtomicUsize, Ordering};

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{
    Attribute, FnArg, Ident, ImplItem, ItemImpl, Meta, Signature, Type, parse_macro_input, parse_quote,
};

use super::attr::LuaAttr;
use super::with_cfg;

/// `&T` reference types that mlua provides as wrapper types via `FromLua`.
static BORROW_WRAPPERS: &[(&str, &str)] = &[
    ("str", "::mlua::string::BorrowedStr"),
    ("[u8]", "::mlua::string::BorrowedBytes"),
];

enum SelfKind {
    Ref(RefKind),
    Owned,
    None,
}

enum RefKind {
    Ref,
    Mut,
}

struct ArgInfo {
    ident: Ident,
    userdata_ref: Option<RefKind>,
    callback_type: Type,
}

struct MethodInfo {
    self_kind: SelfKind,
    has_lua: bool,
    args: Vec<ArgInfo>,
}

/// Extract the inner type from a reference type.
fn ref_inner_type(ty: &Type) -> Type {
    match ty {
        Type::Reference(ref_ty) => (*ref_ty.elem).clone(),
        _ => ty.clone(),
    }
}

/// Check if the type is `&Lua` or `&mlua::Lua`.
fn is_lua_ref(ty: &Type) -> bool {
    let Type::Reference(ref_ty) = ty else { return false };
    match &*ref_ty.elem {
        Type::Path(p) if p.path.segments.len() == 1 => p.path.segments[0].ident == "Lua",
        Type::Path(p) if p.path.segments.len() == 2 => {
            p.path.segments[0].ident == "mlua" && p.path.segments[1].ident == "Lua"
        }
        _ => false,
    }
}

/// Classify a `&[mut] T` parameter, returning the callback wrapper type.
///
/// Known borrow types come from the mapping table `BORROW_WRAPPERS`.
/// Everything else gets `UserDataRef[Mut]<T>`.
fn classify_ref_type(ty: &Type) -> Option<Type> {
    let Type::Reference(ref_ty) = ty else { return None };

    // Check known borrow wrappers:
    // - For `&T` check the path name
    // - For `&[T]` unpack the slice and format the element as `[T]` for lookup
    if ref_ty.mutability.is_none() {
        let lookup_name: Option<String> = match &*ref_ty.elem {
            Type::Path(path) => path.path.segments.last().map(|seg| seg.ident.to_string()),
            Type::Slice(slice) => {
                if let Type::Path(path) = &*slice.elem {
                    path.path.segments.last().map(|seg| format!("[{}]", seg.ident))
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(ref name) = lookup_name {
            for &(inner, wrapper) in BORROW_WRAPPERS {
                if name == inner {
                    let wrapper = syn::parse_str(wrapper).expect("invalid wrapper type");
                    return Some(wrapper);
                }
            }
        }
    }
    // Mutable references to slices are not supported.
    if matches!(&*ref_ty.elem, Type::Slice(_)) && ref_ty.mutability.is_some() {
        return None;
    }

    let inner = ref_inner_type(ty);
    if ref_ty.mutability.is_none() {
        Some(parse_quote! { ::mlua::userdata::UserDataRef<#inner> })
    } else {
        Some(parse_quote! { ::mlua::userdata::UserDataRefMut<#inner> })
    }
}

/// Analyze method signature.
///
/// Determine `self` kind and collect the callback arguments.
/// Auto-detects `&Lua` as the first non-self parameter.
fn analyze_self_and_args(sig: &Signature) -> syn::Result<MethodInfo> {
    let mut self_kind = SelfKind::None;
    let mut has_lua = false;
    let mut args = Vec::new();
    let mut check_first_typed = true;

    for param in &sig.inputs {
        match param {
            FnArg::Receiver(recv) if recv.reference.is_some() && recv.mutability.is_some() => {
                self_kind = SelfKind::Ref(RefKind::Mut);
            }
            FnArg::Receiver(recv) if recv.reference.is_some() => {
                self_kind = SelfKind::Ref(RefKind::Ref);
            }
            FnArg::Receiver(_) => {
                self_kind = SelfKind::Owned;
            }
            FnArg::Typed(typed) => {
                if check_first_typed && is_lua_ref(&typed.ty) {
                    has_lua = true;
                    check_first_typed = false;
                    continue;
                }
                check_first_typed = false;
                if let syn::Pat::Ident(pat_ident) = &*typed.pat {
                    let arg_type = &*typed.ty;
                    let ref_kind = match arg_type {
                        Type::Reference(r) if r.mutability.is_some() => Some(RefKind::Mut),
                        Type::Reference(_) => Some(RefKind::Ref),
                        _ => None,
                    };
                    let callback_type = match &ref_kind {
                        Some(_) => match classify_ref_type(arg_type) {
                            Some(ty) => ty,
                            None => {
                                return Err(syn::Error::new_spanned(
                                    arg_type,
                                    "this reference type is not supported as a callback parameter",
                                ));
                            }
                        },
                        None => arg_type.clone(),
                    };
                    args.push(ArgInfo {
                        ident: pat_ident.ident.clone(),
                        userdata_ref: ref_kind,
                        callback_type,
                    });
                }
            }
        }
    }

    Ok(MethodInfo {
        self_kind,
        has_lua,
        args,
    })
}

fn strip_item_attrs(attrs: &[Attribute]) -> Vec<Attribute> {
    (attrs.iter())
        .filter(|attr| !attr.path().is_ident("lua"))
        .cloned()
        .collect()
}

fn parse_lua_attr(attrs: &[Attribute]) -> syn::Result<LuaAttr> {
    let mut lua_attr = LuaAttr::default();
    for attr in attrs {
        if !attr.path().is_ident("lua") {
            continue;
        }
        match &attr.meta {
            Meta::List(_) => {
                attr.parse_nested_meta(|meta| lua_attr.parse_inner(meta))?;
                validate_lua_attr(&lua_attr, attr.span())?;
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

fn validate_lua_attr(attr: &LuaAttr, span: Span) -> syn::Result<()> {
    for (set, name) in [(attr.get, "get"), (attr.set, "set")] {
        if set {
            return Err(syn::Error::new(
                span,
                format!("`{name}` is not valid for methods"),
            ));
        }
    }
    Ok(())
}

pub fn userdata_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return syn::Error::new_spanned(
            proc_macro2::TokenStream::from(attr),
            "`#[userdata_impl]` does not accept arguments",
        )
        .to_compile_error()
        .into();
    }

    let mut input = parse_macro_input!(item as ItemImpl);

    let type_path = match &*input.self_ty {
        Type::Path(type_path) => &type_path.path,
        _ => {
            return syn::Error::new_spanned(&input.self_ty, "`#[userdata_impl]` requires a simple path type")
                .to_compile_error()
                .into();
        }
    };
    let type_name = (type_path.segments)
        .last()
        .map(|seg| seg.ident.clone())
        .ok_or_else(|| syn::Error::new_spanned(&input.self_ty, "cannot determine type name"));
    let type_name = try_compile!(type_name);

    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let unique_suffix = COUNTER.fetch_add(1, Ordering::Relaxed);
    let register_fn_name = format_ident!("__mlua_register_{type_name}_{unique_suffix}");
    let registration_type_name = format_ident!("__MluaUserDataRegistration_{type_name}");

    let mut registration_calls = Vec::new();
    for item in &input.items {
        match item {
            ImplItem::Const(const_item) => {
                let lua_attr = try_compile!(parse_lua_attr(&const_item.attrs));
                if lua_attr.skip {
                    continue;
                }
                if lua_attr.getter || lua_attr.setter {
                    return syn::Error::new_spanned(
                        &const_item.ident,
                        "const items do not support `getter` or `setter`",
                    )
                    .to_compile_error()
                    .into();
                }
                let const_name = &const_item.ident;
                let lua_name = lua_attr.name(const_name);
                if lua_attr.meta {
                    let tokens = quote! {
                        registry.add_meta_field(#lua_name, #type_path::#const_name);
                    };
                    registration_calls.push(with_cfg(tokens, &const_item.attrs));
                } else {
                    let tokens = quote! {
                        registry.add_field(#lua_name, #type_path::#const_name);
                    };
                    registration_calls.push(with_cfg(tokens, &const_item.attrs));
                }
            }
            ImplItem::Fn(method) => {
                let lua_attr = try_compile!(parse_lua_attr(&method.attrs));
                if lua_attr.skip {
                    continue;
                }

                // Validate mutually exclusive role flags.
                // `getter`, `setter`, `field` are exclusive.
                // `meta` on its own means a metamethod.
                // `meta` combined with `field` means a meta static field.
                // `meta` with `getter` or `setter` is invalid.
                let primary = [lua_attr.getter, lua_attr.setter, lua_attr.field];
                let primary_count = primary.iter().filter(|&&x| x).count();
                if primary_count > 1 {
                    return syn::Error::new_spanned(
                        &method.sig,
                        "at most one of `getter`, `setter`, `field` can be specified",
                    )
                    .to_compile_error()
                    .into();
                }
                if lua_attr.meta && primary_count == 1 && !lua_attr.field {
                    return syn::Error::new_spanned(&method.sig, "`meta` can only be combined with `field`")
                        .to_compile_error()
                        .into();
                }

                let fn_name = &method.sig.ident;
                let info = try_compile!(analyze_self_and_args(&method.sig));
                let is_async = method.sig.asyncness.is_some();

                if lua_attr.getter {
                    if is_async {
                        return syn::Error::new_spanned(&method.sig, "async field getter is not supported")
                            .to_compile_error()
                            .into();
                    }
                    if !matches!(info.self_kind, SelfKind::Ref(RefKind::Ref)) {
                        return syn::Error::new_spanned(&method.sig, "field getter must take `&self`")
                            .to_compile_error()
                            .into();
                    }
                    if !info.args.is_empty() {
                        return syn::Error::new_spanned(
                            &method.sig,
                            "field getter must not take additional arguments",
                        )
                        .to_compile_error()
                        .into();
                    }
                    let tokens = gen_field_getter(type_path, fn_name, &lua_attr, &info);
                    registration_calls.push(with_cfg(tokens, &method.attrs));
                    continue;
                }
                if lua_attr.setter {
                    if is_async {
                        return syn::Error::new_spanned(&method.sig, "async field setter is not supported")
                            .to_compile_error()
                            .into();
                    }
                    if !matches!(info.self_kind, SelfKind::Ref(_)) {
                        return syn::Error::new_spanned(&method.sig, "field setter must take `&[mut] self`")
                            .to_compile_error()
                            .into();
                    }
                    if info.args.len() != 1 {
                        return syn::Error::new_spanned(
                            &method.sig,
                            "field setter must take exactly one value argument",
                        )
                        .to_compile_error()
                        .into();
                    }
                    let tokens = gen_field_setter(type_path, fn_name, &lua_attr, &info);
                    registration_calls.push(with_cfg(tokens, &method.attrs));
                    continue;
                }
                if lua_attr.field {
                    if is_async {
                        return syn::Error::new_spanned(&method.sig, "async field function is not supported")
                            .to_compile_error()
                            .into();
                    }
                    if !matches!(info.self_kind, SelfKind::None) {
                        return syn::Error::new_spanned(&method.sig, "field function must not take `self`")
                            .to_compile_error()
                            .into();
                    }
                    if !info.args.is_empty() {
                        return syn::Error::new_spanned(
                            &method.sig,
                            "field function must not take arguments",
                        )
                        .to_compile_error()
                        .into();
                    }
                    let lua_name = lua_attr.name(fn_name);
                    if lua_attr.meta {
                        let tokens = quote! {
                            registry.add_meta_field(#lua_name, #type_path::#fn_name());
                        };
                        registration_calls.push(with_cfg(tokens, &method.attrs));
                    } else {
                        let tokens = quote! {
                            registry.add_field(#lua_name, #type_path::#fn_name());
                        };
                        registration_calls.push(with_cfg(tokens, &method.attrs));
                    }
                    continue;
                }

                if lua_attr.meta {
                    if matches!(info.self_kind, SelfKind::Owned) {
                        return syn::Error::new_spanned(
                            &method.sig,
                            "meta methods cannot take `self`, use `&[mut] self` instead",
                        )
                        .to_compile_error()
                        .into();
                    }
                    if is_async {
                        let tokens = gen_async_meta(type_path, fn_name, &lua_attr, &info);
                        registration_calls.push(with_cfg(tokens, &method.attrs));
                    } else {
                        let tokens = gen_meta(type_path, fn_name, &lua_attr, &info);
                        registration_calls.push(with_cfg(tokens, &method.attrs));
                    }
                    continue;
                }

                if is_async {
                    let tokens = gen_async_regular_method(type_path, fn_name, &lua_attr, &info);
                    registration_calls.push(with_cfg(tokens, &method.attrs));
                } else {
                    let tokens = gen_regular_method(type_path, fn_name, &lua_attr, &info);
                    registration_calls.push(with_cfg(tokens, &method.attrs));
                }
            }
            _ => {}
        }
    }

    for item in &mut input.items {
        match item {
            ImplItem::Const(c) => c.attrs = strip_item_attrs(&c.attrs),
            ImplItem::Fn(m) => m.attrs = strip_item_attrs(&m.attrs),
            _ => {}
        }
    }
    input.attrs = strip_item_attrs(&input.attrs);

    let output = quote! {
        #[allow(non_snake_case)]
        fn #register_fn_name(registry: &mut ::mlua::userdata::UserDataRegistry<#type_path>) {
            use ::mlua::userdata::{UserDataFields as _, UserDataMethods as _};
            #(#registration_calls)*
        }

        ::mlua::__inventory::submit! {
            #registration_type_name { register: #register_fn_name }
        }

        #input
    };

    output.into()
}

/// Generate the closure argument destructuring pattern.
fn gen_closure_destructure(info: &MethodInfo) -> TokenStream2 {
    if info.args.is_empty() {
        return quote! { () };
    }
    let idents: Vec<_> = (info.args)
        .iter()
        .map(|a| {
            let ident = &a.ident;
            if matches!(a.userdata_ref, Some(RefKind::Mut)) {
                quote! { mut #ident }
            } else {
                quote! { #ident }
            }
        })
        .collect();
    let types: Vec<_> = info.args.iter().map(|a| &a.callback_type).collect();
    quote! { (#(#idents),*): (#(#types),*) }
}

/// Generate call arguments for invoking the original method.
fn gen_call_args(info: &MethodInfo) -> TokenStream2 {
    let mut call_args: Vec<TokenStream2> = Vec::new();

    match info.self_kind {
        SelfKind::None => {}
        _ => call_args.push(quote! { this }),
    }

    if info.has_lua {
        call_args.push(quote! { lua });
    }

    for arg in &info.args {
        let ident = &arg.ident;
        match arg.userdata_ref {
            Some(RefKind::Ref) => call_args.push(quote! { &*#ident }),
            Some(RefKind::Mut) => call_args.push(quote! { &mut *#ident }),
            None => call_args.push(quote! { #ident }),
        }
    }

    quote! { #(#call_args),* }
}

/// Generate call arguments for invoking the original async method.
fn gen_async_call_args(info: &MethodInfo) -> TokenStream2 {
    let mut call_args: Vec<TokenStream2> = Vec::new();

    match info.self_kind {
        SelfKind::None => {}
        SelfKind::Ref(RefKind::Ref) => call_args.push(quote! { &this }),
        SelfKind::Ref(RefKind::Mut) => call_args.push(quote! { &mut this }),
        SelfKind::Owned => call_args.push(quote! { this }),
    }

    if info.has_lua {
        call_args.push(quote! { lua });
    }

    for arg in &info.args {
        let ident = &arg.ident;
        match arg.userdata_ref {
            Some(RefKind::Ref) => call_args.push(quote! { &*#ident }),
            Some(RefKind::Mut) => call_args.push(quote! { &mut *#ident }),
            None => call_args.push(quote! { #ident }),
        }
    }

    quote! { #(#call_args),* }
}

/// Generate the closure params for the registration callback.
fn gen_closure_params(info: &MethodInfo) -> TokenStream2 {
    let destructure = gen_closure_destructure(info);
    match info.self_kind {
        SelfKind::None => quote! { |lua, #destructure| },
        _ => quote! { |lua, this, #destructure| },
    }
}

/// Generate the closure params for an async registration callback.
fn gen_async_closure_params(info: &MethodInfo) -> TokenStream2 {
    let destructure = gen_closure_destructure(info);
    match info.self_kind {
        SelfKind::None => quote! { |lua, #destructure| },
        SelfKind::Ref(RefKind::Mut) => quote! { |lua, mut this, #destructure| },
        _ => quote! { |lua, this, #destructure| },
    }
}

fn gen_field_getter(
    type_path: &syn::Path,
    fn_name: &Ident,
    lua_attr: &LuaAttr,
    info: &MethodInfo,
) -> TokenStream2 {
    let lua_name = lua_attr.name(&fn_name);
    let call_args = gen_call_args(info);

    if lua_attr.infallible {
        return quote! {
            registry.add_field_method_get(#lua_name, |lua, this| {
                let _ = lua; // silence unused variable warning
                Ok(#type_path::#fn_name(#call_args))
            });
        };
    }

    quote! {
        registry.add_field_method_get(#lua_name, |lua, this| {
            let _ = lua; // silence unused variable warning
            #type_path::#fn_name(#call_args)
        });
    }
}

fn gen_field_setter(
    type_path: &syn::Path,
    fn_name: &Ident,
    lua_attr: &LuaAttr,
    info: &MethodInfo,
) -> TokenStream2 {
    let lua_name = lua_attr.name(fn_name);
    let call_args = gen_call_args(info);

    if lua_attr.infallible {
        let val_ident = info.args.first().map(|a| &a.ident);
        return quote! {
            registry.add_field_method_set(#lua_name, |lua, this, #val_ident| {
                let _ = lua; // silence unused variable warning
                Ok(#type_path::#fn_name(#call_args))
            });
        };
    }

    let val_ident = info.args.first().map(|a| &a.ident);
    quote! {
        registry.add_field_method_set(#lua_name, |lua, this, #val_ident| {
            let _ = lua; // silence unused variable warning
            #type_path::#fn_name(#call_args)
        });
    }
}

fn gen_meta(type_path: &syn::Path, fn_name: &Ident, lua_attr: &LuaAttr, info: &MethodInfo) -> TokenStream2 {
    let meta_name = match lua_attr.effective_meta_name(fn_name) {
        Ok(name) => name,
        Err(err) => return err.to_compile_error(),
    };
    let closure_params = if matches!(info.self_kind, SelfKind::None) {
        // Lua always passes `self` to the stack arg, just ignore it.
        if info.args.is_empty() {
            quote! { |lua, _this: ::mlua::AnyUserData| }
        } else {
            let idents: Vec<_> = info.args.iter().map(|a| &a.ident).collect();
            let types: Vec<_> = info.args.iter().map(|a| &a.callback_type).collect();
            quote! { |lua, (_this, #(#idents),*): (::mlua::AnyUserData, #(#types),*) | }
        }
    } else {
        gen_closure_params(info)
    };
    let call_args = gen_call_args(info);
    let fn_path = quote! { #type_path::#fn_name };

    let body = if lua_attr.infallible {
        quote! { Ok(#fn_path(#call_args)) }
    } else {
        quote! { #fn_path(#call_args) }
    };
    match info.self_kind {
        SelfKind::None => quote! {
            registry.add_meta_function(#meta_name, #closure_params { #body });
        },
        SelfKind::Ref(RefKind::Mut) => quote! {
            registry.add_meta_method_mut(#meta_name, #closure_params { #body });
        },
        _ => quote! {
            registry.add_meta_method(#meta_name, #closure_params { #body });
        },
    }
}

fn gen_regular_method(
    type_path: &syn::Path,
    fn_name: &Ident,
    lua_attr: &LuaAttr,
    info: &MethodInfo,
) -> TokenStream2 {
    let fn_path = quote! { #type_path::#fn_name };
    let closure_params = gen_closure_params(info);
    let call_args = gen_call_args(info);
    let lua_name = lua_attr.name(fn_name);

    let body = if lua_attr.infallible {
        quote! { Ok(#fn_path(#call_args)) }
    } else {
        quote! { #fn_path(#call_args) }
    };
    match info.self_kind {
        SelfKind::Ref(RefKind::Ref) => quote! {
            registry.add_method(#lua_name, #closure_params { #body });
        },
        SelfKind::Ref(RefKind::Mut) => quote! {
            registry.add_method_mut(#lua_name, #closure_params { #body });
        },
        SelfKind::Owned => quote! {
            registry.add_method_once(#lua_name, #closure_params { #body });
        },
        SelfKind::None => quote! {
            registry.add_function(#lua_name, #closure_params { #body });
        },
    }
}

fn gen_async_regular_method(
    type_path: &syn::Path,
    fn_name: &Ident,
    lua_attr: &LuaAttr,
    info: &MethodInfo,
) -> TokenStream2 {
    let fn_path = quote! { #type_path::#fn_name };
    let closure_params = gen_async_closure_params(info);
    let call_args = gen_async_call_args(info);
    let lua_name = lua_attr.name(fn_name);

    let body = if lua_attr.infallible {
        quote! { async move { Ok(#fn_path(#call_args).await) } }
    } else {
        quote! { async move { #fn_path(#call_args).await } }
    };
    match info.self_kind {
        SelfKind::Ref(RefKind::Ref) => quote! {
            registry.add_async_method(#lua_name, #closure_params #body);
        },
        SelfKind::Ref(RefKind::Mut) => quote! {
            registry.add_async_method_mut(#lua_name, #closure_params #body);
        },
        SelfKind::Owned => quote! {
            registry.add_async_method_once(#lua_name, #closure_params #body);
        },
        SelfKind::None => quote! {
            registry.add_async_function(#lua_name, #closure_params #body);
        },
    }
}

fn gen_async_meta(
    type_path: &syn::Path,
    fn_name: &Ident,
    lua_attr: &LuaAttr,
    info: &MethodInfo,
) -> TokenStream2 {
    let meta_name = match lua_attr.effective_meta_name(fn_name) {
        Ok(name) => name,
        Err(err) => return err.to_compile_error(),
    };
    let closure_params = if matches!(info.self_kind, SelfKind::None) {
        if info.args.is_empty() {
            quote! { |lua, _this: ::mlua::AnyUserData| }
        } else {
            let idents: Vec<_> = info.args.iter().map(|a| &a.ident).collect();
            let types: Vec<_> = info.args.iter().map(|a| &a.callback_type).collect();
            quote! { |lua, (_this, #(#idents),*): (::mlua::AnyUserData, #(#types),*) | }
        }
    } else {
        gen_async_closure_params(info)
    };
    let call_args = gen_async_call_args(info);
    let fn_path = quote! { #type_path::#fn_name };

    let body = if lua_attr.infallible {
        quote! { async move { Ok(#fn_path(#call_args).await) } }
    } else {
        quote! { async move { #fn_path(#call_args).await } }
    };
    match info.self_kind {
        SelfKind::None => quote! {
            registry.add_async_meta_function(#meta_name, #closure_params #body);
        },
        SelfKind::Ref(RefKind::Mut) => quote! {
            registry.add_async_meta_method_mut(#meta_name, #closure_params #body);
        },
        _ => quote! {
            registry.add_async_meta_method(#meta_name, #closure_params #body);
        },
    }
}
