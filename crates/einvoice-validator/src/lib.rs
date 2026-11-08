//! Validate EN 16931 and Peppol BIS Billing 3.0 e-invoices against the official
//! validation artefacts, in pure Rust: no JVM, no C library, no FFI.
//!
//! A [`Validator`] is built once from a rule set: the XML schema it names is loaded and
//! its official compiled XSLT stylesheets are prepared. It then validates any number of
//! documents, from any number of threads ([`Validator`] is `Send + Sync`). Each
//! validation runs, in order:
//!
//! 1. the XML check: UTF-8, well-formed, namespace-well-formed, no document type
//!    declaration ([`Error::NotXml`], [`Error::Unsupported`]), no element nested deeper
//!    than [`ValidateOptions::max_depth`] ([`Error::Limit`]);
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
//! against first, the stylesheets to run, and the resources they read with
//! `document()` (code lists), each with the SHA-256 of its file, which is checked when
//! the bundle is loaded. [`RuleSet`] is the manifest, parsed. A stylesheet reads the
//! resources by their URI relative to its own (`file:` URIs in a directory,
//! `bundle:/<path>` in memory), and nothing else: `document()` on any other URI is the
//! error `FODC0002`.
//!
//! - Built in ([`BUILTIN_RULE_SETS`], [`Validator::builtin`]), embedded unmodified, each
//!   behind a feature of its name, on by default (the artefacts are under the EUPL-1.2):
//!   `cen-ubl`, the CEN EN 16931 validation artefacts for UBL, release 1.3.17, with the
//!   UBL 2.1 schemas; `cen-cii`, the same release for UN/CEFACT CII D16B, without an XML
//!   schema (the CII D16B schemas are not embedded: documents are checked for
//!   well-formedness, then by the stylesheet). [`Syntax::detect`] tells from the root
//!   element which of the two a document is for.
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
//! format = 2                   # the manifest format: 1, or MANIFEST_FORMAT for [[resource]]
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
//!
//! [[resource]]                 # format 2: zero or more files read with document()
//! path = "ubl/xslt/codes.xml"  # relative, '/'-separated, no '..'
//! sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
//! ```
//!
//! Accepted syntax: comments, `key = value` lines with a bare key, values that are
//! basic strings (escapes `\"` and `\\` only), literal strings or non-negative integers,
//! and `[[stylesheet]]` and `[[resource]]` headers.
//!
//! # PDF files
//!
//! With the feature `pdf` (on by default), the module `pdf` extracts the invoice XML that
//! a Factur-X, `ZUGFeRD` or Order-X PDF/A-3 file embeds, with its `/AFRelationship` and
//! the document's XMP metadata, in pure Rust (the crate einvoice-pdf); the XML is then
//! validated as any other document, with the rule set [`Syntax::detect`] names for it.
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
//! # Licences
//!
//! The code is MIT. The built-in rule sets `cen-ubl` and `cen-cii` embed the CEN
//! stylesheets, under the EUPL-1.2, and [`einvoice_xsd`] embeds the OASIS UBL 2.1 schemas, which keep their own
//! terms (OASIS, UN/CEFACT, W3C). [`licences()`] returns the full text of every licence
//! of what this build embeds, for programs that must show them to their users.
//!
//! # Errors
//!
//! A schema error or a failed assert is not an error: it is in the [`Report`]. An
//! [`Error`] means no complete verdict could be given: a bundle that cannot be loaded,
//! a document that is not XML, a construct the engine refuses rather than evaluating it
//! differently from the reference engine, a document beyond a limit, a dynamic error
//! raised by a stylesheet, or a bug.
//!
//! # Limits
//!
//! The stylesheets walk the document with template rules applied one element level
//! inside another, so the stack a validation needs grows with the depth of the document.
//! A document with an element nested deeper than [`ValidateOptions::max_depth`]
//! ([`DEFAULT_MAX_DEPTH`], 256, libxml2's default) is therefore refused before its schema
//! is checked, with [`Error::Limit`] and the line and column of that element, rather
//! than overflowing the stack of the thread, which would abort the process. At the
//! default maximum, a validation with the built-in rule sets or the Peppol bundle runs on
//! a thread with the default 2 MiB stack of a spawned thread, debug builds included. The
//! template and function invocations of the stylesheets are bounded too: in nesting
//! ([`max_nesting_for_depth`] of the maximum depth) and in the stack they use
//! ([`ValidateOptions::max_stack`], [`DEFAULT_MAX_STACK`]), so that a stylesheet that
//! recurses without end fails with [`Error::Limit`] as well.

mod builtin;
mod detect;
mod error;
mod licences;
mod manifest;
#[cfg(feature = "pdf")]
pub mod pdf;
mod report;
mod resources;
mod validator;

pub use builtin::BUILTIN_RULE_SETS;
pub use detect::Syntax;
pub use error::{Error, ErrorKind};
pub use licences::{Licence, licences};
pub use manifest::{
    MANIFEST_FILE, MANIFEST_FORMAT, ResourceEntry, RuleSet, SchemaId, StylesheetEntry,
};
pub use report::{
    Finding, FindingKind, Report, RuleSetVersion, SchemaError, SchemaErrorKind, Severity,
    StylesheetFailure, StylesheetReport, Verdict,
};
pub use validator::{ValidateOptions, Validator, file_uri};

/// The clock of a validation ([`ValidateOptions::clock`]): the implicit timezone and the
/// value of `fn:current-dateTime()`.
pub use einvoice_xpath_eval::Clock;

/// The default maximum element depth of a document ([`ValidateOptions::max_depth`]), and
/// the limits [`Error::Limit`] reports.
pub use einvoice_xpath_eval::{DEFAULT_MAX_DEPTH, LimitExceeded};

/// The nesting of template and function invocations the stylesheets are allowed on a
/// document at most `max_depth` deep ([`ValidateOptions::max_depth`]): twice the depth,
/// plus 64. Each level needs less than 3 KiB of stack (debug builds included), which is
/// what a caller raising the maximum depth must give the validating thread.
pub use einvoice_xslt_exec::max_nesting_for_depth;

/// The default stack the stylesheets may use ([`ValidateOptions::max_stack`]).
pub use einvoice_xslt_exec::DEFAULT_MAX_STACK;

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
