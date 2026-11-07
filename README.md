# einvoice-validator

Validate EN 16931 and Peppol BIS Billing 3.0 e-invoices against the **official** validation
artefacts, in pure Rust: no JVM, no C library, no FFI.

Status: **pre-alpha**. No code yet.

## Test corpus

`artefacts/` holds unmodified copies of the official validation artefacts the project is tested
against, `corpus/` the test cases derived from them and the results of the reference engine
(Saxon-HE) for each case. See [corpus/README.md](corpus/README.md).

## Licence

MIT, except the official artefacts and the data derived from them, which keep their upstream
licence: `artefacts/cen-en16931-ubl-1.3.17/` and `corpus/cen-en16931-ubl-1.3.17/` are under the
European Union Public Licence 1.2 (EUPL-1.2), as stated in the README of each directory.
