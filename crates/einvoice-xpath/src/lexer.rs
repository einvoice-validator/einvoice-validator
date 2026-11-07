//! The XPath 2.0 tokenizer.
//!
//! Tokens are produced on demand, so that an attribute value template can stop the lexer at
//! the `}` that closes an expression. Which names are keywords depends on where they occur,
//! so the lexer reports every name as [`Tok::Name`] and leaves keywords to the parser; it
//! does apply the longest-match rule for names (`a-b` is one name) and the delimiter rule
//! for numeric literals (`10div` is an error).

use crate::error::{Error, ErrorKind, Result};
use crate::names::{is_name_char, is_name_start_char, is_xml_whitespace};

/// A token kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tok {
    /// An `NCName` or a `QName` written without whitespace around the colon.
    Name {
        prefix: Option<String>,
        local: String,
    },
    /// `prefix:*`
    PrefixWildcard(String),
    /// `*:local`
    LocalWildcard(String),
    Star,
    String(String),
    Integer(String),
    Decimal(String),
    Double(String),
    Dollar,
    LParen,
    RParen,
    LBracket,
    RBracket,
    RBrace,
    Comma,
    Slash,
    DoubleSlash,
    At,
    Dot,
    DotDot,
    ColonColon,
    Pipe,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// `<<`
    Precedes,
    /// `>>`
    Follows,
    Plus,
    Minus,
    Question,
    Eof,
}

impl Tok {
    /// How the token reads in an error message.
    pub(crate) fn describe(&self) -> String {
        let fixed = match self {
            Self::Name { prefix, local } => {
                return match prefix {
                    Some(p) => format!("name `{p}:{local}`"),
                    None => format!("name `{local}`"),
                };
            }
            Self::PrefixWildcard(p) => return format!("`{p}:*`"),
            Self::LocalWildcard(l) => return format!("`*:{l}`"),
            Self::String(_) => "a string literal",
            Self::Integer(n) | Self::Decimal(n) | Self::Double(n) => {
                return format!("number `{n}`");
            }
            Self::Star => "`*`",
            Self::Dollar => "`$`",
            Self::LParen => "`(`",
            Self::RParen => "`)`",
            Self::LBracket => "`[`",
            Self::RBracket => "`]`",
            Self::RBrace => "`}`",
            Self::Comma => "`,`",
            Self::Slash => "`/`",
            Self::DoubleSlash => "`//`",
            Self::At => "`@`",
            Self::Dot => "`.`",
            Self::DotDot => "`..`",
            Self::ColonColon => "`::`",
            Self::Pipe => "`|`",
            Self::Eq => "`=`",
            Self::Ne => "`!=`",
            Self::Lt => "`<`",
            Self::Le => "`<=`",
            Self::Gt => "`>`",
            Self::Ge => "`>=`",
            Self::Precedes => "`<<`",
            Self::Follows => "`>>`",
            Self::Plus => "`+`",
            Self::Minus => "`-`",
            Self::Question => "`?`",
            Self::Eof => "the end of the expression",
        };
        fixed.to_owned()
    }

    /// Whether the token is the unprefixed name `word`.
    pub(crate) fn is_word(&self, word: &str) -> bool {
        matches!(self, Self::Name { prefix: None, local } if local == word)
    }
}

/// A token and the byte range it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Token {
    pub(crate) tok: Tok,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// The tokenizer state: the text and the current byte offset.
#[derive(Debug)]
pub(crate) struct Lexer<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    /// A lexer that starts at byte `pos` of `src`.
    pub(crate) fn new(src: &'a str, pos: usize) -> Self {
        Self { src, pos }
    }

    fn rest(&self) -> &'a str {
        self.src.get(self.pos..).unwrap_or("")
    }

    fn peek_char(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn peek_char_at(&self, n: usize) -> Option<char> {
        self.rest().chars().nth(n)
    }

    fn error(&self, kind: ErrorKind, offset: usize, message: impl Into<String>) -> Error {
        Error::new(kind, self.src, offset, message.into())
    }

    /// The next token, skipping whitespace.
    pub(crate) fn next_token(&mut self) -> Result<Token> {
        while self.peek_char().is_some_and(is_xml_whitespace) {
            self.pos += 1;
        }
        let start = self.pos;
        let Some(c) = self.peek_char() else {
            return Ok(Token {
                tok: Tok::Eof,
                start,
                end: start,
            });
        };
        let tok = if is_name_start_char(c) {
            self.name()
        } else if c.is_ascii_digit()
            || (c == '.' && self.peek_char_at(1).is_some_and(|d| d.is_ascii_digit()))
        {
            self.number()?
        } else if c == '"' || c == '\'' {
            self.string(c)?
        } else {
            self.symbol(c)?
        };
        Ok(Token {
            tok,
            start,
            end: self.pos,
        })
    }

    fn ncname(&mut self) -> String {
        let rest = self.rest();
        let len = rest
            .char_indices()
            .find(|&(i, c)| {
                if i == 0 {
                    !is_name_start_char(c)
                } else {
                    !is_name_char(c)
                }
            })
            .map_or(rest.len(), |(i, _)| i);
        self.pos += len;
        rest.get(..len).unwrap_or("").to_owned()
    }

    /// `NCName`, `prefix:local` or `prefix:*`; `name::` is left for the axis separator.
    fn name(&mut self) -> Tok {
        let first = self.ncname();
        if self.peek_char() == Some(':') {
            match self.peek_char_at(1) {
                Some(c) if is_name_start_char(c) => {
                    self.pos += 1;
                    let local = self.ncname();
                    return Tok::Name {
                        prefix: Some(first),
                        local,
                    };
                }
                Some('*') => {
                    self.pos += 2;
                    return Tok::PrefixWildcard(first);
                }
                _ => {}
            }
        }
        Tok::Name {
            prefix: None,
            local: first,
        }
    }

    fn digits(&mut self) -> usize {
        let rest = self.rest();
        let len = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        self.pos += len;
        len
    }

    /// \[71\] `IntegerLiteral`, \[72\] `DecimalLiteral`, \[73\] `DoubleLiteral`.
    fn number(&mut self) -> Result<Tok> {
        let start = self.pos;
        self.digits();
        let mut decimal = false;
        if self.peek_char() == Some('.') {
            decimal = true;
            self.pos += 1;
            self.digits();
        }
        let mut double = false;
        if matches!(self.peek_char(), Some('e' | 'E')) {
            double = true;
            self.pos += 1;
            if matches!(self.peek_char(), Some('+' | '-')) {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return Err(self.error(
                    ErrorKind::Syntax,
                    start,
                    "malformed double literal: no digits after the exponent mark",
                ));
            }
        }
        if let Some(c) = self.peek_char()
            && (is_name_start_char(c) || c == '.')
        {
            return Err(self.error(
                ErrorKind::Syntax,
                self.pos,
                format!("numeric literal followed by `{c}` without a delimiter"),
            ));
        }
        let text = self.src.get(start..self.pos).unwrap_or("").to_owned();
        Ok(if double {
            Tok::Double(text)
        } else if decimal {
            Tok::Decimal(text)
        } else {
            Tok::Integer(text)
        })
    }

    /// \[74\] `StringLiteral`, with doubled delimiters as escapes.
    fn string(&mut self, quote: char) -> Result<Tok> {
        let start = self.pos;
        self.pos += 1;
        let mut value = String::new();
        loop {
            let Some(c) = self.peek_char() else {
                return Err(self.error(ErrorKind::Syntax, start, "unterminated string literal"));
            };
            self.pos += c.len_utf8();
            if c == quote {
                if self.peek_char() == Some(quote) {
                    self.pos += 1;
                    value.push(quote);
                } else {
                    return Ok(Tok::String(value));
                }
            } else {
                value.push(c);
            }
        }
    }

    fn symbol(&mut self, c: char) -> Result<Tok> {
        let start = self.pos;
        let next = self.peek_char_at(1);
        let (tok, len) = match (c, next) {
            ('(', Some(':')) => {
                return Err(self.error(ErrorKind::Unsupported, start, "XPath comment `(: :)`"));
            }
            ('(', _) => (Tok::LParen, 1),
            (')', _) => (Tok::RParen, 1),
            ('[', _) => (Tok::LBracket, 1),
            (']', _) => (Tok::RBracket, 1),
            ('}', _) => (Tok::RBrace, 1),
            (',', _) => (Tok::Comma, 1),
            ('@', _) => (Tok::At, 1),
            ('$', _) => (Tok::Dollar, 1),
            ('?', _) => (Tok::Question, 1),
            ('+', _) => (Tok::Plus, 1),
            ('-', _) => (Tok::Minus, 1),
            ('/', Some('/')) => (Tok::DoubleSlash, 2),
            ('/', _) => (Tok::Slash, 1),
            ('.', Some('.')) => (Tok::DotDot, 2),
            ('.', _) => (Tok::Dot, 1),
            (':', Some(':')) => (Tok::ColonColon, 2),
            (':', Some('=')) => {
                return Err(self.error(
                    ErrorKind::Unsupported,
                    start,
                    "`:=` (let expression, XPath 3.0)",
                ));
            }
            ('|', Some('|')) => {
                return Err(self.error(
                    ErrorKind::Unsupported,
                    start,
                    "string concatenation operator `||` (XPath 3.0)",
                ));
            }
            ('|', _) => (Tok::Pipe, 1),
            ('=', Some('>')) => {
                return Err(self.error(
                    ErrorKind::Unsupported,
                    start,
                    "arrow operator `=>` (XPath 3.1)",
                ));
            }
            ('=', _) => (Tok::Eq, 1),
            ('!', Some('=')) => (Tok::Ne, 2),
            ('!', _) => {
                return Err(self.error(
                    ErrorKind::Unsupported,
                    start,
                    "simple map operator `!` (XPath 3.0)",
                ));
            }
            ('<', Some('=')) => (Tok::Le, 2),
            ('<', Some('<')) => (Tok::Precedes, 2),
            ('<', _) => (Tok::Lt, 1),
            ('>', Some('=')) => (Tok::Ge, 2),
            ('>', Some('>')) => (Tok::Follows, 2),
            ('>', _) => (Tok::Gt, 1),
            ('*', Some(':')) if self.peek_char_at(2).is_some_and(is_name_start_char) => {
                self.pos += 2;
                return Ok(Tok::LocalWildcard(self.ncname()));
            }
            ('*', _) => (Tok::Star, 1),
            _ => {
                return Err(self.error(
                    ErrorKind::Syntax,
                    start,
                    format!("unexpected character `{c}`"),
                ));
            }
        };
        self.pos += len;
        Ok(tok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(src: &str) -> Vec<Tok> {
        let mut lexer = Lexer::new(src, 0);
        let mut out = Vec::new();
        loop {
            let token = lexer.next_token().unwrap();
            if token.tok == Tok::Eof {
                return out;
            }
            out.push(token.tok);
        }
    }

    fn name(local: &str) -> Tok {
        Tok::Name {
            prefix: None,
            local: local.into(),
        }
    }

    #[test]
    fn names_take_the_longest_match() {
        assert_eq!(lex("a-b"), vec![name("a-b")]);
        assert_eq!(lex("a - b"), vec![name("a"), Tok::Minus, name("b")]);
        assert_eq!(
            lex("$XR-SKONTO-REGEX"),
            vec![Tok::Dollar, name("XR-SKONTO-REGEX")]
        );
        assert_eq!(
            lex("child::x"),
            vec![name("child"), Tok::ColonColon, name("x")]
        );
        assert_eq!(
            lex("p:x p:* *:x *"),
            vec![
                Tok::Name {
                    prefix: Some("p".into()),
                    local: "x".into()
                },
                Tok::PrefixWildcard("p".into()),
                Tok::LocalWildcard("x".into()),
                Tok::Star
            ]
        );
    }

    #[test]
    fn numbers_keep_their_kind_and_text() {
        assert_eq!(
            lex("1 1.50 .5 1. 1e3 1.5E-2"),
            vec![
                Tok::Integer("1".into()),
                Tok::Decimal("1.50".into()),
                Tok::Decimal(".5".into()),
                Tok::Decimal("1.".into()),
                Tok::Double("1e3".into()),
                Tok::Double("1.5E-2".into()),
            ]
        );
        assert!(Lexer::new("10div 3", 0).next_token().is_err());
        assert!(Lexer::new("1.2.3", 0).next_token().is_err());
        assert!(Lexer::new("1e", 0).next_token().is_err());
    }

    #[test]
    fn strings_unescape_doubled_quotes() {
        assert_eq!(lex("'it''s'"), vec![Tok::String("it's".into())]);
        assert_eq!(lex(r#""a""b""#), vec![Tok::String(r#"a"b"#.into())]);
        assert!(Lexer::new("'open", 0).next_token().is_err());
    }

    #[test]
    fn operators() {
        assert_eq!(
            lex("!= <= << >= >> // .. :: | = < >"),
            vec![
                Tok::Ne,
                Tok::Le,
                Tok::Precedes,
                Tok::Ge,
                Tok::Follows,
                Tok::DoubleSlash,
                Tok::DotDot,
                Tok::ColonColon,
                Tok::Pipe,
                Tok::Eq,
                Tok::Lt,
                Tok::Gt
            ]
        );
        for unsupported in ["(: c :)", "a || b", "a ! b", "a => b", "$x := 1"] {
            let mut lexer = Lexer::new(unsupported, 0);
            let err = loop {
                match lexer.next_token() {
                    Ok(t) if t.tok == Tok::Eof => panic!("{unsupported}: no error"),
                    Ok(_) => {}
                    Err(e) => break e,
                }
            };
            assert_eq!(err.kind, ErrorKind::Unsupported, "{unsupported}");
        }
    }
}
