//! Errors raised by operations on atomic values.

use std::fmt;

/// The error code of a failed operation: an XPath 2.0 / F&O 2.0 error code, or
/// [`ErrorCode::Unsupported`] for an operation this crate refuses on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorCode {
    /// `FOAR0001`: division by zero.
    Foar0001,
    /// `FOAR0002`: numeric operation overflow (`idiv` of NaN or infinity).
    Foar0002,
    /// `FOCA0002`: invalid lexical value (NaN or infinity cast to a decimal or an integer).
    Foca0002,
    /// `FODT0001`: date/time value out of the supported range.
    Fodt0001,
    /// `FORG0001`: invalid value for a cast or constructor.
    Forg0001,
    /// `FORG0006`: invalid argument type (effective boolean value, `sum`).
    Forg0006,
    /// `FORX0001`: invalid regular expression flags.
    Forx0001,
    /// `FORX0002`: invalid regular expression.
    Forx0002,
    /// `FORX0003`: the regular expression matches a zero-length string.
    Forx0003,
    /// `FORX0004`: invalid replacement string.
    Forx0004,
    /// `XPTY0004`: type error.
    Xpty0004,
    /// Not an XPath error: a valid operation that this crate does not implement and
    /// refuses explicitly rather than evaluating differently from the reference engine.
    Unsupported,
}

impl ErrorCode {
    /// The code as written in the specifications, e.g. `"FORG0001"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Foar0001 => "FOAR0001",
            Self::Foar0002 => "FOAR0002",
            Self::Foca0002 => "FOCA0002",
            Self::Fodt0001 => "FODT0001",
            Self::Forg0001 => "FORG0001",
            Self::Forg0006 => "FORG0006",
            Self::Forx0001 => "FORX0001",
            Self::Forx0002 => "FORX0002",
            Self::Forx0003 => "FORX0003",
            Self::Forx0004 => "FORX0004",
            Self::Xpty0004 => "XPTY0004",
            Self::Unsupported => "UNSUPPORTED",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A failed operation: its error code and a human-readable explanation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XdmError {
    code: ErrorCode,
    message: String,
}

impl XdmError {
    /// A new error.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// The error code.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    /// The explanation.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for XdmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for XdmError {}

/// Shorthand for the result of an operation on atomic values.
pub type Result<T> = std::result::Result<T, XdmError>;

pub(crate) fn err<T>(code: ErrorCode, message: impl Into<String>) -> Result<T> {
    Err(XdmError::new(code, message))
}
