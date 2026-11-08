//! Extract the embedded e-invoice XML from a Factur-X, `ZUGFeRD` or Order-X PDF (a PDF/A-3
//! file with the invoice as an associated file), with its XMP metadata, in pure Rust.
//!
//! * [`extract`] returns the embedded file named as the specifications require
//!   ([`INVOICE_FILE_NAMES`]: `factur-x.xml`, `zugferd-invoice.xml`, `ZUGFeRD-invoice.xml`,
//!   `xrechnung.xml`, `order-x.xml`), its `/AFRelationship`, and the XMP metadata of the
//!   document catalog with the PDF/A identification and the Factur-X / `ZUGFeRD` properties
//!   (`DocumentType`, `DocumentFileName`, `Version`, `ConformanceLevel`).
//! * [`attachments`] returns every embedded file.
//!
//! Only the part of PDF (ISO 32000-2) needed to reach embedded files is read: the header,
//! cross-reference tables and streams with their trailers and incremental updates (`/Prev`,
//! `/XRefStm`), indirect objects, object streams, the object syntax, `FlateDecode` with the
//! PNG predictors, the catalog's `/AF` and `/Names /EmbeddedFiles` name tree, file
//! specification dictionaries and embedded file streams (`/Params /Size` and `/CheckSum`
//! are verified). Anything else the reader meets on that path, such as another filter, is
//! refused with [`Error::Unsupported`]; a file that breaks the syntax is refused with
//! [`Error::Malformed`], never repaired. Encrypted files are refused ([`Error::Encrypted`]):
//! PDF/A forbids encryption. Page content is never read.
//!
//! The input is untrusted: nesting, reference chains, name tree depth, the number of
//! cross-reference sections and of embedded files, and decoded sizes are bounded by
//! [`Limits`], each object and object stream is parsed at most once, and cycles are errors.
//! The crate has no `unsafe` code and builds for `wasm32-unknown-unknown`.
//!
//! ```no_run
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let pdf = std::fs::read("invoice.pdf")?;
//!     let invoice = einvoice_pdf::extract(&pdf)?;
//!     println!("{}: {} bytes", invoice.xml.file_name, invoice.xml.data.len());
//!     println!("relationship: {:?}", invoice.xml.relationship);
//!     if let Some(fx) = invoice.metadata.as_ref().and_then(|m| m.invoice.as_ref()) {
//!         println!("{:?} {:?} {:?}", fx.document_type, fx.version, fx.conformance_level);
//!     }
//!     for attachment in einvoice_pdf::attachments(&pdf)? {
//!         println!("embedded: {} ({:?})", attachment.file_name, attachment.mime_type);
//!     }
//!     Ok(())
//! }
//! ```

mod catalog;
mod document;
mod error;
mod filter;
mod md5;
mod object;
mod parser;
mod text;
mod xmp;
mod xref;

use std::fmt;

pub use error::{Error, Location};

use document::Document;

/// The names under which the specifications embed the invoice XML: Factur-X (and
/// `ZUGFeRD` 2.1 and later), `ZUGFeRD` 2.0, `ZUGFeRD` 1.0, `XRechnung` in `ZUGFeRD`, Order-X.
pub const INVOICE_FILE_NAMES: [&str; 5] = [
    "factur-x.xml",
    "zugferd-invoice.xml",
    "ZUGFeRD-invoice.xml",
    "xrechnung.xml",
    "order-x.xml",
];

/// Bounds on the work done for one file, against hostile input. Every bound that is
/// reached is an [`Error::LimitExceeded`] naming the field.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Nesting of arrays and dictionaries in one object. Default 64.
    pub max_depth: usize,
    /// Chains of indirect references, and objects whose reading requires reading others
    /// (a stream's indirect `/Length`, an object stream). Default 32.
    pub max_resolution_depth: usize,
    /// Cross-reference sections (incremental updates). Default 1024.
    pub max_xref_sections: usize,
    /// Depth of the `/EmbeddedFiles` name tree. Default 32.
    pub max_name_tree_depth: usize,
    /// Embedded files (name tree entries, then distinct files). Default 10 000.
    pub max_attachments: usize,
    /// Decoded size of one stream, in bytes. Default 64 MiB.
    pub max_stream_size: usize,
    /// Decoded size of all streams together, in bytes. Default 256 MiB.
    pub max_total_decoded: usize,
    /// Nodes of the XMP metadata XML tree. Default 1 000 000.
    pub max_xmp_nodes: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: 64,
            max_resolution_depth: 32,
            max_xref_sections: 1024,
            max_name_tree_depth: 32,
            max_attachments: 10_000,
            max_stream_size: 64 << 20,
            max_total_decoded: 256 << 20,
            max_xmp_nodes: 1_000_000,
        }
    }
}

/// The relationship of an associated file to the document (`/AFRelationship`,
/// ISO 32000-2, 14.13.2, Table 43).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AfRelationship {
    /// `/Source`.
    Source,
    /// `/Data`.
    Data,
    /// `/Alternative`.
    Alternative,
    /// `/Supplement`.
    Supplement,
    /// `/EncryptedPayload`.
    EncryptedPayload,
    /// `/FormData`.
    FormData,
    /// `/Schema`.
    Schema,
    /// `/Unspecified`.
    Unspecified,
    /// Any other name (second-class names are allowed), as written, `#xx` escapes
    /// decoded and bytes outside UTF-8 replaced.
    Other(String),
}

impl AfRelationship {
    fn from_name(name: &[u8]) -> Self {
        match name {
            b"Source" => Self::Source,
            b"Data" => Self::Data,
            b"Alternative" => Self::Alternative,
            b"Supplement" => Self::Supplement,
            b"EncryptedPayload" => Self::EncryptedPayload,
            b"FormData" => Self::FormData,
            b"Schema" => Self::Schema,
            b"Unspecified" => Self::Unspecified,
            other => Self::Other(String::from_utf8_lossy(other).into_owned()),
        }
    }

    /// The name, without its `/`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Source => "Source",
            Self::Data => "Data",
            Self::Alternative => "Alternative",
            Self::Supplement => "Supplement",
            Self::EncryptedPayload => "EncryptedPayload",
            Self::FormData => "FormData",
            Self::Schema => "Schema",
            Self::Unspecified => "Unspecified",
            Self::Other(name) => name,
        }
    }
}

impl fmt::Display for AfRelationship {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An embedded file: its file specification and its decoded content.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Attachment {
    /// The file name: the file specification's `/UF`, else its `/F`, else its key in the
    /// `/EmbeddedFiles` name tree.
    pub file_name: String,
    /// Its key in the `/EmbeddedFiles` name tree, when it is listed there.
    pub key: Option<String>,
    /// Whether it is listed in the catalog's `/AF` (associated files).
    pub in_associated_files: bool,
    /// `/AFRelationship`.
    pub relationship: Option<AfRelationship>,
    /// `/Desc`.
    pub description: Option<String>,
    /// The MIME type of the embedded file stream (`/Subtype`), such as `text/xml`.
    pub mime_type: Option<String>,
    /// `/Params /Size`, verified against the decoded content.
    pub size: Option<u64>,
    /// `/Params /CheckSum`, the MD5 digest, verified against the decoded content. Written
    /// as the 16 bytes of the digest (ISO 32000-2) or, as some producers do, as its 32
    /// hexadecimal digits: both are read.
    pub checksum: Option<[u8; 16]>,
    /// `/Params /CreationDate`, as written (a PDF date string).
    pub creation_date: Option<String>,
    /// `/Params /ModDate`, as written (a PDF date string).
    pub modification_date: Option<String>,
    /// The decoded content, byte for byte.
    pub data: Vec<u8>,
}

/// The Factur-X / `ZUGFeRD` / Order-X extension schema properties of the XMP metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct InvoiceMetadata {
    /// The namespace of the properties, such as
    /// `urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#` (any
    /// `urn:<scheme>:pdfa:CrossIndustryDocument:…` namespace is recognised).
    pub namespace: String,
    /// `DocumentType`, such as `INVOICE` or `ORDER`.
    pub document_type: Option<String>,
    /// `DocumentFileName`, such as `factur-x.xml`.
    pub document_file_name: Option<String>,
    /// `Version`, such as `1.0`.
    pub version: Option<String>,
    /// `ConformanceLevel`, the profile, such as `EN 16931`.
    pub conformance_level: Option<String>,
}

/// The XMP metadata of the document catalog (`/Metadata`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Metadata {
    /// The decoded metadata stream (UTF-8).
    pub xmp: String,
    /// `pdfaid:part`, such as `3`.
    pub pdfa_part: Option<String>,
    /// `pdfaid:conformance`, such as `B`.
    pub pdfa_conformance: Option<String>,
    /// The Factur-X / `ZUGFeRD` / Order-X properties, when the packet has any.
    pub invoice: Option<InvoiceMetadata>,
}

/// What [`extract`] returns.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Embedded {
    /// The embedded invoice (or order) XML.
    pub xml: Attachment,
    /// The XMP metadata of the document catalog, when it has any.
    pub metadata: Option<Metadata>,
    /// The PDF version of the header, such as `1.7`.
    pub pdf_version: String,
}

/// The embedded invoice XML of a Factur-X, `ZUGFeRD` or Order-X PDF, with the default
/// [`Limits`].
///
/// # Errors
///
/// When the file cannot be read (see [`Error`]), when no embedded file has one of the
/// [`INVOICE_FILE_NAMES`] ([`Error::NoInvoice`]) or several do ([`Error::Ambiguous`]),
/// and when the XMP metadata is not well-formed ([`Error::Metadata`]).
pub fn extract(pdf: &[u8]) -> Result<Embedded, Error> {
    extract_with(pdf, &Limits::default())
}

/// [`extract`] with explicit [`Limits`].
///
/// # Errors
///
/// As [`extract`].
pub fn extract_with(pdf: &[u8], limits: &Limits) -> Result<Embedded, Error> {
    let mut doc = Document::load(pdf, limits)?;
    let catalog = catalog::catalog(&mut doc)?;
    let object::Object::Dict(catalog) = &*catalog else {
        return Err(Error::malformed(
            None,
            "the document catalog is not a dictionary",
        ));
    };
    let specs = catalog::file_specs(&mut doc, catalog)?;
    let candidates: Vec<&catalog::FileSpec> = specs
        .iter()
        .filter(|spec| INVOICE_FILE_NAMES.contains(&spec.file_name.as_str()))
        .collect();
    let spec = match candidates.as_slice() {
        [] => {
            return Err(Error::NoInvoice {
                attachments: specs.iter().map(|s| s.file_name.clone()).collect(),
            });
        }
        [spec] => *spec,
        several => {
            return Err(Error::Ambiguous {
                names: several.iter().map(|s| s.file_name.clone()).collect(),
            });
        }
    };
    let xml = catalog::attachment(&mut doc, spec)?;
    let metadata = catalog::metadata_stream(&mut doc, catalog)?
        .map(|bytes| xmp::parse(bytes, limits.max_xmp_nodes))
        .transpose()?;
    Ok(Embedded {
        xml,
        metadata,
        pdf_version: doc.version.clone(),
    })
}

/// Every embedded file of a PDF, with the default [`Limits`]: those of the
/// `/EmbeddedFiles` name tree in key order, then those of the catalog's `/AF` not
/// already listed.
///
/// # Errors
///
/// When the file cannot be read, or an embedded file cannot be decoded or fails its
/// `/Params` checks (see [`Error`]).
pub fn attachments(pdf: &[u8]) -> Result<Vec<Attachment>, Error> {
    attachments_with(pdf, &Limits::default())
}

/// [`attachments`] with explicit [`Limits`].
///
/// # Errors
///
/// As [`attachments`].
pub fn attachments_with(pdf: &[u8], limits: &Limits) -> Result<Vec<Attachment>, Error> {
    let mut doc = Document::load(pdf, limits)?;
    let catalog = catalog::catalog(&mut doc)?;
    let object::Object::Dict(catalog) = &*catalog else {
        return Err(Error::malformed(
            None,
            "the document catalog is not a dictionary",
        ));
    };
    let specs = catalog::file_specs(&mut doc, catalog)?;
    specs
        .iter()
        .map(|spec| catalog::attachment(&mut doc, spec))
        .collect()
}
