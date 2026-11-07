//! Regular expressions of `fn:matches`, `fn:replace` and `fn:tokenize`.
//!
//! The XPath 2.0 dialect (XSD regular expressions plus `^`, `$`, reluctant
//! quantifiers and back-references) is translated into the syntax of the `regex`
//! crate, with the semantics of Saxon-HE 9.9 made explicit:
//!
//! * `.` matches any character except `\n` and `\r`;
//! * `\s` is `[ \t\n\r]` only, `\d` the decimal digits of Unicode 6.2 (Saxon's
//!   tables), not those of the toolchain's Unicode version;
//! * `^` and `$` match only at the start and the end of the whole string (`$` does
//!   not match before a final newline);
//! * every literal character is written as an escaped code point.
//!
//! Only what the official rule sets need is accepted. Anything else is refused at
//! compile time with an error, never translated approximately: flags, back-references,
//! `\i \c \w \p{..}` and their complements, character class subtraction,
//! non-capturing groups and other XPath 3.0 syntax.

use std::fmt::Write as _;

use crate::error::{ErrorCode, Result, XdmError, err};
use crate::tables::SAXON_ND;

/// A compiled XPath regular expression.
#[derive(Debug, Clone)]
pub struct XPathRegex {
    pattern: String,
    regex: regex::Regex,
    groups: usize,
}

fn syntax(pattern: &str, reason: &str) -> XdmError {
    XdmError::new(
        ErrorCode::Forx0002,
        format!("Invalid regular expression \"{pattern}\": {reason}"),
    )
}

fn unsupported(pattern: &str, what: &str) -> XdmError {
    XdmError::new(
        ErrorCode::Unsupported,
        format!("Regular expression \"{pattern}\": {what} is not supported"),
    )
}

/// A character class as a list of inclusive ranges and nested negated classes,
/// written in `regex` syntax.
#[derive(Debug, Default)]
struct ClassBuilder {
    items: String,
}

impl ClassBuilder {
    fn char(&mut self, c: char) {
        let _ = write!(self.items, "\\x{{{:X}}}", u32::from(c));
    }

    fn range(&mut self, low: char, high: char) {
        let _ = write!(
            self.items,
            "\\x{{{:X}}}-\\x{{{:X}}}",
            u32::from(low),
            u32::from(high)
        );
    }

    fn raw(&mut self, text: &str) {
        self.items.push_str(text);
    }
}

/// `[ \t\n\r]` written as class items.
const SPACE_ITEMS: &str = "\\x{20}\\x{9}\\x{A}\\x{D}";

fn digit_items() -> String {
    let mut items = String::new();
    for &(low, high) in SAXON_ND {
        let _ = write!(items, "\\x{{{low:X}}}-\\x{{{high:X}}}");
    }
    items
}

/// A multi-character escape (`\s \S \d \D`) as a class, or `None` for a
/// single-character escape (returned separately).
enum Escape {
    Char(char),
    Class { items: String, negated: bool },
}

struct Translator<'a> {
    pattern: &'a str,
    chars: Vec<char>,
    index: usize,
    out: String,
    groups: usize,
}

impl Translator<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.index).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.index + offset).copied()
    }

    fn error(&self, reason: &str) -> XdmError {
        syntax(self.pattern, reason)
    }

    fn translate(&mut self) -> Result<()> {
        self.branches(0)?;
        if self.index != self.chars.len() {
            return Err(self.error("unexpected ')'"));
        }
        Ok(())
    }

    /// regExp ::= branch ( '|' branch )*
    fn branches(&mut self, depth: usize) -> Result<()> {
        loop {
            self.branch(depth)?;
            match self.peek() {
                Some('|') => {
                    self.index += 1;
                    self.out.push('|');
                }
                _ => return Ok(()),
            }
        }
    }

    /// branch ::= piece*
    fn branch(&mut self, depth: usize) -> Result<()> {
        while let Some(c) = self.peek() {
            match c {
                '|' => return Ok(()),
                ')' => {
                    if depth == 0 {
                        return Err(self.error("unmatched ')'"));
                    }
                    return Ok(());
                }
                _ => self.piece(depth)?,
            }
        }
        Ok(())
    }

    /// piece ::= atom quantifier?
    fn piece(&mut self, depth: usize) -> Result<()> {
        let quantifiable = self.atom(depth)?;
        let Some(c) = self.peek() else {
            return Ok(());
        };
        if !matches!(c, '?' | '*' | '+' | '{') {
            return Ok(());
        }
        if !quantifiable {
            return Err(self.error("quantifier after an anchor"));
        }
        match c {
            '?' | '*' | '+' => {
                self.index += 1;
                self.out.push(c);
            }
            _ => self.quantity()?,
        }
        if self.peek() == Some('?') {
            // Reluctant quantifier (XPath 2.0).
            self.index += 1;
            self.out.push('?');
        }
        Ok(())
    }

    /// '{' n '}' | '{' n ',' '}' | '{' n ',' m '}'
    fn quantity(&mut self) -> Result<()> {
        self.index += 1;
        let low = self
            .number()
            .ok_or_else(|| self.error("expected a number after '{'"))?;
        let mut high = Some(low);
        if self.peek() == Some(',') {
            self.index += 1;
            high = self.number();
            if let Some(h) = high
                && h < low
            {
                return Err(self.error("quantifier range out of order"));
            }
        }
        if self.peek() != Some('}') {
            return Err(self.error("expected '}'"));
        }
        self.index += 1;
        match high {
            Some(h) if h == low => {
                let _ = write!(self.out, "{{{low}}}");
            }
            Some(h) => {
                let _ = write!(self.out, "{{{low},{h}}}");
            }
            None => {
                let _ = write!(self.out, "{{{low},}}");
            }
        }
        Ok(())
    }

    fn number(&mut self) -> Option<u32> {
        let start = self.index;
        let mut value: u32 = 0;
        while let Some(d) = self
            .peek()
            .and_then(|c| c.to_digit(10).filter(|_| c.is_ascii_digit()))
        {
            value = value.checked_mul(10)?.checked_add(d)?;
            self.index += 1;
        }
        (self.index > start).then_some(value)
    }

    /// One atom; returns whether a quantifier may follow.
    fn atom(&mut self, depth: usize) -> Result<bool> {
        let Some(c) = self.peek() else {
            return Ok(false);
        };
        match c {
            '^' => {
                self.index += 1;
                self.out.push_str("\\A");
                Ok(false)
            }
            '$' => {
                self.index += 1;
                self.out.push_str("\\z");
                Ok(false)
            }
            '.' => {
                self.index += 1;
                self.out.push_str("[^\\x{A}\\x{D}]");
                Ok(true)
            }
            '(' => {
                self.index += 1;
                if self.peek() == Some('?') {
                    return Err(unsupported(self.pattern, "a group starting with '(?'"));
                }
                self.groups += 1;
                self.out.push('(');
                self.branches(depth + 1)?;
                if self.peek() != Some(')') {
                    return Err(self.error("missing ')'"));
                }
                self.index += 1;
                self.out.push(')');
                Ok(true)
            }
            '[' => {
                self.class()?;
                Ok(true)
            }
            '\\' => {
                match self.escape(false)? {
                    Escape::Char(c) => {
                        let _ = write!(self.out, "\\x{{{:X}}}", u32::from(c));
                    }
                    Escape::Class { items, negated } => {
                        let caret = if negated { "^" } else { "" };
                        let _ = write!(self.out, "[{caret}{items}]");
                    }
                }
                Ok(true)
            }
            '?' | '*' | '+' | '{' => Err(self.error("quantifier without an atom")),
            '}' | ']' => Err(self.error(&format!("unescaped '{c}'"))),
            _ => {
                self.index += 1;
                let _ = write!(self.out, "\\x{{{:X}}}", u32::from(c));
                Ok(true)
            }
        }
    }

    /// An escape sequence starting at the backslash.
    fn escape(&mut self, in_class: bool) -> Result<Escape> {
        self.index += 1;
        let Some(c) = self.peek() else {
            return Err(self.error("escape at the end of the pattern"));
        };
        self.index += 1;
        Ok(match c {
            'n' => Escape::Char('\n'),
            'r' => Escape::Char('\r'),
            't' => Escape::Char('\t'),
            '\\' | '|' | '.' | '-' | '^' | '?' | '*' | '+' | '{' | '}' | '(' | ')' | '[' | ']'
            | '$' => Escape::Char(c),
            's' | 'S' => Escape::Class {
                items: SPACE_ITEMS.to_owned(),
                negated: c == 'S',
            },
            'd' | 'D' => Escape::Class {
                items: digit_items(),
                negated: c == 'D',
            },
            'i' | 'I' | 'c' | 'C' | 'w' | 'W' | 'p' | 'P' => {
                return Err(unsupported(self.pattern, &format!("the escape \\{c}")));
            }
            '1'..='9' if !in_class => {
                return Err(unsupported(self.pattern, "a back-reference"));
            }
            _ => return Err(self.error(&format!("invalid escape \\{c}"))),
        })
    }

    /// A character class expression `[...]`.
    fn class(&mut self) -> Result<()> {
        self.index += 1;
        let mut builder = ClassBuilder::default();
        let negated = self.peek() == Some('^');
        if negated {
            self.index += 1;
        }
        if self.peek() == Some(']') {
            return Err(self.error("empty character class"));
        }
        let mut first = true;
        loop {
            let Some(c) = self.peek() else {
                return Err(self.error("missing ']'"));
            };
            if c == ']' {
                self.index += 1;
                break;
            }
            if c == '[' {
                return Err(self.error("unescaped '[' within a character class"));
            }
            if c == '-' {
                if self.peek_at(1) == Some('[') {
                    return Err(unsupported(self.pattern, "character class subtraction"));
                }
                if first || self.peek_at(1) == Some(']') {
                    self.index += 1;
                    builder.char('-');
                    first = false;
                    continue;
                }
                return Err(unsupported(
                    self.pattern,
                    "a hyphen inside a character class other than first or last",
                ));
            }
            first = false;
            let low = if c == '\\' {
                match self.escape(true)? {
                    Escape::Char(ch) => ch,
                    Escape::Class { items, negated } => {
                        if negated {
                            builder.raw(&format!("[^{items}]"));
                        } else {
                            builder.raw(&items);
                        }
                        continue;
                    }
                }
            } else {
                self.index += 1;
                c
            };
            if self.peek() == Some('-') && self.peek_at(1).is_some_and(|n| n != ']') {
                if self.peek_at(1) == Some('[') {
                    return Err(unsupported(self.pattern, "character class subtraction"));
                }
                self.index += 1;
                let Some(h) = self.peek() else {
                    return Err(self.error("missing ']'"));
                };
                let high = if h == '\\' {
                    match self.escape(true)? {
                        Escape::Char(ch) => ch,
                        Escape::Class { .. } => {
                            return Err(self.error("multi-character escape cannot end a range"));
                        }
                    }
                } else if h == '[' || h == '-' {
                    return Err(unsupported(self.pattern, "this character class range"));
                } else {
                    self.index += 1;
                    h
                };
                if low > high {
                    return Err(self.error("character range out of order"));
                }
                builder.range(low, high);
            } else {
                builder.char(low);
            }
        }
        let caret = if negated { "^" } else { "" };
        let _ = write!(self.out, "[{caret}{}]", builder.items);
        Ok(())
    }
}

impl XPathRegex {
    /// Compiles an XPath regular expression with the given flags (only the empty
    /// flag string is supported).
    ///
    /// # Errors
    ///
    /// `FORX0001` for invalid flags, `FORX0002` for an invalid expression,
    /// `Unsupported` for valid syntax this crate does not implement.
    pub fn new(pattern: &str, flags: &str) -> Result<Self> {
        if let Some(bad) = flags.chars().find(|c| !"smixq".contains(*c)) {
            return err(
                ErrorCode::Forx0001,
                format!("Invalid character '{bad}' in regular expression flags"),
            );
        }
        if !flags.is_empty() {
            return err(
                ErrorCode::Unsupported,
                format!("Regular expression flags \"{flags}\" are not supported"),
            );
        }
        let mut translator = Translator {
            pattern,
            chars: pattern.chars().collect(),
            index: 0,
            out: String::with_capacity(pattern.len() * 8),
            groups: 0,
        };
        translator.translate()?;
        let regex = regex::RegexBuilder::new(&translator.out)
            .size_limit(1 << 24)
            .build()
            .map_err(|e| syntax(pattern, &e.to_string()))?;
        Ok(Self {
            pattern: pattern.to_owned(),
            regex,
            groups: translator.groups,
        })
    }

    /// The pattern as written in XPath.
    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// The translated pattern, in `regex` crate syntax.
    #[must_use]
    pub fn translated(&self) -> &str {
        self.regex.as_str()
    }

    /// `fn:matches`: whether the input contains a match.
    #[must_use]
    pub fn is_match(&self, input: &str) -> bool {
        self.regex.is_match(input)
    }

    fn check_not_empty_match(&self) -> Result<()> {
        if self.regex.is_match("") {
            return err(
                ErrorCode::Forx0003,
                format!(
                    "The regular expression \"{}\" must not be one that matches a zero-length string",
                    self.pattern
                ),
            );
        }
        Ok(())
    }

    /// `fn:replace`: every non-overlapping match replaced, `$N` in the replacement
    /// standing for the Nth captured group (`$0` the whole match) and `\$`, `\\` for
    /// the characters.
    ///
    /// # Errors
    ///
    /// `FORX0003` when the expression matches the empty string, `FORX0004` for an
    /// invalid replacement string.
    pub fn replace(&self, input: &str, replacement: &str) -> Result<String> {
        self.check_not_empty_match()?;
        let template = parse_replacement(replacement, self.groups)?;
        let mut out = String::with_capacity(input.len());
        let mut last = 0;
        for captures in self.regex.captures_iter(input) {
            let Some(whole) = captures.get(0) else {
                continue;
            };
            out.push_str(&input[last..whole.start()]);
            for part in &template {
                match part {
                    ReplacementPart::Text(text) => out.push_str(text),
                    ReplacementPart::Group(n) => {
                        if let Some(m) = captures.get(*n) {
                            out.push_str(m.as_str());
                        }
                    }
                }
            }
            last = whole.end();
        }
        out.push_str(&input[last..]);
        Ok(out)
    }

    /// `fn:tokenize`: the substrings between matches; empty input gives no tokens.
    ///
    /// # Errors
    ///
    /// `FORX0003` when the expression matches the empty string.
    pub fn tokenize<'a>(&self, input: &'a str) -> Result<Vec<&'a str>> {
        self.check_not_empty_match()?;
        if input.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self.regex.split(input).collect())
    }
}

#[derive(Debug)]
enum ReplacementPart {
    Text(String),
    Group(usize),
}

/// Saxon's reading of a replacement string (`Replace.checkReplacement` then
/// `REMatcher.replace`): with at most 9 groups `$` takes one digit, otherwise as
/// many digits as keep the number within the group count; a group number beyond
/// the count stands for the empty string.
fn parse_replacement(replacement: &str, groups: usize) -> Result<Vec<ReplacementPart>> {
    let chars: Vec<char> = replacement.chars().collect();
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut index = 0;
    let invalid = |reason: &str| {
        XdmError::new(
            ErrorCode::Forx0004,
            format!("Invalid replacement string in replace(): {reason}"),
        )
    };
    while let Some(&c) = chars.get(index) {
        index += 1;
        match c {
            '\\' => match chars.get(index) {
                Some(&next @ ('\\' | '$')) => {
                    text.push(next);
                    index += 1;
                }
                Some(_) => return Err(invalid("\\ character must be followed by \\ or $")),
                None => return Err(invalid("\\ character at end of string")),
            },
            '$' => {
                let Some(first) = chars
                    .get(index)
                    .and_then(|c| c.to_digit(10).filter(|_| c.is_ascii_digit()))
                else {
                    return Err(invalid("$ sign must be followed by digit 0-9"));
                };
                index += 1;
                let mut n = usize::try_from(first).unwrap_or(0);
                if groups > 9 {
                    while let Some(d) = chars
                        .get(index)
                        .and_then(|c| c.to_digit(10).filter(|_| c.is_ascii_digit()))
                    {
                        let m = n * 10 + usize::try_from(d).unwrap_or(0);
                        if m > groups {
                            break;
                        }
                        n = m;
                        index += 1;
                    }
                }
                if !text.is_empty() {
                    parts.push(ReplacementPart::Text(std::mem::take(&mut text)));
                }
                if n <= groups {
                    parts.push(ReplacementPart::Group(n));
                }
            }
            _ => text.push(c),
        }
    }
    if !text.is_empty() {
        parts.push(ReplacementPart::Text(text));
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn re(p: &str) -> XPathRegex {
        XPathRegex::new(p, "").unwrap()
    }

    #[test]
    fn xpath_semantics() {
        assert!(!re("^.$").is_match("\r"));
        assert!(!re("^.$").is_match("\n"));
        assert!(re("^.$").is_match("\u{1F600}"));
        assert!(!re("abc$").is_match("abc\n"));
        assert!(!re("^\\s$").is_match("\u{A0}"));
        assert!(re("^\\s$").is_match("\r"));
        assert!(re("^\\d$").is_match("\u{663}"));
        assert!(!re("^\\d$").is_match("\u{A9F0}"));
        assert!(re("^[^@\\s]+@([^@.\\s]+\\.)+[^@.\\s]+$").is_match("a@b.c"));
        assert!(
            re("^(0?[1-9]|[12][0-9]|3[01])[-\\\\/ ]?(0?[1-9]|1[0-2])[-\\\\/ ]?(19|20)[0-9]{2}")
                .is_match("31/12/2020")
        );
        assert!(re("#.+#").is_match("a#b#"));
        assert!(re("^([a-zA-Z])([a-zA-Z0-9+.-])+:.*").is_match("https://x"));
    }

    #[test]
    fn refusals() {
        for p in [
            "\\p{L}",
            "\\w",
            "\\i",
            "(a)\\1",
            "(?:a)",
            "[a-z-[aeiou]]",
            "[a-c-e]",
        ] {
            assert_eq!(
                XPathRegex::new(p, "").unwrap_err().code(),
                ErrorCode::Unsupported,
                "{p}"
            );
        }
        for p in ["[", "(", "a)", "*", "a{2,1}", "\\", "\\q", "[]", "[b-a]"] {
            assert_eq!(
                XPathRegex::new(p, "").unwrap_err().code(),
                ErrorCode::Forx0002,
                "{p}"
            );
        }
        assert_eq!(
            XPathRegex::new("a", "i").unwrap_err().code(),
            ErrorCode::Unsupported
        );
        assert_eq!(
            XPathRegex::new("a", "k").unwrap_err().code(),
            ErrorCode::Forx0001
        );
    }

    #[test]
    fn replace_and_tokenize() {
        assert_eq!(re("\\s").replace(" a b ", "").unwrap(), "ab");
        assert_eq!(re("(a)(b)").replace("abab", "$2$1\\$").unwrap(), "ba$ba$");
        assert_eq!(re("\\s").tokenize(" a b ").unwrap(), vec!["", "a", "b", ""]);
        assert_eq!(re("\\s").tokenize("").unwrap(), Vec::<&str>::new());
        assert_eq!(
            re("x*").tokenize("a").unwrap_err().code(),
            ErrorCode::Forx0003
        );
        assert_eq!(
            re("a").replace("a", "$").unwrap_err().code(),
            ErrorCode::Forx0004
        );
    }
}
