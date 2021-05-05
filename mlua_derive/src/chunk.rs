use proc_macro::{TokenStream, TokenTree};

use crate::token::{Pos, Token, Tokens};

#[derive(Debug, Clone)]
pub(crate) struct Capture {
    key: Token,
    rust: TokenTree,
}

impl Capture {
    fn new(key: Token, rust: TokenTree) -> Self {
        Self { key, rust }
    }

    /// Token string inside `chunk!`
    pub(crate) fn key(&self) -> &Token {
        &self.key
    }

    /// As rust variable, e.g. `x`
    pub(crate) fn as_rust(&self) -> &TokenTree {
        &self.rust
    }
}

#[derive(Debug)]
pub(crate) struct Captures(Vec<Capture>);

impl Captures {
    pub(crate) fn new() -> Self {
        Self(Vec::new())
    }

    pub(crate) fn add(&mut self, token: &Token) -> Capture {
        let tt = token.tree();
        let key = token.clone();

        match self.0.iter().find(|arg| arg.key() == &key) {
            Some(arg) => arg.clone(),
            None => {
                let arg = Capture::new(key, tt.clone());
                self.0.push(arg.clone());
                arg
            }
        }
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

        let mut pos: Option<Pos> = None;
        for t in tokens {
            if t.is_cap() {
                caps.add(&t);
            }

            let (line, col) = (t.start().line, t.start().column);
            let (prev_line, prev_col) = pos
                .take()
                .map(|lc| (lc.line, lc.column))
                .unwrap_or_else(|| (line, col));

            #[allow(clippy::comparison_chain)]
            if line > prev_line {
                source.push('\n');
            } else if line == prev_line {
                for _ in 0..col.saturating_sub(prev_col) {
                    source.push(' ');
                }
            }
            source.push_str(&t.to_string());

            pos = Some(t.end());
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
