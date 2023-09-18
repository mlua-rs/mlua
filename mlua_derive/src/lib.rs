use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::quote;
use syn::meta::ParseNestedMeta;
use syn::{parse_macro_input, ItemFn, LitStr, Result};

#[cfg(feature = "macros")]
use {
    crate::chunk::Chunk, proc_macro::TokenTree, proc_macro2::TokenStream as TokenStream2,
    proc_macro_error::proc_macro_error,
};

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

#[proc_macro_attribute]
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
    let skip_memory_check = args.skip_memory_check;

    let wrapped = quote! {
        ::mlua::require_module_feature!();

        #func

        #[no_mangle]
        unsafe extern "C-unwind" fn #ext_entrypoint_name(state: *mut ::mlua::lua_State) -> ::std::ffi::c_int {
            let lua = ::mlua::Lua::init_from_ptr(state);
            lua.skip_memory_check(#skip_memory_check);
            lua.entrypoint1(#func_name)
        }
    };

    wrapped.into()
}

#[cfg(feature = "macros")]
fn to_ident(tt: &TokenTree) -> TokenStream2 {
    let s: TokenStream = tt.clone().into();
    s.into()
}

#[cfg(feature = "macros")]
#[proc_macro]
#[proc_macro_error]
pub fn chunk(input: TokenStream) -> TokenStream {
    let chunk = Chunk::new(input);

    let source = chunk.source();

    let caps_len = chunk.captures().len();
    let caps = chunk.captures().iter().map(|cap| {
        let cap_name = cap.as_rust().to_string();
        let cap = to_ident(cap.as_rust());
        quote! { env.raw_set(#cap_name, #cap)?; }
    });

    let wrapped_code = quote! {{
        use ::mlua::{AsChunk, ChunkMode, Lua, Result, Table};
        use ::std::borrow::Cow;
        use ::std::cell::Cell;
        use ::std::io::Result as IoResult;
        use ::std::marker::PhantomData;

        struct InnerChunk<'lua, F: FnOnce(&'lua Lua) -> Result<Table<'lua>>>(Cell<Option<F>>, PhantomData<&'lua ()>);

        impl<'lua, F> AsChunk<'lua, 'static> for InnerChunk<'lua, F>
        where
            F: FnOnce(&'lua Lua) -> Result<Table<'lua>>,
        {
            fn environment(&self, lua: &'lua Lua) -> Result<Option<Table<'lua>>> {
                if #caps_len > 0 {
                    if let Some(make_env) = self.0.take() {
                        return make_env(lua).map(Some);
                    }
                }
                Ok(None)
            }

            fn mode(&self) -> Option<ChunkMode> {
                Some(ChunkMode::Text)
            }

            fn source(self) -> IoResult<Cow<'static, [u8]>> {
                Ok(Cow::Borrowed((#source).as_bytes()))
            }
        }

        fn annotate<'a, F: FnOnce(&'a Lua) -> Result<Table<'a>>>(f: F) -> F { f }

        let make_env = annotate(move |lua: &Lua| -> Result<Table> {
            let globals = lua.globals();
            let env = lua.create_table()?;
            let meta = lua.create_table()?;
            meta.raw_set("__index", globals.clone())?;
            meta.raw_set("__newindex", globals)?;

            // Add captured variables
            #(#caps)*

            env.set_metatable(Some(meta));
            Ok(env)
        });

        InnerChunk(Cell::new(Some(make_env)), PhantomData)
    }};

    wrapped_code.into()
}

#[cfg(feature = "macros")]
#[proc_macro_derive(FromLua)]
pub fn from_lua(input: TokenStream) -> TokenStream {
    from_lua::from_lua(input)
}

#[cfg(feature = "macros")]
mod chunk;
#[cfg(feature = "macros")]
mod from_lua;
#[cfg(feature = "macros")]
mod token;
