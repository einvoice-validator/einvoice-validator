//! The UN/CEFACT Cross Industry Invoice D16B schemas (uncoupled code lists) vendored
//! in `schemas/uncefact-cii-d16b/` (see its README for their origin and licence),
//! embedded in the crate.

/// The entry point of the CII D16B schemas: the schema of `rsm:CrossIndustryInvoice`
/// (namespace `urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100`).
pub const CII_D16B_ENTRY_POINT: &str = "uncefact/data/standard/CrossIndustryInvoice_100pD16B.xsd";

macro_rules! vendored {
    ($($path:literal),* $(,)?) => {
        /// Every vendored schema document of CII D16B: its path relative to the
        /// `CII/` directory of the UNECE package (D16B SCRDM (Subset) CII, uncoupled
        /// code lists), and its text, byte-identical to the published file (CRLF line
        /// ends included). These are the documents the entry point
        /// [`CII_D16B_ENTRY_POINT`] imports, directly or not: no code-list module.
        pub const CII_D16B_FILES: &[(&str, &str)] = &[
            $(($path, include_str!(concat!("../schemas/uncefact-cii-d16b/", $path))),)*
        ];
    };
}

vendored!(
    "uncefact/data/standard/CrossIndustryInvoice_100pD16B.xsd",
    "uncefact/data/standard/CrossIndustryInvoice_QualifiedDataType_100pD16B.xsd",
    "uncefact/data/standard/CrossIndustryInvoice_ReusableAggregateBusinessInformationEntity_100pD16B.xsd",
    "uncefact/data/standard/CrossIndustryInvoice_UnqualifiedDataType_100pD16B.xsd",
);
