//! A reader of JSON documents (RFC 8259), for the output of `cargo metadata`: values are
//! kept as they are written, numbers as their text.

/// A JSON value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Value {
    Null,
    Bool(bool),
    /// A number, as written.
    Number(String),
    String(String),
    Array(Vec<Value>),
    /// The members in document order.
    Object(Vec<(String, Value)>),
}

impl Value {
    /// The member `key` of an object.
    pub(crate) fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }
}

/// The JSON document `text`, one value with nothing but white space around it.
pub(crate) fn parse(text: &str) -> Result<Value, String> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        at: 0,
        depth: 0,
    };
    let value = parser.value()?;
    parser.space();
    if parser.at == parser.bytes.len() {
        Ok(value)
    } else {
        Err(parser.error("data after the value"))
    }
}

/// Values nested deeper than this are refused (the recursion would be that deep).
const MAX_DEPTH: usize = 128;

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
    depth: usize,
}

impl Parser<'_> {
    fn error(&self, what: &str) -> String {
        format!("JSON: {what} at byte {}", self.at)
    }

    fn space(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.bytes.get(self.at) {
            self.at += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.bytes.get(self.at) == Some(&byte) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn literal(&mut self, word: &str, value: Value) -> Result<Value, String> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(self.error("unknown literal"))
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        self.space();
        match self.bytes.get(self.at) {
            None => Err(self.error("end of input instead of a value")),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'"') => self.string().map(Value::String),
            Some(b'[' | b'{') => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err(self.error("values nested too deep"));
                }
                let value = if self.bytes[self.at] == b'[' {
                    self.array()
                } else {
                    self.object()
                };
                self.depth -= 1;
                value
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.error("unexpected character")),
        }
    }

    fn array(&mut self) -> Result<Value, String> {
        self.at += 1;
        let mut items = Vec::new();
        self.space();
        if self.eat(b']') {
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.space();
            if self.eat(b']') {
                return Ok(Value::Array(items));
            }
            if !self.eat(b',') {
                return Err(self.error("expected , or ]"));
            }
        }
    }

    fn object(&mut self) -> Result<Value, String> {
        self.at += 1;
        let mut members = Vec::new();
        self.space();
        if self.eat(b'}') {
            return Ok(Value::Object(members));
        }
        loop {
            self.space();
            if self.bytes.get(self.at) != Some(&b'"') {
                return Err(self.error("expected a member name"));
            }
            let key = self.string()?;
            self.space();
            if !self.eat(b':') {
                return Err(self.error("expected :"));
            }
            members.push((key, self.value()?));
            self.space();
            if self.eat(b'}') {
                return Ok(Value::Object(members));
            }
            if !self.eat(b',') {
                return Err(self.error("expected , or }"));
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
            self.at += 1;
        }
        self.at - start
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.at;
        self.eat(b'-');
        if self.eat(b'0') {
            if self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
                return Err(self.error("leading zero"));
            }
        } else if self.digits() == 0 {
            return Err(self.error("expected a digit"));
        }
        if self.eat(b'.') && self.digits() == 0 {
            return Err(self.error("expected a digit after ."));
        }
        if self.eat(b'e') || self.eat(b'E') {
            if !self.eat(b'+') {
                self.eat(b'-');
            }
            if self.digits() == 0 {
                return Err(self.error("expected an exponent"));
            }
        }
        // Only ASCII was consumed.
        Ok(Value::Number(
            String::from_utf8_lossy(&self.bytes[start..self.at]).into_owned(),
        ))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .bytes
            .get(self.at..self.at + 4)
            .and_then(|d| std::str::from_utf8(d).ok())
            .and_then(|d| {
                u32::from_str_radix(d, 16)
                    .ok()
                    .filter(|_| !d.starts_with('+'))
            })
            .ok_or_else(|| self.error("expected four hexadecimal digits"))?;
        self.at += 4;
        Ok(digits)
    }

    fn string(&mut self) -> Result<String, String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let start = self.at;
            while let Some(&b) = self.bytes.get(self.at) {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.at += 1;
            }
            // The input is a &str, and these bounds are at ASCII bytes.
            out.push_str(
                std::str::from_utf8(&self.bytes[start..self.at])
                    .map_err(|_| self.error("not UTF-8"))?,
            );
            match self.bytes.get(self.at) {
                None => return Err(self.error("unterminated string")),
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    let escape = *self
                        .bytes
                        .get(self.at)
                        .ok_or_else(|| self.error("unterminated escape"))?;
                    self.at += 1;
                    let c = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode_escape()?,
                        _ => return Err(self.error("unknown escape")),
                    };
                    out.push(c);
                }
                Some(_) => return Err(self.error("control character in a string")),
            }
        }
    }

    /// The character of a `\u` escape (after the `u`), a surrogate pair taking two.
    fn unicode_escape(&mut self) -> Result<char, String> {
        let first = self.hex4()?;
        let code = if (0xd800..0xdc00).contains(&first) {
            if !(self.eat(b'\\') && self.eat(b'u')) {
                return Err(self.error("lone high surrogate"));
            }
            let second = self.hex4()?;
            if !(0xdc00..0xe000).contains(&second) {
                return Err(self.error("high surrogate without its low surrogate"));
            }
            0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00)
        } else {
            first
        };
        char::from_u32(code).ok_or_else(|| self.error("lone low surrogate"))
    }
}

#[cfg(test)]
mod tests {
    use super::{Value, parse};

    fn s(text: &str) -> Value {
        Value::String(text.to_owned())
    }

    #[test]
    fn reads_every_kind_of_value() {
        let v = parse(
            r#" {"a": [1, -0.5e+3, true, false, null, "x"], "b": {}, "c": [], "d": "\"\\\/\b\f\n\r\t\u00e9\ud83d\ude00"} "#,
        )
        .unwrap();
        assert_eq!(
            v.get("a").unwrap().as_array().unwrap(),
            [
                Value::Number("1".into()),
                Value::Number("-0.5e+3".into()),
                Value::Bool(true),
                Value::Bool(false),
                Value::Null,
                s("x"),
            ]
        );
        assert_eq!(v.get("b"), Some(&Value::Object(Vec::new())));
        assert_eq!(v.get("c"), Some(&Value::Array(Vec::new())));
        assert_eq!(
            v.get("d").unwrap().as_str(),
            Some("\"\\/\u{8}\u{c}\n\r\té\u{1f600}")
        );
        assert_eq!(v.get("e"), None);
        assert_eq!(parse(r#""é ü""#).unwrap(), s("é ü"));
    }

    #[test]
    fn refuses_what_is_not_json() {
        for text in [
            "",
            "[1,]",
            "{\"a\" 1}",
            "{a: 1}",
            "01",
            "1.",
            "1e",
            "-",
            "\"abc",
            "\"\\x\"",
            "\"\\u12\"",
            "\"\\u+123\"",
            "\"\\ud800\"",
            "\"\\udc00\"",
            "\"\\ud800\\u0041\"",
            "\"a\nb\"",
            "nul",
            "[] []",
            "[[[[",
        ] {
            assert!(parse(text).is_err(), "{text:?}");
        }
        let deep = "[".repeat(200) + &"]".repeat(200);
        assert!(parse(&deep).is_err());
        let fine = "[".repeat(100) + &"]".repeat(100);
        assert!(parse(&fine).is_ok());
    }
}
