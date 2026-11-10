# einvoice-xslt

Loader for the XSLT 2.0 subset that the official e-invoicing validation stylesheets use
(Schematron compiled to XSLT), and that the ISO Schematron skeleton uses to compile
Schematron (imports, `use-when`, node construction and copying, messages): templates with their match patterns, modes and priorities,
named templates, functions, global and local variables and parameters, and the instructions
of each sequence constructor, each with its source line. XPath expressions are parsed with
[`einvoice-xpath`](https://crates.io/crates/einvoice-xpath); nothing is evaluated. The
subset is closed: any other XSLT element, attribute or value is an error naming it and its
line. Instructions nested deeper than `MAX_NESTING` (64 levels) are refused too, so that
loading, preparing and running the stylesheet needs a bounded call stack.

This crate is part of [einvoice-validator](https://github.com/einvoice-validator/einvoice-validator), which validates EN 16931 and Peppol BIS
Billing 3.0 e-invoices against the official validation artefacts in pure Rust.
Documentation: <https://docs.rs/einvoice-xslt>.

## Licence

MIT (`LICENSES/MIT.txt`).
