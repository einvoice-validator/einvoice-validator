//! The errors of loading a rule set and of validating a document.

use std::fmt;
use std::path::PathBuf;

use einvoice_xpath_eval::LimitExceeded;

/// Why a rule set could not be loaded, or a document could not be validated.
///
/// Validity problems of a document (schema errors, failed asserts) are not errors: they
/// are in the [`Report`](crate::Report). An error means that no complete verdict could
/// be given.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A file could not be read.
    Io {
        /// The file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The bundle is not usable: a manifest that does not follow the format, a missing
    /// file, an unknown schema name, a stylesheet that is not well-formed XSLT, or one
    /// whose result is not an SVRL report.
    Bundle {
        /// What is wrong, naming the file and line when known.
        message: String,
    },
    /// A file of the bundle does not have the SHA-256 its manifest records.
    Checksum {
        /// The file, relative to the bundle.
        path: String,
        /// The SHA-256 the manifest records (lower-case hexadecimal).
        expected: String,
        /// The SHA-256 of the file.
        actual: String,
    },
    /// The document is not well-formed XML (or not namespace-well-formed, or with bytes
    /// that are not characters of its encoding, or first bytes and an XML declaration
    /// that name different encodings).
    NotXml {
        /// 1-based line.
        line: u32,
        /// 1-based column.
        column: u32,
        /// The parser's message.
        message: String,
    },
    /// The document or a rule-set artefact needs a construct that this engine does not
    /// implement (an encoding the validator does not read, a document type declaration,
    /// an XSLT or
    /// XML Schema construct outside the supported subset). Refused rather than answered
    /// differently from the reference engine.
    Unsupported {
        /// What is not supported, and where.
        message: String,
    },
    /// The document exceeds a limit that keeps its validation within a bounded call
    /// stack: an element nested deeper than [`ValidateOptions::max_depth`]
    /// ([`LimitExceeded::Depth`]), in the document or in a document a stylesheet loads
    /// with `document()`; or a stylesheet invoking templates and functions one inside
    /// another beyond the nesting that maximum allows ([`LimitExceeded::Nesting`]).
    /// Refused rather than risking a stack overflow, which would abort the process.
    ///
    /// [`ValidateOptions::max_depth`]: crate::ValidateOptions::max_depth
    Limit {
        /// The limit exceeded, with its value and, for a depth, the line and column of
        /// the element.
        limit: LimitExceeded,
        /// Where: the document, or the stylesheet (by its name in the bundle) and the
        /// line of its instruction.
        message: String,
    },
    /// An option of the validation beyond what this target allows: on WebAssembly, a
    /// [`ValidateOptions::max_depth`] above [`WASM_MAX_DEPTH`] or a
    /// [`ValidateOptions::max_stack`] above [`WASM_MAX_STACK`], for which the native stack
    /// of the JavaScript engines has no room. Never on other targets.
    ///
    /// [`ValidateOptions::max_depth`]: crate::ValidateOptions::max_depth
    /// [`ValidateOptions::max_stack`]: crate::ValidateOptions::max_stack
    /// [`WASM_MAX_DEPTH`]: crate::WASM_MAX_DEPTH
    /// [`WASM_MAX_STACK`]: crate::WASM_MAX_STACK
    OptionOutOfRange {
        /// The option: `max_depth` or `max_stack`.
        option: &'static str,
        /// The value asked for.
        value: u32,
        /// The largest value allowed.
        max: u32,
    },
    /// A stylesheet of the rule set raised a dynamic error on the document (the
    /// reference engine fails on it as well).
    Transform {
        /// The stylesheet, by its name in the bundle.
        stylesheet: String,
        /// The XPath or XSLT error code (`FORG0001`, ...).
        code: String,
        /// The line of the stylesheet instruction, when known.
        line: Option<u32>,
        /// The explanation.
        message: String,
    },
    /// A bug of this engine: a result it cannot read back.
    Internal {
        /// What went wrong.
        message: String,
    },
    /// A fetched rule set ([`fetched_rule_sets`](crate::fetched_rule_sets)) that is not
    /// in the cache: it must be fetched first (`RuleSetCache::fetch`, feature `fetch`;
    /// `einvoice-validate rules fetch NAME`). Nothing is downloaded without being asked.
    NotCached {
        /// The rule set.
        name: String,
        /// The cache directory.
        dir: PathBuf,
    },
    /// A rule set could not be fetched: a download failed or did not have its pinned
    /// digest, an archive is not as published, or its bundle could not be built (no
    /// Java for the Schematron compiler of Peppol BIS Billing, ...).
    Fetch {
        /// What went wrong, naming the URL or file concerned.
        message: String,
    },
    /// No invoice XML could be extracted from a PDF file ([`crate::pdf::extract`]): its
    /// kind is [`ErrorKind::Unsupported`] for an encrypted file, a PDF feature outside the
    /// subset read or a limit exceeded, and [`ErrorKind::Io`] otherwise (not a PDF, a
    /// malformed file, no embedded file with an invoice name or several, XMP metadata
    /// that is not well-formed): the file cannot be read as an e-invoice.
    #[cfg(feature = "pdf")]
    Pdf(einvoice_pdf::Error),
}

/// The kind of an [`Error`], for callers that only need to classify it (the exit code of
/// a command-line tool, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// [`Error::Io`] and [`Error::Fetch`], and an `Error::Pdf` (feature `pdf`) for a PDF
    /// file no invoice can be read from.
    Io,
    /// [`Error::Bundle`], [`Error::Checksum`] and [`Error::NotCached`].
    Bundle,
    /// [`Error::NotXml`].
    NotXml,
    /// [`Error::Unsupported`], [`Error::Limit`] and [`Error::OptionOutOfRange`]: input
    /// refused rather than processed differently from the reference engine, or with an
    /// unbounded stack, and options beyond what the target allows; and
    /// an `Error::Pdf` (feature `pdf`) for an encrypted PDF file, a PDF feature outside
    /// the subset read, or a limit of the PDF reader.
    Unsupported,
    /// [`Error::Transform`].
    Transform,
    /// [`Error::Internal`].
    Internal,
}

impl ErrorKind {
    /// A stable identifier of the kind: `io`, `bundle`, `not-xml`, `unsupported`,
    /// `transform` or `internal`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Io => "io",
            Self::Bundle => "bundle",
            Self::NotXml => "not-xml",
            Self::Unsupported => "unsupported",
            Self::Transform => "transform",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Error {
    /// The kind of the error.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::Io { .. } | Self::Fetch { .. } => ErrorKind::Io,
            Self::Bundle { .. } | Self::Checksum { .. } | Self::NotCached { .. } => {
                ErrorKind::Bundle
            }
            Self::NotXml { .. } => ErrorKind::NotXml,
            Self::Unsupported { .. } | Self::Limit { .. } | Self::OptionOutOfRange { .. } => {
                ErrorKind::Unsupported
            }
            Self::Transform { .. } => ErrorKind::Transform,
            Self::Internal { .. } => ErrorKind::Internal,
            #[cfg(feature = "pdf")]
            Self::Pdf(
                einvoice_pdf::Error::Encrypted
                | einvoice_pdf::Error::Unsupported { .. }
                | einvoice_pdf::Error::LimitExceeded { .. },
            ) => ErrorKind::Unsupported,
            #[cfg(feature = "pdf")]
            Self::Pdf(_) => ErrorKind::Io,
        }
    }

    pub(crate) fn bundle(message: impl Into<String>) -> Self {
        Self::Bundle {
            message: message.into(),
        }
    }

    pub(crate) fn unsupported(message: impl Into<String>) -> Self {
        Self::Unsupported {
            message: message.into(),
        }
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Bundle { message } => write!(f, "invalid rule-set bundle: {message}"),
            Self::Checksum {
                path,
                expected,
                actual,
            } => write!(
                f,
                "invalid rule-set bundle: {path}: SHA-256 {actual}, the manifest records {expected}"
            ),
            Self::NotXml {
                line,
                column,
                message,
            } => write!(f, "{line}:{column}: not well-formed XML: {message}"),
            Self::Unsupported { message } => write!(f, "unsupported: {message}"),
            Self::Limit { message, .. } => write!(f, "limit exceeded: {message}"),
            Self::OptionOutOfRange { option, value, max } => write!(
                f,
                "option {option} is {value}, more than {max}, the most this target allows"
            ),
            Self::Transform {
                stylesheet,
                code,
                line,
                message,
            } => {
                write!(f, "stylesheet {stylesheet}")?;
                if let Some(line) = line {
                    write!(f, ", line {line}")?;
                }
                write!(f, ": {code}: {message}")
            }
            Self::Internal { message } => write!(f, "internal error: {message}"),
            Self::NotCached { name, dir } => write!(
                f,
                "rule set {name} is not in the cache ({}): it must be fetched first",
                dir.display()
            ),
            Self::Fetch { message } => write!(f, "cannot fetch the rule set: {message}"),
            #[cfg(feature = "pdf")]
            Self::Pdf(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            #[cfg(feature = "pdf")]
            Self::Pdf(error) => Some(error),
            _ => None,
        }
    }
}
