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
It then does the same for the synthetic suite and for the CII suite below.

```sh
cargo run --release -p einvoice-xsd --example xmllint-oracle             # check both expected.tsv against xmllint
cargo run --release -p einvoice-xsd --example xmllint-oracle -- --write  # rewrite them
```

The `xmllint.rs` module runs xmllint and reads its verdicts and errors; the example and the
local check below share it.

## Synthetic suite

`synthetic/` is a second suite of the same kind, for the constructs that the schemas of the
Cross Industry Invoice family add to those of UBL. Its schema set, `synthetic/schemas/`, is
written for this project in the style of those schemas (an entry point, aggregates, data types
in their own namespace): `fixed` and `default` values on attributes and elements of every
built-in type that allows them, length facets on the string types (through several restriction
steps, with an enumeration), anonymous complex types in local element declarations (simple,
element-only with a choice, mixed and empty content, nested), and `xsi:type` on elements with
a value constraint.

| file | content |
|---|---|
| `schemas/` | the synthetic schema set; entry point `invoice.xsd` |
| `documents/` | synthetic documents valid for it |
| `mutations.tsv` | edits of those documents, in the format of `../mutations.tsv` |
| `schema-mutations.tsv` | edits of one schema document each: id, file, then find and replace pairs; the verdict of such a case is whether the edited set loads (`loads` or `fails`; xmllint compiles it, this crate can also say `unsupported`) |
| `expected.tsv` | xmllint's verdict and errors for every case |
| `disagreements.tsv` | every case where this crate differs from xmllint, with the reason: libxml2 departs from XML Schema 1.0, reports more errors for the same verdict, or the construct is refused as unsupported |

The cases are, in order: every file of `documents/` (`document/<stem>`), every mutation
(`mutation/<id>`), every schema mutation (`schema/<id>`). `tests/oracle.rs` checks them as it
checks the UBL suite; the example runs xmllint on them with the schema set as it is in the
repository, each schema mutation written to `target/xsd-oracle/synthetic/schema/<id>/`.

## CII suite

`cii/` compares `Schema::cii_d16b` with xmllint loading the same four vendored documents
(`../../schemas/uncefact-cii-d16b/`, entry point `CrossIndustryInvoice_100pD16B.xsd`), on every
case of the CEN EN 16931 CII corpus (`corpus/cen-en16931-cii-1.3.17/`: `corpus/<case>` for the
24 cases of its `cases.tsv`, `mutation/<id>` for the 808 of its `mutations.tsv`).

| file | content |
|---|---|
| `expected.tsv` | xmllint's verdict and errors for every case |
| `disagreements.tsv` | every case where this crate differs from xmllint, with the reason; none may be a corpus case |
| `fetched-disagreements.tsv` | the same for the CII corpora of the fetched rule sets (below), case ids prefixed with the name of the corpus |

`tests/oracle.rs` checks it as it checks the other suites; the example runs xmllint on it.

The CII corpora of the fetched rule sets (XRechnung 3.0.2 CII, Factur-X 1.09.2 and the French
rules 1.4.0.04 CII, see `corpus/README.md`) are not committed: their fetch scripts and
`cargo xtask <family>` build them in `target/`. The test `fetched_cii_corpora_agree_with_xmllint`
validates every case and mutation case of each one that is there with this crate and with
xmllint, and compares them against `cii/fetched-disagreements.tsv` (no difference is tolerated
on an upstream case). A corpus that is not there is SKIPPED, and so is everything without
xmllint, unless `EINVOICE_XSD_FETCHED_REQUIRED` names it (comma-separated names:
`xrechnung-cii-3.0.2`, `factur-x-1.09.2`, `fr-ctc-cii-1.4.0.04`), as the facturx, xrechnung and
fr-ctc jobs of `scripts/check.sh` do.

```sh
EINVOICE_XSD_FETCHED_REQUIRED=xrechnung-cii-3.0.2 cargo test --release -p einvoice-xsd --test oracle fetched_cii -- --nocapture
```

## Schema sets outside the repository

The test `extra_schema_sets_agree_with_xmllint` of `tests/oracle.rs` compares this crate with
xmllint on schema sets that are not part of the repository, such as the CII D16B and D22B
schemas and the Factur-X profiles, and on documents of their own. It runs only when
`EINVOICE_XSD_EXTRA_SETS` names a manifest of them, and needs xmllint; otherwise it prints
`SKIPPED` and passes. The manifest is a tab-separated file, one set per line: a name, the
directory of the schema set (every `.xsd` below it is given to this crate), the entry point
relative to that directory, then one or more directories of documents (every `.xml` below them
whose document element is in the target namespace of the entry point); relative paths are
relative to the manifest; `#` starts a comment. Every set must load with this crate and compile
with xmllint, and both must agree on the verdict and the errors of every document; the test
prints, for each set, the number of documents, of valid ones and of agreements.

```sh
EINVOICE_XSD_EXTRA_SETS=/path/to/sets.tsv cargo test --release -p einvoice-xsd --test oracle extra_schema_sets -- --nocapture
```
