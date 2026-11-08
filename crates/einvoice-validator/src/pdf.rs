//! Factur-X, `ZUGFeRD` and Order-X PDF files (feature `pdf`, on by default): the invoice
//! XML a PDF/A-3 file embeds, extracted with [`einvoice_pdf`], to be validated as any
//! other document.
//!
//! [`extract`] returns the embedded file named as the specifications require
//! ([`INVOICE_FILE_NAMES`]), byte for byte, with its file specification
//! (`/AFRelationship`, ...) and the XMP metadata of the document (the PDF/A
//! identification, the Factur-X / `ZUGFeRD` properties). [`Syntax::detect`] then tells
//! which built-in rule set the XML is for:
//!
//! ```
//! # #[cfg(all(feature = "cen-ubl", feature = "cen-cii"))]
//! fn validate(file: &[u8]) -> Result<einvoice_validator::Report, einvoice_validator::Error> {
//!     use einvoice_validator::{Syntax, Validator, pdf};
//!
//!     let xml = if pdf::is_pdf(file) {
//!         let embedded = pdf::extract(file)?;
//!         println!("{} ({:?})", embedded.xml.file_name, embedded.xml.relationship);
//!         embedded.xml.data
//!     } else {
//!         file.to_vec()
//!     };
//!     let rule_set = Syntax::detect(&xml).map_or("cen-ubl", Syntax::builtin_rule_set);
//!     Validator::builtin(rule_set)?.validate_owned(xml)
//! }
//! ```
//!
//! Only the part of PDF on the path to the embedded files is read (see [`einvoice_pdf`]);
//! anything else met there is refused rather than read approximately. The errors are
//! [`Error::Pdf`].
//!
//! [`Syntax::detect`]: crate::Syntax::detect

pub use einvoice_pdf::{
    AfRelationship, Attachment, Embedded, INVOICE_FILE_NAMES, InvoiceMetadata, Limits, Metadata,
};

use crate::Error;

/// The start of every PDF file (the header, ISO 32000-2, 7.5.2).
const HEADER: &[u8] = b"%PDF-";

/// Whether `bytes` start as a PDF file does (`%PDF-`), which no XML document does.
#[must_use]
pub fn is_pdf(bytes: &[u8]) -> bool {
    bytes.starts_with(HEADER)
}

/// The embedded invoice XML of a Factur-X, `ZUGFeRD` or Order-X PDF, with its file
/// specification and the document's XMP metadata, with the default [`Limits`].
///
/// # Errors
///
/// [`Error::Pdf`]: a file that is not a PDF, is malformed, encrypted, uses a PDF feature
/// outside the subset read, exceeds a limit, has no embedded file with an invoice name or
/// several, or has XMP metadata that is not well-formed.
pub fn extract(pdf: &[u8]) -> Result<Embedded, Error> {
    extract_with(pdf, &Limits::default())
}

/// [`extract`] with explicit [`Limits`].
///
/// # Errors
///
/// As [`extract`].
pub fn extract_with(pdf: &[u8], limits: &Limits) -> Result<Embedded, Error> {
    einvoice_pdf::extract_with(pdf, limits).map_err(Error::Pdf)
}

#[cfg(test)]
mod tests {
    use super::{extract, is_pdf};
    use crate::{Error, ErrorKind};

    #[test]
    fn the_pdf_header_tells_a_pdf() {
        assert!(is_pdf(b"%PDF-1.7\n"));
        assert!(is_pdf(b"%PDF-"));
        assert!(!is_pdf(b"<?xml version=\"1.0\"?><a/>"));
        assert!(!is_pdf(b" %PDF-1.7"));
        assert!(!is_pdf(b""));
    }

    #[test]
    fn errors_are_pdf_errors_with_their_kind() {
        let e = extract(b"%PDF-1.7\nnothing else").unwrap_err();
        assert!(
            matches!(e, Error::Pdf(einvoice_pdf::Error::Malformed { .. })),
            "{e:?}"
        );
        assert_eq!(e.kind(), ErrorKind::Io);
        assert!(e.to_string().starts_with("malformed PDF: "), "{e}");
        let e = extract(b"<a/>").unwrap_err();
        assert!(
            matches!(e, Error::Pdf(einvoice_pdf::Error::NotPdf)),
            "{e:?}"
        );
        assert_eq!(e.kind(), ErrorKind::Io);
    }
}
