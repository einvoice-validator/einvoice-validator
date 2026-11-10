//! A JSON reader (RFC 8259), enough to read the report of
//! `einvoice-validate --format json`: objects keep their members in order,
//! numbers their text. A document that is not JSON is refused with the byte
//! offset where it stops being JSON.

use crate::util::{Error, Result};

/// A JSON value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Value {
    Null,
    Bool(bool),
    /// The number as written.
    Number(String),
    String(String),
    Array(Vec<Value>),
    /// The members, in order.
    Object(Vec<(String, Value)>),
}

impl Value {
    /// The member `key` of an object; `None` for a missing member or another
    /// kind of value.
    pub(crate) fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn as_array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// A number that is a non-negative integer.
    pub(crate) fn as_count(&self) -> Option<usize> {
        match self {
            Self::Number(n) => n.parse().ok(),
            _ => None,
        }
    }
}

/// Parses a whole JSON text.
pub(crate) fn parse(text: &str) -> Result<Value> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        pos: 0,
        depth: 0,
    };
    parser.space();
    let value = parser.value()?;
    parser.space();
    if parser.pos != parser.bytes.len() {
        return Err(parser.error("text after the value"));
    }
    Ok(value)
}

/// Nesting deeper than this is refused, rather than risking the stack.
const MAX_DEPTH: usize = 128;

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn error(&self, what: &str) -> Error {
        Error::new(format!("JSON: {what} at byte {}", self.pos))
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<()> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.error(&format!("expected {:?}", char::from(byte))))
        }
    }

    fn literal(&mut self, word: &str, value: Value) -> Result<Value> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.error("unexpected character"))
        }
    }

    fn value(&mut self) -> Result<Value> {
        match self.peek() {
            Some(b'{') => self.nested(Self::object),
            Some(b'[') => self.nested(Self::array),
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.error("unexpected character")),
            None => Err(self.error("unexpected end")),
        }
    }

    fn nested(&mut self, parse: fn(&mut Self) -> Result<Value>) -> Result<Value> {
        if self.depth == MAX_DEPTH {
            return Err(self.error("nesting too deep"));
        }
        self.depth += 1;
        let value = parse(self);
        self.depth -= 1;
        value
    }

    fn object(&mut self) -> Result<Value> {
        self.expect(b'{')?;
        self.space();
        let mut members = Vec::new();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Object(members));
        }
        loop {
            self.space();
            if self.peek() != Some(b'"') {
                return Err(self.error("expected a member name"));
            }
            let key = self.string()?;
            self.space();
            self.expect(b':')?;
            self.space();
            members.push((key, self.value()?));
            self.space();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Object(members));
                }
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
    }

    fn array(&mut self) -> Result<Value> {
        self.expect(b'[')?;
        self.space();
        let mut items = Vec::new();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.space();
            items.push(self.value()?);
            self.space();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        self.pos - start
    }

    fn number(&mut self) -> Result<Value> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(self.error("bad number")),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if self.digits() == 0 {
                return Err(self.error("bad number"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return Err(self.error("bad number"));
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| self.error("bad number"))?;
        Ok(Value::Number(text.to_owned()))
    }

    fn hex4(&mut self) -> Result<u32> {
        let digits = self
            .bytes
            .get(self.pos..self.pos + 4)
            .and_then(|d| std::str::from_utf8(d).ok())
            .and_then(|d| u32::from_str_radix(d, 16).ok())
            .ok_or_else(|| self.error("bad \\u escape"))?;
        self.pos += 4;
        Ok(digits)
    }

    fn string(&mut self) -> Result<String> {
        self.expect(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let byte = self
                .peek()
                .ok_or_else(|| self.error("unterminated string"))?;
            self.pos += 1;
            match byte {
                b'"' => break,
                b'\\' => {
                    let escape = self
                        .peek()
                        .ok_or_else(|| self.error("unterminated string"))?;
                    self.pos += 1;
                    let c = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let high = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&high) {
                                if !self.bytes[self.pos..].starts_with(b"\\u") {
                                    return Err(self.error("lone surrogate"));
                                }
                                self.pos += 2;
                                let low = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&low) {
                                    return Err(self.error("lone surrogate"));
                                }
                                0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
                            } else {
                                high
                            };
                            char::from_u32(code).ok_or_else(|| self.error("lone surrogate"))?
                        }
                        _ => return Err(self.error("bad escape")),
                    };
                    let mut buffer = [0; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
                }
                0..=0x1F => return Err(self.error("control character in a string")),
                _ => out.push(byte),
            }
        }
        String::from_utf8(out).map_err(|_| self.error("string not UTF-8"))
    }
}

#[cfg(test)]
mod tests {
    use super::{Value, parse};

    #[test]
    fn values() {
        let v = parse(
            r#" {"a": [1, -2.5e3, 0], "b": {"c": null, "d": true, "e": false},
                 "s": "x\"\\\/\b\f\n\r\t\u00e9\ud83d\ude00", "empty": [], "o": {}} "#,
        )
        .unwrap();
        assert_eq!(
            v.get("a").unwrap().as_array().unwrap(),
            [
                Value::Number("1".into()),
                Value::Number("-2.5e3".into()),
                Value::Number("0".into())
            ]
        );
        assert_eq!(
            v.get("a").unwrap().as_array().unwrap()[0].as_count(),
            Some(1)
        );
        assert_eq!(v.get("b").unwrap().get("c"), Some(&Value::Null));
        assert_eq!(v.get("b").unwrap().get("d"), Some(&Value::Bool(true)));
        assert_eq!(
            v.get("s").unwrap().as_str(),
            Some("x\"\\/\u{8}\u{c}\n\r\t\u{e9}\u{1F600}")
        );
        assert_eq!(v.get("empty"), Some(&Value::Array(Vec::new())));
        assert_eq!(v.get("o"), Some(&Value::Object(Vec::new())));
        assert_eq!(v.get("missing"), None);
        assert_eq!(parse("\"é\"").unwrap().as_str(), Some("é"));
    }

    #[test]
    fn not_json() {
        for (bad, message) in [
            ("", "unexpected end at byte 0"),
            ("{\"a\" 1}", "expected ':' at byte 5"),
            ("[1,]", "unexpected character at byte 3"),
            ("[1 2]", "expected ',' or ']' at byte 3"),
            ("{1: 2}", "expected a member name at byte 1"),
            ("01", "text after the value at byte 1"),
            ("1.", "bad number at byte 2"),
            ("\"a", "unterminated string at byte 2"),
            ("\"\\x\"", "bad escape at byte 3"),
            ("\"\\ud800\"", "lone surrogate at byte 7"),
            ("\"\n\"", "control character in a string at byte 2"),
            ("tru", "unexpected character at byte 0"),
            ("{} {}", "text after the value at byte 3"),
        ] {
            let error = parse(bad).unwrap_err().to_string();
            assert_eq!(error, format!("JSON: {message}"), "{bad:?}");
        }
        let deep = "[".repeat(200);
        assert_eq!(
            parse(&deep).unwrap_err().to_string(),
            "JSON: nesting too deep at byte 128"
        );
    }
}
