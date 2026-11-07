# xmllint oracle of `einvoice-xsd`

xmllint (libxml2) is the XML Schema validator that e-invoicing validators commonly run before
Schematron. Every expected result in this directory is produced by xmllint, never written by
hand: libxml2 2.9.14, both UBL 2.1 entry points loaded together through a generated schema that
imports `UBL-Invoice-2.1.xsd` and `UBL-CreditNote-2.1.xsd`.

| file | content |
|---|---|
| `documents/` | synthetic documents written for this project (no real business data): a full invoice exercising the extension point, an XAdES signature and every built-in type, the smallest valid credit note, a global element other than Invoice or CreditNote as document element, and a UBL document type outside the two entry points |
| `mutations.tsv` | edits of those documents, one per line: each targets one construct (content models, wildcards, attributes, `xsi:` attributes, each built-in type, empty and simple content, IDs, DTD entities and defaults) |
| `expected.tsv` | xmllint's verdict (`valid`, `invalid` or `malformed`) and errors (`line:category`, sorted) for every case |
| `disagreements.tsv` | every case where this crate differs from xmllint, with the reason; in each one libxml2 departs from XML Schema 1.0, except one document refused as unsupported on purpose |
| `format.rs` | the cases and file formats, shared by the test and the example |

The cases are, in order: every case of the CEN EN 16931 UBL corpus
(`corpus/cen-en16931-ubl-1.3.17/cases.tsv`), every file of `documents/`, every mutation.

The test `tests/oracle.rs` validates every case with this crate and compares with
`expected.tsv`: a difference not listed in `disagreements.tsv`, or a listed one that no longer
occurs, fails the test. It needs neither xmllint nor libxml2. Errors are compared by line and
category: the categories map xmllint's messages and this crate's `ErrorKind`s onto a common
set, and lines are those of the end of each element's start tag, where libxml2 reports them
(this crate reports the start).

The `xmllint-oracle` example runs xmllint on every case (`$XMLLINT`, else `xmllint`), checks
or rewrites `expected.tsv`, and prints how this crate compares, case by case:

```sh
cargo run --release -p einvoice-xsd --example xmllint-oracle             # check expected.tsv against xmllint
cargo run --release -p einvoice-xsd --example xmllint-oracle -- --write  # rewrite expected.tsv
```
