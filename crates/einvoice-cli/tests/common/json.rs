//! A strict JSON reader (RFC 8259), to check that the JSON output is JSON and to read
//! it back. Numbers are read as unsigned integers: the report has no other.

use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Value {
    Null,
    Bool(bool),
    Number(u64),
    String(String),
    Array(Vec<Value>),
    /// Members in order; keys must be unique.
    Object(Vec<(String, Value)>),
}

impl Value {
    pub(crate) fn get(&self, key: &str) -> &Value {
        match self {
            Value::Object(members) => members
                .iter()
                .find(|(k, _)| k == key)
                .map_or_else(|| panic!("no key {key:?}"), |(_, v)| v),
            other => panic!("not an object: {other:?}"),
        }
    }

    pub(crate) fn keys(&self) -> Vec<&str> {
        match self {
            Value::Object(members) => members.iter().map(|(k, _)| k.as_str()).collect(),
            other => panic!("not an object: {other:?}"),
        }
    }

    pub(crate) fn str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            Value::Null => None,
            other => panic!("not a string: {other:?}"),
        }
    }

    pub(crate) fn number(&self) -> u64 {
        match self {
            Value::Number(n) => *n,
            other => panic!("not a number: {other:?}"),
        }
    }

    pub(crate) fn array(&self) -> &[Value] {
        match self {
            Value::Array(items) => items,
            other => panic!("not an array: {other:?}"),
        }
    }
}

pub(crate) fn parse(text: &str) -> Result<Value, String> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        at: 0,
    };
    let value = parser.value()?;
    parser.blank();
    if parser.at != parser.bytes.len() {
        return Err(format!("trailing content at byte {}", parser.at));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn blank(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.at += 1;
        }
    }

    fn expect(&mut self, literal: &str) -> Result<(), String> {
        if self.bytes[self.at..].starts_with(literal.as_bytes()) {
            self.at += literal.len();
            Ok(())
        } else {
            Err(format!("expected {literal:?} at byte {}", self.at))
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        self.blank();
        match self.bytes.get(self.at) {
            Some(b'n') => self.expect("null").map(|()| Value::Null),
            Some(b't') => self.expect("true").map(|()| Value::Bool(true)),
            Some(b'f') => self.expect("false").map(|()| Value::Bool(false)),
            Some(b'"') => self.string().map(Value::String),
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                self.blank();
                if self.bytes.get(self.at) == Some(&b']') {
                    self.at += 1;
                    return Ok(Value::Array(items));
                }
                loop {
                    items.push(self.value()?);
                    self.blank();
                    match self.bytes.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b']') => {
                            self.at += 1;
                            return Ok(Value::Array(items));
                        }
                        _ => return Err(format!("expected , or ] at byte {}", self.at)),
                    }
                }
            }
            Some(b'{') => {
                self.at += 1;
                let mut members: Vec<(String, Value)> = Vec::new();
                let mut seen = BTreeSet::new();
                self.blank();
                if self.bytes.get(self.at) == Some(&b'}') {
                    self.at += 1;
                    return Ok(Value::Object(members));
                }
                loop {
                    self.blank();
                    let key = self.string()?;
                    if !seen.insert(key.clone()) {
                        return Err(format!("duplicate key {key:?}"));
                    }
                    self.blank();
                    self.expect(":")?;
                    let value = self.value()?;
                    members.push((key, value));
                    self.blank();
                    match self.bytes.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b'}') => {
                            self.at += 1;
                            return Ok(Value::Object(members));
                        }
                        _ => return Err(format!("expected , or }} at byte {}", self.at)),
                    }
                }
            }
            Some(b'0'..=b'9') => {
                let start = self.at;
                while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
                    self.at += 1;
                }
                let digits = std::str::from_utf8(&self.bytes[start..self.at]).unwrap();
                if digits.len() > 1 && digits.starts_with('0') {
                    return Err(format!("leading zero at byte {start}"));
                }
                digits.parse().map(Value::Number).map_err(|e| e.to_string())
            }
            _ => Err(format!("unexpected input at byte {}", self.at)),
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect("\"")?;
        let mut out = Vec::new();
        loop {
            let Some(&b) = self.bytes.get(self.at) else {
                return Err("unterminated string".to_owned());
            };
            self.at += 1;
            match b {
                b'"' => return String::from_utf8(out).map_err(|e| e.to_string()),
                b'\\' => {
                    let Some(&e) = self.bytes.get(self.at) else {
                        return Err("unterminated escape".to_owned());
                    };
                    self.at += 1;
                    let c = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'u' => {
                            let hex = std::str::from_utf8(&self.bytes[self.at..self.at + 4])
                                .map_err(|e| e.to_string())?;
                            self.at += 4;
                            let code = u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
                            char::from_u32(code).ok_or("surrogate escapes are not used")?
                        }
                        _ => return Err(format!("bad escape at byte {}", self.at)),
                    };
                    let mut buffer = [0; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
                }
                b if b < 0x20 => return Err(format!("raw control character at byte {}", self.at)),
                b => out.push(b),
            }
        }
    }
}
