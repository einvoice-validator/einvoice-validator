//! The licences of what the crate embeds in a program, with their texts.

pub use einvoice_xsd::Licence;

/// The MIT licence of the code, from `LICENSES/MIT.txt` (the repository's `LICENSE`).
const MIT: Licence = Licence {
    id: "MIT",
    covers: MIT_COVERS,
    text: include_str!("../LICENSES/MIT.txt"),
};

/// What the MIT licence covers in this build: the crates of the project it is built from
/// (einvoice-pdf with the feature `pdf`).
#[cfg(feature = "pdf")]
const MIT_COVERS: &str = "the code of einvoice-validator and of the crates of the project it \
     is built from (einvoice-pdf, einvoice-svrl, einvoice-xdm, einvoice-xml, einvoice-xpath, \
     einvoice-xpath-eval, einvoice-xsd, einvoice-xslt, einvoice-xslt-exec)";
#[cfg(not(feature = "pdf"))]
const MIT_COVERS: &str = "the code of einvoice-validator and of the crates of the project it \
     is built from (einvoice-svrl, einvoice-xdm, einvoice-xml, einvoice-xpath, \
     einvoice-xpath-eval, einvoice-xsd, einvoice-xslt, einvoice-xslt-exec)";

/// The EUPL-1.2 of the CEN stylesheets of the built-in rule sets `cen-ubl` and `cen-cii`.
#[cfg(any(feature = "cen-ubl", feature = "cen-cii"))]
const EUPL: Licence = Licence {
    id: "EUPL-1.2",
    covers: EUPL_COVERS,
    text: include_str!("../LICENSES/EUPL-1.2.txt"),
};

/// What the EUPL-1.2 covers in this build: the stylesheets of the enabled features.
#[cfg(all(feature = "cen-ubl", feature = "cen-cii"))]
const EUPL_COVERS: &str = "the CEN EN 16931 UBL and CII validation stylesheets, release 1.3.17 \
     (ubl/xslt/EN16931-UBL-validation.xslt and cii/xslt/EN16931-CII-validation.xslt, \
     unmodified), published by CEN/TC 434 and embedded by einvoice-validator as the built-in \
     rule sets cen-ubl and cen-cii";
#[cfg(all(feature = "cen-ubl", not(feature = "cen-cii")))]
const EUPL_COVERS: &str = "the CEN EN 16931 UBL validation stylesheet, release 1.3.17 \
     (ubl/xslt/EN16931-UBL-validation.xslt, unmodified), published by CEN/TC 434 and \
     embedded by einvoice-validator as the built-in rule set cen-ubl";
#[cfg(all(feature = "cen-cii", not(feature = "cen-ubl")))]
const EUPL_COVERS: &str = "the CEN EN 16931 CII validation stylesheet, release 1.3.17 \
     (cii/xslt/EN16931-CII-validation.xslt, unmodified), published by CEN/TC 434 and \
     embedded by einvoice-validator as the built-in rule set cen-cii";

/// Every licence of what this build of the crate embeds in a program, with its full
/// text: the MIT licence of the code (of this crate and of the crates of the project
/// it depends on, which share that licence), the EUPL-1.2 of the CEN stylesheets when
/// the feature `cen-ubl` or `cen-cii` is on, then the terms of the UBL 2.1 schemas embedded by
/// [`einvoice_xsd`] (OASIS, UN/CEFACT, W3C).
///
/// The W3C licence of the XML Signature schema asks for its full notice "in a location
/// viewable to users": a program built on this crate can show these texts, as
/// `einvoice-validate --licences` does. Rule sets loaded from bundles at run time are
/// not embedded and not listed.
#[must_use]
pub fn licences() -> Vec<Licence> {
    let mut out = vec![MIT];
    #[cfg(any(feature = "cen-ubl", feature = "cen-cii"))]
    out.push(EUPL);
    // einvoice-xsd's MIT text is the same as this crate's (a test checks it).
    out.extend(
        einvoice_xsd::LICENCES
            .iter()
            .filter(|l| l.id != "MIT")
            .copied(),
    );
    out
}
