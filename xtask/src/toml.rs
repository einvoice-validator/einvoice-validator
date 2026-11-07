//! A strict subset of TOML 1.0, enough for the mutation catalogue
//! (`corpus/mutations.toml`): the file is a sequence of `[[name]]` array-of-tables
//! headers, each followed by `key = value` lines, where a value is a string
//! (basic `"..."` with the TOML escapes, or literal `'...'`) or an array of
//! values, which may span lines and end with a comma. Comments (`#`) and blank
//! lines are allowed anywhere a TOML document allows them.
//!
//! Everything else TOML has (top-level keys, `[table]` headers, dotted or
//! quoted keys, numbers, booleans, dates, inline tables, multi-line strings)
//! is refused with the line where it occurs, so that a file this parser reads
//! means the same to any TOML reader.

use crate::util::{Error, Result};

/// A value: a string or an array of values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Value {
    String(String),
    Array(Vec<Value>),
}

/// One `[[name]]` table: its header name, the line of its header (1-based) and
/// its keys in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Table {
    pub(crate) name: String,
    pub(crate) line: usize,
    pub(crate) entries: Vec<(String, Value)>,
}

/// Parses `text`; `file` names it in errors.
pub(crate) fn parse(text: &str, file: &str) -> Result<Vec<Table>> {
    let mut parser = Parser { text, pos: 0, file };
    parser.document()
}

struct Parser<'a> {
    text: &'a str,
    pos: usize,
    file: &'a str,
}

impl Parser<'_> {
    fn line(&self) -> usize {
        self.text[..self.pos].matches('\n').count() + 1
    }

    fn error(&self, message: &str) -> Error {
        Error::new(format!("{}:{}: {message}", self.file, self.line()))
    }

    fn peek(&self) -> Option<char> {
        self.text[self.pos..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn eat(&mut self, s: &str) -> bool {
        if self.text[self.pos..].starts_with(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    /// Spaces and tabs.
    fn blanks(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.pos += 1;
        }
    }

    /// A comment up to (not including) the end of the line.
    fn comment(&mut self) -> Result<()> {
        if self.peek() == Some('#') {
            while let Some(c) = self.peek() {
                if c == '\n' || c == '\r' {
                    break;
                }
                if c.is_control() && c != '\t' {
                    return Err(self.error("control character in a comment"));
                }
                self.pos += c.len_utf8();
            }
        }
        Ok(())
    }

    /// The end of a line: blanks, an optional comment, then a newline or the
    /// end of the text.
    fn end_of_line(&mut self) -> Result<()> {
        self.blanks();
        self.comment()?;
        if self.eat("\r\n") || self.eat("\n") || self.pos == self.text.len() {
            Ok(())
        } else {
            Err(self.error("unexpected text at the end of the line"))
        }
    }

    /// Blanks, comments and newlines, inside an array.
    fn skip_space(&mut self) -> Result<()> {
        loop {
            self.blanks();
            self.comment()?;
            if !(self.eat("\r\n") || self.eat("\n")) {
                return Ok(());
            }
        }
    }

    fn document(&mut self) -> Result<Vec<Table>> {
        let mut tables: Vec<Table> = Vec::new();
        while self.pos < self.text.len() {
            self.blanks();
            match self.peek() {
                None => break,
                Some('#' | '\r' | '\n') => self.end_of_line()?,
                Some('[') => {
                    let line = self.line();
                    if !self.eat("[[") {
                        return Err(self.error("only [[array of tables]] headers are supported"));
                    }
                    let name = self.bare_key()?;
                    if !self.eat("]]") {
                        return Err(self.error("expected ]] after the table name"));
                    }
                    self.end_of_line()?;
                    tables.push(Table {
                        name,
                        line,
                        entries: Vec::new(),
                    });
                }
                Some(_) => {
                    let key = self.bare_key()?;
                    self.blanks();
                    if !self.eat("=") {
                        return Err(self.error("expected = after the key"));
                    }
                    self.blanks();
                    let value = self.value()?;
                    self.end_of_line()?;
                    let error = self.error(&format!("key {key} outside a [[table]]"));
                    let table = tables.last_mut().ok_or(error)?;
                    if table.entries.iter().any(|(k, _)| *k == key) {
                        return Err(self.error(&format!("duplicate key {key}")));
                    }
                    table.entries.push((key, value));
                }
            }
        }
        Ok(tables)
    }

    fn bare_key(&mut self) -> Result<String> {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            self.pos += 1;
        }
        if self.pos == start {
            return Err(self.error("expected a bare key (A-Z a-z 0-9 _ -)"));
        }
        Ok(self.text[start..self.pos].to_owned())
    }

    fn value(&mut self) -> Result<Value> {
        match self.peek() {
            Some('"') => {
                if self.text[self.pos..].starts_with("\"\"\"") {
                    return Err(self.error("multi-line strings are not supported"));
                }
                self.pos += 1;
                self.basic_string().map(Value::String)
            }
            Some('\'') => {
                if self.text[self.pos..].starts_with("'''") {
                    return Err(self.error("multi-line strings are not supported"));
                }
                self.pos += 1;
                self.literal_string().map(Value::String)
            }
            Some('[') => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_space()?;
                    if self.eat("]") {
                        return Ok(Value::Array(items));
                    }
                    items.push(self.value()?);
                    self.skip_space()?;
                    if self.eat("]") {
                        return Ok(Value::Array(items));
                    }
                    if !self.eat(",") {
                        return Err(self.error("expected , or ] in an array"));
                    }
                }
            }
            _ => Err(self.error("only strings and arrays are supported as values")),
        }
    }

    fn basic_string(&mut self) -> Result<String> {
        let mut out = String::new();
        loop {
            let c = self
                .bump()
                .ok_or_else(|| self.error("unterminated string"))?;
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let escape = self
                        .bump()
                        .ok_or_else(|| self.error("unterminated string"))?;
                    match escape {
                        'b' => out.push('\u{8}'),
                        't' => out.push('\t'),
                        'n' => out.push('\n'),
                        'f' => out.push('\u{c}'),
                        'r' => out.push('\r'),
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        'u' => out.push(self.unicode_escape(4)?),
                        'U' => out.push(self.unicode_escape(8)?),
                        other => {
                            return Err(self.error(&format!("unknown escape \\{other}")));
                        }
                    }
                }
                c if c.is_control() && c != '\t' => {
                    return Err(self.error("control character in a string (use an escape)"));
                }
                c => out.push(c),
            }
        }
    }

    fn unicode_escape(&mut self, digits: usize) -> Result<char> {
        let hex = self
            .text
            .get(self.pos..self.pos + digits)
            .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| self.error("bad unicode escape"))?;
        self.pos += digits;
        u32::from_str_radix(hex, 16)
            .ok()
            .and_then(char::from_u32)
            .ok_or_else(|| self.error("unicode escape is not a scalar value"))
    }

    fn literal_string(&mut self) -> Result<String> {
        let start = self.pos;
        loop {
            let c = self
                .bump()
                .ok_or_else(|| self.error("unterminated string"))?;
            if c == '\'' {
                return Ok(self.text[start..self.pos - 1].to_owned());
            }
            if c.is_control() && c != '\t' {
                return Err(self.error("control character in a literal string"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Table, Value, parse};

    fn s(text: &str) -> Value {
        Value::String(text.to_owned())
    }

    #[test]
    fn reads_tables_strings_and_arrays() {
        let text = "# head\n\n[[m]] # c\nid = \"a\\tb\\u00e9\\\"\"\nraw = 'C:\\x'\nedits = [\n  [\"set\", '/x'], # one\n  [\"remove\", \"/y\"],\n]\n[[m]]\nid=\"b\"\n";
        let tables = parse(text, "t").unwrap();
        assert_eq!(
            tables,
            vec![
                Table {
                    name: "m".to_owned(),
                    line: 3,
                    entries: vec![
                        ("id".to_owned(), s("a\tb\u{e9}\"")),
                        ("raw".to_owned(), s("C:\\x")),
                        (
                            "edits".to_owned(),
                            Value::Array(vec![
                                Value::Array(vec![s("set"), s("/x")]),
                                Value::Array(vec![s("remove"), s("/y")]),
                            ])
                        ),
                    ],
                },
                Table {
                    name: "m".to_owned(),
                    line: 10,
                    entries: vec![("id".to_owned(), s("b"))],
                },
            ]
        );
    }

    #[test]
    fn refuses_what_it_does_not_support() {
        for (text, message) in [
            ("a = \"x\"\n", "outside a [[table]]"),
            ("[t]\n", "only [[array of tables]]"),
            ("[[t]]\na = 1\n", "only strings and arrays"),
            ("[[t]]\na = true\n", "only strings and arrays"),
            ("[[t]]\na = \"\"\"x\"\"\"\n", "multi-line"),
            ("[[t]]\na = \"x\"\na = \"y\"\n", "duplicate key a"),
            ("[[t]]\na = \"x\" b\n", "end of the line"),
            ("[[t]]\na = \"\\q\"\n", "unknown escape"),
            ("[[t]]\na = \"x\n", "control character"),
            ("[[t]]\na.b = \"x\"\n", "expected ="),
            ("[[t]]\na = [\"x\" \"y\"]\n", "expected , or ]"),
            ("[[t]]\na = \"\\ud800\"\n", "not a scalar value"),
        ] {
            let error = parse(text, "f").unwrap_err().to_string();
            assert!(error.contains(message), "{text:?}: {error}");
            assert!(error.starts_with("f:"), "{error}");
        }
    }
}
