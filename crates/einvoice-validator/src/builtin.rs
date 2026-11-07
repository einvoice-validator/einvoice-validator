//! The rule sets embedded in the crate, each behind a cargo feature.

use crate::Error;

/// The names of the built-in rule sets of this build (each is a cargo feature of the
/// same name, `cen-ubl` on by default).
pub const BUILTIN_RULE_SETS: &[&str] = &[
    #[cfg(feature = "cen-ubl")]
    "cen-ubl",
];

/// The files of a bundle: path in the manifest, content.
pub(crate) type Files = &'static [(&'static str, &'static [u8])];

/// The manifest and the files of the built-in rule set `name`.
pub(crate) fn get(name: &str) -> Result<(&'static str, Files), Error> {
    match name {
        #[cfg(feature = "cen-ubl")]
        // Vendored in the crate (bundles/cen-ubl/README.md), from
        // cen-en16931-ubl-1.3.17.
        "cen-ubl" => Ok((
            include_str!("../bundles/cen-ubl/bundle.toml"),
            &[(
                "ubl/xslt/EN16931-UBL-validation.xslt",
                include_bytes!("../bundles/cen-ubl/ubl/xslt/EN16931-UBL-validation.xslt"),
            )],
        )),
        _ => Err(Error::bundle(format!(
            "no built-in rule set named {name:?} (built in: {})",
            if BUILTIN_RULE_SETS.is_empty() {
                "none".to_owned()
            } else {
                BUILTIN_RULE_SETS.join(", ")
            }
        ))),
    }
}
