//! The error type.

use std::fmt;

/// Where in the input an error was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Location {
    /// A byte offset in the PDF file.
    File(usize),
    /// A byte offset in the decoded data of the object stream with this object number.
    ObjectStream {
        /// The object number of the object stream.
        stream: u32,
        /// The byte offset in its decoded data.
        offset: usize,
    },
    /// In the object with this object number (no finer position is known).
    Object(u32),
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(offset) => write!(f, "at byte {offset}"),
            Self::ObjectStream { stream, offset } => {
                write!(f, "at byte {offset} of object stream {stream}")
            }
            Self::Object(number) => write!(f, "in object {number}"),
        }
    }
}

/// Why a file could not be read, or why no invoice could be extracted from it.
///
/// A file that uses a PDF feature outside the supported subset is refused with
/// [`Error::Unsupported`], never read approximately.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The input does not start with a PDF header (`%PDF-1.x` or `%PDF-2.0`).
    NotPdf,
    /// The file is encrypted (its trailer has `/Encrypt`); PDF/A forbids encryption.
    Encrypted,
    /// The file breaks the PDF syntax or structure.
    Malformed {
        /// Where the problem was found.
        location: Option<Location>,
        /// What is wrong.
        message: String,
    },
    /// The file uses a PDF feature this crate does not read.
    Unsupported {
        /// Where the feature was found.
        location: Option<Location>,
        /// The feature.
        feature: String,
    },
    /// A limit of [`Limits`](crate::Limits) was exceeded.
    LimitExceeded {
        /// The limit, by its field name in [`Limits`](crate::Limits).
        limit: &'static str,
        /// The value of the limit.
        value: usize,
    },
    /// No embedded file has one of the names of [`INVOICE_FILE_NAMES`](crate::INVOICE_FILE_NAMES).
    NoInvoice {
        /// The file names of the embedded files found, in document order.
        attachments: Vec<String>,
    },
    /// More than one embedded file has a name of [`INVOICE_FILE_NAMES`](crate::INVOICE_FILE_NAMES).
    Ambiguous {
        /// The names of the candidates, in document order.
        names: Vec<String>,
    },
    /// The XMP metadata stream is not UTF-8, not well-formed XML, or gives conflicting
    /// values for one property.
    Metadata(String),
}

impl Error {
    pub(crate) fn malformed(location: Option<Location>, message: impl Into<String>) -> Self {
        Self::Malformed {
            location,
            message: message.into(),
        }
    }

    pub(crate) fn unsupported(location: Option<Location>, feature: impl Into<String>) -> Self {
        Self::Unsupported {
            location,
            feature: feature.into(),
        }
    }

    /// The error with `location` when it has none yet.
    pub(crate) fn at(self, at: Location) -> Self {
        match self {
            Self::Malformed {
                location: None,
                message,
            } => Self::Malformed {
                location: Some(at),
                message,
            },
            Self::Unsupported {
                location: None,
                feature,
            } => Self::Unsupported {
                location: Some(at),
                feature,
            },
            other => other,
        }
    }
}

fn suffix(location: Option<&Location>) -> String {
    location.map_or_else(String::new, |l| format!(" ({l})"))
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPdf => f.write_str("not a PDF file: no %PDF-1.x or %PDF-2.0 header at byte 0"),
            Self::Encrypted => f.write_str("encrypted PDF (PDF/A forbids encryption)"),
            Self::Malformed { location, message } => {
                write!(f, "malformed PDF: {message}{}", suffix(location.as_ref()))
            }
            Self::Unsupported { location, feature } => {
                write!(
                    f,
                    "unsupported PDF feature: {feature}{}",
                    suffix(location.as_ref())
                )
            }
            Self::LimitExceeded { limit, value } => {
                write!(f, "limit exceeded: {limit} ({value})")
            }
            Self::NoInvoice { attachments } => {
                if attachments.is_empty() {
                    f.write_str("no embedded invoice: the PDF has no embedded file")
                } else {
                    write!(
                        f,
                        "no embedded invoice: no embedded file has an invoice name; embedded files: {}",
                        attachments.join(", ")
                    )
                }
            }
            Self::Ambiguous { names } => write!(
                f,
                "ambiguous embedded invoice: several embedded files have an invoice name: {}",
                names.join(", ")
            ),
            Self::Metadata(message) => write!(f, "XMP metadata: {message}"),
        }
    }
}

impl std::error::Error for Error {}

pub(crate) type Result<T> = std::result::Result<T, Error>;
