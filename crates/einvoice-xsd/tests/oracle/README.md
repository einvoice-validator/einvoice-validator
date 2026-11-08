# xmllint oracle of `einvoice-xsd`

xmllint (libxml2) is the XML Schema validator that e-invoicing validators commonly run before
Schematron. Every expected result in this directory is produced by xmllint, never written by
hand: libxml2 2.9.14, both UBL 2.1 entry points loaded together through a generated schema that
imports `UBL-Invoice-2.1.xsd` and `UBL-CreditNote-2.1.xsd`, from the complete OASIS UBL 2.1
schema set: the vendored documents and the two ETSI XAdES schemas of the OASIS package, which
the repository does not ship (`../../schemas/oasis-ubl-2.1/README.md`).

| file | content |
|---|---|
| `documents/` | synthetic documents written for this project (no real business data): a full invoice exercising the extension point, an XAdES signature and every built-in type, the smallest valid credit note, a global element other than Invoice or CreditNote as document element, and a UBL document type outside the two entry points |
| `mutations.tsv` | edits of those documents, one per line: each targets one construct (content models, wildcards, attributes, `xsi:` attributes, each built-in type, empty and simple content, IDs, DTD entities and defaults, XAdES content as extension content) |
| `expected.tsv` | xmllint's verdict (`valid`, `invalid` or `malformed`) and errors (`line:category`, sorted) for every case |
| `disagreements.tsv` | every case where this crate differs from xmllint, with the reason: libxml2 departs from XML Schema 1.0, or the case has XAdES content, which this crate validates laxly since it does not ship the XAdES schemas (explanation beginning with `XAdES schemas not shipped: content validated laxly.`), or, once, a document refused as unsupported on purpose |
| `format.rs` | the cases and file formats, shared by the test and the example |

The cases are, in order: every case of the CEN EN 16931 UBL corpus
(`corpus/cen-en16931-ubl-1.3.17/cases.tsv`), every file of `documents/`, every mutation.

The test `tests/oracle.rs` validates every case with this crate (`Schema::ubl_2_1`, the
shipped schemas) and compares with `expected.tsv`: a difference not listed in
`disagreements.tsv`, or a listed one that no longer occurs, fails the test, and so does any
difference on a corpus case. It needs neither xmllint nor libxml2. Errors are compared by line
and category: the categories map xmllint's messages and this crate's `ErrorKind`s onto a
common set, and lines are those of the end of each element's start tag, where libxml2 reports
them (this crate reports the start).

The `xmllint-oracle` example assembles the complete schema set in
`target/xsd-oracle/oasis-ubl-2.1/` (the vendored documents and the two XAdES schemas, taken
from an extracted OASIS UBL 2.1 package when `$OASIS_UBL_2_1_DIR` names its root, else
downloaded from `docs.oasis-open.org`; SHA-256 checked either way), runs xmllint with it on
every case (`$XMLLINT`, else `xmllint`), checks or rewrites `expected.tsv`, and prints how this
crate compares, case by case, twice: with the shipped schemas, where every listed disagreement
must occur, and with the complete set, where the XAdES ones must not and every other one must.

```sh
cargo run --release -p einvoice-xsd --example xmllint-oracle             # check expected.tsv against xmllint
cargo run --release -p einvoice-xsd --example xmllint-oracle -- --write  # rewrite expected.tsv
```
