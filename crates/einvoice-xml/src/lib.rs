//! A streaming (pull) parser of XML 1.0 documents with namespaces, in pure Rust: no
//! tree is built, so that well-formedness checking and validation can run in one pass
//! over a document in memory that does not grow with the document.
//!
//! [`Reader::next_event`] returns the [`Event`]s of a document one by one. Names, and
//! text and attribute values without references or characters to normalise, are
//! slices of the input; the reader keeps only the open elements, the namespace
//! declarations in scope, the attributes of the current start tag and the distinct
//! namespace declarations seen (at most [`Limits::namespaces`]).
//!
//! ```
//! use einvoice_xml::{Event, Reader};
//!
//! let mut reader = Reader::new(r#"<i:Invoice xmlns:i="urn:i" id="1">a &amp; b</i:Invoice>"#);
//! let mut events = Vec::new();
//! while let Some(event) = reader.next_event()? {
//!     match event {
//!         Event::Start(start) => {
//!             let name = start.name();
//!             assert_eq!((name.namespace, name.local), (Some("urn:i"), "Invoice"));
//!             assert_eq!(start.attribute(None, "id").map(|v| v.as_ref()), Some("1"));
//!             events.push("start");
//!         }
//!         Event::Text(text) => {
//!             assert_eq!(text.value, "a & b");
//!             events.push("text");
//!         }
//!         Event::End(_) => events.push("end"),
//!         _ => {}
//!     }
//! }
//! assert_eq!(events, ["start", "text", "end"]);
//! assert!(einvoice_xml::check("<a><b></a>").is_err());
//! # Ok::<(), einvoice_xml::Error>(())
//! ```
//!
//! # What is accepted
//!
//! The text is UTF-8 (a `&str`; a byte order mark at the start is skipped; the
//! `encoding` of the XML declaration is checked for syntax only). A document given as
//! bytes is decoded to such a text first with [`decode`] (see [Encodings](#encodings)).
//! Supported: the XML
//! declaration, elements and attributes, namespaces (prefixes, the default namespace
//! and its undeclaration with `xmlns=""`, the predefined `xml` prefix for attributes),
//! comments, processing instructions, CDATA sections, character references and the five
//! predefined entity references, line-end normalisation and attribute-value
//! normalisation for the CDATA type. A document type declaration is refused
//! ([`Error::DtdDetected`]), and so is a reference to any other entity.
//!
//! The reader accepts the documents that the `roxmltree` crate (0.21, document type
//! declarations refused) accepts and that are namespace-well-formed: on a document it
//! refuses, the reader reports the same error, with the same message and position,
//! unless the document contains one of the constructs below, which `roxmltree` accepts
//! although XML 1.0 or Namespaces in XML 1.0 forbids them; each is refused with its own
//! error:
//!
//! * a character reference to a surrogate or to a number above `0x10FFFF`
//!   ([`Error::InvalidCharacterReference`]; `roxmltree` reads U+FFFD);
//! * a processing instruction whose target is `xml` in any case or contains a colon,
//!   or that has no whitespace between its target and its data
//!   ([`Error::InvalidPiTarget`], [`Error::PiWithoutWhitespace`]);
//! * an XML declaration with a pseudo-attribute other than `version`, `encoding` and
//!   `standalone` as written, a version other than `1.` and digits, an encoding that is
//!   not an encoding name, or a `standalone` other than `yes` and `no`
//!   ([`Error::InvalidXmlDeclaration`]);
//! * a name that starts with a colon ([`Error::InvalidQName`]);
//! * `xmlns:p=""` ([`Error::EmptyPrefixedNamespace`]), a declaration of the `xmlns`
//!   prefix ([`Error::XmlnsPrefixDeclared`]), the default namespace or the `xml` prefix
//!   declared twice on an element ([`Error::DuplicatedNamespaceDeclaration`]);
//! * an attribute named `p:xmlns`, which `roxmltree` takes for a declaration of the
//!   default namespace ([`Error::PrefixedXmlnsAttribute`]).
//!
//! The errors that only these constructs cause are raised once the checks that
//! `roxmltree` makes on the same start tag, text, processing instruction or XML
//! declaration have passed, so that a document with an error of both kinds in one tag
//! gets the error that `roxmltree` reports.
//!
//! As with `roxmltree`, an element name with the `xml` prefix is refused
//! ([`Error::UnknownNamespace`]), although XML allows it.
//!
//! The data differs from `roxmltree`'s, following XML 1.0, in three cases: a carriage
//! return written next to a reference in character data is a line end like any other
//! (`roxmltree` can keep it as a carriage return); comments and processing
//! instructions have their line ends normalised; and `<?xml` followed by a tab, a line
//! feed or a carriage return at the start of the text is an XML declaration (for
//! `roxmltree`, a processing instruction).
//!
//! # Encodings
//!
//! [`decode`] finds the encoding of a document given as bytes as XML 1.0 (4.3.3 and
//! Appendix F) says, from its byte order mark, its first bytes and its XML declaration,
//! and decodes it to UTF-8 (without a copy when it is UTF-8 already). It reads the
//! encodings that the reference parsers of e-invoice validation, libxml2 (`xmllint`) and
//! the XML parser of Saxon-HE (Xerces, in the JDK), both read in the same way, under the
//! names both accept ([`encoding_names`]): UTF-8, UTF-16 (with a byte order mark, or
//! without one when the document starts with `<?`), US-ASCII, ISO-8859-1, ISO-8859-15 and
//! windows-1252 ([`Encoding`]). A document in any other encoding is refused with
//! [`Error::UnsupportedEncoding`], one whose first bytes and declaration disagree with
//! [`Error::EncodingMismatch`], and bytes that are not a character of the encoding with
//! [`Error::InvalidEncodedBytes`], where both reference parsers, or one of them, refuse
//! them too (the test `tests/encoding.rs` checks each case against what they do, as the
//! example `encoding-oracle` records it). A position in a decoded text counts its
//! characters, as for any text.
//!
//! # Limits
//!
//! [`Limits`] bounds the number of nodes, the depth, the number of attributes and the
//! number of distinct namespace declarations; the defaults are `roxmltree`'s. Time is
//! linear in the length of the text: duplicate attributes and prefixes are found
//! through hash sets when an element has many.

mod chars;
mod decode;
mod encoding;
mod error;
mod event;
mod namespaces;
mod reader;

pub use encoding::{Encoding, decode, decode_owned, encoding_names, encoding_of};
pub use error::{Error, TextPos};
pub use event::{
    Attribute, Comment, EndElement, Event, Name, ProcessingInstruction, StartElement, Text,
};
pub use reader::{Limits, Reader, XML_NAMESPACE, XMLNS_NAMESPACE};

/// Checks that `text` is a well-formed document that the [`Reader`] accepts, with the
/// default [`Limits`].
///
/// # Errors
///
/// The first error.
pub fn check(text: &str) -> Result<(), Error> {
    Reader::new(text).read_to_end()
}
