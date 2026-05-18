use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::quote;
use syn::meta::ParseNestedMeta;
use syn::{ItemFn, LitStr, Result, parse_macro_input};

#[derive(Default)]
struct ModuleAttributes {
    name: Option<Ident>,
    skip_memory_check: bool,
}

impl ModuleAttributes {
    fn parse(&mut self, meta: ParseNestedMeta) -> Result<()> {
        if meta.path.is_ident("name") {
            match meta.value() {
                Ok(value) => {
                    self.name = Some(value.parse::<LitStr>()?.parse()?);
                }
                Err(_) => {
                    return Err(meta.error("`name` attribute must have a value"));
                }
            }
        } else if meta.path.is_ident("skip_memory_check") {
            if meta.value().is_ok() {
                return Err(meta.error("`skip_memory_check` attribute have no values"));
            }
            self.skip_memory_check = true;
        } else {
            return Err(meta.error("unsupported module attribute"));
        }
        Ok(())
    }
}

pub fn lua_module(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut args = ModuleAttributes::default();
    if !attr.is_empty() {
        let args_parser = syn::meta::parser(|meta| args.parse(meta));
        parse_macro_input!(attr with args_parser);
    }

    let func = parse_macro_input!(item as ItemFn);
    let func_name = &func.sig.ident;
    let module_name = args.name.unwrap_or_else(|| func_name.clone());
    let ext_entrypoint_name = Ident::new(&format!("luaopen_{module_name}"), Span::call_site());
    let skip_memory_check = if args.skip_memory_check {
        quote! { lua.skip_memory_check(true); }
    } else {
        quote! {}
    };

    let wrapped = quote! {
        mlua::require_module_feature!();

        #func

        #[unsafe(no_mangle)]
        unsafe extern "C-unwind" fn #ext_entrypoint_name(state: *mut mlua::lua_State) -> ::std::os::raw::c_int {
            mlua::Lua::entrypoint1(state, move |lua| {
                #skip_memory_check
                #func_name(lua)
            })
        }
    };

    wrapped.into()
}
