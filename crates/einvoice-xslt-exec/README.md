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
template or function invocations nested beyond a maximum nesting or stack (the expressions
they evaluate included), end the transformation with a `LIMIT` error rather than overflowing
the stack.

It also runs the three stylesheets of the ISO Schematron skeleton that ph-schematron runs
to compile Schematron, with the same results as Saxon-HE (used by
[`einvoice-schematron`](https://crates.io/crates/einvoice-schematron)): `xsl:copy`,
`xsl:copy-of`, `xsl:element`, `xsl:namespace`, `xsl:processing-instruction`,
`xsl:message`, imports, namespace aliases, `document()` through a resolver given by the
caller, `id()`, `generate-id()`, and named templates that end with a call of another
template run in place (tail calls, bounded by `TransformOptions::max_tail_calls`).

This crate is part of [einvoice-validator](https://github.com/einvoice-validator/einvoice-validator), which validates EN 16931 and Peppol BIS
Billing 3.0 e-invoices against the official validation artefacts in pure Rust. On every case
of the CEN EN 16931 UBL 1.3.17 and Peppol BIS Billing 3.0.22 test corpora it produces the same
SVRL report as Saxon-HE. Documentation: <https://docs.rs/einvoice-xslt-exec>.

## Licence

MIT (`LICENSES/MIT.txt`).
