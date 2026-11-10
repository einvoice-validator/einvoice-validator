# einvoice-validator

Validate EN 16931 and Peppol BIS Billing 3.0 e-invoices against the **official** validation
artefacts, in pure Rust: no JVM, no C library, no FFI.

A `Validator` is built once from a rule set (the XML schema it names loaded, its official
compiled XSLT stylesheets prepared), is `Send + Sync`, and validates any number of
documents. Each validation runs the XML check (UTF-8, well-formed, no document type
declaration, no element nested deeper than 256 levels or the maximum set with
`ValidateOptions::max_depth`, so that the stylesheets run within a bounded stack), the XML schema of the rule set if any (UBL 2.1, with
[`einvoice-xsd`](https://crates.io/crates/einvoice-xsd)), then every stylesheet of the rule set with
[`einvoice-xslt-exec`](https://crates.io/crates/einvoice-xslt-exec), which executes the
official compiled Schematron with the behaviour of Saxon-HE 12.10. The `Report` holds the
schema errors, the findings of each stylesheet (failed asserts and successful reports with
their id, flag, severity, location and message), the raw SVRL and the verdict: invalid with a
schema error or a fatal finding, fatal as the reference validator of the rule set reads the
report (the flag `fatal`; for Factur-X and XRechnung also a failed assert without a flag, and
for XRechnung the levels the KoSIT scenario of the document gives rule ids, as their bundle
manifests say). The XRechnung bundles select by scenario, as the KoSIT validator runs its
configuration: the `match` expression of each scenario is evaluated on the document, and the
one that matches names the stylesheets that run (the CEN rules alone, or the CEN and XRechnung
rules); a document that matches no scenario, or several, is invalid (`Report::no_scenario`).
Each bundle validates documents against the XML schema of its scenarios first, as the KoSIT
validator does: UBL 2.1, or the UN/CEFACT CII D16B schema.
The XML check and the schema are one pass of the streaming reader
[`einvoice-xml`](https://crates.io/crates/einvoice-xml) over the document, which builds no
tree; the stylesheets run on a tree of the document each, and a rule set that selects by
scenario reads one more, on which the `match` expressions are evaluated. The findings are read
from the SVRL report as the stylesheet writes it, and the report is kept compactly, each
distinct piece of its text once (`StylesheetReport::svrl`): the SVRL of a large invoice, a
fired rule per node, takes a few bytes per fired rule.

```toml
[dependencies]
einvoice-validator = "0.1"
```

```rust
use einvoice_validator::{Validator, Verdict};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Built once: the UBL 2.1 schemas loaded, the CEN stylesheet prepared.
    let validator = Validator::cen_ubl()?;
    let invoice = br#"<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"/>"#;
    let report = validator.validate(invoice)?;
    assert_eq!(report.verdict(), Verdict::Invalid);
    for finding in report.findings() {
        let id = finding.id.as_deref().unwrap_or("-");
        println!("{:?} {id}: {} (at {})", finding.severity, finding.text, finding.location);
    }
    Ok(())
}
```

## Rule sets

- Built in, embedded unmodified, each behind a default feature of its name: `cen-ubl`, the CEN
  EN 16931 validation artefacts for UBL, release 1.3.17, with the UBL 2.1 schemas; `cen-cii`,
  the same release for UN/CEFACT CII D16B, with the UN/CEFACT CII D16B schemas.
  `Syntax::detect(document)` tells from the root element whether a document is UBL or CII,
  and `builtin_rule_set()` names its built-in rule set.
- Bundles: a directory or in-memory set of files with a `bundle.toml` manifest naming the
  stylesheets, the resources they read with `document()` (code lists), and their SHA-256
  (`Validator::from_bundle_dir`, `Validator::from_bundle`).
- Fetched on demand: Factur-X (five profiles), XRechnung (UBL and CII), the French rules
  (fourteen bundles) and Peppol BIS Billing 3.0 may not be redistributed, so the crate does
  not embed them. `fetched_rule_sets()` is their catalogue (pinned URLs and digests of the
  publishers' files, the manifest of each bundle); `RuleSetCache` keeps their bundles in the
  platform's cache directory (or `EINVOICE_VALIDATOR_CACHE`), and `Validator::from_cache(name)`
  loads one, every file checked, never downloading. With the feature `fetch`,
  `RuleSetCache::fetch(name, &FetchOptions::new())` downloads the files of the family, checks
  their digests and builds the bundles exactly as the repository's `scripts/fetch-*.sh` do
  (Peppol BIS Billing, published as Schematron only, is compiled as ph-schematron compiles
  it, in Rust, with [`einvoice-schematron`](https://crates.io/crates/einvoice-schematron)
  and the ISO Schematron skeleton of the pinned jar of ph-schematron-isosch: no Java). The
  feature adds a minimal HTTPS client on rustls, with a cryptography
  provider on the RustCrypto crates (pure Rust) and the Mozilla roots of webpki-roots; it is
  ignored on WebAssembly.

With the default feature `pdf`, the module `pdf` extracts the invoice XML a Factur-X, ZUGFeRD
or Order-X PDF/A-3 file embeds (`pdf::extract`, with
[`einvoice-pdf`](https://crates.io/crates/einvoice-pdf)), with its `/AFRelationship` and the
document's XMP metadata, to be validated like any other document; a PDF no invoice can be
read from is an `Error::Pdf`.

On every case of the test corpora of CEN EN 16931 UBL and CII 1.3.17 and Peppol BIS Billing
3.0.22, the verdicts and findings agree with Saxon-HE. The crate also builds for
`wasm32-unknown-unknown`, where the caller gives the clock. The command-line tool is
[`einvoice-cli`](https://crates.io/crates/einvoice-cli) (`einvoice-validate`).

This crate is part of [einvoice-validator](https://github.com/einvoice-validator/einvoice-validator). Documentation:
<https://docs.rs/einvoice-validator>.

## Licence

`MIT AND EUPL-1.2 AND LicenseRef-OASIS-Copyright-Notice AND LicenseRef-UN-CEFACT-Copyright-Notice AND W3C-19980720`:
the code is MIT; the default features `cen-ubl` and `cen-cii` embed the CEN EN 16931
stylesheets, vendored unmodified in `bundles/cen-ubl/` and `bundles/cen-cii/`, which are under
the EUPL-1.2 (built with `default-features = false`, the crate embeds no rule set); and
through `einvoice-xsd`, every program built with the crate embeds the OASIS UBL 2.1 schemas
and the UN/CEFACT CII D16B schemas, which keep their own terms (OASIS, UN/CEFACT, W3C). The
texts are in `LICENSES/`, and `einvoice_validator::licences()` returns those of what a build
embeds, for programs that must show them to their users.
