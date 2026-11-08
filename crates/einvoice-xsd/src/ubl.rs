//! The OASIS UBL 2.1 schemas vendored in `schemas/oasis-ubl-2.1/` (see its
//! README for their origin and licence), embedded in the crate, and the two
//! namespaces whose schemas are not shipped.

/// The entry point for UBL 2.1 invoices.
pub const INVOICE_ENTRY_POINT: &str = "xsd/maindoc/UBL-Invoice-2.1.xsd";
/// The entry point for UBL 2.1 credit notes.
pub const CREDIT_NOTE_ENTRY_POINT: &str = "xsd/maindoc/UBL-CreditNote-2.1.xsd";

/// The namespaces of the ETSI `XAdES` schemas (ETSI TS 101 903, versions 1.3.2 and
/// 1.4.1), which the OASIS UBL 2.1 package includes as
/// `xsd/common/UBL-XAdESv132-2.1.xsd` and `xsd/common/UBL-XAdESv141-2.1.xsd` and
/// which this crate does not ship: they carry no licence that allows their
/// redistribution.
///
/// This is the closed list of namespaces whose import may be left out of a schema
/// set: an `xs:import` of one of them whose schema document is not in the set is not
/// loaded and is recorded in [`Schema::omitted_imports`](crate::Schema::omitted_imports);
/// when the document is in the set, it is loaded as any other. A missing document
/// for any other import is a [`LoadErrorKind::MissingFile`](crate::LoadErrorKind)
/// error, and so is a reference from a loaded schema document to a component of an
/// omitted namespace. In the UBL 2.1 schemas the two namespaces are only imported
/// (by `UBL-SignatureAggregateComponents-2.1.xsd`), never referred to: their
/// elements can only occur where a lax wildcard allows them, in the UBL extension
/// content (`ext:ExtensionContent`) and in `ds:Object`, and are then validated
/// laxly: as `xs:anyType`, so that only their descendants with a global declaration
/// (the `ds:` elements of the W3C XML Signature schema, for instance) are validated
/// against it.
pub const XADES_NAMESPACES: [&str; 2] = [
    "http://uri.etsi.org/01903/v1.3.2#",
    "http://uri.etsi.org/01903/v1.4.1#",
];

macro_rules! vendored {
    ($($path:literal),* $(,)?) => {
        /// Every vendored schema document: its path relative to the root of the
        /// OASIS UBL 2.1 package, and its text (byte-identical to the published file).
        /// The two `XAdES` schemas of the package are not among them (see
        /// [`XADES_NAMESPACES`]).
        pub const UBL_2_1_FILES: &[(&str, &str)] = &[
            $(($path, include_str!(concat!("../schemas/oasis-ubl-2.1/", $path))),)*
        ];
    };
}

vendored!(
    "xsd/common/CCTS_CCT_SchemaModule-2.1.xsd",
    "xsd/common/UBL-CommonAggregateComponents-2.1.xsd",
    "xsd/common/UBL-CommonBasicComponents-2.1.xsd",
    "xsd/common/UBL-CommonExtensionComponents-2.1.xsd",
    "xsd/common/UBL-CommonSignatureComponents-2.1.xsd",
    "xsd/common/UBL-ExtensionContentDataType-2.1.xsd",
    "xsd/common/UBL-QualifiedDataTypes-2.1.xsd",
    "xsd/common/UBL-SignatureAggregateComponents-2.1.xsd",
    "xsd/common/UBL-SignatureBasicComponents-2.1.xsd",
    "xsd/common/UBL-UnqualifiedDataTypes-2.1.xsd",
    "xsd/common/UBL-xmldsig-core-schema-2.1.xsd",
    "xsd/maindoc/UBL-CreditNote-2.1.xsd",
    "xsd/maindoc/UBL-Invoice-2.1.xsd",
);
