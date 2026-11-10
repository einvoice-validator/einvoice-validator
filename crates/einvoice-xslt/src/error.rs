//! Load errors.

use std::fmt;

/// What is wrong with the stylesheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadErrorKind {
    /// The text is not well-formed XML.
    Xml(String),
    /// An XSLT element outside the supported subset.
    UnsupportedElement {
        /// The element, as `xsl:local-name`.
        element: String,
    },
    /// An attribute outside the supported subset (or not allowed by XSLT).
    UnsupportedAttribute {
        /// The element.
        element: String,
        /// The attribute.
        attribute: String,
    },
    /// An attribute value outside the supported subset.
    UnsupportedValue {
        /// The element.
        element: String,
        /// The attribute.
        attribute: String,
        /// The value.
        value: String,
    },
    /// A required attribute is missing.
    MissingAttribute {
        /// The element.
        element: String,
        /// The attribute.
        attribute: String,
    },
    /// An XPath expression, pattern, sequence type or attribute value template does not
    /// parse, or is outside the supported subset.
    XPath {
        /// The element.
        element: String,
        /// The attribute.
        attribute: String,
        /// The parser's error.
        error: einvoice_xpath::Error,
    },
    /// Any other static error of the stylesheet; the message names the XSLT error code.
    Invalid(String),
    /// Instructions or literal result elements nested deeper than `max` levels
    /// ([`MAX_NESTING`](crate::MAX_NESTING)): the stylesheet is refused rather than read,
    /// prepared and run with a call stack in proportion to its depth.
    TooDeep {
        /// The maximum nesting.
        max: u32,
    },
}

/// One load error, with the line where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError {
    /// The URI of the stylesheet module, for a stylesheet loaded with its imports
    /// ([`load_with_imports`](crate::load_with_imports)).
    pub module: Option<String>,
    /// The 1-based line in the stylesheet module.
    pub line: u32,
    /// What is wrong.
    pub kind: LoadErrorKind,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(module) = &self.module {
            write!(f, "{module}: ")?;
        }
        write!(f, "line {}: ", self.line)?;
        match &self.kind {
            LoadErrorKind::Xml(message) => write!(f, "not well-formed XML: {message}"),
            LoadErrorKind::UnsupportedElement { element } => {
                write!(f, "unsupported XSLT element {element}")
            }
            LoadErrorKind::UnsupportedAttribute { element, attribute } => {
                write!(f, "unsupported attribute `{attribute}` on {element}")
            }
            LoadErrorKind::UnsupportedValue {
                element,
                attribute,
                value,
            } => write!(
                f,
                "unsupported value `{value}` of attribute `{attribute}` on {element}"
            ),
            LoadErrorKind::MissingAttribute { element, attribute } => {
                write!(f, "{element} needs a `{attribute}` attribute")
            }
            LoadErrorKind::XPath {
                element,
                attribute,
                error,
            } => write!(f, "{element}/@{attribute}: {error}"),
            LoadErrorKind::Invalid(message) => f.write_str(message),
            LoadErrorKind::TooDeep { max } => write!(
                f,
                "instructions nested deeper than the maximum of {max} levels"
            ),
        }
    }
}

impl std::error::Error for LoadError {}

/// Every error found while loading a stylesheet (at least one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadErrors(pub Vec<LoadError>);

impl fmt::Display for LoadErrors {
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

impl std::error::Error for LoadErrors {}
