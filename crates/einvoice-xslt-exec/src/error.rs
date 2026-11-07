//! Errors: refusals and static errors found when a stylesheet is prepared, dynamic errors
//! raised by a transformation.

use std::fmt;

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransformError {
    code: String,
    message: String,
    line: Option<u32>,
}

impl TransformError {
    /// A new error with an XPath, XSLT or serialization error code.
    #[must_use]
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            line: None,
        }
    }

    /// The error code (`XPTY0004`, `XTDE0640`, ...). `UNSUPPORTED` marks a valid
    /// construct or value that this crate refuses rather than evaluating it differently
    /// from the reference engine, `INTERNAL` a bug.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// The explanation.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The line of the stylesheet instruction that raised the error, when known.
    #[must_use]
    pub const fn line(&self) -> Option<u32> {
        self.line
    }

    /// Sets the line if none is set yet (the innermost instruction wins).
    #[must_use]
    pub(crate) fn at(mut self, line: u32) -> Self {
        if self.line.is_none() {
            self.line = Some(line);
        }
        self
    }
}

impl fmt::Display for TransformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(line) = self.line {
            write!(f, "line {line}: ")?;
        }
        write!(f, "{}: {}", self.code, self.message)
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
