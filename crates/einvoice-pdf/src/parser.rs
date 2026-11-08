//! The PDF object syntax (ISO 32000-2, 7.2 and 7.3): a recursive-descent parser over a
//! byte buffer, the file or the decoded data of an object stream. Nesting is bounded by
//! [`Limits::max_depth`](crate::Limits::max_depth), so a hostile file cannot exhaust
//! the stack.

use std::collections::btree_map::Entry;

use crate::error::{Error, Location, Result};
use crate::object::{Dict, ObjRef, Object, display_name};

/// PDF white-space characters (7.2.3, Table 1).
pub(crate) fn is_whitespace(byte: u8) -> bool {
    matches!(byte, 0 | b'\t' | b'\n' | 0x0C | b'\r' | b' ')
}

/// PDF delimiter characters (7.2.3, Table 2).
pub(crate) fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn is_regular(byte: u8) -> bool {
    !is_whitespace(byte) && !is_delimiter(byte)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// What follows the value of an indirect object.
#[derive(Debug)]
pub(crate) enum Body {
    /// `endobj`: the object is complete.
    Object(Object),
    /// `stream`: the dictionary of a stream whose data starts at this offset.
    Stream(Dict, usize),
}

#[derive(Debug)]
pub(crate) struct Parser<'a> {
    data: &'a [u8],
    pos: usize,
    max_depth: usize,
    /// The object stream whose decoded data this is, `None` for the file itself.
    stream: Option<u32>,
}

impl<'a> Parser<'a> {
    pub(crate) fn new(data: &'a [u8], pos: usize, max_depth: usize) -> Self {
        Self {
            data,
            pos,
            max_depth,
            stream: None,
        }
    }

    pub(crate) fn in_object_stream(data: &'a [u8], stream: u32, max_depth: usize) -> Self {
        Self {
            data,
            pos: 0,
            max_depth,
            stream: Some(stream),
        }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }

    pub(crate) fn location(&self) -> Location {
        self.location_at(self.pos)
    }

    fn location_at(&self, offset: usize) -> Location {
        match self.stream {
            None => Location::File(offset),
            Some(stream) => Location::ObjectStream { stream, offset },
        }
    }

    pub(crate) fn error(&self, message: impl Into<String>) -> Error {
        Error::malformed(Some(self.location()), message)
    }

    fn error_at(&self, offset: usize, message: impl Into<String>) -> Error {
        Error::malformed(Some(self.location_at(offset)), message)
    }

    pub(crate) fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    /// Skips white space and comments.
    pub(crate) fn skip_whitespace(&mut self) {
        while let Some(byte) = self.peek() {
            if is_whitespace(byte) {
                self.pos += 1;
            } else if byte == b'%' {
                while let Some(byte) = self.peek() {
                    if byte == b'\r' || byte == b'\n' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    /// Skips white space only (no comments).
    pub(crate) fn skip_blanks(&mut self) {
        while self.peek().is_some_and(is_whitespace) {
            self.pos += 1;
        }
    }

    /// The run of regular characters at the current position (possibly empty).
    pub(crate) fn token(&mut self) -> &'a [u8] {
        let start = self.pos;
        while self.peek().is_some_and(is_regular) {
            self.pos += 1;
        }
        &self.data[start..self.pos]
    }

    /// Skips white space and comments, then requires the keyword `keyword`.
    pub(crate) fn expect_keyword(&mut self, keyword: &str) -> Result<()> {
        self.skip_whitespace();
        let start = self.pos;
        let token = self.token();
        if token == keyword.as_bytes() {
            Ok(())
        } else {
            Err(self.error_at(
                start,
                format!(
                    "expected `{keyword}`, found {}",
                    describe_token(token, self.data.get(start))
                ),
            ))
        }
    }

    /// Whether the next token (after white space and comments) is `keyword`; consumed
    /// when it is.
    pub(crate) fn eat_keyword(&mut self, keyword: &str) -> bool {
        self.skip_whitespace();
        let start = self.pos;
        if self.token() == keyword.as_bytes() {
            true
        } else {
            self.pos = start;
            false
        }
    }

    /// An unsigned decimal integer (digits only), after white space and comments.
    pub(crate) fn unsigned(&mut self) -> Result<u64> {
        self.skip_whitespace();
        let start = self.pos;
        let token = self.token();
        parse_unsigned(token).ok_or_else(|| {
            self.error_at(
                start,
                format!(
                    "expected an unsigned integer, found {}",
                    describe_token(token, self.data.get(start))
                ),
            )
        })
    }

    /// The header `num gen obj` of an indirect object.
    pub(crate) fn object_header(&mut self) -> Result<ObjRef> {
        let start = self.pos;
        let num = self.unsigned()?;
        let generation = self.unsigned()?;
        self.expect_keyword("obj")?;
        let num = u32::try_from(num)
            .map_err(|_| self.error_at(start, format!("object number {num} out of range")))?;
        let generation = u16::try_from(generation).map_err(|_| {
            self.error_at(
                start,
                format!("generation number {generation} out of range"),
            )
        })?;
        Ok(ObjRef { num, generation })
    }

    /// The value of an indirect object, then `endobj` or `stream` and its end-of-line
    /// marker (7.3.8.1: CRLF or LF, never CR alone).
    pub(crate) fn object_body(&mut self) -> Result<Body> {
        let value = self.object()?;
        self.skip_whitespace();
        let start = self.pos;
        match self.token() {
            b"endobj" => Ok(Body::Object(value)),
            b"stream" => {
                let Object::Dict(dict) = value else {
                    return Err(self.error_at(
                        start,
                        format!("`stream` after {}, not a dictionary", value.kind()),
                    ));
                };
                match (self.peek(), self.data.get(self.pos + 1)) {
                    (Some(b'\n'), _) => self.pos += 1,
                    (Some(b'\r'), Some(b'\n')) => self.pos += 2,
                    _ => {
                        return Err(
                            self.error("the keyword `stream` must be followed by CRLF or LF")
                        );
                    }
                }
                Ok(Body::Stream(dict, self.pos))
            }
            token => Err(self.error_at(
                start,
                format!(
                    "expected `endobj` or `stream`, found {}",
                    describe_token(token, self.data.get(start))
                ),
            )),
        }
    }

    /// After stream data ending at `end`: an optional end-of-line marker (CRLF, LF or
    /// CR, 7.3.8.1), `endstream`, `endobj`. Nothing else may come between the data and
    /// `endstream`, so that a wrong /Length is detected rather than absorbed.
    pub(crate) fn stream_end(&mut self, end: usize) -> Result<()> {
        self.pos = end;
        match (self.peek(), self.data.get(self.pos + 1)) {
            (Some(b'\r'), Some(b'\n')) => self.pos += 2,
            (Some(b'\r' | b'\n'), _) => self.pos += 1,
            _ => {}
        }
        let start = self.pos;
        if self.token() != b"endstream" {
            return Err(self.error_at(
                start,
                "no `endstream` where the stream's /Length says its data ends",
            ));
        }
        self.expect_keyword("endobj")
    }

    /// One object, after white space and comments.
    pub(crate) fn object(&mut self) -> Result<Object> {
        self.value(0)
    }

    fn value(&mut self, depth: usize) -> Result<Object> {
        self.skip_whitespace();
        let start = self.pos;
        let Some(byte) = self.peek() else {
            return Err(self.error("unexpected end of data, expected an object"));
        };
        match byte {
            b'/' => {
                self.pos += 1;
                Ok(Object::Name(self.name()?))
            }
            b'(' => {
                self.pos += 1;
                Ok(Object::String(self.literal_string()?))
            }
            b'<' if self.data.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                self.enter(depth)?;
                Ok(Object::Dict(self.dict(depth + 1)?))
            }
            b'<' => {
                self.pos += 1;
                Ok(Object::String(self.hex_string()?))
            }
            b'[' => {
                self.pos += 1;
                self.enter(depth)?;
                let mut items = Vec::new();
                loop {
                    self.skip_whitespace();
                    match self.peek() {
                        Some(b']') => {
                            self.pos += 1;
                            return Ok(Object::Array(items));
                        }
                        None => return Err(self.error("unterminated array")),
                        Some(_) => items.push(self.value(depth + 1)?),
                    }
                }
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => self.number(),
            b'{' | b'}' => Err(Error::unsupported(
                Some(self.location()),
                "PostScript calculator braces outside a content stream",
            )),
            _ => {
                let token = self.token();
                match token {
                    b"true" => Ok(Object::Bool(true)),
                    b"false" => Ok(Object::Bool(false)),
                    b"null" => Ok(Object::Null),
                    _ => Err(self.error_at(
                        start,
                        format!(
                            "expected an object, found {}",
                            describe_token(token, self.data.get(start))
                        ),
                    )),
                }
            }
        }
    }

    fn enter(&self, depth: usize) -> Result<()> {
        if depth >= self.max_depth {
            Err(Error::LimitExceeded {
                limit: "max_depth",
                value: self.max_depth,
            })
        } else {
            Ok(())
        }
    }

    fn dict(&mut self, depth: usize) -> Result<Dict> {
        let mut dict = Dict::default();
        loop {
            self.skip_whitespace();
            let start = self.pos;
            match self.peek() {
                Some(b'>') => {
                    if self.data.get(self.pos + 1) == Some(&b'>') {
                        self.pos += 2;
                        return Ok(dict);
                    }
                    return Err(self.error("`>` alone in a dictionary"));
                }
                Some(b'/') => {
                    self.pos += 1;
                    let key = self.name()?;
                    let value = self.value(depth)?;
                    match dict.0.entry(key) {
                        Entry::Vacant(entry) => {
                            entry.insert(value);
                        }
                        Entry::Occupied(entry) => {
                            return Err(self.error_at(
                                start,
                                format!(
                                    "key {} repeated in a dictionary",
                                    display_name(entry.key())
                                ),
                            ));
                        }
                    }
                }
                None => return Err(self.error("unterminated dictionary")),
                Some(_) => {
                    return Err(self.error("expected a name as dictionary key"));
                }
            }
        }
    }

    /// A name, after its `/` (7.3.5): regular characters, `#xx` escapes.
    fn name(&mut self) -> Result<Vec<u8>> {
        let mut name = Vec::new();
        while let Some(byte) = self.peek() {
            if !is_regular(byte) {
                break;
            }
            if byte == b'#' {
                let high = self.data.get(self.pos + 1).copied().and_then(hex_value);
                let low = self.data.get(self.pos + 2).copied().and_then(hex_value);
                let (Some(high), Some(low)) = (high, low) else {
                    return Err(self.error("`#` in a name not followed by two hexadecimal digits"));
                };
                name.push(high << 4 | low);
                self.pos += 3;
            } else {
                name.push(byte);
                self.pos += 1;
            }
        }
        Ok(name)
    }

    /// A literal string, after its `(` (7.3.4.2).
    fn literal_string(&mut self) -> Result<Vec<u8>> {
        let start = self.pos.saturating_sub(1);
        let mut out = Vec::new();
        let mut open = 0usize;
        loop {
            let Some(byte) = self.peek() else {
                return Err(self.error_at(start, "unterminated literal string"));
            };
            self.pos += 1;
            match byte {
                b'\\' => {
                    let Some(escaped) = self.peek() else {
                        return Err(self.error_at(start, "unterminated literal string"));
                    };
                    self.pos += 1;
                    match escaped {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0C),
                        b'0'..=b'7' => {
                            let mut value = u32::from(escaped - b'0');
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(digit @ b'0'..=b'7') => {
                                        value = value * 8 + u32::from(digit - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            // High-order overflow is ignored (7.3.4.2).
                            out.push(value.to_le_bytes()[0]);
                        }
                        // A backslash at the end of a line continues the string.
                        b'\r' => {
                            if self.peek() == Some(b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        // \( \) \\ and any other character: the character itself.
                        other => out.push(other),
                    }
                }
                b'(' => {
                    open += 1;
                    out.push(byte);
                }
                b')' => {
                    if open == 0 {
                        return Ok(out);
                    }
                    open -= 1;
                    out.push(byte);
                }
                // An end-of-line marker in a string is read as LF.
                b'\r' => {
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                    out.push(b'\n');
                }
                _ => out.push(byte),
            }
        }
    }

    /// A hexadecimal string, after its `<` (7.3.4.3).
    fn hex_string(&mut self) -> Result<Vec<u8>> {
        let start = self.pos.saturating_sub(1);
        let mut out = Vec::new();
        let mut high: Option<u8> = None;
        loop {
            let Some(byte) = self.peek() else {
                return Err(self.error_at(start, "unterminated hexadecimal string"));
            };
            self.pos += 1;
            if byte == b'>' {
                if let Some(high) = high {
                    out.push(high << 4);
                }
                return Ok(out);
            }
            if is_whitespace(byte) {
                continue;
            }
            let Some(value) = hex_value(byte) else {
                return Err(self.error_at(
                    self.pos - 1,
                    "a character that is not a hexadecimal digit in a hexadecimal string",
                ));
            };
            match high.take() {
                Some(h) => out.push(h << 4 | value),
                None => high = Some(value),
            }
        }
    }

    /// A number (7.3.3), or an indirect reference `num gen R` (7.3.10).
    fn number(&mut self) -> Result<Object> {
        let start = self.pos;
        let token = self.token();
        let (sign, digits) = match token.first() {
            Some(b'+' | b'-') => (true, &token[1..]),
            _ => (false, token),
        };
        let dots = digits.split(|&b| b == b'.').count() - 1;
        if digits.is_empty()
            || dots > 1
            || !digits.iter().all(|&b| b.is_ascii_digit() || b == b'.')
            || digits == b"."
        {
            return Err(self.error_at(
                start,
                format!(
                    "malformed number {}",
                    describe_token(token, self.data.get(start))
                ),
            ));
        }
        if dots == 1 {
            return Ok(Object::Real);
        }
        let text =
            std::str::from_utf8(token).map_err(|_| self.error_at(start, "malformed number"))?;
        let value: i64 = text
            .parse()
            .map_err(|_| self.error_at(start, format!("integer {text} out of range")))?;
        if !sign
            && let Ok(num) = u32::try_from(value)
            && let Some(generation) = self.reference_tail()
        {
            return Ok(Object::Ref(ObjRef { num, generation }));
        }
        Ok(Object::Integer(value))
    }

    /// After an unsigned integer: ` gen R` when it follows (consumed), else nothing
    /// (position unchanged).
    fn reference_tail(&mut self) -> Option<u16> {
        let saved = self.pos;
        self.skip_whitespace();
        let generation = parse_unsigned(self.token()).and_then(|g| u16::try_from(g).ok());
        if let Some(generation) = generation {
            self.skip_whitespace();
            if self.token() == b"R" {
                return Some(generation);
            }
        }
        self.pos = saved;
        None
    }
}

/// The value of a token made of decimal digits only.
pub(crate) fn parse_unsigned(token: &[u8]) -> Option<u64> {
    if token.is_empty() || !token.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(token).ok()?.parse().ok()
}

/// A token for an error message: its text, or the character found when it is empty.
fn describe_token(token: &[u8], at: Option<&u8>) -> String {
    if token.is_empty() {
        match at {
            None => "the end of the data".to_owned(),
            Some(byte) if byte.is_ascii_graphic() => format!("`{}`", char::from(*byte)),
            Some(byte) => format!("byte 0x{byte:02X}"),
        }
    } else {
        let shown: String = token
            .iter()
            .take(40)
            .map(|&b| {
                if b.is_ascii_graphic() {
                    char::from(b)
                } else {
                    '?'
                }
            })
            .collect();
        format!("`{shown}`")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &[u8]) -> Result<Object> {
        Parser::new(text, 0, 32).object()
    }

    fn dict(text: &[u8]) -> Dict {
        match parse(text).unwrap() {
            Object::Dict(d) => d,
            other => panic!("not a dictionary: {other:?}"),
        }
    }

    #[test]
    fn scalars() {
        assert_eq!(parse(b"true").unwrap(), Object::Bool(true));
        assert_eq!(parse(b" false ").unwrap(), Object::Bool(false));
        assert_eq!(parse(b"null").unwrap(), Object::Null);
        assert_eq!(parse(b"-17").unwrap(), Object::Integer(-17));
        assert_eq!(parse(b"+17").unwrap(), Object::Integer(17));
        assert_eq!(parse(b"0042").unwrap(), Object::Integer(42));
        assert_eq!(parse(b"3.14").unwrap(), Object::Real);
        assert_eq!(parse(b"-.5").unwrap(), Object::Real);
        assert_eq!(parse(b"4.").unwrap(), Object::Real);
        assert!(parse(b"1.2.3").is_err());
        assert!(parse(b"--1").is_err());
        assert!(parse(b"12abc").is_err());
        assert!(parse(b"99999999999999999999").is_err());
        assert!(parse(b"bogus").is_err());
        assert!(parse(b"").is_err());
    }

    #[test]
    fn references() {
        assert_eq!(
            parse(b"12 0 R").unwrap(),
            Object::Ref(ObjRef {
                num: 12,
                generation: 0
            })
        );
        assert_eq!(
            parse(b"[1 2 R 3]").unwrap(),
            Object::Array(vec![
                Object::Ref(ObjRef {
                    num: 1,
                    generation: 2
                }),
                Object::Integer(3)
            ])
        );
        assert_eq!(
            parse(b"[1 2 3]").unwrap(),
            Object::Array(vec![
                Object::Integer(1),
                Object::Integer(2),
                Object::Integer(3)
            ])
        );
        // A generation number above 65535 is not a reference.
        assert!(parse(b"[1 70000 R]").is_err());
        // Neither is a signed object number.
        assert!(parse(b"[+1 0 R]").is_err());
    }

    #[test]
    fn names() {
        assert_eq!(parse(b"/Type").unwrap(), Object::Name(b"Type".to_vec()));
        assert_eq!(
            parse(b"/text#2Fxml").unwrap(),
            Object::Name(b"text/xml".to_vec())
        );
        assert_eq!(parse(b"/").unwrap(), Object::Name(Vec::new()));
        assert!(parse(b"/a#zz").is_err());
        assert!(parse(b"/a#4").is_err());
    }

    #[test]
    fn literal_strings() {
        assert_eq!(
            parse(b"(a (nested) b)").unwrap(),
            Object::String(b"a (nested) b".to_vec())
        );
        assert_eq!(
            parse(b"(\\n\\r\\t\\b\\f\\(\\)\\\\\\q)").unwrap(),
            Object::String(b"\n\r\t\x08\x0C()\\q".to_vec())
        );
        assert_eq!(
            parse(b"(\\101\\0601\\7\\777)").unwrap(),
            Object::String(b"A01\x07\xFF".to_vec())
        );
        assert_eq!(
            parse(b"(line\\\r\ncontinued\r\nnext\rlast)").unwrap(),
            Object::String(b"linecontinued\nnext\nlast".to_vec())
        );
        assert!(parse(b"(unbalanced (paren)").is_err());
        assert!(parse(b"(trailing backslash\\").is_err());
    }

    #[test]
    fn hex_strings() {
        assert_eq!(
            parse(b"<48 65 6c6C 6f>").unwrap(),
            Object::String(b"Hello".to_vec())
        );
        assert_eq!(
            parse(b"<901FA>").unwrap(),
            Object::String(vec![0x90, 0x1F, 0xA0])
        );
        assert_eq!(parse(b"<>").unwrap(), Object::String(Vec::new()));
        assert!(parse(b"<12G4>").is_err());
        assert!(parse(b"<1234").is_err());
    }

    #[test]
    fn dictionaries_and_comments() {
        let d = dict(b"<< /A 1 % comment\n /B [ /C (x) ] /N null /D << /E 2 0 R >> >>");
        assert_eq!(d.get(b"A"), Some(&Object::Integer(1)));
        assert_eq!(d.get(b"N"), None);
        assert!(d.0.contains_key(b"N".as_slice()));
        assert!(matches!(d.get(b"D"), Some(Object::Dict(_))));
        assert!(parse(b"<< /A 1 /A 2 >>").is_err());
        assert!(parse(b"<< /A 1 > >>").is_err());
        assert!(parse(b"<< 1 2 >>").is_err());
        assert!(parse(b"<< /A >>").is_err());
        assert!(parse(b"<< /A 1").is_err());
        assert!(parse(b"{ 1 }").is_err());
    }

    #[test]
    fn nesting_is_bounded() {
        let deep = "[".repeat(100_000);
        assert!(matches!(
            parse(deep.as_bytes()),
            Err(Error::LimitExceeded {
                limit: "max_depth",
                ..
            })
        ));
        let mut ok = "[".repeat(31);
        ok.push_str(&"]".repeat(31));
        assert!(parse(ok.as_bytes()).is_ok());
        let mut too_deep = "[".repeat(33);
        too_deep.push_str(&"]".repeat(33));
        assert!(parse(too_deep.as_bytes()).is_err());
        let dicts = "<< /A ".repeat(40);
        assert!(parse(dicts.as_bytes()).is_err());
    }

    #[test]
    fn indirect_objects() {
        let data = b"7 0 obj\n<< /Length 3 >>\nstream\r\nabc\nendstream\nendobj";
        let mut p = Parser::new(data, 0, 32);
        assert_eq!(
            p.object_header().unwrap(),
            ObjRef {
                num: 7,
                generation: 0
            }
        );
        let Body::Stream(d, start) = p.object_body().unwrap() else {
            panic!("not a stream")
        };
        assert_eq!(d.get(b"Length"), Some(&Object::Integer(3)));
        assert_eq!(&data[start..start + 3], b"abc");
        p.stream_end(start + 3).unwrap();
        assert!(p.stream_end(start + 2).is_err());

        let mut p = Parser::new(b"1 0 obj << >> stream\rabc", 0, 32);
        p.object_header().unwrap();
        assert!(p.object_body().is_err());
        let mut p = Parser::new(b"1 0 obj 5 stream\nabc", 0, 32);
        p.object_header().unwrap();
        assert!(p.object_body().is_err());
        let mut p = Parser::new(b"1 0 obj 5 6", 0, 32);
        p.object_header().unwrap();
        assert!(p.object_body().is_err());
        assert!(Parser::new(b"1 0 ob", 0, 32).object_header().is_err());
        assert!(
            Parser::new(b"5000000000 0 obj", 0, 32)
                .object_header()
                .is_err()
        );
        assert!(Parser::new(b"1 65536 obj", 0, 32).object_header().is_err());
    }
}
