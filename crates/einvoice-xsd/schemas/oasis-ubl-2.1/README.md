# OASIS UBL 2.1 schemas (Invoice and CreditNote)

Unmodified copies of the W3C XML Schema documents of the OASIS Universal Business Language
(UBL) 2.1 OASIS Standard that are reachable from the two entry points
`xsd/maindoc/UBL-Invoice-2.1.xsd` and `xsd/maindoc/UBL-CreditNote-2.1.xsd` through
`xs:import` and `xs:include`: thirteen of those fifteen files, no other (the two left out are
described below). No file is edited or renamed; paths are the ones in the published package,
relative to its root. `einvoice-xsd` embeds them.

| | |
|---|---|
| Publisher | OASIS Universal Business Language (UBL) TC |
| Release | UBL 2.1 OASIS Standard, 4 November 2013 |
| Package | <https://docs.oasis-open.org/ubl/os-UBL-2.1/UBL-2.1.zip> |
| Package SHA-256 | `60b80d76394a8a2add90723ecb8e0e2e9d826775de9749df37a72d60703f86ed` |

## The XAdES schemas are not included

The package also contains `xsd/common/UBL-XAdESv132-2.1.xsd` and
`xsd/common/UBL-XAdESv141-2.1.xsd`, the ETSI XAdES schemas (ETSI TS 101 903, XML Advanced
Electronic Signatures, namespaces `http://uri.etsi.org/01903/v1.3.2#` and
`http://uri.etsi.org/01903/v1.4.1#`) as modified by OASIS, which
`xsd/common/UBL-SignatureAggregateComponents-2.1.xsd` imports. They are not in this
directory: neither these copies nor the schemas ETSI publishes carry a licence notice, the
ETSI deliverable they belong to states that no part may be reproduced except as authorized by
written permission, and the UBL 2.1 specification describes them as provided only as a
convenience for those users who choose to include a XAdES extension inside a digital
signature. The OASIS files that import them are unchanged.

What it means for validation: `einvoice-xsd` does not load the two imports of the XAdES
namespaces (the only imports it allows to be absent) and reports them as omitted imports. The
UBL schemas only import those namespaces and never refer to their components, so XAdES
elements can only occur where a lax wildcard allows them, in `ext:ExtensionContent` and in
`ds:Object` (where a XAdES signature puts its `xades:QualifyingProperties`). Without their
declarations they are validated laxly, as `xs:anyType`: their content models, values and
attributes are not checked, while the elements below them that have a global declaration
(`ds:DigestMethod`, `cbc:` elements, ...) still are. Everything else in a UBL invoice or
credit note is validated as with the complete package. The crate's oracle compares it with
xmllint loading the complete package and lists the resulting differences
(`tests/oracle/disagreements.tsv`).

## Licence

These files are **not** covered by the MIT licence of this repository. Each keeps the terms
stated in its own header, which is preserved. The published crate therefore declares the SPDX
expression `MIT AND LicenseRef-OASIS-Copyright-Notice AND LicenseRef-UN-CEFACT-Copyright-Notice
AND W3C-19980720`: `MIT` for its code, one term per set of terms below, each with its text in
the crate's `LICENSES/` directory.

- the UBL schema modules (`UBL-*-2.1.xsd` other than the one below): copyright OASIS Open
  2013, under the notice at the end of each file, which allows them to be copied and
  distributed in whole provided the copyright notice and that paragraph are kept, and does not
  allow the documents themselves to be modified (`LicenseRef-OASIS-Copyright-Notice`);
- `xsd/common/CCTS_CCT_SchemaModule-2.1.xsd`: copyright UN/CEFACT 2006, under the notice in its
  header, with the same conditions (`LicenseRef-UN-CEFACT-Copyright-Notice`);
- `xsd/common/UBL-xmldsig-core-schema-2.1.xsd`: the W3C XML Signature schema (copyright The
  Internet Society and W3C), under the W3C Software License referenced in its header, as
  modified by OASIS (DOCTYPE identifiers removed) and published in the UBL 2.1 package
  (`W3C-19980720`, the W3C Software Notice and License of 1998-07-20).

This `README.md` and `SHA256SUMS` are not upstream files: they record where the files come
from and their checksums.

## Integrity

`SHA256SUMS` lists the SHA-256 of every schema document in this directory. The test
`tests/manifest.rs` of the crate checks it, fails on any file missing from the list or absent
from the directory, checks that the embedded copies are these files, and that no XAdES schema
is among them. By hand:

```sh
(cd crates/einvoice-xsd/schemas/oasis-ubl-2.1 && sha256sum -c SHA256SUMS)
```
