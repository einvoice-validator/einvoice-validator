# einvoice-xsd

XML Schema 1.0 validation of UBL 2.1 Invoice and CreditNote documents against the official
OASIS UBL 2.1 schemas, which it embeds, in pure Rust. It implements exactly the XML Schema
constructs those schemas use and refuses any other when a schema set is loaded, so a schema
it cannot handle is never half-checked. Every validity error is reported with a stable kind,
the line and column of the element concerned, and a message. A document is validated in one
pass of the streaming reader [`einvoice-xml`](https://crates.io/crates/einvoice-xml), without
building a tree: the memory used grows with the depth of the document, not its size
(`Schema::validation` takes the reader's events from a caller that reads the document
itself).

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let schema = einvoice_xsd::Schema::ubl_2_1()?;
    let report = schema.validate(r#"<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"/>"#)?;
    assert!(!report.errors.is_empty());
    Ok(())
}
```

The two ETSI XAdES schemas of the OASIS package are not redistributable and not embedded:
XAdES content (in UBL extensions and signatures) is validated laxly; the crate documentation
says what that means. The verdicts are compared with xmllint (libxml2) in the test suite.

`Schema::load` also loads the schemas of the UN/CEFACT Cross Industry Invoice family, which are
not embedded: CII D16B and D22B and the Factur-X / ZUGFeRD profiles. For them the supported
subset includes `fixed` and `default` values (fixed values compared in the value space of their
type), anonymous complex types in local element declarations, and the `minLength` and
`maxLength` facets; the crate documentation lists what remains outside it.

This crate is part of [einvoice-validator](https://github.com/einvoice-validator/einvoice-validator), which validates EN 16931 and Peppol BIS
Billing 3.0 e-invoices against the official validation artefacts in pure Rust.
Documentation: <https://docs.rs/einvoice-xsd>.

## Licence

`MIT AND LicenseRef-OASIS-Copyright-Notice AND LicenseRef-UN-CEFACT-Copyright-Notice AND
W3C-19980720`: the code is MIT; the embedded schemas, unmodified, keep the terms stated in
their own headers (OASIS, UN/CEFACT, and the W3C Software Notice and License for the XML
Signature schema), as `schemas/oasis-ubl-2.1/README.md` details file by file. The texts are
in `LICENSES/`, and `einvoice_xsd::LICENCES` gives them to programs that must show them to
their users.
