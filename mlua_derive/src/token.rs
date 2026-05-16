use std::cmp::{Eq, PartialEq};
use std::fmt::{self, Display, Formatter};
use std::vec::IntoIter;

use itertools::Itertools;
use proc_macro::{Delimiter, Span, TokenStream, TokenTree};
use proc_macro2::Span as Span2;

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

fn span_pos(span: &Span) -> (Pos, Pos) {
    let span2: Span2 = (*span).into();
    let start = span2.start();
    let end = span2.end();

    // Rust 1.88 stabilized Span APIs, so this branch must be unreachable
    if start.line == 0 || end.line == 0 {
        proc_macro_error2::abort_call_site!(
            "cannot retrieve span location information; mlua requires nightly Rust or stable >= 1.88"
        );
    }

    (Pos::new(start.line, start.column), Pos::new(end.line, end.column))
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
    fn new(tree: TokenTree) -> Self {
        let (start, end) = span_pos(&tree.span());
        Self {
            source: tree.to_string(),
            start,
            end,
            tree,
            attr: TokenAttr::None,
        }
    }

    fn new_delim(source: String, tree: TokenTree, open: bool) -> Self {
        let (start, end) = span_pos(&tree.span());
        let (start, end) = if open {
            (start, start.right())
        } else {
            (end.left(), end)
        };

        Self {
            source,
            tree,
            start,
            end,
            attr: TokenAttr::None,
        }
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
    pub(crate) fn retokenize(tt: TokenStream) -> Tokens {
        Tokens(
            tt.into_iter()
                .flat_map(Tokens::from)
                .peekable()
                .batching(|iter| {
                    // Find variable tokens: `$` + `ident` => `$ident`
                    let t = iter.next()?;
                    if t.is("$") {
                        if let Some(next) = iter.next()
                            && matches!(next.tree, TokenTree::Ident(_))
                        {
                            Some(next.attr(TokenAttr::Cap))
                        } else {
                            proc_macro_error2::abort!(t.tree.span(), "`$` must be followed by an identifier");
                        }
                    } else {
                        Some(t)
                    }
                })
                .collect(),
        )
    }
}

impl IntoIterator for Tokens {
    type Item = Token;
    type IntoIter = IntoIter<Token>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl From<TokenTree> for Tokens {
    fn from(tt: TokenTree) -> Self {
        let tts = match tt.clone() {
            TokenTree::Group(g) => {
                let (b, e) = match g.delimiter() {
                    Delimiter::Parenthesis => ("(", ")"),
                    Delimiter::Brace => ("{", "}"),
                    Delimiter::Bracket => ("[", "]"),
                    Delimiter::None => ("", ""),
                };
                let (b, e) = (b.into(), e.into());

                vec![Token::new_delim(b, tt.clone(), true)]
                    .into_iter()
                    .chain(g.stream().into_iter().flat_map(Tokens::from))
                    .chain(vec![Token::new_delim(e, tt, false)])
                    .collect()
            }
            _ => vec![Token::new(tt)],
        };
        Tokens(tts)
    }
}

impl Display for Token {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}", self.source)
    }
}
