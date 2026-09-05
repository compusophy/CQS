//! JSON the size of the need. One `Value`, a parser, a writer. No serde.
//!
//! Objects are an ordered `Vec<(String, Value)>`: the API's objects are small,
//! insertion order makes request bodies stable and diffs readable, and a
//! linear `get` on a dozen keys is faster than hashing them.

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

static NULL: Value = Value::Null;

impl Value {
    pub fn obj() -> Value {
        Value::Obj(Vec::new())
    }
    pub fn arr() -> Value {
        Value::Arr(Vec::new())
    }

    /// Builder: set `key` on an object (replacing an existing key). No-op on
    /// non-objects, so a chain never panics.
    pub fn with(mut self, key: &str, v: impl Into<Value>) -> Value {
        self.set(key, v);
        self
    }
    /// Builder: set `key` only when `v` is `Some`.
    pub fn with_opt<T: Into<Value>>(self, key: &str, v: Option<T>) -> Value {
        match v {
            Some(v) => self.with(key, v),
            None => self,
        }
    }
    pub fn set(&mut self, key: &str, v: impl Into<Value>) {
        if let Value::Obj(o) = self {
            let v = v.into();
            match o.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = v,
                None => o.push((key.to_string(), v)),
            }
        }
    }
    pub fn push(&mut self, v: impl Into<Value>) {
        if let Value::Arr(a) = self {
            a.push(v.into());
        }
    }

    pub fn get(&self, key: &str) -> &Value {
        match self {
            Value::Obj(o) => o
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v)
                .unwrap_or(&NULL),
            _ => &NULL,
        }
    }
    pub fn at(&self, i: usize) -> &Value {
        match self {
            Value::Arr(a) => a.get(i).unwrap_or(&NULL),
            _ => &NULL,
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        self.as_f64().filter(|n| n.fract() == 0.0).map(|n| n as i64)
    }
    pub fn as_u32(&self) -> Option<u32> {
        self.as_i64()
            .filter(|n| *n >= 0 && *n <= u32::MAX as i64)
            .map(|n| n as u32)
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> &[Value] {
        match self {
            Value::Arr(a) => a,
            _ => &[],
        }
    }
    pub fn as_obj(&self) -> &[(String, Value)] {
        match self {
            Value::Obj(o) => o,
            _ => &[],
        }
    }
    /// `as_str` with an owned, lossy fallback: numbers and bools render, the
    /// rest is empty. For pulling loosely-typed model output into a `String`.
    pub fn to_text(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            Value::Num(_) | Value::Bool(_) => self.to_string(),
            _ => String::new(),
        }
    }

    pub fn parse(src: &str) -> Result<Value, JsonError> {
        let mut p = Parser {
            s: src.as_bytes(),
            i: 0,
            depth: 0,
        };
        p.ws();
        let v = p.value()?;
        p.ws();
        if p.i != p.s.len() {
            return Err(p.err("trailing characters"));
        }
        Ok(v)
    }

    pub fn write(&self, out: &mut String) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Num(n) => write_num(*n, out),
            Value::Str(s) => write_str(s, out),
            Value::Arr(a) => {
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Value::Obj(o) => {
                out.push('{');
                for (i, (k, v)) in o.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_str(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }

    /// Indented rendering, for logs and humans.
    pub fn pretty(&self) -> String {
        let mut out = String::new();
        self.write_pretty(&mut out, 0);
        out
    }
    fn write_pretty(&self, out: &mut String, depth: usize) {
        fn pad(out: &mut String, d: usize) {
            for _ in 0..d {
                out.push_str("  ");
            }
        }
        match self {
            Value::Arr(a) if !a.is_empty() => {
                out.push_str("[\n");
                for (i, v) in a.iter().enumerate() {
                    pad(out, depth + 1);
                    v.write_pretty(out, depth + 1);
                    out.push_str(if i + 1 < a.len() { ",\n" } else { "\n" });
                }
                pad(out, depth);
                out.push(']');
            }
            Value::Obj(o) if !o.is_empty() => {
                out.push_str("{\n");
                for (i, (k, v)) in o.iter().enumerate() {
                    pad(out, depth + 1);
                    write_str(k, out);
                    out.push_str(": ");
                    v.write_pretty(out, depth + 1);
                    out.push_str(if i + 1 < o.len() { ",\n" } else { "\n" });
                }
                pad(out, depth);
                out.push('}');
            }
            _ => self.write(out),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::new();
        self.write(&mut s);
        f.write_str(&s)
    }
}

fn write_num(n: f64, out: &mut String) {
    if !n.is_finite() {
        out.push_str("null");
    } else if n.fract() == 0.0 && n.abs() < 1e15 {
        out.push_str(&format!("{}", n as i64));
    } else {
        out.push_str(&format!("{}", n));
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}
impl From<f64> for Value {
    fn from(n: f64) -> Value {
        Value::Num(n)
    }
}
impl From<f32> for Value {
    fn from(n: f32) -> Value {
        Value::Num(n as f64)
    }
}
impl From<i64> for Value {
    fn from(n: i64) -> Value {
        Value::Num(n as f64)
    }
}
impl From<i32> for Value {
    fn from(n: i32) -> Value {
        Value::Num(n as f64)
    }
}
impl From<u32> for Value {
    fn from(n: u32) -> Value {
        Value::Num(n as f64)
    }
}
impl From<u64> for Value {
    fn from(n: u64) -> Value {
        Value::Num(n as f64)
    }
}
impl From<usize> for Value {
    fn from(n: usize) -> Value {
        Value::Num(n as f64)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::Str(s.to_string())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::Str(s)
    }
}
impl From<&String> for Value {
    fn from(s: &String) -> Value {
        Value::Str(s.clone())
    }
}
impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Value {
        Value::Arr(v.into_iter().map(Into::into).collect())
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Value {
        v.map(Into::into).unwrap_or(Value::Null)
    }
}
impl From<&Value> for Value {
    fn from(v: &Value) -> Value {
        v.clone()
    }
}

/// `obj!{"k" => v, ...}` builds an object; values go through `Into<Value>`.
#[macro_export]
macro_rules! obj {
    ($($k:expr => $v:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut o: Vec<(String, $crate::json::Value)> = Vec::new();
        $( o.push(($k.to_string(), $crate::json::Value::from($v))); )*
        $crate::json::Value::Obj(o)
    }};
}
/// `arr![a, b, c]` builds an array; values go through `Into<Value>`.
#[macro_export]
macro_rules! arr {
    ($($v:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut a: Vec<$crate::json::Value> = Vec::new();
        $( a.push($crate::json::Value::from($v)); )*
        $crate::json::Value::Arr(a)
    }};
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError {
    pub pos: usize,
    pub msg: String,
}
impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "json: {} at byte {}", self.msg, self.pos)
    }
}
impl std::error::Error for JsonError {}

const MAX_DEPTH: usize = 128;

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn err(&self, msg: &str) -> JsonError {
        JsonError {
            pos: self.i,
            msg: msg.to_string(),
        }
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.i += 1;
        }
    }
    fn eat(&mut self, lit: &[u8]) -> bool {
        if self.s[self.i..].starts_with(lit) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }
    fn value(&mut self) -> Result<Value, JsonError> {
        match self.peek() {
            None => Err(self.err("unexpected end")),
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b't') if self.eat(b"true") => Ok(Value::Bool(true)),
            Some(b'f') if self.eat(b"false") => Ok(Value::Bool(false)),
            Some(b'n') if self.eat(b"null") => Ok(Value::Null),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.number(),
            Some(_) => Err(self.err("unexpected character")),
        }
    }
    fn enter(&mut self) -> Result<(), JsonError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        Ok(())
    }
    fn object(&mut self) -> Result<Value, JsonError> {
        self.enter()?;
        self.i += 1;
        let mut o = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            self.depth -= 1;
            return Ok(Value::Obj(o));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(self.err("expected string key"));
            }
            let k = self.string()?;
            self.ws();
            if self.peek() != Some(b':') {
                return Err(self.err("expected ':'"));
            }
            self.i += 1;
            self.ws();
            let v = self.value()?;
            o.push((k, v));
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    self.depth -= 1;
                    return Ok(Value::Obj(o));
                }
                _ => return Err(self.err("expected ',' or '}'")),
            }
        }
    }
    fn array(&mut self) -> Result<Value, JsonError> {
        self.enter()?;
        self.i += 1;
        let mut a = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            self.depth -= 1;
            return Ok(Value::Arr(a));
        }
        loop {
            self.ws();
            a.push(self.value()?);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    self.depth -= 1;
                    return Ok(Value::Arr(a));
                }
                _ => return Err(self.err("expected ',' or ']'")),
            }
        }
    }
    fn number(&mut self) -> Result<Value, JsonError> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-') {
                self.i += 1;
            } else {
                break;
            }
        }
        let text = self.slice(start, self.i)?;
        text.parse::<f64>()
            .map(Value::Num)
            .map_err(|_| self.err("bad number"))
    }
    fn hex4(&mut self) -> Result<u32, JsonError> {
        if self.i + 4 > self.s.len() {
            return Err(self.err("short \\u escape"));
        }
        let h = self.slice(self.i, self.i + 4)?;
        let v = u32::from_str_radix(h, 16).map_err(|_| self.err("bad \\u escape"))?;
        self.i += 4;
        Ok(v)
    }
    fn string(&mut self) -> Result<String, JsonError> {
        self.i += 1; // opening quote
        let mut out = String::new();
        let mut run = self.i; // start of the current unescaped run
        loop {
            let c = match self.peek() {
                None => return Err(self.err("unterminated string")),
                Some(c) => c,
            };
            match c {
                b'"' => {
                    out.push_str(self.slice(run, self.i)?);
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => {
                    out.push_str(self.slice(run, self.i)?);
                    self.i += 1;
                    let e = self.peek().ok_or_else(|| self.err("bad escape"))?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) {
                                // high surrogate: a low one must follow
                                if self.eat(b"\\u") {
                                    let lo = self.hex4()?;
                                    if (0xDC00..0xE000).contains(&lo) {
                                        cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                    } else {
                                        cp = 0xFFFD;
                                    }
                                } else {
                                    cp = 0xFFFD;
                                }
                            }
                            out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                    run = self.i;
                }
                c if c < 0x20 => return Err(self.err("control character in string")),
                _ => self.i += 1,
            }
        }
    }
    fn slice(&self, a: usize, b: usize) -> Result<&'a str, JsonError> {
        std::str::from_utf8(&self.s[a..b]).map_err(|_| self.err("invalid utf-8"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let src = r#"{"a":[1,2.5,-3,1e3,true,false,null],"b":{"c":"d\"\\\né😀😀"},"e":""}"#;
        let v = Value::parse(src).unwrap();
        assert_eq!(v.get("a").at(1).as_f64(), Some(2.5));
        assert_eq!(v.get("a").at(3).as_i64(), Some(1000));
        assert_eq!(v.get("b").get("c").as_str(), Some("d\"\\\né😀😀"));
        let again = Value::parse(&v.to_string()).unwrap();
        assert_eq!(v, again);
    }

    #[test]
    fn builders_and_macros() {
        let v =
            obj! {"name" => "x", "n" => 3, "list" => arr![1, "two"], "none" => Option::<i32>::None}
                .with_opt("skip", Option::<i32>::None)
                .with("late", true);
        assert_eq!(
            v.to_string(),
            r#"{"name":"x","n":3,"list":[1,"two"],"none":null,"late":true}"#
        );
        assert_eq!(v.get("missing").get("deeper").at(3).as_str(), None);
    }

    #[test]
    fn errors_carry_positions() {
        let e = Value::parse("{\"a\": tru}").unwrap_err();
        assert!(e.pos >= 6, "{e}");
        assert!(Value::parse("[1,2,]").is_err());
        assert!(Value::parse("\"unterminated").is_err());
        let deep = "[".repeat(200) + &"]".repeat(200);
        assert!(Value::parse(&deep).is_err());
    }

    #[test]
    fn numbers_render_plainly() {
        assert_eq!(Value::from(3.0).to_string(), "3");
        assert_eq!(Value::from(0.5).to_string(), "0.5");
        assert_eq!(Value::from(-7i64).to_string(), "-7");
        assert_eq!(Value::from(f64::NAN).to_string(), "null");
    }
}
