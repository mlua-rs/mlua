use std::array;
use std::collections::HashMap;
use std::iter::Peekable;
use std::ops::Index;
use std::str::CharIndices;

// A simple JSON parser and representation.
// This parser supports only a subset of JSON specification and is intended for Luau's use cases.

#[derive(Debug, PartialEq)]
pub(crate) enum Json<'a> {
    Null,
    Bool(bool),
    Integer(i64),
    Number(f64),
    String(&'a str),
    Array(Vec<Json<'a>>),
    Object(HashMap<&'a str, Json<'a>>),
}

impl<'a> Index<&str> for Json<'a> {
    type Output = Json<'a>;

    fn index(&self, key: &str) -> &Self::Output {
        match self {
            Json::Object(map) => map.get(key).unwrap_or(&Json::Null),
            _ => &Json::Null,
        }
    }
}

impl PartialEq<&str> for Json<'_> {
    fn eq(&self, other: &&str) -> bool {
        matches!(self, Json::String(s) if s == other)
    }
}

impl<'a> Json<'a> {
    pub(crate) fn as_str(&self) -> Option<&'a str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Integer(i) => Some(*i),
            Json::Number(n) if n.fract() == 0.0 => Some(*n as i64),
            _ => None,
        }
    }

    pub(crate) fn as_u64(&self) -> Option<u64> {
        self.as_i64()
            .and_then(|i| if i >= 0 { Some(i as u64) } else { None })
    }

    pub(crate) fn as_array(&self) -> Option<&[Json<'a>]> {
        match self {
            Json::Array(arr) => Some(arr),
            _ => None,
        }
    }

    pub(crate) fn as_object(&self) -> Option<&HashMap<&'a str, Json<'a>>> {
        match self {
            Json::Object(map) => Some(map),
            _ => None,
        }
    }
}

pub(crate) fn parse<'a>(s: &'a str) -> Result<Json<'a>, &'static str> {
    let s = s.trim_ascii();
    let mut chars = s.char_indices().peekable();
    let value = parse_value(s, &mut chars)?;
    Ok(value)
}

fn parse_value<'a>(s: &'a str, chars: &mut Peekable<CharIndices>) -> Result<Json<'a>, &'static str> {
    skip_whitespace(chars);
    match chars.peek() {
        Some((_, '{')) => parse_object(s, chars),
        Some((_, '[')) => parse_array(s, chars),
        Some((_, '"')) => parse_string(s, chars).map(Json::String),
        Some((_, 't' | 'f')) => parse_bool(chars),
        Some((_, 'n')) => parse_null(chars),
        Some((_, '-' | '0'..='9')) => parse_number(chars),
        Some(_) => Err("unexpected character"),
        None => Err("unexpected end of input"),
    }
}

fn parse_object<'a>(s: &'a str, chars: &mut Peekable<CharIndices>) -> Result<Json<'a>, &'static str> {
    chars.next(); // consume '{'

    let mut map = HashMap::new();
    skip_whitespace(chars);
    if matches!(chars.peek(), Some((_, '}'))) {
        chars.next();
        return Ok(Json::Object(map));
    }
    loop {
        skip_whitespace(chars);
        let key = parse_string(s, chars)?;
        skip_whitespace(chars);
        if !matches!(chars.next(), Some((_, ':'))) {
            return Err("expected ':'");
        }
        let value = parse_value(s, chars)?;
        map.insert(key, value);
        skip_whitespace(chars);
        match chars.next() {
            Some((_, ',')) => continue,
            Some((_, '}')) => break,
            _ => return Err("expected ',' or '}'"),
        }
    }
    Ok(Json::Object(map))
}

fn parse_array<'a>(s: &'a str, chars: &mut Peekable<CharIndices>) -> Result<Json<'a>, &'static str> {
    chars.next(); // consume '['

    let mut arr = Vec::new();
    skip_whitespace(chars);
    if matches!(chars.peek(), Some((_, ']'))) {
        chars.next();
        return Ok(Json::Array(arr));
    }
    loop {
        skip_whitespace(chars);
        arr.push(parse_value(s, chars)?);
        skip_whitespace(chars);
        match chars.next() {
            Some((_, ',')) => continue,
            Some((_, ']')) => return Ok(Json::Array(arr)),
            _ => return Err("expected ',' or ']'"),
        }
    }
}

fn parse_string<'a>(s: &'a str, chars: &mut Peekable<CharIndices>) -> Result<&'a str, &'static str> {
    if !matches!(chars.next(), Some((_, '"'))) {
        return Err("expected string starting with '\"'");
    }
    let start = chars.peek().map(|(i, _)| *i).unwrap_or(0);
    for (i, c) in chars {
        if c == '"' {
            return Ok(&s[start..i]);
        }
    }
    Err("unterminated string")
}

fn parse_number(chars: &mut Peekable<CharIndices>) -> Result<Json<'static>, &'static str> {
    let mut is_float = false;
    let mut num = String::new();
    while let Some((_, c @ ('0'..='9' | '-' | '.' | 'e' | 'E' | '+'))) = chars.peek() {
        num.push(*c);
        is_float = is_float || matches!(c, '.' | 'e' | 'E');
        chars.next();
    }
    if !is_float {
        let i = num.parse::<i64>().map_err(|_| "invalid integer")?;
        return Ok(Json::Integer(i));
    }
    let n = num.parse::<f64>().map_err(|_| "invalid number")?;
    Ok(Json::Number(n))
}

fn parse_bool(chars: &mut Peekable<CharIndices>) -> Result<Json<'static>, &'static str> {
    let bool = next_chars(chars);
    if bool == [Some('t'), Some('r'), Some('u'), Some('e')] {
        return Ok(Json::Bool(true));
    }
    if bool == [Some('f'), Some('a'), Some('l'), Some('s')] && matches!(chars.next(), Some((_, 'e'))) {
        return Ok(Json::Bool(false));
    }
    Err("invalid boolean literal")
}

fn parse_null(chars: &mut Peekable<CharIndices>) -> Result<Json<'static>, &'static str> {
    if next_chars(chars) == [Some('n'), Some('u'), Some('l'), Some('l')] {
        return Ok(Json::Null);
    }
    Err("invalid \"null\" literal")
}

fn skip_whitespace(chars: &mut Peekable<CharIndices>) {
    while let Some((_, ' ' | '\n' | '\r' | '\t')) = chars.peek() {
        chars.next();
    }
}

fn next_chars<const N: usize>(chars: &mut Peekable<CharIndices>) -> [Option<char>; N] {
    array::from_fn(|_| chars.next().map(|(_, c)| c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse() {
        assert_eq!(parse("null").unwrap(), Json::Null);
        assert_eq!(parse("true").unwrap(), Json::Bool(true));
        assert_eq!(parse("false").unwrap(), Json::Bool(false));
        assert_eq!(parse("42").unwrap(), Json::Integer(42));
        assert_eq!(parse("42.0").unwrap(), Json::Number(42.0));
        assert_eq!(parse(r#""hello""#).unwrap(), Json::String("hello"));
        assert_eq!(
            parse("[1,2.0,3]").unwrap(),
            Json::Array(vec![Json::Integer(1), Json::Number(2.0), Json::Integer(3)])
        );
        let mut obj = HashMap::new();
        obj.insert("key", Json::String("value"));
        assert_eq!(parse(r#"{"key":"value"}"#).unwrap(), Json::Object(obj));
    }

    #[test]
    fn test_whitespace_handling() {
        assert_eq!(parse("  null  ").unwrap(), Json::Null);
        assert_eq!(parse("  true  ").unwrap(), Json::Bool(true));
        assert_eq!(
            parse(" [ 1 , 2.0 , 3 ] ").unwrap(),
            Json::Array(vec![Json::Integer(1), Json::Number(2.0), Json::Integer(3)])
        );
        let mut obj = HashMap::new();
        obj.insert("key", Json::String("value"));
        assert_eq!(parse(r#"  { "key" : "value" }  "#).unwrap(), Json::Object(obj));
    }

    #[test]
    fn test_empty_collections() {
        assert_eq!(parse("[]").unwrap(), Json::Array(vec![]));
        assert_eq!(parse("{}").unwrap(), Json::Object(HashMap::new()));
        assert_eq!(parse("[ ]").unwrap(), Json::Array(vec![]));
        assert_eq!(parse("{ }").unwrap(), Json::Object(HashMap::new()));
    }

    #[test]
    fn test_nested_structures() {
        assert_eq!(
            parse(r#"{"nested":{"inner":"value"}}"#).unwrap(),
            Json::Object({
                let mut outer = HashMap::new();
                let mut inner = HashMap::new();
                inner.insert("inner", Json::String("value"));
                outer.insert("nested", Json::Object(inner));
                outer
            })
        );
        assert_eq!(
            parse("[[1,2],[3,4]]").unwrap(),
            Json::Array(vec![
                Json::Array(vec![Json::Integer(1), Json::Integer(2)]),
                Json::Array(vec![Json::Integer(3), Json::Integer(4)])
            ])
        );
    }

    #[test]
    fn test_numbers() {
        assert_eq!(parse("0").unwrap(), Json::Integer(0));
        assert_eq!(parse("-42").unwrap(), Json::Integer(-42));
        assert_eq!(parse("3.14").unwrap(), Json::Number(3.14));
        assert_eq!(parse("-3.14").unwrap(), Json::Number(-3.14));
        assert_eq!(parse("1e10").unwrap(), Json::Number(1e10));
        assert_eq!(parse("1E10").unwrap(), Json::Number(1E10));
        assert_eq!(parse("1e-10").unwrap(), Json::Number(1e-10));
        assert_eq!(parse("1.5e+10").unwrap(), Json::Number(1.5e+10));
    }

    #[test]
    fn test_strings() {
        assert_eq!(parse(r#""""#).unwrap(), Json::String(""));
        assert_eq!(parse(r#""hello world""#).unwrap(), Json::String("hello world"));
        assert_eq!(
            parse(r#""with spaces and 123""#).unwrap(),
            Json::String("with spaces and 123")
        );
    }

    #[test]
    fn test_mixed_array() {
        assert_eq!(
            parse(r#"[null, true, false, 35.1, 42, "text", [], {}]"#).unwrap(),
            Json::Array(vec![
                Json::Null,
                Json::Bool(true),
                Json::Bool(false),
                Json::Number(35.1),
                Json::Integer(42),
                Json::String("text"),
                Json::Array(vec![]),
                Json::Object(HashMap::new())
            ])
        );
    }

    #[test]
    fn test_object_multiple_keys() {
        let mut obj = HashMap::new();
        obj.insert("a", Json::Integer(1));
        obj.insert("b", Json::Bool(true));
        obj.insert("c", Json::Null);
        assert_eq!(parse(r#"{"a":1,"b":true,"c":null}"#).unwrap(), Json::Object(obj));
    }

    #[test]
    fn test_error_cases() {
        assert!(parse("").is_err());
        assert!(parse("nul").is_err());
        assert!(parse("tru").is_err());
        assert!(parse("fals").is_err());
        assert!(parse(r#""unterminated"#).is_err());
        assert!(parse("[1,2,]").is_err());
        assert!(parse(r#"{"key""#).is_err());
        assert!(parse(r#"{"key":"value""#).is_err());
        assert!(parse(r#"{"key":"value",}"#).is_err());
        assert!(parse("invalid").is_err());
        assert!(parse("[1 2]").is_err());
        assert!(parse(r#"{"key":"value" "key2":"value2"}"#).is_err());
    }
}
