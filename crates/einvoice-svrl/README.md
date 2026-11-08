# einvoice-svrl

The canonical, line-oriented form of SVRL reports (the Schematron Validation Report Language
of ISO/IEC 19757-3): one line per SVRL element, attributes sorted, text whitespace normalised,
the URI of the validated document abstracted. Two Schematron engines agree on a document when
the canonical forms of their reports are identical, so their results can be compared with a
plain diff. Anything the form cannot represent faithfully is an error, never dropped.

The format is specified in
[`corpus/SVRL-CANONICAL.md`](https://github.com/einvoice-validator/einvoice-validator/blob/main/corpus/SVRL-CANONICAL.md).

This crate is part of [einvoice-validator](https://github.com/einvoice-validator/einvoice-validator), which validates EN 16931 and Peppol BIS
Billing 3.0 e-invoices against the official validation artefacts in pure Rust; it compares the
engine's reports with those of Saxon-HE. Documentation: <https://docs.rs/einvoice-svrl>.

## Licence

MIT (`LICENSES/MIT.txt`).
