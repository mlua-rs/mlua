use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::quote;
use syn::{parse_macro_input, AttributeArgs, Error, ItemFn};

#[cfg(feature = "macros")]
use {
    crate::chunk::Chunk, proc_macro::TokenTree, proc_macro2::TokenStream as TokenStream2,
    proc_macro_error::proc_macro_error,
};

#[proc_macro_attribute]
pub fn lua_module(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as AttributeArgs);
    let func = parse_macro_input!(item as ItemFn);

    if !args.is_empty() {
        let err = Error::new(Span::call_site(), "the macro does not support arguments")
            .to_compile_error();
        return err.into();
    }

    let func_name = func.sig.ident.clone();
    let ext_entrypoint_name = Ident::new(&format!("luaopen_{}", func_name), Span::call_site());

    let wrapped = quote! {
        ::mlua::require_module_feature!();

        #func

        #[no_mangle]
        unsafe extern "C" fn #ext_entrypoint_name(state: *mut ::mlua::lua_State) -> ::std::os::raw::c_int {
            ::mlua::Lua::init_from_ptr(state)
                .entrypoint1(#func_name)
                .expect("cannot initialize module")
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
        use ::mlua::{AsChunk, ChunkMode, Lua, Result, Value};
        use ::std::borrow::Cow;
        use ::std::io::Result as IoResult;
        use ::std::marker::PhantomData;
        use ::std::sync::Mutex;

        fn annotate<'a, F: FnOnce(&'a Lua) -> Result<Value<'a>>>(f: F) -> F { f }

        struct InnerChunk<'a, F: FnOnce(&'a Lua) -> Result<Value<'a>>>(Mutex<Option<F>>, PhantomData<&'a ()>);

        impl<'lua, F> AsChunk<'lua> for InnerChunk<'lua, F>
        where
            F: FnOnce(&'lua Lua) -> Result<Value<'lua>>,
        {
            fn source(&self) -> IoResult<Cow<[u8]>> {
                Ok(Cow::Borrowed((#source).as_bytes()))
            }

            fn env(&self, lua: &'lua Lua) -> Result<Option<Value<'lua>>> {
                if #caps_len > 0 {
                    if let Ok(mut make_env) = self.0.lock() {
                        if let Some(make_env) = make_env.take() {
                            return make_env(lua).map(Some);
                        }
                    }
                }
                Ok(None)
            }

            fn mode(&self) -> Option<ChunkMode> {
                Some(ChunkMode::Text)
            }
        }

        let make_env = annotate(move |lua: &Lua| -> Result<Value> {
            let globals = lua.globals();
            let env = lua.create_table()?;
            let meta = lua.create_table()?;
            meta.raw_set("__index", globals.clone())?;
            meta.raw_set("__newindex", globals)?;

            // Add captured variables
            #(#caps)*

            env.set_metatable(Some(meta));
            Ok(Value::Table(env))
        });

        &InnerChunk(Mutex::new(Some(make_env)), PhantomData)
    }};

    wrapped_code.into()
}

#[cfg(feature = "macros")]
mod chunk;
#[cfg(feature = "macros")]
mod token;
