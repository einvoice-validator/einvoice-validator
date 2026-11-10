//! Errors of schema loading, document parsing and validation.

use std::fmt;

/// Why a schema set could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError {
    /// What kind of failure.
    pub kind: LoadErrorKind,
    /// The schema document, as its path relative to the root of the schema set.
    pub file: String,
    /// 1-based line in that document (0 when the error concerns the whole file).
    pub line: u32,
    /// What went wrong.
    pub message: String,
}

/// The kinds of [`LoadError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoadErrorKind {
    /// A schema document is missing from the schema set.
    MissingFile,
    /// A schema document is not well-formed XML.
    Malformed,
    /// A construct that is valid XML Schema 1.0 but outside the subset this crate
    /// implements: refused rather than ignored or approximated.
    Unsupported,
    /// The schemas are not a correct XML Schema 1.0 schema set (unresolved
    /// reference, duplicate definition, circular derivation, non-deterministic
    /// content model, ...).
    Invalid,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.kind {
            LoadErrorKind::MissingFile => "missing schema document",
            LoadErrorKind::Malformed => "malformed schema document",
            LoadErrorKind::Unsupported => "unsupported schema construct",
            LoadErrorKind::Invalid => "invalid schema",
        };
        write!(f, "{}:{}: {kind}: {}", self.file, self.line, self.message)
    }
}

impl std::error::Error for LoadError {}

/// Why a document could not be validated at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentError {
    /// The document is not well-formed (namespace-well-formed) XML.
    Malformed {
        /// 1-based line.
        line: u32,
        /// 1-based column.
        column: u32,
        /// The parser's message.
        message: String,
    },
    /// The document needs a feature outside the subset this crate implements (an
    /// encoding that [`einvoice_xml::decode`] does not read, an `xs:ENTITY` value in a
    /// document with a document type declaration, whose unparsed entities this crate
    /// does not read, or a value constraint of a declaration taken as a value of a
    /// type valid only in a context, such as `xs:QName`). Refused rather than answered
    /// wrongly.
    Unsupported {
        /// 1-based line.
        line: u32,
        /// 1-based column.
        column: u32,
        /// What is not supported.
        message: String,
    },
}

impl fmt::Display for DocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed {
                line,
                column,
                message,
            } => {
                write!(f, "{line}:{column}: malformed document: {message}")
            }
            Self::Unsupported {
                line,
                column,
                message,
            } => {
                write!(f, "{line}:{column}: unsupported: {message}")
            }
        }
    }
}

impl std::error::Error for DocumentError {}

/// The kind of a [`ValidationError`]. The set and the names of [`ErrorKind::as_str`]
/// are stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ErrorKind {
    /// The document element has no global element declaration (cvc-elt.1).
    UnknownRoot,
    /// A child element is not allowed here by the content model (cvc-complex-type.2.4).
    UnexpectedElement,
    /// The content model needs more child elements (cvc-complex-type.2.4).
    MissingElement,
    /// Non-whitespace text in element-only content (cvc-complex-type.2.3).
    TextNotAllowed,
    /// A child element where the type has simple content (cvc-complex-type.2.2,
    /// cvc-type.3.1.2).
    ElementInSimpleContent,
    /// Text (even whitespace) or a child element where the content type is empty
    /// (cvc-complex-type.2.1).
    ContentNotEmpty,
    /// An element matched by a strict wildcard has no global declaration
    /// (cvc-wildcard, cvc-assess-elt).
    UndeclaredElement,
    /// An attribute matched by a strict attribute wildcard has no global
    /// declaration (cvc-complex-type.3.2.2).
    UndeclaredAttribute,
    /// An attribute the type does not allow (cvc-complex-type.3.2).
    AttributeNotAllowed,
    /// A required attribute is absent (cvc-complex-type.4).
    MissingAttribute,
    /// The text of an element is not a valid value of its simple type (cvc-type.3.1.3,
    /// cvc-complex-type.2.2).
    InvalidValue,
    /// An attribute value is not valid for its type (cvc-attribute.3).
    InvalidAttributeValue,
    /// An `xs:ID` value already used by an earlier attribute of the document (cvc-id.2).
    DuplicateId,
    /// An `xs:IDREF` value (or an item of an `xs:IDREFS`) that is not the value of an
    /// `xs:ID` of the document (cvc-id.1).
    UnresolvedIdref,
    /// `xsi:type` is not a `QName`, does not resolve to a type, or names a type not
    /// validly derived from the declared type (cvc-elt.4).
    InvalidXsiType,
    /// The type of the element is abstract (cvc-elt.2, cvc-type.2).
    AbstractType,
    /// `xsi:nil` on an element whose declaration is not nillable (cvc-elt.3.1).
    NotNillable,
    /// `xsi:nil` with a value that is not a boolean (cvc-elt.3).
    InvalidXsiNil,
}

impl ErrorKind {
    /// A stable identifier of the kind.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownRoot => "unknown-root",
            Self::UnexpectedElement => "unexpected-element",
            Self::MissingElement => "missing-element",
            Self::TextNotAllowed => "text-not-allowed",
            Self::ElementInSimpleContent => "element-in-simple-content",
            Self::ContentNotEmpty => "content-not-empty",
            Self::UndeclaredElement => "undeclared-element",
            Self::UndeclaredAttribute => "undeclared-attribute",
            Self::AttributeNotAllowed => "attribute-not-allowed",
            Self::MissingAttribute => "missing-attribute",
            Self::InvalidValue => "invalid-value",
            Self::InvalidAttributeValue => "invalid-attribute-value",
            Self::DuplicateId => "duplicate-id",
            Self::UnresolvedIdref => "unresolved-idref",
            Self::InvalidXsiType => "invalid-xsi-type",
            Self::AbstractType => "abstract-type",
            Self::NotNillable => "not-nillable",
            Self::InvalidXsiNil => "invalid-xsi-nil",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One validity error of a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// What is wrong.
    pub kind: ErrorKind,
    /// 1-based line of the start tag of the element concerned.
    pub line: u32,
    /// 1-based column (in characters) of that start tag.
    pub column: u32,
    /// The element concerned, in Clark notation (`{namespace}local`).
    pub element: String,
    /// The attribute concerned, in Clark notation, for errors about one attribute
    /// (including `xsi:type` and `xsi:nil`).
    pub attribute: Option<String>,
    /// A description, naming the attribute, value or expected elements involved.
    pub message: String,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: {}: element {}",
            self.line, self.column, self.kind, self.element
        )?;
        if let Some(attribute) = &self.attribute {
            write!(f, ", attribute {attribute}")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// The result of validating a document: its validity errors, sorted by the position
/// of the element they concern; errors of the same element in the order they were
/// found (`xsi:nil`, `xsi:type`, attributes, content).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    /// Every validity error; empty when the document is valid.
    pub errors: Vec<ValidationError>,
}

impl Report {
    /// Whether the document is schema-valid.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}
