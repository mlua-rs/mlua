use std::borrow::Cow;
use std::fmt;
use std::iter::Peekable;
use std::str::CharIndices;

use crate::error::{Error, Result};
use crate::state::Lua;
use crate::traits::IntoLua;
use crate::types::Integer;
use crate::value::Value;

#[derive(Debug)]
pub(crate) enum PathKey<'a> {
    Str(Cow<'a, str>),
    Int(Integer),
}

impl fmt::Display for PathKey<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PathKey::Str(s) => write!(f, "{}", s),
            PathKey::Int(i) => write!(f, "{}", i),
        }
    }
}

impl IntoLua for PathKey<'_> {
    fn into_lua(self, lua: &Lua) -> Result<Value> {
        match self {
            PathKey::Str(s) => Ok(Value::String(lua.create_string(s.as_ref())?)),
            PathKey::Int(i) => Ok(Value::Integer(i)),
        }
    }
}

// Parses a path like `a.b[3]?.c["d"]` into segments of `(key, safe_nil)`.
pub(crate) fn parse_path<'a>(path: &'a str) -> Result<Vec<(PathKey<'a>, bool)>> {
    fn read_ident<'a>(path: &'a str, chars: &mut Peekable<CharIndices<'a>>) -> (Cow<'a, str>, bool) {
        let mut safe_nil = false;
        let start = chars.peek().map(|&(i, _)| i).unwrap_or(path.len());
        let mut end = start;
        while let Some(&(pos, c)) = chars.peek() {
            if c == '.' || c == '?' || c.is_ascii_whitespace() || c == '[' {
                if c == '?' {
                    safe_nil = true;
                    chars.next(); // consume '?'
                }
                break;
            }
            end = pos + c.len_utf8();
            chars.next();
        }
        (Cow::Borrowed(&path[start..end]), safe_nil)
    }

    let mut segments = Vec::new();
    let mut chars = path.char_indices().peekable();
    while let Some(&(pos, next)) = chars.peek() {
        match next {
            '.' => {
                // Dot notation: identifier
                chars.next();
                let (key, safe_nil) = read_ident(path, &mut chars);
                if key.is_empty() {
                    return Err(Error::runtime(format!("empty key in path at position {pos}")));
                }
                segments.push((PathKey::Str(key), safe_nil));
            }
            '[' => {
                // Bracket notation: either integer or quoted string
                chars.next();
                let key = match chars.peek() {
                    Some(&(pos, c @ '0'..='9' | c @ '-')) => {
                        // Integer key
                        let negative = c == '-';
                        if negative {
                            chars.next(); // consume '-'
                        }
                        let mut num: Option<Integer> = None;
                        while let Some(&(_, c @ '0'..='9')) = chars.peek() {
                            let new_num = num
                                .unwrap_or(0)
                                .checked_mul(10)
                                .and_then(|n| n.checked_add((c as u8 - b'0') as Integer))
                                .ok_or_else(|| {
                                    Error::runtime(format!("integer overflow in path at position {pos}"))
                                })?;
                            num = Some(new_num);
                            chars.next(); // consume digit
                        }
                        match num {
                            Some(n) if negative => PathKey::Int(-n),
                            Some(n) => PathKey::Int(n),
                            None => {
                                let err = format!("invalid integer in path at position {pos}");
                                return Err(Error::runtime(err));
                            }
                        }
                    }
                    Some((_, '\'' | '"')) => {
                        // Quoted string
                        PathKey::Str(unquote_string(path, &mut chars)?)
                    }
                    Some((_, ']')) => {
                        return Err(Error::runtime(format!("empty key in path at position {pos}")));
                    }
                    Some((pos, c)) => {
                        let err = format!("unexpected character '{c}' in path at position {pos}");
                        return Err(Error::runtime(err));
                    }
                    None => {
                        return Err(Error::runtime("unexpected end of path"));
                    }
                };
                // Expect closing bracket
                let mut safe_nil = false;
                match chars.next() {
                    Some((_, ']')) => {
                        // Check for optional safe-nil operator
                        if let Some(&(_, '?')) = chars.peek() {
                            safe_nil = true;
                            chars.next(); // consume '?'
                        }
                    }
                    Some((pos, c)) => {
                        let err = format!("expected ']' in path at position {pos}, found '{c}'");
                        return Err(Error::runtime(err));
                    }
                    None => {
                        return Err(Error::runtime("unexpected end of path"));
                    }
                }
                segments.push((key, safe_nil));
            }
            c if c.is_ascii_whitespace() => {
                chars.next(); // Skip whitespace
            }
            _ if segments.is_empty() => {
                // First segment without dot/bracket notation
                let (key_cow, safe_nil) = read_ident(path, &mut chars);
                if key_cow.is_empty() {
                    return Err(Error::runtime(format!("empty key in path at position {pos}")));
                }
                segments.push((PathKey::Str(key_cow), safe_nil));
            }
            c => {
                let err = format!("unexpected character '{c}' in path at position {pos}");
                return Err(Error::runtime(err));
            }
        }
    }
    Ok(segments)
}

fn unquote_string<'a>(path: &'a str, chars: &mut Peekable<CharIndices<'a>>) -> Result<Cow<'a, str>> {
    let (start_pos, first_quote) = chars.next().unwrap();
    let mut result = String::new();
    loop {
        match chars.next() {
            Some((pos, '\\')) => {
                if result.is_empty() {
                    // First escape found, copy everything up to this point
                    result.push_str(&path[start_pos + 1..pos]);
                }
                match chars.next() {
                    Some((_, '\\')) => result.push('\\'),
                    Some((_, '"')) => result.push('"'),
                    Some((_, '\'')) => result.push('\''),
                    Some((_, other)) => {
                        result.push('\\');
                        result.push(other);
                    }
                    None => continue, // will be handled by outer loop
                }
            }
            Some((pos, c)) if c == first_quote => {
                if !result.is_empty() {
                    return Ok(Cow::Owned(result));
                }
                // No escapes, return borrowed slice
                return Ok(Cow::Borrowed(&path[start_pos + 1..pos]));
            }
            Some((_, c)) => {
                if !result.is_empty() {
                    result.push(c);
                }
                // If no escapes yet, continue tracking for potential borrowed slice
            }
            None => {
                let err = format!("unexpected end of string at position {start_pos}");
                return Err(Error::runtime(err));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_path, PathKey};

    #[test]
    fn test_parse_path() {
        // Test valid paths
        let path = parse_path("a.b[3]?.c['d']").unwrap();
        assert_eq!(path.len(), 5);
        assert!(matches!(path[0], (PathKey::Str(ref s), false) if s == "a"));
        assert!(matches!(path[1], (PathKey::Str(ref s), false) if s == "b"));
        assert!(matches!(path[2], (PathKey::Int(3), true)));
        assert!(matches!(path[3], (PathKey::Str(ref s), false) if s == "c"));
        assert!(matches!(path[4], (PathKey::Str(ref s), false) if s == "d"));

        // Test empty path
        let path = parse_path("").unwrap();
        assert_eq!(path.len(), 0);
        let path = parse_path("   ").unwrap();
        assert_eq!(path.len(), 0);

        // Test invalid dot syntax
        let err = parse_path("a..b").unwrap_err().to_string();
        assert_eq!(err, "runtime error: empty key in path at position 1");
        let err = parse_path("a.b.").unwrap_err().to_string();
        assert_eq!(err, "runtime error: empty key in path at position 3");

        // Test invalid bracket syntax
        let err = parse_path("a[unclosed").unwrap_err().to_string();
        assert_eq!(
            err,
            "runtime error: unexpected character 'u' in path at position 2"
        );
        let err = parse_path("a[]").unwrap_err().to_string();
        assert_eq!(err, "runtime error: empty key in path at position 1");
        let err = parse_path(r#"a["unclosed"#).unwrap_err().to_string();
        assert_eq!(err, "runtime error: unexpected end of string at position 2");
        let err = parse_path(r#"a["#).unwrap_err().to_string();
        assert_eq!(err, "runtime error: unexpected end of path");
        let err = parse_path(r#"a[123"#).unwrap_err().to_string();
        assert_eq!(err, "runtime error: unexpected end of path");
        let err = parse_path(r#"a['bla'123"#).unwrap_err().to_string();
        assert_eq!(
            err,
            "runtime error: expected ']' in path at position 7, found '1'"
        );
        let err = parse_path(r#"a["bla"]x"#).unwrap_err().to_string();
        assert_eq!(
            err,
            "runtime error: unexpected character 'x' in path at position 8"
        );

        // Test bad integers
        let err = parse_path("a[99999999999999999999]").unwrap_err().to_string();
        assert_eq!(err, "runtime error: integer overflow in path at position 2");
        let err = parse_path("a[-]").unwrap_err().to_string();
        assert_eq!(err, "runtime error: invalid integer in path at position 2");
    }
}
