//! Validate EN 16931 and Peppol BIS Billing 3.0 e-invoices against the official
//! validation artefacts, in pure Rust: no JVM, no C library, no FFI.
//!
//! A [`Validator`] is built once from a rule set: the XML schema it names is loaded and
//! its official compiled XSLT stylesheets are prepared. It then validates any number of
//! documents, from any number of threads ([`Validator`] is `Send + Sync`). Each
//! validation runs, in order:
//!
//! 1. the XML check: UTF-8, well-formed, namespace-well-formed, no document type
//!    declaration ([`Error::NotXml`], [`Error::Unsupported`]);
//! 2. the XML schema of the rule set, if any (UBL 2.1 for the UBL rule sets), with
//!    [`einvoice_xsd`]: every validity error, with its line and column;
//! 3. every stylesheet of the rule set, with [`einvoice_xslt_exec`], which executes the
//!    official compiled Schematron with the behaviour of the reference engine
//!    (Saxon-HE 12.10) and writes its SVRL report.
//!
//! The [`Report`] holds the schema errors, then, for each stylesheet, its raw SVRL
//! report, its findings (failed asserts and successful reports, with their id, flag,
//! location, rule context, test and message) and the canonical form of the SVRL. The
//! verdict is [`Verdict::Valid`] when there is no schema error and no finding with flag
//! `fatal`.
//!
//! ```
//! # #[cfg(feature = "cen-ubl")] {
//! use einvoice_validator::{Validator, Verdict};
//!
//! let validator = Validator::cen_ubl()?;
//! let invoice = br#"<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"/>"#;
//! let report = validator.validate(invoice)?;
//! assert_eq!(report.verdict(), Verdict::Invalid);
//! // The schema wants the mandatory elements; the CEN rules report what is missing.
//! assert!(!report.schema_errors.is_empty());
//! assert!(report.findings().any(|f| f.id.as_deref() == Some("BR-01")));
//! assert_eq!(report.rule_set.version, "1.3.17");
//! # }
//! # Ok::<(), einvoice_validator::Error>(())
//! ```
//!
//! # Rule sets and bundles
//!
//! A rule set is described by a bundle: a manifest and the files it names. The
//! manifest gives the rule set's name, title and version, the XML schema to validate
//! against first, and the stylesheets to run, each with the SHA-256 of its file, which
//! is checked when the bundle is loaded. [`RuleSet`] is the manifest, parsed.
//!
//! - Built in ([`BUILTIN_RULE_SETS`], [`Validator::builtin`]): `cen-ubl`, the CEN
//!   EN 16931 validation artefacts for UBL, release 1.3.17, embedded unmodified (feature
//!   `cen-ubl`, on by default; the artefacts are under the EUPL-1.2).
//! - From a directory ([`Validator::from_bundle_dir`]): the manifest is
//!   `<dir>/bundle.toml` ([`MANIFEST_FILE`]), and the paths it gives are relative to
//!   `<dir>`. This is how rule sets whose artefacts cannot be redistributed are used:
//!   fetch them into a directory, add the manifest.
//! - From memory ([`Validator::from_bundle`]).
//!
//! The manifest is TOML, restricted to the subset below; anything else is refused with
//! the line concerned (unknown keys included).
//!
//! ```toml
//! format = 1                   # the manifest format, MANIFEST_FORMAT
//! name = "cen-ubl"             # ASCII letters, digits, '-', '.', '_'
//! title = "CEN EN 16931 UBL"
//! version = "1.3.17"           # the version of the official artefacts
//! schema = "ubl-2.1"           # optional: the XML schema validated first
//!
//! [[stylesheet]]               # one or more, run in this order
//! name = "EN16931-UBL"         # unique in the bundle
//! path = "ubl/xslt/EN16931-UBL-validation.xslt"   # relative, '/'-separated, no '..'
//! sha256 = "fc4e6cfa8596bba0c2abd19038a98960c6ee9fc1f44566787c115efd4f4aca7c"
//! # version = "..."            # optional: when it differs from the bundle's
//! ```
//!
//! Accepted syntax: comments, `key = value` lines with a bare key, values that are
//! basic strings (escapes `\"` and `\\` only), literal strings or non-negative integers,
//! and `[[stylesheet]]` headers.
//!
//! # WebAssembly
//!
//! The crate builds for `wasm32-unknown-unknown` and needs nothing from the host there:
//! the built-in rule set is embedded, and a bundle can be given from memory. That target
//! has no system clock, so the caller gives the one the stylesheets read with
//! [`ValidateOptions::clock`] (for instance [`Clock::from_unix_millis`] with
//! JavaScript's `Date.now()`); without it, a validation fails with
//! [`Error::Unsupported`]. The `wasm-validate` example validates two embedded documents
//! that way and runs under wasmtime.
//!
//! # Errors
//!
//! A schema error or a failed assert is not an error: it is in the [`Report`]. An
//! [`Error`] means no complete verdict could be given: a bundle that cannot be loaded,
//! a document that is not XML, a construct the engine refuses rather than evaluating it
//! differently from the reference engine, a dynamic error raised by a stylesheet, or a
//! bug.

mod builtin;
mod error;
mod manifest;
mod report;
mod validator;

pub use builtin::BUILTIN_RULE_SETS;
pub use error::{Error, ErrorKind};
pub use manifest::{MANIFEST_FILE, MANIFEST_FORMAT, RuleSet, SchemaId, StylesheetEntry};
pub use report::{
    Finding, FindingKind, Report, RuleSetVersion, SchemaError, SchemaErrorKind, Severity,
    StylesheetFailure, StylesheetReport, Verdict,
};
pub use validator::{ValidateOptions, Validator, file_uri};

/// The clock of a validation ([`ValidateOptions::clock`]): the implicit timezone and the
/// value of `fn:current-dateTime()`.
pub use einvoice_xpath_eval::Clock;

/// The version of this crate, as published.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn version_is_major_minor_patch() {
        let parts: Vec<&str> = VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "{VERSION}");
        for part in parts {
            assert!(
                !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()),
                "{VERSION}"
            );
        }
    }
}
