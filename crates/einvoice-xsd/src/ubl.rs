//! The OASIS UBL 2.1 schemas vendored in `schemas/oasis-ubl-2.1/` (see its
//! README for their origin and licence), embedded in the crate.

/// The entry point for UBL 2.1 invoices.
pub const INVOICE_ENTRY_POINT: &str = "xsd/maindoc/UBL-Invoice-2.1.xsd";
/// The entry point for UBL 2.1 credit notes.
pub const CREDIT_NOTE_ENTRY_POINT: &str = "xsd/maindoc/UBL-CreditNote-2.1.xsd";

macro_rules! vendored {
    ($($path:literal),* $(,)?) => {
        /// Every vendored schema document: its path relative to the root of the
        /// OASIS UBL 2.1 package, and its text (byte-identical to the published file).
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
    "xsd/common/UBL-XAdESv132-2.1.xsd",
    "xsd/common/UBL-XAdESv141-2.1.xsd",
    "xsd/common/UBL-xmldsig-core-schema-2.1.xsd",
    "xsd/maindoc/UBL-CreditNote-2.1.xsd",
    "xsd/maindoc/UBL-Invoice-2.1.xsd",
);
