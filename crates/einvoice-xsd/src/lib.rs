//! XML Schema 1.0 validation of UBL 2.1 Invoice and `CreditNote` documents against the
//! official OASIS UBL 2.1 schemas, in pure Rust.
//!
//! This is not a general XML Schema processor: it implements exactly the constructs
//! that the schemas reachable from `UBL-Invoice-2.1.xsd` and `UBL-CreditNote-2.1.xsd`
//! use (see the inventory below) and refuses any other construct when a schema set is
//! loaded ([`LoadErrorKind::Unsupported`]), so that a schema it cannot handle is never
//! half-checked.
//!
//! ```
//! let schema = einvoice_xsd::Schema::ubl_2_1()?;
//! let report = schema.validate(
//!     r#"<CreditNote xmlns="urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2"
//!         xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2">
//!       <cbc:ID>CN-1</cbc:ID><cbc:IssueDate>2024-02-30</cbc:IssueDate>
//!     </CreditNote>"#,
//! )?;
//! // Sorted by the position of the element concerned: the incomplete CreditNote
//! // first, then its invalid date.
//! let kinds: Vec<&str> = report.errors.iter().map(|e| e.kind.as_str()).collect();
//! assert_eq!(kinds, ["missing-element", "invalid-value"]);
//! assert_eq!((report.errors[1].line, report.errors[1].element.as_str()), (3, "{urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2}IssueDate"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Schemas
//!
//! The fifteen schema documents reachable from the two entry points are vendored
//! byte for byte in `schemas/oasis-ubl-2.1/` and embedded ([`UBL_2_1_FILES`]);
//! [`Schema::ubl_2_1`] loads both entry points into one schema set, as an XML Schema
//! processor given both does. [`Schema::load`] loads any set of schema documents
//! within the supported subset, from memory: `xs:import` and `xs:include` are
//! resolved as paths relative to the importing document, never fetched.
//!
//! # Validation
//!
//! [`Schema::validate`] parses the document (namespace-aware, internal DTD subset
//! allowed for entities, no external resources) and returns a [`Report`] listing
//! every validity error with a stable [`ErrorKind`], the line and column of the
//! element concerned and a message. The document element must match a global element
//! declaration of the set. Validation follows XML Schema 1.0 Second Edition:
//!
//! * content models are compiled to deterministic automata (Unique Particle
//!   Attribution is checked at load); `minOccurs`/`maxOccurs` on elements,
//!   sequences, choices and wildcards;
//! * wildcards: `##any` and `##other`, `processContents` `strict`, `lax` and `skip`.
//!   Lax content without a declaration is validated as `xs:anyType`, whose own
//!   content is lax: an element anywhere below that has a global declaration (for
//!   instance a `cbc:` element inside `ext:ExtensionContent`) is validated against it;
//! * attributes: declared uses (`required`/`optional`), attribute wildcards, the
//!   four `xsi:` attributes (`xsi:type` with derivation checks and abstract types,
//!   `xsi:nil` refused since no declaration is nillable, `xsi:schemaLocation` and
//!   `xsi:noNamespaceSchemaLocation` ignored as hints);
//! * simple values: whitespace processing and the lexical space of every built-in
//!   type the schemas use, the `enumeration` facet, and uniqueness of `xs:ID` values.
//!
//! The decimal and integer types have no digit limit, dates follow the XML Schema
//! 1.0 rules (no year zero, at least four year digits, timezones up to ±14:00,
//! `24:00:00` as end of day), `anyURI` is checked as XML Schema 1.0 defines it
//! (escaping per `XLink` 1.0 section 5.4, then RFC 2396 as amended by RFC 2732).
//!
//! # Comparison with xmllint
//!
//! The test suite compares every verdict with xmllint (libxml2 2.9.14) on the CEN
//! EN 16931 UBL test corpus and on synthetic mutations covering each construct
//! (`tests/oracle/`). The cases where xmllint departs from XML Schema 1.0 and this
//! crate does not are listed with their cause in `tests/oracle/disagreements.tsv`.
//!
//! # Inventory of the UBL 2.1 schemas
//!
//! Counted over the fifteen vendored documents by the `inventory` test, which fails
//! when this table no longer matches them. Annotations are not descended into.
//!
//! In short: 15 schema documents, all with `elementFormDefault="qualified"`; 24
//! `xs:import` and one `xs:include` (the extension content type, included by the
//! common extension components); 1,622 global element declarations and 1,219 named
//! complex types; content models made of `xs:sequence` and `xs:choice` of element
//! references, local elements and wildcards, with `minOccurs` 0 or 1 and `maxOccurs`
//! 1 or `unbounded` only. There are no anonymous types, no groups or attribute groups,
//! no substitution groups, no identity constraints, no default or fixed values, no
//! `xs:all`, no lists or unions, and no facet other than `xs:enumeration` (two
//! values, on the `XAdES` `QualifierType`).
//!
//! * The UBL extension point: `ext:UBLExtensions` holds one or more `ext:UBLExtension`,
//!   each ending with `ext:ExtensionContent`, whose type is a sequence of exactly one
//!   `xs:any namespace="##other" processContents="lax"` (`##other` relative to the
//!   extension components namespace, so neither that namespace nor no namespace).
//!   The signature extension (`sig:UBLDocumentSignatures`) is declared and imported,
//!   so it is validated strictly; any other extension content is validated laxly.
//! * The CCTS core component types (`CCTS_CCT_SchemaModule-2.1.xsd`) are complex types
//!   with simple content extending `xs:decimal`, `xs:base64Binary`,
//!   `xs:normalizedString` and `xs:string` with optional supplementary-component
//!   attributes; the unqualified data types extend them (or `xs:date`, `xs:dateTime`,
//!   `xs:time`, `xs:boolean`), and seven of them are simple-content restrictions
//!   making one attribute required (`currencyID`, `mimeCode`, `unitCode`). Every
//!   `cbc:` type extends one of these.
//! * The W3C XML Signature and ETSI `XAdES` 1.3.2/1.4.1 schemas bring the rest: mixed
//!   content, nested and repeated choices and sequences, strict and lax wildcards, an
//!   attribute wildcard (strict), an abstract type with two complex-content
//!   restrictions, `xs:ID` attributes, local elements typed directly with built-in
//!   types, and the four simple types (restrictions of `xs:base64Binary` twice,
//!   `xs:integer`, and `xs:string` with the only enumeration).
//!
//! INVENTORY-START
//! | Construct | Count |
//! |---|---:|
//! | `built-in type xs:ID` | 27 |
//! | `built-in type xs:anyURI` | 30 |
//! | `built-in type xs:base64Binary` | 13 |
//! | `built-in type xs:boolean` | 2 |
//! | `built-in type xs:date` | 1 |
//! | `built-in type xs:dateTime` | 5 |
//! | `built-in type xs:decimal` | 4 |
//! | `built-in type xs:integer` | 4 |
//! | `built-in type xs:language` | 2 |
//! | `built-in type xs:normalizedString` | 26 |
//! | `built-in type xs:string` | 32 |
//! | `built-in type xs:time` | 1 |
//! | `xs:annotation` | 2566 |
//! | `xs:any` | 15 |
//! | `xs:any @maxOccurs="1"` | 1 |
//! | `xs:any @maxOccurs="unbounded"` | 5 |
//! | `xs:any @minOccurs="0"` | 6 |
//! | `xs:any @minOccurs="1"` | 1 |
//! | `xs:any @namespace="##any"` | 3 |
//! | `xs:any @namespace="##other"` | 12 |
//! | `xs:any @processContents="lax"` | 12 |
//! | `xs:anyAttribute` | 1 |
//! | `xs:anyAttribute @namespace="##any"` | 1 |
//! | `xs:attribute` | 92 |
//! | `xs:attribute @use="optional"` | 75 |
//! | `xs:attribute @use="required"` | 16 |
//! | `xs:choice` | 15 |
//! | `xs:choice @maxOccurs="unbounded"` | 7 |
//! | `xs:choice @minOccurs="0"` | 3 |
//! | `xs:complexContent` | 2 |
//! | `xs:complexType` | 1219 |
//! | `xs:complexType @abstract="true"` | 1 |
//! | `xs:complexType @mixed="true"` | 9 |
//! | `xs:element (global)` | 1622 |
//! | `xs:element (local, name)` | 118 |
//! | `xs:element (local, name) @maxOccurs="unbounded"` | 18 |
//! | `xs:element (local, name) @minOccurs="0"` | 46 |
//! | `xs:element (local, ref)` | 2293 |
//! | `xs:element (local, ref) @maxOccurs="1"` | 1657 |
//! | `xs:element (local, ref) @maxOccurs="unbounded"` | 607 |
//! | `xs:element (local, ref) @minOccurs="0"` | 2034 |
//! | `xs:element (local, ref) @minOccurs="1"` | 233 |
//! | `xs:enumeration` | 2 |
//! | `xs:extension (in xs:simpleContent)` | 907 |
//! | `xs:import` | 24 |
//! | `xs:import @namespace="http://uri.etsi.org/01903/v1.3.2#"` | 2 |
//! | `xs:import @namespace="http://uri.etsi.org/01903/v1.4.1#"` | 1 |
//! | `xs:import @namespace="http://www.w3.org/2000/09/xmldsig#"` | 2 |
//! | `xs:import @namespace="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2"` | 2 |
//! | `xs:import @namespace="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2"` | 5 |
//! | `xs:import @namespace="urn:oasis:names:specification:ubl:schema:xsd:CommonExtensionComponents-2"` | 2 |
//! | `xs:import @namespace="urn:oasis:names:specification:ubl:schema:xsd:CommonSignatureComponents-2"` | 1 |
//! | `xs:import @namespace="urn:oasis:names:specification:ubl:schema:xsd:QualifiedDataTypes-2"` | 2 |
//! | `xs:import @namespace="urn:oasis:names:specification:ubl:schema:xsd:SignatureAggregateComponents-2"` | 1 |
//! | `xs:import @namespace="urn:oasis:names:specification:ubl:schema:xsd:SignatureBasicComponents-2"` | 1 |
//! | `xs:import @namespace="urn:oasis:names:specification:ubl:schema:xsd:UnqualifiedDataTypes-2"` | 4 |
//! | `xs:import @namespace="urn:un:unece:uncefact:data:specification:CoreComponentTypeSchemaModule:2"` | 1 |
//! | `xs:include` | 1 |
//! | `xs:restriction (in xs:complexContent)` | 2 |
//! | `xs:restriction (in xs:simpleContent)` | 7 |
//! | `xs:restriction (in xs:simpleType)` | 4 |
//! | `xs:schema` | 15 |
//! | `xs:schema @attributeFormDefault="unqualified"` | 12 |
//! | `xs:schema @elementFormDefault="qualified"` | 15 |
//! | `xs:sequence` | 298 |
//! | `xs:sequence @maxOccurs="unbounded"` | 5 |
//! | `xs:sequence @minOccurs="0"` | 4 |
//! | `xs:simpleContent` | 914 |
//! | `xs:simpleType` | 4 |
//! INVENTORY-END

mod automaton;
mod datatypes;
mod error;
mod load;
mod model;
mod ubl;
mod uri;
mod validate;

pub use datatypes::Builtin;
pub use error::{DocumentError, ErrorKind, LoadError, LoadErrorKind, Report, ValidationError};
pub use model::Schema;
pub use ubl::{CREDIT_NOTE_ENTRY_POINT, INVOICE_ENTRY_POINT, UBL_2_1_FILES};

impl Schema {
    /// Loads a schema set from memory. `files` maps paths (relative,
    /// `/`-separated) to the text of each schema document; `entry_points` are the
    /// paths to start from. Every document reachable through `xs:import` and
    /// `xs:include` must be in `files`.
    ///
    /// # Errors
    ///
    /// A [`LoadError`]: a missing or malformed document, a construct outside the
    /// supported subset, or an incorrect schema.
    pub fn load(files: &[(&str, &str)], entry_points: &[&str]) -> Result<Self, LoadError> {
        load::load(files, entry_points)
    }

    /// Loads the embedded OASIS UBL 2.1 schemas, with both entry points
    /// ([`INVOICE_ENTRY_POINT`] and [`CREDIT_NOTE_ENTRY_POINT`]).
    ///
    /// # Errors
    ///
    /// None in practice: the embedded schemas are within the supported subset (a
    /// test loads them). The error is returned rather than hidden.
    pub fn ubl_2_1() -> Result<Self, LoadError> {
        Self::load(
            UBL_2_1_FILES,
            &[INVOICE_ENTRY_POINT, CREDIT_NOTE_ENTRY_POINT],
        )
    }

    /// Validates a document.
    ///
    /// # Errors
    ///
    /// [`DocumentError::Malformed`] when the text is not well-formed XML with
    /// namespaces; [`DocumentError::Unsupported`] when the document needs a feature
    /// outside the implemented subset (`xsi:type` naming a built-in type the schemas
    /// do not use). Validity errors are not errors: they are in the [`Report`].
    pub fn validate(&self, xml: &str) -> Result<Report, DocumentError> {
        validate::validate(self, xml)
    }

    /// Validates a document given as bytes: UTF-8, with or without a byte order
    /// mark.
    ///
    /// # Errors
    ///
    /// As [`Schema::validate`]; also [`DocumentError::Unsupported`] when the XML
    /// declaration names an encoding other than UTF-8 (or US-ASCII) or the bytes
    /// start with a UTF-16 byte order mark, and [`DocumentError::Malformed`] when the
    /// bytes are not UTF-8.
    pub fn validate_bytes(&self, bytes: &[u8]) -> Result<Report, DocumentError> {
        let unsupported = |message: String| DocumentError::Unsupported {
            line: 1,
            column: 1,
            message,
        };
        if bytes.starts_with(&[0xFE, 0xFF]) || bytes.starts_with(&[0xFF, 0xFE]) {
            return Err(unsupported("UTF-16 documents".to_owned()));
        }
        let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
        if let Some(encoding) = declared_encoding(bytes) {
            let utf8 = ["utf-8", "utf8", "us-ascii", "ascii"];
            if !utf8.iter().any(|e| e.eq_ignore_ascii_case(encoding)) {
                return Err(unsupported(format!(
                    "the {encoding} encoding: only UTF-8 is supported"
                )));
            }
        }
        let text = std::str::from_utf8(bytes).map_err(|e| {
            let before = &bytes[..e.valid_up_to()];
            let line = before.split(|&b| b == b'\n').count();
            DocumentError::Malformed {
                line: u32::try_from(line).unwrap_or(u32::MAX),
                column: 1,
                message: format!("not UTF-8: {e}"),
            }
        })?;
        self.validate(text)
    }
}

/// The `encoding` pseudo-attribute of the XML declaration, if any.
fn declared_encoding(bytes: &[u8]) -> Option<&str> {
    let declaration = bytes.strip_prefix(b"<?xml")?;
    let end = declaration.windows(2).position(|w| w == b"?>")?;
    let declaration = std::str::from_utf8(&declaration[..end]).ok()?;
    let rest = &declaration[declaration.find("encoding")? + "encoding".len()..];
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let value = &rest[1..];
    value.find(quote).map(|end| &value[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings() {
        assert_eq!(
            declared_encoding(br#"<?xml version="1.0" encoding="UTF-8"?><a/>"#),
            Some("UTF-8")
        );
        assert_eq!(
            declared_encoding(b"<?xml version='1.0' encoding = 'latin1' ?><a/>"),
            Some("latin1")
        );
        assert_eq!(declared_encoding(b"<?xml version='1.0'?><a/>"), None);
        assert_eq!(declared_encoding(b"<a/>"), None);
    }
}
