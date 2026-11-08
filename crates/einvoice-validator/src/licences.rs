//! The licences of what the crate embeds in a program, with their texts.

pub use einvoice_xsd::Licence;

/// The MIT licence of the code, from `LICENSES/MIT.txt` (the repository's `LICENSE`).
const MIT: Licence = Licence {
    id: "MIT",
    covers: "the code of einvoice-validator and of the crates of the project it is built \
             from (einvoice-svrl, einvoice-xdm, einvoice-xpath, einvoice-xpath-eval, \
             einvoice-xsd, einvoice-xslt, einvoice-xslt-exec)",
    text: include_str!("../LICENSES/MIT.txt"),
};

/// The EUPL-1.2 of the CEN stylesheet of the built-in rule set `cen-ubl`.
#[cfg(feature = "cen-ubl")]
const EUPL: Licence = Licence {
    id: "EUPL-1.2",
    covers: "the CEN EN 16931 UBL validation stylesheet, release 1.3.17 \
             (ubl/xslt/EN16931-UBL-validation.xslt, unmodified), published by CEN/TC 434 \
             and embedded by einvoice-validator as the built-in rule set cen-ubl",
    text: include_str!("../LICENSES/EUPL-1.2.txt"),
};

/// Every licence of what this build of the crate embeds in a program, with its full
/// text: the MIT licence of the code (of this crate and of the crates of the project
/// it depends on, which share that licence), the EUPL-1.2 of the CEN stylesheet when
/// the feature `cen-ubl` is on, then the terms of the UBL 2.1 schemas embedded by
/// [`einvoice_xsd`] (OASIS, UN/CEFACT, W3C).
///
/// The W3C licence of the XML Signature schema asks for its full notice "in a location
/// viewable to users": a program built on this crate can show these texts, as
/// `einvoice-validate --licences` does. Rule sets loaded from bundles at run time are
/// not embedded and not listed.
#[must_use]
pub fn licences() -> Vec<Licence> {
    let mut out = vec![MIT];
    #[cfg(feature = "cen-ubl")]
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
