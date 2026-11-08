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
their id, flag, location and message), the raw SVRL and the verdict.

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
  the same release for UN/CEFACT CII D16B, without an XML schema (the CII D16B schemas are not
  embedded: a document is checked for well-formedness, then by the stylesheet).
  `Syntax::detect(document)` tells from the root element whether a document is UBL or CII,
  and `builtin_rule_set()` names its built-in rule set.
- Bundles: a directory or in-memory set of files with a `bundle.toml` manifest naming the
  stylesheets and their SHA-256 (`Validator::from_bundle_dir`, `Validator::from_bundle`).
  Peppol BIS Billing 3.0 is used this way, since its files may not be redistributed: the
  repository's `scripts/fetch-peppol.sh` downloads the pinned release, compiles its
  stylesheets and writes the manifest.

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
through `einvoice-xsd`, every program built with the crate embeds the OASIS UBL 2.1 schemas,
which keep their own terms (OASIS, UN/CEFACT, W3C). The texts are in `LICENSES/`, and
`einvoice_validator::licences()` returns those of what a build embeds, for programs that must
show them to their users.
