//! Parse errors.

use std::fmt;

/// What went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// The text is not valid XPath 2.0 (or not a valid pattern, sequence type or attribute
    /// value template): `err:XPST0003`, or `err:XTSE0350`/`err:XTSE0370` for templates.
    Syntax,
    /// The text is valid but uses a construct outside the supported subset.
    Unsupported,
    /// A prefix is not bound to a namespace (`err:XPST0081`).
    UndeclaredPrefix,
    /// A type name that is not an atomic type (`err:XPST0051`).
    UnknownType,
    /// The expression (or pattern) is nested deeper than [`MAX_DEPTH`](crate::MAX_DEPTH):
    /// it is refused rather than read, compiled and evaluated with a call stack in
    /// proportion to its depth.
    TooDeep,
}

/// A parse error, with its position in the parsed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// What went wrong.
    pub kind: ErrorKind,
    /// The byte offset in the parsed text where the problem was found.
    pub offset: usize,
    /// The column of that offset, counted in characters from 1.
    pub column: usize,
    /// A description naming the offending construct.
    pub message: String,
}

impl Error {
    pub(crate) fn new(kind: ErrorKind, src: &str, offset: usize, message: String) -> Self {
        let column = src.get(..offset).map_or(offset, |s| s.chars().count()) + 1;
        Self {
            kind,
            offset,
            column,
            message,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.kind {
            ErrorKind::Syntax => "syntax error",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::UndeclaredPrefix => "undeclared namespace prefix",
            ErrorKind::UnknownType => "unknown type",
            ErrorKind::TooDeep => "too deep",
        };
        write!(f, "{kind} at column {}: {}", self.column, self.message)
    }
}

impl std::error::Error for Error {}

/// The result of a parse.
pub type Result<T> = std::result::Result<T, Error>;
