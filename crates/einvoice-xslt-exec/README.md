# einvoice-xslt-exec

Executor of the XSLT 2.0 subset that the official e-invoicing validation stylesheets use
(Schematron compiled to XSLT), with the behaviour of Saxon-HE 12.10: the same template rules
chosen, the same result. A stylesheet loaded by
[`einvoice-xslt`](https://crates.io/crates/einvoice-xslt) is prepared once, then runs on any
number of documents and writes its result, for a compiled Schematron the SVRL report: as a
string, or kept compactly (each distinct piece of the text once) and read as it is written by
an observer (`Executable::transform_compact`).
Constructs whose result would differ from Saxon-HE's are refused when a stylesheet is
prepared. A document nested deeper than 256 elements (`TransformOptions::max_depth`), and
template or function invocations nested beyond a maximum nesting or stack, end the
transformation with a `LIMIT` error rather than overflowing the stack.

This crate is part of [einvoice-validator](https://github.com/einvoice-validator/einvoice-validator), which validates EN 16931 and Peppol BIS
Billing 3.0 e-invoices against the official validation artefacts in pure Rust. On every case
of the CEN EN 16931 UBL 1.3.17 and Peppol BIS Billing 3.0.22 test corpora it produces the same
SVRL report as Saxon-HE. Documentation: <https://docs.rs/einvoice-xslt-exec>.

## Licence

MIT (`LICENSES/MIT.txt`).
