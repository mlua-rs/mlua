use std::cmp::{Eq, PartialEq};
use std::convert::TryFrom;
use std::fmt::{self, Display, Formatter};
use std::vec::IntoIter;

use proc_macro::{Delimiter, Span, TokenStream, TokenTree};
use proc_macro2::{Span as Span2, TokenStream as TokenStream2};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Pos {
    pub(crate) line: usize,
    pub(crate) column: usize,
}

impl Pos {
    fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }

    fn left(&self) -> Self {
        Self {
            line: self.line,
            column: self.column.saturating_sub(1),
        }
    }

    fn right(&self) -> Self {
        Self {
            line: self.line,
            column: self.column.saturating_add(1),
        }
    }
}

fn span_pos(span: &Span) -> Result<(Pos, Pos), TokenStream2> {
    let span2: Span2 = (*span).into();
    let start = span2.start();
    let end = span2.end();

    // Rust 1.88 stabilized Span APIs, so this branch must be unreachable
    if start.line == 0 || end.line == 0 {
        return Err(syn::Error::new(
            Span2::call_site(),
            "cannot retrieve span location information; mlua requires nightly Rust or stable >= 1.88",
        )
        .to_compile_error());
    }

    Ok((Pos::new(start.line, start.column), Pos::new(end.line, end.column)))
}

/// Attribute of token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokenAttr {
    /// No attribute
    None,
    /// Starts with `$`
    Cap,
}

#[derive(Clone, Debug)]
pub(crate) struct Token {
    source: String,
    tree: TokenTree,
    start: Pos,
    end: Pos,
    attr: TokenAttr,
}

impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source && self.attr == other.attr
    }
}

impl Eq for Token {}

impl Token {
    fn new(tree: TokenTree) -> Result<Self, TokenStream2> {
        let (start, end) = span_pos(&tree.span())?;
        let source = tree.span().source_text().unwrap_or_else(|| tree.to_string());
        Ok(Self {
            source,
            start,
            end,
            tree,
            attr: TokenAttr::None,
        })
    }

    fn new_delim(source: String, tree: TokenTree, open: bool) -> Result<Self, TokenStream2> {
        let (start, end) = span_pos(&tree.span())?;
        let (start, end) = if open {
            (start, start.right())
        } else {
            (end.left(), end)
        };
        Ok(Self {
            source,
            tree,
            start,
            end,
            attr: TokenAttr::None,
        })
    }

    pub(crate) fn tree(&self) -> &TokenTree {
        &self.tree
    }

    pub(crate) fn is_cap(&self) -> bool {
        self.attr == TokenAttr::Cap
    }

    pub(crate) fn start(&self) -> Pos {
        self.start
    }

    pub(crate) fn end(&self) -> Pos {
        self.end
    }

    fn is(&self, s: &str) -> bool {
        self.source == s
    }

    fn attr(mut self, attr: TokenAttr) -> Self {
        self.attr = attr;
        self
    }
}

#[derive(Debug)]
pub(crate) struct Tokens(pub(crate) Vec<Token>);

impl Tokens {
    pub(crate) fn retokenize(tt: TokenStream) -> Result<Tokens, TokenStream2> {
        let mut flat = Vec::new();
        for tree in tt {
            flat.extend(Tokens::try_from(tree)?);
        }

        let mut tokens = Vec::new();
        let mut iter = flat.into_iter();
        while let Some(t) = iter.next() {
            // Find variable tokens: `$` + `ident` => `$ident`
            if t.is("$") {
                if let Some(next) = iter.next()
                    && matches!(next.tree, TokenTree::Ident(_))
                {
                    tokens.push(next.attr(TokenAttr::Cap));
                } else {
                    return Err(syn::Error::new(
                        t.tree.span().into(),
                        "`$` must be followed by an identifier",
                    )
                    .to_compile_error());
                }
            } else {
                tokens.push(t);
            }
        }
        Ok(Tokens(tokens))
    }
}

impl IntoIterator for Tokens {
    type Item = Token;
    type IntoIter = IntoIter<Token>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl TryFrom<TokenTree> for Tokens {
    type Error = TokenStream2;

    fn try_from(tt: TokenTree) -> Result<Self, TokenStream2> {
        let tts = match tt.clone() {
            TokenTree::Group(g) => {
                let (b, e) = match g.delimiter() {
                    Delimiter::Parenthesis => ("(", ")"),
                    Delimiter::Brace => ("{", "}"),
                    Delimiter::Bracket => ("[", "]"),
                    Delimiter::None => ("", ""),
                };
                let (b, e) = (b.into(), e.into());

                let mut result = vec![Token::new_delim(b, tt.clone(), true)?];
                for inner in g.stream() {
                    result.extend(Tokens::try_from(inner)?);
                }
                result.push(Token::new_delim(e, tt, false)?);
                result
            }
            _ => vec![Token::new(tt)?],
        };
        Ok(Tokens(tts))
    }
}

impl Display for Token {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}", self.source)
    }
}
