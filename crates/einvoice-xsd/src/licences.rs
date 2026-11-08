//! The licences of the crate, with their texts, embedded from `LICENSES/`.

/// A licence under which part of a crate is distributed: one term of the crate's
/// licence expression, what it covers, and its full text, as shipped in the crate's
/// `LICENSES/` directory.
///
/// A program that embeds the crate can show these texts to its users (the W3C licence
/// of the XML Signature schema asks for its notice "in a location viewable to users").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Licence {
    /// The SPDX identifier, or `LicenseRef-` reference, of the licence expression.
    pub id: &'static str,
    /// What it covers, in a sentence.
    pub covers: &'static str,
    /// The full text.
    pub text: &'static str,
}

/// The licences of this crate, one per term of its licence expression (`MIT AND
/// LicenseRef-OASIS-Copyright-Notice AND LicenseRef-UN-CEFACT-Copyright-Notice AND
/// W3C-19980720`): its code, then the three sets of terms of the embedded UBL 2.1
/// schemas ([`UBL_2_1_FILES`](crate::UBL_2_1_FILES)), the UN/CEFACT one also covering
/// the embedded CII D16B schemas ([`CII_D16B_FILES`](crate::CII_D16B_FILES)).
pub const LICENCES: &[Licence] = &[
    Licence {
        id: "MIT",
        covers: "the code of einvoice-xsd",
        text: include_str!("../LICENSES/MIT.txt"),
    },
    Licence {
        id: "LicenseRef-OASIS-Copyright-Notice",
        covers: "the OASIS UBL 2.1 schema modules embedded by einvoice-xsd \
                 (xsd/maindoc/UBL-*-2.1.xsd and xsd/common/UBL-*-2.1.xsd, except \
                 UBL-xmldsig-core-schema-2.1.xsd), copyright OASIS Open 2013",
        text: include_str!("../LICENSES/LicenseRef-OASIS-Copyright-Notice.txt"),
    },
    Licence {
        id: "LicenseRef-UN-CEFACT-Copyright-Notice",
        covers: "xsd/common/CCTS_CCT_SchemaModule-2.1.xsd of the UBL 2.1 schemas embedded \
                 by einvoice-xsd, copyright UN/CEFACT 2006; and the UN/CEFACT CII D16B \
                 schemas embedded by einvoice-xsd, unmodified, copyright UN/CEFACT 2016: \
                 uncefact/data/standard/CrossIndustryInvoice_100pD16B.xsd, \
                 uncefact/data/standard/CrossIndustryInvoice_QualifiedDataType_100pD16B.xsd, \
                 uncefact/data/standard/CrossIndustryInvoice_ReusableAggregateBusinessInformationEntity_100pD16B.xsd \
                 and uncefact/data/standard/CrossIndustryInvoice_UnqualifiedDataType_100pD16B.xsd",
        text: include_str!("../LICENSES/LicenseRef-UN-CEFACT-Copyright-Notice.txt"),
    },
    Licence {
        id: "W3C-19980720",
        covers: "xsd/common/UBL-xmldsig-core-schema-2.1.xsd of the UBL 2.1 schemas embedded \
                 by einvoice-xsd: the W3C XML Signature schema (copyright The Internet \
                 Society and W3C), as modified by OASIS (DOCTYPE identifiers removed) and \
                 published in the UBL 2.1 package; einvoice-xsd does not modify it",
        text: include_str!("../LICENSES/W3C-19980720.txt"),
    },
];
