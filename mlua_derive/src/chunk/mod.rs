use std::ops::Deref;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{ToTokens, quote};

use self::token::{Pos, Token, Tokens};

mod token;

#[derive(Debug, Clone)]
pub(crate) struct Capture(Token);

impl Deref for Capture {
    type Target = Token;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Capture {
    fn new(token: &Token) -> Self {
        Self(token.clone())
    }

    pub(crate) fn name(&self) -> String {
        self.0.to_string()
    }
}

impl ToTokens for Capture {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        let ts: TokenStream = self.0.tree().clone().into();
        tokens.extend(TokenStream2::from(ts));
    }
}

#[derive(Debug)]
pub(crate) struct Captures(Vec<Capture>);

impl Captures {
    pub(crate) fn new() -> Self {
        Self(Vec::new())
    }

    pub(crate) fn add(&mut self, token: &Token) {
        if self.0.iter().any(|arg| &**arg == token) {
            return;
        }
        self.0.push(Capture::new(token));
    }

    pub(crate) fn captures(&self) -> &[Capture] {
        &self.0
    }
}

#[derive(Debug)]
pub(crate) struct Chunk {
    source: String,
    caps: Captures,
}

impl Chunk {
    pub(crate) fn new(tokens: TokenStream) -> Result<Self, TokenStream2> {
        let tokens = Tokens::retokenize(tokens)?;

        let mut source = String::new();
        let mut caps = Captures::new();

        let mut prev_end: Option<Pos> = None;
        for t in tokens {
            if t.is_cap() {
                caps.add(&t);
            }

            let (line, col) = (t.start().line, t.start().column);
            if let Some(prev) = prev_end {
                if line > prev.line {
                    source.push('\n');
                    source.push_str(&" ".repeat(col.saturating_sub(1)));
                } else if line == prev.line {
                    source.push_str(&" ".repeat(col.saturating_sub(prev.column)));
                }
            } else {
                source.push_str(&" ".repeat(col.saturating_sub(1)));
            }
            source.push_str(&t.to_string());

            prev_end = Some(t.end());
        }

        let source = source.trim_end().to_string();
        Ok(Self { source, caps })
    }

    pub(crate) fn captures(&self) -> &[Capture] {
        self.caps.captures()
    }

    pub(crate) fn expand(&self) -> TokenStream2 {
        let source = &self.source;

        let caps_len = self.captures().len();
        let caps = self.captures().iter().map(|cap| {
            let cap_name = cap.name();
            quote! { env.raw_set(#cap_name, #cap)?; }
        });

        quote! {{
            use mlua::chunk::{AsChunk, ChunkMode};
            use mlua::{Lua, Result, Table};
            use ::std::borrow::Cow;
            use ::std::cell::Cell;
            use ::std::io::Result as IoResult;

            struct InnerChunk<F: FnOnce(&Lua) -> Result<Table>>(Cell<Option<F>>);

            impl<F> AsChunk for InnerChunk<F>
            where
                F: FnOnce(&Lua) -> Result<Table>,
            {
                fn environment(&self, lua: &Lua) -> Result<Option<Table>> {
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

                fn source<'a>(&self) -> IoResult<Cow<'a, [u8]>> {
                    Ok(Cow::Borrowed((#source).as_bytes()))
                }
            }

            let make_env = move |lua: &Lua| -> Result<Table> {
                let globals = lua.globals();
                let env = lua.create_table()?;
                let meta = lua.create_table()?;
                meta.raw_set("__index", &globals)?;
                meta.raw_set("__newindex", &globals)?;

                // Add captured variables
                #(#caps)*

                env.set_metatable(Some(meta))?;
                Ok(env)
            };

            InnerChunk(Cell::new(Some(make_env)))
        }}
    }
}
