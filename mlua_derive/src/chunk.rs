use std::ops::Deref;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::ToTokens;

use crate::token::{Pos, Token, Tokens};

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
    pub(crate) fn new(tokens: TokenStream) -> Self {
        let tokens = Tokens::retokenize(tokens);

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

        Self {
            source: source.trim_end().to_string(),
            caps,
        }
    }

    pub(crate) fn source(&self) -> &str {
        &self.source
    }

    pub(crate) fn captures(&self) -> &[Capture] {
        self.caps.captures()
    }
}
