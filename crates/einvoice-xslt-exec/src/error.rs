//! Errors: refusals and static errors found when a stylesheet is prepared, dynamic errors
//! raised by a transformation.

use std::fmt;

use einvoice_xpath_eval::LimitExceeded;

/// The code of [`TransformError::limit_exceeded`].
pub(crate) const LIMIT: &str = "LIMIT";

/// A problem found while preparing a stylesheet: an XPath expression that does not
/// compile (unknown function or variable, ...), a static error of the stylesheet, or a
/// construct outside the supported subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareError {
    /// The 1-based line of the stylesheet element.
    pub line: u32,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for PrepareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for PrepareError {}

/// Every problem found while preparing a stylesheet (at least one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareErrors(pub Vec<PrepareError>);

impl fmt::Display for PrepareErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, error) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for PrepareErrors {}

/// A dynamic error that ended a transformation.
///
/// Its contents are boxed: a `Result` of the executor is then a pointer wide, which keeps
/// small the frames of the functions every level of recursion through the templates goes
/// through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransformError(Box<Inner>);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Inner {
    code: String,
    message: String,
    line: Option<u32>,
    limit: Option<LimitExceeded>,
}

impl TransformError {
    /// A new error with an XPath, XSLT or serialization error code.
    #[must_use]
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self(Box::new(Inner {
            code: code.to_owned(),
            message: message.into(),
            line: None,
            limit: None,
        }))
    }

    /// The error for a limit exceeded, with the code `LIMIT`.
    #[must_use]
    pub fn limit_exceeded(limit: LimitExceeded) -> Self {
        Self(Box::new(Inner {
            code: LIMIT.to_owned(),
            message: limit.to_string(),
            line: None,
            limit: Some(limit),
        }))
    }

    /// The error code (`XPTY0004`, `XTDE0640`, ...). `UNSUPPORTED` marks a valid
    /// construct or value that this crate refuses rather than evaluating it differently
    /// from the reference engine, `LIMIT` a document or a transformation beyond a limit
    /// of [`TransformOptions`](crate::TransformOptions) (see [`TransformError::limit`]),
    /// `INTERNAL` a bug.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.0.code
    }

    /// The limit exceeded, for an error with the code `LIMIT`: an element of the
    /// document, or of a document loaded by `document()`, nested deeper than
    /// [`TransformOptions::max_depth`](crate::TransformOptions::max_depth), or
    /// templates and functions invoked one inside another more than
    /// [`TransformOptions::max_nesting`](crate::TransformOptions::max_nesting) deep.
    #[must_use]
    pub fn limit(&self) -> Option<LimitExceeded> {
        self.0.limit
    }

    /// The explanation.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.0.message
    }

    /// The line of the stylesheet instruction that raised the error, when known.
    #[must_use]
    pub fn line(&self) -> Option<u32> {
        self.0.line
    }

    /// The message prefixed with the URI of the document concerned (a document loaded
    /// by `document()`).
    #[must_use]
    pub(crate) fn in_document(mut self, uri: &str) -> Self {
        self.0.message = format!("{uri}: {}", self.0.message);
        self
    }

    /// Sets the line if none is set yet (the innermost instruction wins).
    #[must_use]
    pub(crate) fn at(mut self, line: u32) -> Self {
        if self.0.line.is_none() {
            self.0.line = Some(line);
        }
        self
    }
}

impl fmt::Display for TransformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(line) = self.0.line {
            write!(f, "line {line}: ")?;
        }
        write!(f, "{}: {}", self.0.code, self.0.message)
    }
}

impl std::error::Error for TransformError {}

impl From<einvoice_xpath_eval::Error> for TransformError {
    fn from(e: einvoice_xpath_eval::Error) -> Self {
        Self::new(e.code().as_str(), e.message())
    }
}

impl From<einvoice_xdm::XdmError> for TransformError {
    fn from(e: einvoice_xdm::XdmError) -> Self {
        einvoice_xpath_eval::Error::from(e).into()
    }
}

/// The result of a transformation step.
pub(crate) type Result<T> = std::result::Result<T, TransformError>;

pub(crate) fn err<T>(code: &str, message: impl Into<String>) -> Result<T> {
    Err(TransformError::new(code, message))
}
