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
//! 2. the XML schema of the rule set, if any (UBL 2.1 for the UBL rule sets, CII D16B
//!    for `cen-cii`), with [`einvoice_xsd`]: every validity error, with its line and
//!    column. The XML check and the schema run in one pass of the streaming reader of
//!    [`einvoice_xml`] over the text, which builds no tree;
//! 3. every stylesheet of the rule set, with [`einvoice_xslt_exec`], which executes the
//!    official compiled Schematron with the behaviour of the reference engine
//!    (Saxon-HE 12.10) and writes its SVRL report. In a rule set that selects by
//!    scenario ([`RuleSet::selects_by_scenario`], the XRechnung bundles), the `match`
//!    expressions of its scenarios are evaluated on the document first, as the KoSIT
//!    validator evaluates them: the one scenario that matches gives the stylesheets that
//!    run; a document that matches none, or more than one, is invalid
//!    ([`Report::no_scenario`]) and nothing runs on it.
//!
//! The [`Report`] holds the schema errors, then, for each stylesheet, its raw SVRL
//! report, its findings (failed asserts and successful reports, with their id, flag,
//! location, rule context, test and message) and the canonical form of the SVRL. The
//! verdict is [`Verdict::Valid`] when there is no schema error and no finding of severity
//! [`Severity::Fatal`]: the flag `fatal`, or what the manifest of the rule set reads as
//! fatal, as the reference validator of the rule set does (see [Severities](#severities)),
//! and, in a rule set that selects by scenario, the document matched one.
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
//!   UBL 2.1 schemas; `cen-cii`, the same release for UN/CEFACT CII D16B, with the
//!   UN/CEFACT CII D16B schemas. [`Syntax::detect`] tells from the root element which of
//!   the two a document is for.
//! - From a directory ([`Validator::from_bundle_dir`]): the manifest is
//!   `<dir>/bundle.toml` ([`MANIFEST_FILE`]), and the paths it gives are relative to
//!   `<dir>`. This is how rule sets whose artefacts cannot be redistributed are used:
//!   fetch them into a directory, add the manifest.
//! - From memory ([`Validator::from_bundle`]).
//! - Fetched on demand from their publishers, and kept in a cache
//!   ([`RuleSetCache`], [`Validator::from_cache`]): see the next section.
//!
//! # Rule sets fetched on demand
//!
//! Factur-X (the five profiles), XRechnung (UBL and CII), the French rules (the
//! fourteen bundles of the FNFE-MPE package) and Peppol BIS Billing 3.0 may not be
//! redistributed, so the crate does not embed them: [`fetched_rule_sets`] is their
//! catalogue, which gives, for each, the files of its family at their publishers (pinned
//! URL and SHA-256, the pins of the fetch scripts of the repository) and the
//! manifest of its bundle. Not on WebAssembly.
//!
//! - With the feature `fetch` (off by default; on in `einvoice-cli`),
//!   `RuleSetCache::fetch` downloads the files of a family (or takes them from a
//!   local directory, `FetchOptions::from_dir`), checks each against its pinned
//!   digest, assembles the bundles exactly as the fetch scripts do (a test checks they
//!   are the same, byte for byte) and installs them in the cache in one rename. Peppol
//!   BIS Billing publishes its rules as Schematron only: its bundle is built by
//!   compiling them as ph-schematron compiles them for the reference validators, in
//!   Rust (`einvoice-schematron`, with the ISO Schematron skeleton of the pinned jar of
//!   ph-schematron-isosch): no Java. The HTTP client is
//!   minimal and pure Rust, its cryptography included (rustls, with a provider on the
//!   `RustCrypto` crates, and the Mozilla roots of webpki-roots): HTTPS only (plain
//!   HTTP only to a loopback address, for a local mirror), at most five redirects,
//!   never from HTTPS to HTTP, five attempts with a doubling delay on an I/O error and
//!   on HTTP 408, 429 and 5xx (honouring `Retry-After`), an interrupted download resumed
//!   where it stopped, a size limit (`DEFAULT_MAX_SIZE`, 64 MiB); the archives (ZIP,
//!   gzip, tar) are read in memory, refusing absolute paths, `..`, links and sizes
//!   beyond their limits.
//! - Nothing is downloaded otherwise: [`RuleSetCache::load`] and
//!   [`Validator::from_cache`] read the cache only, and fail with
//!   [`Error::NotCached`] for a rule set that has not been fetched. On every load the
//!   cached manifest must be, byte for byte, the catalogue's, and every file must have
//!   its SHA-256, so a cache works offline and is never trusted.
//!
//! ```no_run
//! # #[cfg(feature = "fetch")] {
//! use einvoice_validator::{FetchOptions, RuleSetCache, Validator};
//!
//! // The platform's cache directory, or EINVOICE_VALIDATOR_CACHE.
//! let cache = RuleSetCache::new()?;
//! // Downloads and builds xrechnung-ubl and xrechnung-cii (once: then from the cache).
//! cache.fetch("xrechnung", &FetchOptions::new())?;
//! let validator = cache.load("xrechnung-ubl")?;
//! // The same, from the default cache, never downloading.
//! let same = Validator::from_cache("xrechnung-ubl")?;
//! assert_eq!(validator.rule_set(), same.rule_set());
//! # }
//! # Ok::<(), einvoice_validator::Error>(())
//! ```
//!
//! The manifest is TOML, restricted to the subset below; anything else is refused with
//! the line concerned (unknown keys included).
//!
//! ```toml
//! format = 2                   # the manifest format: 1, or MANIFEST_FORMAT for what says format 2
//! name = "cen-ubl"             # ASCII letters, digits, '-', '.', '_'
//! title = "CEN EN 16931 UBL"
//! version = "1.3.17"           # the version of the official artefacts
//! schema = "ubl-2.1"           # optional: the XML schema validated first, ubl-2.1 or cii-d16b
//! # unchecked-schema = "..."   # format 2, optional: an XML schema the reference
//! #                            # validator also checks and this rule set does not
//!
//! [[stylesheet]]               # one or more, run in this order
//! name = "EN16931-UBL"         # unique in the bundle
//! path = "ubl/xslt/EN16931-UBL-validation.xslt"   # relative, '/'-separated, no '..'
//! sha256 = "fc4e6cfa8596bba0c2abd19038a98960c6ee9fc1f44566787c115efd4f4aca7c"
//! # version = "..."            # optional: when it differs from the bundle's
//! # unflagged-assert = "fatal" # format 2, optional: the severity of a failed assert
//! #                            # without a flag (fatal, warning or other; default other)
//!
//! [[resource]]                 # format 2: zero or more files read with document()
//! path = "ubl/xslt/codes.xml"  # relative, '/'-separated, no '..'
//! sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
//!
//! [[scenario]]                 # format 2: zero or more, tried in this order
//! name = "EN16931 XRechnung (UBL Invoice)"        # unique in the bundle
//! # A scenario of a KoSIT configuration: its namespaces (prefix=uri, separated by
//! # spaces), its match expression (XPath, as written) and the stylesheets it runs.
//! namespaces = "cbc=urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2 invoice=urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"
//! match = "exists(/invoice:Invoice/cbc:CustomizationID[ . = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0'])"
//! stylesheets = "EN16931-UBL XRechnung-UBL"    # optional: by default all, in order
//! fatal = "UBL-CR-646"         # optional: rule ids and the severity the scenario
//! warning = "BR-CL-23 BR-CL-21"   # gives them, separated by spaces, each in one list
//! # Or, instead of namespaces, match and stylesheets: the root element and the
//! # specification identifier (BT-24) of the documents it gives severities to (at least
//! # one of fatal, warning and other then), every stylesheet running.
//! # namespace = "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"
//! # root = "Invoice"           # with namespace: a UBL Invoice or CreditNote, a CII CrossIndustryInvoice
//! # identifier = "urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0"
//! ```
//!
//! The scenarios of a manifest all have a `match` expression, or none has. With them, the
//! rule set selects by scenario ([`Scenario`]): a document must match exactly one, whose
//! stylesheets run, in its order, and whose severities apply; a document that matches none,
//! or several, is invalid, with no schema error reported and no stylesheet run, as the
//! KoSIT validator rejects it with its fallback scenario (which has no schema). The expressions are evaluated
//! with [`einvoice_xpath_eval`], with the document node as the context item, their
//! effective boolean value; one that raises a dynamic error is false, as for the KoSIT
//! validator; one outside the engine's subset is refused when the bundle is loaded.
//!
//! Accepted syntax: comments, `key = value` lines with a bare key, values that are
//! basic strings (escapes `\"` and `\\` only), literal strings or non-negative integers,
//! and `[[stylesheet]]`, `[[resource]]` and `[[scenario]]` headers.
//!
//! # Severities
//!
//! Each finding has a [`Severity`], and a fatal finding makes the document invalid. By
//! default it is the one its Schematron flag stands for ([`Severity::from_flag`]: `fatal`,
//! `warning`, any other flag or none [`Severity::Other`]), which is how the reference
//! validators read the CEN and Peppol rules, which flag every assert. The manifest gives
//! the others what their reference validators read:
//!
//! - `unflagged-assert`, for a stylesheet whose failed asserts may have no flag: the
//!   Factur-X stylesheets leave it out of most of their asserts, which the Mustang validator
//!   reads as errors, so the Factur-X bundles (and those of the French rules that run a
//!   Factur-X profile) say `unflagged-assert = "fatal"`, as do the XRechnung bundles, the
//!   KoSIT validator reading a missing flag as an error too. A successful report without a
//!   flag stays [`Severity::Other`].
//! - `[[scenario]]` ([`Scenario`]): the levels a scenario of a KoSIT validator
//!   configuration gives rule ids (`customLevel`), whatever their flags, in the documents
//!   it matches; the XRechnung bundles have the scenarios of their configuration, each
//!   with its `match` expression and its stylesheets (`error` is [`Severity::Fatal`],
//!   `information` [`Severity::Other`]). The [`Report`] names the scenario the document
//!   matched, or says why it has none.
//!
//! The KoSIT validator also validates each document against the XML schema of its
//! scenario: the UBL 2.1 schema, which the XRechnung UBL bundle runs, or the UN/CEFACT
//! CII D16B schema, which the XRechnung CII bundle runs ([`SchemaId::CiiD16b`]). A
//! bundle whose reference validator checks a schema this crate does not embed names it
//! in `unchecked-schema` ([`RuleSet::unchecked_schema`],
//! [`RuleSetVersion::unchecked_schema`] in the report): a document it would reject for a
//! schema error alone is valid here.
//!
//! [`ValidateOptions::fail_closed`] then makes every finding that is still
//! [`Severity::Other`] fatal, for rule sets whose flags the caller does not know.
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
//! stylesheets, under the EUPL-1.2, and [`einvoice_xsd`] embeds the OASIS UBL 2.1 schemas
//! and the UN/CEFACT CII D16B schemas, which keep their own terms (OASIS, UN/CEFACT, W3C).
//! [`licences()`] returns the full text of every licence of what this build embeds, for
//! programs that must show them to their users.
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
#[cfg(not(target_family = "wasm"))]
mod rulesets;
mod validator;

pub use builtin::BUILTIN_RULE_SETS;
pub use detect::Syntax;
pub use error::{Error, ErrorKind};
pub use licences::{Licence, licences};
pub use manifest::{
    MANIFEST_FILE, MANIFEST_FORMAT, ResourceEntry, RuleSet, Scenario, SchemaId, StylesheetEntry,
};
pub use report::{
    Finding, FindingKind, NoScenario, Report, RuleSetVersion, SchemaError, SchemaErrorKind,
    Severity, StylesheetFailure, StylesheetReport, Verdict,
};
pub use validator::{ValidateOptions, Validator, file_uri};

#[cfg(not(target_family = "wasm"))]
pub use rulesets::{
    CACHE_ENV, CacheStatus, FetchedRuleSet, RuleSetCache, Source, fetched_rule_set,
    fetched_rule_set_aliases, fetched_rule_sets,
};
#[cfg(all(feature = "fetch", not(target_family = "wasm")))]
pub use rulesets::{DEFAULT_MAX_SIZE, FetchOptions, Fetched, Progress};

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

/// An SVRL report as the stylesheet wrote it, kept compactly
/// ([`StylesheetReport::svrl`]).
pub use einvoice_xslt_exec::CompactOutput;

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
