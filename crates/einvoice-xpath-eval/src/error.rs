//! Static and dynamic errors, with the error codes of XPath 2.0, F&O 2.0 and XSLT 2.0
//! that Saxon-HE reports.

use std::fmt;

use einvoice_xdm::{ErrorCode as XdmCode, XdmError};

use crate::limit::LimitExceeded;

/// The error code of a failed compilation or evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorCode {
    /// `XPST0008`: a variable that is not in scope (static).
    Xpst0008,
    /// `XPST0017`: no function with that name and arity (static).
    Xpst0017,
    /// `XPDY0002`: the context item is absent.
    Xpdy0002,
    /// `XPDY0050`: the root of the tree containing the context item is not a document node.
    Xpdy0050,
    /// `XPTY0004`: type error.
    Xpty0004,
    /// `XPTY0018`: the last step of a path returns both nodes and atomic values.
    Xpty0018,
    /// `XPTY0019`: a step of a path (other than the last) returns an atomic value.
    Xpty0019,
    /// `XPTY0020`: the context item of an axis step is not a node.
    Xpty0020,
    /// `FOAR0001`: division by zero.
    Foar0001,
    /// `FOAR0002`: numeric operation overflow.
    Foar0002,
    /// `FOCA0002`: invalid lexical value.
    Foca0002,
    /// `FOCH0001`: a code point that is not a valid XML character.
    Foch0001,
    /// `FODT0001`: date/time value out of range.
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
    /// `XTDE1360`: `current()` where no current item exists (XSLT).
    Xtde1360,
    /// `XTDE1390`: the argument of `system-property()` is not a valid `QName` (XSLT).
    Xtde1390,
    /// An error reported by the host (the XSLT layer): variables, user functions,
    /// `document()`. The code is in the message.
    Host,
    /// Not an XPath error: a valid construct or value that this crate refuses on purpose
    /// rather than evaluating it differently from the reference engine.
    Unsupported,
    /// Not an XPath error: a broken invariant of this crate (a bug).
    Internal,
    /// Not an XPath error: the evaluation went beyond a limit that keeps it within a
    /// bounded call stack ([`Error::limit`]).
    Limit,
}

impl ErrorCode {
    /// The code as written in the specifications, e.g. `"XPTY0004"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Xpst0008 => "XPST0008",
            Self::Xpst0017 => "XPST0017",
            Self::Xpdy0002 => "XPDY0002",
            Self::Xpdy0050 => "XPDY0050",
            Self::Xpty0004 => "XPTY0004",
            Self::Xpty0018 => "XPTY0018",
            Self::Xpty0019 => "XPTY0019",
            Self::Xpty0020 => "XPTY0020",
            Self::Foar0001 => "FOAR0001",
            Self::Foar0002 => "FOAR0002",
            Self::Foca0002 => "FOCA0002",
            Self::Foch0001 => "FOCH0001",
            Self::Fodt0001 => "FODT0001",
            Self::Forg0001 => "FORG0001",
            Self::Forg0006 => "FORG0006",
            Self::Forx0001 => "FORX0001",
            Self::Forx0002 => "FORX0002",
            Self::Forx0003 => "FORX0003",
            Self::Forx0004 => "FORX0004",
            Self::Xtde1360 => "XTDE1360",
            Self::Xtde1390 => "XTDE1390",
            Self::Host => "HOST",
            Self::Unsupported => "UNSUPPORTED",
            Self::Internal => "INTERNAL",
            Self::Limit => "LIMIT",
        }
    }

    /// Whether the error is raised when an expression is compiled rather than evaluated.
    #[must_use]
    pub const fn is_static(self) -> bool {
        matches!(self, Self::Xpst0008 | Self::Xpst0017)
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A failed compilation or evaluation. One pointer wide: the results of the
/// evaluator, one on every level of its recursion, stay small.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(Box<Inner>);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Inner {
    code: ErrorCode,
    message: String,
    limit: Option<LimitExceeded>,
}

impl Error {
    /// A new error.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self(Box::new(Inner {
            code,
            message: message.into(),
            limit: None,
        }))
    }

    /// The error of an evaluation beyond a limit ([`ErrorCode::Limit`]).
    #[must_use]
    pub fn limit_exceeded(limit: LimitExceeded) -> Self {
        Self(Box::new(Inner {
            code: ErrorCode::Limit,
            message: limit.to_string(),
            limit: Some(limit),
        }))
    }

    /// The limit exceeded, for an [`ErrorCode::Limit`] error.
    #[must_use]
    pub fn limit(&self) -> Option<LimitExceeded> {
        self.0.limit
    }

    /// The error code.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        self.0.code
    }

    /// The explanation.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.0.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.0.code, self.0.message)
    }
}

impl std::error::Error for Error {}

impl From<XdmError> for Error {
    fn from(e: XdmError) -> Self {
        let code = match e.code() {
            XdmCode::Foar0001 => ErrorCode::Foar0001,
            XdmCode::Foar0002 => ErrorCode::Foar0002,
            XdmCode::Foca0002 => ErrorCode::Foca0002,
            XdmCode::Fodt0001 => ErrorCode::Fodt0001,
            XdmCode::Forg0001 => ErrorCode::Forg0001,
            XdmCode::Forg0006 => ErrorCode::Forg0006,
            XdmCode::Forx0001 => ErrorCode::Forx0001,
            XdmCode::Forx0002 => ErrorCode::Forx0002,
            XdmCode::Forx0003 => ErrorCode::Forx0003,
            XdmCode::Forx0004 => ErrorCode::Forx0004,
            XdmCode::Xpty0004 => ErrorCode::Xpty0004,
            _ => ErrorCode::Unsupported,
        };
        Self::new(code, e.message())
    }
}

/// The result of a compilation or evaluation.
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn err<T>(code: ErrorCode, message: impl Into<String>) -> Result<T> {
    Err(Error::new(code, message))
}
