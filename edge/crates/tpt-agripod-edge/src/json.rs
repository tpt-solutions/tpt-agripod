// SPDX-License-Identifier: MIT OR Apache-2.0

//! A small JSON parser (RFC 8259) for profiles arriving over MQTT.
//!
//! Written here rather than taken from a crate: the crates that fit either carry an advisory
//! or pull in a dependency the licence policy rejects (`ryu` is Apache-2.0 OR BSL-1.0). The
//! parser is strict. It rejects duplicate keys, trailing data, control characters in strings,
//! non-finite numbers, and nesting deeper than [`MAX_DEPTH`]. Those are the cases a crafted
//! profile would use to confuse a validator.

/// Deepest nesting accepted. A Crop Profile is about three levels deep.
pub const MAX_DEPTH: usize = 32;

/// A parsed JSON value. Objects keep their keys in source order.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `null`
    Null,
    /// `true` or `false`
    Bool(bool),
    /// A finite number.
    Number(f64),
    /// A string.
    String(String),
    /// An array.
    Array(Vec<Value>),
    /// An object, with keys in source order. Keys are unique.
    Object(Vec<(String, Value)>),
}

impl Value {
    /// The member `key` of an object, or `None`.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The number, if this is one.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// The string, if this is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// The members, if this is an object.
    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(m) => Some(m),
            _ => None,
        }
    }

    /// The items, if this is an array.
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }
}

/// Serialises a value as compact JSON. Object members keep their order. Numbers use Rust's
/// shortest round-trip form, so `parse(to_string(v)) == v` for every finite value.
pub fn to_string(v: &Value) -> String {
    let mut out = String::new();
    write_value(v, &mut out);
    out
}

fn write_value(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&format!("{n}")),
        Value::String(s) => out.push_str(&quote(s)),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out);
            }
            out.push(']');
        }
        Value::Object(members) => {
            out.push('{');
            for (i, (k, val)) in members.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&quote(k));
                out.push(':');
                write_value(val, out);
            }
            out.push('}');
        }
    }
}

/// A JSON string literal for `s`, with the escapes RFC 8259 requires.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A parse failure with the byte offset where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    /// Byte offset into the input.
    pub offset: usize,
    /// What went wrong.
    pub message: &'static str,
}

/// Parses one JSON document. Trailing non-whitespace is an error.
pub fn parse(input: &str) -> Result<Value, JsonError> {
    let mut p = Parser {
        b: input.as_bytes(),
        i: 0,
        depth: 0,
    };
    p.skip_ws();
    let v = p.value()?;
    p.skip_ws();
    if p.i != p.b.len() {
        return Err(p.err("trailing data after the document"));
    }
    Ok(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    depth: usize,
}

impl Parser<'_> {
    fn err(&self, message: &'static str) -> JsonError {
        JsonError {
            offset: self.i,
            message,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn expect(&mut self, c: u8, message: &'static str) -> Result<(), JsonError> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.err(message))
        }
    }

    fn value(&mut self) -> Result<Value, JsonError> {
        match self.peek() {
            Some(b'{') => self.nested(Self::object),
            Some(b'[') => self.nested(Self::array),
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.literal(b"true", Value::Bool(true)),
            Some(b'f') => self.literal(b"false", Value::Bool(false)),
            Some(b'n') => self.literal(b"null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("unexpected character")),
            None => Err(self.err("unexpected end of input")),
        }
    }

    fn nested(&mut self, f: fn(&mut Self) -> Result<Value, JsonError>) -> Result<Value, JsonError> {
        if self.depth >= MAX_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        self.depth += 1;
        let v = f(self);
        self.depth -= 1;
        v
    }

    fn literal(&mut self, word: &[u8], v: Value) -> Result<Value, JsonError> {
        if self.b[self.i..].starts_with(word) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(self.err("invalid literal"))
        }
    }

    fn object(&mut self) -> Result<Value, JsonError> {
        self.expect(b'{', "expected '{'")?;
        let mut members: Vec<(String, Value)> = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Value::Object(members));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(self.err("expected a string key"));
            }
            let key = self.string()?;
            if members.iter().any(|(k, _)| *k == key) {
                return Err(self.err("duplicate key"));
            }
            self.skip_ws();
            self.expect(b':', "expected ':' after key")?;
            self.skip_ws();
            let v = self.value()?;
            members.push((key, v));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Value::Object(members));
                }
                _ => return Err(self.err("expected ',' or '}'")),
            }
        }
    }

    fn array(&mut self) -> Result<Value, JsonError> {
        self.expect(b'[', "expected '['")?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.err("expected ',' or ']'")),
            }
        }
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.expect(b'"', "expected '\"'")?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = self.peek().ok_or_else(|| self.err("unterminated string"))?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = self.peek().ok_or_else(|| self.err("unterminated escape"))?;
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode_escape()?,
                        _ => return Err(self.err("invalid escape")),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                0x00..=0x1f => return Err(self.err("control character in string")),
                _ => out.push(c),
            }
        }
        // Input was valid UTF-8 and every escape produced a valid char, so this cannot fail.
        String::from_utf8(out).map_err(|_| self.err("invalid UTF-8"))
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let mut v = 0u32;
        for _ in 0..4 {
            let c = self
                .peek()
                .ok_or_else(|| self.err("short unicode escape"))?;
            let d = match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                b'A'..=b'F' => c - b'A' + 10,
                _ => return Err(self.err("bad hex digit in unicode escape")),
            };
            v = v * 16 + u32::from(d);
            self.i += 1;
        }
        Ok(v)
    }

    fn unicode_escape(&mut self) -> Result<char, JsonError> {
        let hi = self.hex4()?;
        let code = if (0xD800..0xDC00).contains(&hi) {
            // High surrogate: a low surrogate must follow.
            if self.b[self.i..].starts_with(b"\\u") {
                self.i += 2;
                let lo = self.hex4()?;
                if !(0xDC00..0xE000).contains(&lo) {
                    return Err(self.err("unpaired surrogate"));
                }
                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
            } else {
                return Err(self.err("unpaired surrogate"));
            }
        } else if (0xDC00..0xE000).contains(&hi) {
            return Err(self.err("unpaired surrogate"));
        } else {
            hi
        };
        char::from_u32(code).ok_or_else(|| self.err("invalid code point"))
    }

    fn number(&mut self) -> Result<Value, JsonError> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        match self.peek() {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err(self.err("expected a digit")),
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.err("expected a digit after '.'"));
            }
            self.digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.err("expected a digit in exponent"));
            }
            self.digits();
        }
        // The grammar above guarantees this slice is a valid float literal.
        let text =
            std::str::from_utf8(&self.b[start..self.i]).map_err(|_| self.err("invalid number"))?;
        let v: f64 = text.parse().map_err(|_| self.err("invalid number"))?;
        if !v.is_finite() {
            return Err(self.err("number out of range"));
        }
        Ok(Value::Number(v))
    }

    fn digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> Value {
        parse(s).unwrap_or_else(|e| panic!("{s}: {e:?}"))
    }

    fn bad(s: &str) {
        assert!(parse(s).is_err(), "accepted {s}");
    }

    #[test]
    fn parses_the_shapes_profiles_use() {
        let v = ok(r#"{"a":1.5,"b":[true,false,null],"c":{"d":"x\ny"},"e":-0.25e2}"#);
        assert_eq!(v.get("a").and_then(Value::as_f64), Some(1.5));
        assert_eq!(
            v.get("c").and_then(|c| c.get("d")).and_then(Value::as_str),
            Some("x\ny")
        );
        assert_eq!(v.get("e").and_then(Value::as_f64), Some(-25.0));
        assert_eq!(
            v.get("b").and_then(Value::as_array).map(|a| a.len()),
            Some(3)
        );
    }

    #[test]
    fn shipped_example_profile_parses() {
        let text = include_str!("../../../../profiles/examples/example-plant-v0.json");
        assert!(parse(text).is_ok());
    }

    #[test]
    fn decodes_escapes_and_surrogate_pairs() {
        assert_eq!(ok(r#""\u00e9""#), Value::String("é".into()));
        assert_eq!(ok(r#""\ud83c\udf31""#), Value::String("🌱".into()));
        assert_eq!(
            ok(r#""\"\\\/\b\f\n\r\t""#),
            Value::String("\"\\/\u{8}\u{c}\n\r\t".into())
        );
    }

    #[test]
    fn rejects_malformed_documents() {
        for s in [
            "",
            "{",
            "[1,]",
            "{\"a\":1,}",
            "{\"a\" 1}",
            "01",
            "1.",
            "1e",
            "+1",
            ".5",
            "NaN",
            "\"unterminated",
            "\"\u{1}\"",
            "\"\\x\"",
            "\"\\ud800\"",
            "\"\\udc00\"",
            "{} x",
            "tru",
            "1e999",
        ] {
            bad(s);
        }
    }

    #[test]
    fn rejects_duplicate_keys() {
        bad(r#"{"kind":"plant","kind":"culture"}"#);
    }

    #[test]
    fn caps_nesting_depth() {
        let deep = "[".repeat(MAX_DEPTH + 1) + &"]".repeat(MAX_DEPTH + 1);
        bad(&deep);
        let fine = "[".repeat(MAX_DEPTH) + &"]".repeat(MAX_DEPTH);
        assert!(parse(&fine).is_ok());
    }

    #[test]
    fn never_panics_on_random_bytes() {
        // Seeded, so failures reproduce. Random printable and structural bytes.
        let alphabet = b"{}[]\":,0123456789.-+eE tfnrua\\";
        let mut state: u64 = 0x5EED_0000_1234_5678;
        for _ in 0..50_000 {
            let mut s = String::new();
            let len = {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (state >> 33) % 24
            };
            for _ in 0..len {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                s.push(alphabet[((state >> 33) as usize) % alphabet.len()] as char);
            }
            let _ = parse(&s);
        }
    }
}
