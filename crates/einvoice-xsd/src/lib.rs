//! XML Schema 1.0 validation of UBL 2.1 Invoice and `CreditNote` documents against the
//! official OASIS UBL 2.1 schemas, and of UN/CEFACT Cross Industry Invoice D16B
//! documents against the official UN/CEFACT CII D16B schemas, in pure Rust.
//!
//! This is not a general XML Schema processor: it implements exactly the constructs
//! that the schemas reachable from `UBL-Invoice-2.1.xsd` and `UBL-CreditNote-2.1.xsd`
//! use (see the inventory below), and those that the schemas of the Cross Industry
//! Invoice family add (see [CII-family schemas](#cii-family-schemas)), and refuses any
//! other construct when a schema set is loaded ([`LoadErrorKind::Unsupported`]), so
//! that a schema it cannot handle is never half-checked.
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
//! Thirteen of the fifteen schema documents reachable from the two entry points are
//! vendored byte for byte in `schemas/oasis-ubl-2.1/` and embedded
//! ([`UBL_2_1_FILES`]); [`Schema::ubl_2_1`] loads both entry points into one schema
//! set, as an XML Schema processor given both does. [`Schema::load`] loads any set of
//! schema documents within the supported subset, from memory: `xs:import` and
//! `xs:include` are resolved as paths relative to the importing document, never
//! fetched.
//!
//! The UN/CEFACT CII D16B schemas with uncoupled code lists (D16B SCRDM (Subset),
//! the schemas the CEN EN 16931 CII validation artefacts are built against) are
//! vendored byte for byte in `schemas/uncefact-cii-d16b/` and embedded
//! ([`CII_D16B_FILES`]): the entry point `CrossIndustryInvoice_100pD16B.xsd` and the
//! three modules it imports, its qualified and unqualified data types and its
//! reusable aggregates. Those import no code-list module: code values are checked
//! by the CEN rules, not by the schema. [`Schema::cii_d16b`] loads them.
//!
//! The other two are the ETSI `XAdES` schemas (`UBL-XAdESv132-2.1.xsd` and
//! `UBL-XAdESv141-2.1.xsd`), which the OASIS package includes but no licence allows
//! to redistribute, so they are not shipped. Their two namespaces
//! ([`XADES_NAMESPACES`]) are the only ones whose import may be left out: an
//! `xs:import` of one of them whose document is not in the set is not loaded, and is
//! listed by [`Schema::omitted_imports`]; any other missing document is a
//! [`LoadErrorKind::MissingFile`] error. The UBL schemas only import these
//! namespaces and never refer to their components, so `XAdES` elements can only occur
//! where a lax wildcard allows them (in `ext:ExtensionContent` and in `ds:Object`,
//! where a `XAdES` signature puts its `xades:QualifyingProperties`): without the
//! schemas they have no declaration and are validated laxly, as `xs:anyType`. Their
//! content models, values and attributes are then not checked; their descendants that
//! have a global declaration (`ds:DigestMethod`, `cbc:` elements, ...) still are. A
//! user who has the two files can add them to the embedded ones and load the
//! complete set with [`Schema::load`]: they are then loaded and validated strictly.
//!
//! # Validation
//!
//! [`Schema::validate`] reads the document in one pass of the streaming reader of
//! [`einvoice_xml`] (namespace-aware, no external resources), without building a tree,
//! so that the memory it uses grows with the depth of the document rather than its size;
//! a document with a document type declaration, which that reader refuses, is parsed
//! with `roxmltree` instead (internal DTD subset allowed for entities).
//! [`Schema::validation`] is the same validation for a caller that reads the document
//! itself and gives it the reader's events. Either returns a [`Report`] listing
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
//!   type the schemas use, the `enumeration`, `minLength` and `maxLength` facets
//!   (lengths in characters, after whitespace processing, for the types derived from
//!   `xs:string`), and uniqueness of `xs:ID` values;
//! * value constraints (`default` and `fixed`) of element and attribute
//!   declarations: a fixed value is compared with the value of the element or
//!   attribute in the value space of its type (cvc-elt.5.2.2, cvc-attribute.4), so
//!   that `1.5` matches a fixed `1.50` of an `xs:decimal` and `1` a fixed `true` of
//!   an `xs:boolean`; an element with no character or element children takes its
//!   declaration's default or fixed value, which is then valid (cvc-elt.5.1; with
//!   `xsi:type`, it must be valid for the actual type). This crate reports validity
//!   only and does not build the post-schema-validation infoset, so defaults are not
//!   added to the document: an absent attribute with a default, or an empty element,
//!   is simply valid.
//!
//! The decimal and integer types have no digit limit, dates follow the XML Schema
//! 1.0 rules (no year zero, at least four year digits, timezones up to ±14:00,
//! `24:00:00` as end of day), `anyURI` is checked as XML Schema 1.0 defines it
//! (escaping per `XLink` 1.0 section 5.4, then RFC 2396 as amended by RFC 2732).
//!
//! # CII-family schemas
//!
//! [`Schema::load`] also loads the schema sets of the UN/CEFACT Cross Industry
//! Invoice: CII D16B (the schemas of the CEN EN 16931 CII validation artefacts,
//! with and without the coupled code lists), CII D22B (with and without code lists),
//! and the five Factur-X (`ZUGFeRD`) profiles. Of these, only CII D16B with uncoupled
//! code lists is shipped with this crate ([`Schema::cii_d16b`]); the CII D22B
//! schemas carry no licence notice that allows their redistribution. Over the UBL
//! subset, they need:
//!
//! * `fixed` and `default` on `xs:attribute` (D16B, D22B, Factur-X EXTENDED);
//! * anonymous `xs:complexType` in local element declarations (every set has some:
//!   the `DateTimeString`, `DateString` or `IndicatorString` elements of the data
//!   types, with simple content); any content the subset supports is accepted
//!   there (simple, element-only, mixed or empty);
//! * `minLength` and `maxLength` on restrictions of `xs:token` (D16B and D22B);
//! * `enumeration` on restrictions of `xs:token`, imports of the code-list schemas
//!   and finite `maxOccurs` above one (already in the subset).
//!
//! `fixed` and `default` on `xs:element` are supported the same way, although none
//! of these schemas use them. What stays outside the subset, refused when the schema
//! is loaded: anonymous simple types, anonymous complex types in global element
//! declarations, a fixed value of a date or time type (comparing such values is not
//! implemented; a default one is supported), a value constraint on an element with
//! mixed content, the length facets on `xs:anyURI` and `xs:base64Binary`, the
//! `length` facet, facets with `fixed="true"`, and a restriction redeclaring an
//! attribute that has a fixed value in its base type.
//!
//! # Comparison with xmllint
//!
//! The test suite compares every verdict with xmllint (libxml2 2.9.14), which loads
//! the complete OASIS schema set (`XAdES` schemas included), on the CEN EN 16931 UBL
//! test corpus and on synthetic mutations covering each construct (`tests/oracle/`).
//! Every corpus case agrees. The other differences are listed with their cause in
//! `tests/oracle/disagreements.tsv`: the cases where xmllint departs from XML Schema
//! 1.0 and this crate does not, and the cases with `XAdES` content, which this crate
//! validates laxly with the shipped schemas. The `xmllint-oracle` example also runs
//! this crate with the complete set, where the second kind does not occur.
//!
//! A second suite does the same on a synthetic schema set in the style of the CII
//! schemas (`tests/oracle/synthetic/`), covering value constraints, length facets
//! and anonymous complex types, with mutations of its documents and of the schemas
//! themselves (whether the edited set loads). Its listed differences are cases where
//! libxml2 departs from XML Schema 1.0 (it compares the text of an element with a
//! fixed value as a string, before whitespace processing, rather than its value;
//! it accepts length facets that contradict each other), cases with more errors
//! reported by libxml2 for the same verdict, and constructs refused as unsupported.
//!
//! A third suite compares [`Schema::cii_d16b`] with xmllint loading the same four
//! documents, on every case of the CEN EN 16931 CII test corpus and every one of its
//! mutation cases (`tests/oracle/cii/`). Every corpus case agrees; the mutation
//! cases that do not are listed with their cause (decimals beyond libxml2's 24
//! digits, a document deeper than libxml2's parser accepts, a second error reported
//! by libxml2 for content already invalid). The same comparison runs locally on the
//! CII corpora of the fetched rule sets (XRechnung, Factur-X, the French rules) when
//! they are built.
//!
//! # Licences
//!
//! The code is MIT; the embedded schemas keep the terms stated in their own headers
//! (OASIS and UN/CEFACT for the UBL 2.1 schemas, W3C for the XML Signature schema
//! they include, UN/CEFACT for the CII D16B schemas). [`LICENCES`] holds the full
//! text of each, as shipped in the crate's `LICENSES/` directory, for programs that
//! must show them to their users.
//!
//! # Inventory of the UBL 2.1 schemas
//!
//! Counted over the thirteen vendored documents by the `inventory` test, which fails
//! when this table no longer matches them. Annotations are not descended into.
//!
//! In short: 13 schema documents, all with `elementFormDefault="qualified"`; 22
//! `xs:import` (two of them, of the `XAdES` namespaces, not loaded) and one
//! `xs:include` (the extension content type, included by the common extension
//! components); 1,581 global element declarations and 1,168 named complex types;
//! content models made of `xs:sequence` and `xs:choice` of element references, local
//! elements and wildcards, with `minOccurs` 0 or 1 and `maxOccurs` 1 or `unbounded`
//! only. There are no anonymous types, no groups or attribute groups, no substitution
//! groups, no identity constraints, no default or fixed values, no `xs:all`, no lists
//! or unions, and no facets.
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
//! * The W3C XML Signature schema brings the rest: mixed content, nested and
//!   repeated choices and sequences, strict and lax wildcards, `xs:ID` attributes,
//!   local elements typed directly with built-in types, and the three simple types
//!   (restrictions of `xs:base64Binary` twice and of `xs:integer`).
//!
//! The supported subset also covers what the two `XAdES` schemas add, so that the
//! complete OASIS set loads: an attribute wildcard (`xs:anyAttribute`, strict), an
//! abstract type with two complex-content restrictions, and a simple type
//! restricting `xs:string` with the `enumeration` facet.
//!
//! INVENTORY-START
//! | Construct | Count |
//! |---|---:|
//! | `built-in type xs:ID` | 9 |
//! | `built-in type xs:anyURI` | 15 |
//! | `built-in type xs:base64Binary` | 11 |
//! | `built-in type xs:boolean` | 1 |
//! | `built-in type xs:date` | 1 |
//! | `built-in type xs:dateTime` | 1 |
//! | `built-in type xs:decimal` | 4 |
//! | `built-in type xs:integer` | 2 |
//! | `built-in type xs:language` | 2 |
//! | `built-in type xs:normalizedString` | 26 |
//! | `built-in type xs:string` | 20 |
//! | `built-in type xs:time` | 1 |
//! | `xs:annotation` | 2566 |
//! | `xs:any` | 13 |
//! | `xs:any @maxOccurs="1"` | 1 |
//! | `xs:any @maxOccurs="unbounded"` | 5 |
//! | `xs:any @minOccurs="0"` | 6 |
//! | `xs:any @minOccurs="1"` | 1 |
//! | `xs:any @namespace="##any"` | 2 |
//! | `xs:any @namespace="##other"` | 11 |
//! | `xs:any @processContents="lax"` | 11 |
//! | `xs:attribute` | 62 |
//! | `xs:attribute @use="optional"` | 49 |
//! | `xs:attribute @use="required"` | 12 |
//! | `xs:choice` | 6 |
//! | `xs:choice @maxOccurs="unbounded"` | 3 |
//! | `xs:choice @minOccurs="0"` | 1 |
//! | `xs:complexType` | 1168 |
//! | `xs:complexType @mixed="true"` | 8 |
//! | `xs:element (global)` | 1581 |
//! | `xs:element (local, name)` | 22 |
//! | `xs:element (local, name) @minOccurs="0"` | 4 |
//! | `xs:element (local, ref)` | 2278 |
//! | `xs:element (local, ref) @maxOccurs="1"` | 1657 |
//! | `xs:element (local, ref) @maxOccurs="unbounded"` | 603 |
//! | `xs:element (local, ref) @minOccurs="0"` | 2026 |
//! | `xs:element (local, ref) @minOccurs="1"` | 233 |
//! | `xs:extension (in xs:simpleContent)` | 905 |
//! | `xs:import` | 22 |
//! | `xs:import @namespace="http://uri.etsi.org/01903/v1.3.2#"` | 1 |
//! | `xs:import @namespace="http://uri.etsi.org/01903/v1.4.1#"` | 1 |
//! | `xs:import @namespace="http://www.w3.org/2000/09/xmldsig#"` | 1 |
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
//! | `xs:restriction (in xs:simpleContent)` | 7 |
//! | `xs:restriction (in xs:simpleType)` | 3 |
//! | `xs:schema` | 13 |
//! | `xs:schema @attributeFormDefault="unqualified"` | 12 |
//! | `xs:schema @elementFormDefault="qualified"` | 13 |
//! | `xs:sequence` | 255 |
//! | `xs:sequence @maxOccurs="unbounded"` | 3 |
//! | `xs:sequence @minOccurs="0"` | 3 |
//! | `xs:simpleContent` | 912 |
//! | `xs:simpleType` | 3 |
//! INVENTORY-END

mod automaton;
mod cii;
mod datatypes;
mod error;
mod licences;
mod load;
mod model;
mod ubl;
mod uri;
mod validate;

pub use cii::{CII_D16B_ENTRY_POINT, CII_D16B_FILES};
pub use datatypes::Builtin;
pub use error::{DocumentError, ErrorKind, LoadError, LoadErrorKind, Report, ValidationError};
pub use licences::{LICENCES, Licence};
pub use model::{OmittedImport, Schema};
pub use ubl::{CREDIT_NOTE_ENTRY_POINT, INVOICE_ENTRY_POINT, UBL_2_1_FILES, XADES_NAMESPACES};
pub use validate::Validation;

impl Schema {
    /// Loads a schema set from memory. `files` maps paths (relative,
    /// `/`-separated) to the text of each schema document; `entry_points` are the
    /// paths to start from. Every document reachable through `xs:import` and
    /// `xs:include` must be in `files`, except that of an import of one of
    /// [`XADES_NAMESPACES`], which is then not loaded ([`Schema::omitted_imports`]).
    ///
    /// # Errors
    ///
    /// A [`LoadError`]: a missing or malformed document, a construct outside the
    /// supported subset, or an incorrect schema.
    pub fn load(files: &[(&str, &str)], entry_points: &[&str]) -> Result<Self, LoadError> {
        load::load(files, entry_points)
    }

    /// Loads the embedded OASIS UBL 2.1 schemas, with both entry points
    /// ([`INVOICE_ENTRY_POINT`] and [`CREDIT_NOTE_ENTRY_POINT`]). The two imports
    /// of the `XAdES` namespaces are not loaded ([`Schema::omitted_imports`]):
    /// `XAdES` content is validated laxly.
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

    /// Loads the embedded UN/CEFACT Cross Industry Invoice D16B schemas (uncoupled
    /// code lists, the schemas of the CEN EN 16931 CII validation artefacts), from
    /// their entry point [`CII_D16B_ENTRY_POINT`]: the four documents of
    /// [`CII_D16B_FILES`], no import omitted.
    ///
    /// ```
    /// let schema = einvoice_xsd::Schema::cii_d16b()?;
    /// let report = schema.validate(
    ///     r#"<rsm:CrossIndustryInvoice xmlns:rsm="urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100"/>"#,
    /// )?;
    /// // The exchanged document context and the rest are missing.
    /// assert_eq!(report.errors[0].kind.as_str(), "missing-element");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// None in practice: the embedded schemas are within the supported subset (a
    /// test loads them). The error is returned rather than hidden.
    pub fn cii_d16b() -> Result<Self, LoadError> {
        Self::load(CII_D16B_FILES, &[CII_D16B_ENTRY_POINT])
    }

    /// Validates a document, in one pass of the streaming reader of the `einvoice-xml`
    /// crate over the text (no tree is built). A document with a document type
    /// declaration, which that reader refuses, is read with `roxmltree` instead (the
    /// entities of its internal subset expanded, attribute defaults not applied).
    ///
    /// # Errors
    ///
    /// [`DocumentError::Malformed`] when the text is not well-formed XML with
    /// namespaces, wherever the error is (it comes before the others);
    /// [`DocumentError::Unsupported`] when the document needs a feature outside the
    /// implemented subset (`xsi:type` naming a built-in type the schemas do not use).
    /// Validity errors are not errors: they are in the [`Report`].
    pub fn validate(&self, xml: &str) -> Result<Report, DocumentError> {
        validate::validate(self, xml)
    }

    /// A validation of `text` to be given its events, read by the caller with an
    /// [`einvoice_xml::Reader`] of `text`: for a caller that checks the document
    /// otherwise in the same pass (its depth, ...). [`Schema::validate`] is this
    /// validation given every event of the reader, the reader's errors being
    /// [`DocumentError::Malformed`].
    ///
    /// ```
    /// use einvoice_xml::Reader;
    ///
    /// let schema = einvoice_xsd::Schema::ubl_2_1()?;
    /// let text = r#"<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"/>"#;
    /// let mut validation = schema.validation(text);
    /// let mut reader = Reader::new(text);
    /// while let Some(event) = reader.next_event()? {
    ///     validation.event(event);
    /// }
    /// let report = validation.finish()?;
    /// assert_eq!(report, schema.validate(text)?);
    /// assert_eq!(report.errors[0].kind.as_str(), "missing-element");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn validation<'a>(&self, text: &'a str) -> Validation<'_, 'a> {
        Validation::new(self, text)
    }

    /// Validates a document given as bytes, in any encoding that
    /// [`einvoice_xml::decode`] reads (UTF-8, UTF-16, US-ASCII, ISO-8859-1, ISO-8859-15,
    /// windows-1252), as its byte order mark, first bytes and XML declaration say.
    ///
    /// # Errors
    ///
    /// As [`Schema::validate`]; also [`DocumentError::Unsupported`] for another
    /// encoding, and [`DocumentError::Malformed`] for bytes that are not a character of
    /// the encoding or first bytes and an XML declaration that disagree.
    pub fn validate_bytes(&self, bytes: &[u8]) -> Result<Report, DocumentError> {
        let text = einvoice_xml::decode(bytes).map_err(|e| match e {
            einvoice_xml::Error::UnsupportedEncoding(name) => DocumentError::Unsupported {
                line: 1,
                column: 1,
                message: format!("the {name} encoding"),
            },
            e => DocumentError::Malformed {
                line: e.pos().row,
                column: e.pos().col,
                message: e.to_string(),
            },
        })?;
        self.validate(&text)
    }
}
