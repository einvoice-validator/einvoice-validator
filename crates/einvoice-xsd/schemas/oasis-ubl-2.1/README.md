# OASIS UBL 2.1 schemas (Invoice and CreditNote)

Unmodified copies of the W3C XML Schema documents of the OASIS Universal Business Language
(UBL) 2.1 OASIS Standard that are reachable from the two entry points
`xsd/maindoc/UBL-Invoice-2.1.xsd` and `xsd/maindoc/UBL-CreditNote-2.1.xsd` through
`xs:import` and `xs:include`: fifteen files, no other. No file is edited or renamed; paths
are the ones in the published package, relative to its root. `einvoice-xsd` embeds them.

| | |
|---|---|
| Publisher | OASIS Universal Business Language (UBL) TC |
| Release | UBL 2.1 OASIS Standard, 4 November 2013 |
| Package | <https://docs.oasis-open.org/ubl/os-UBL-2.1/UBL-2.1.zip> |
| Package SHA-256 | `60b80d76394a8a2add90723ecb8e0e2e9d826775de9749df37a72d60703f86ed` |

## Licence

These files are **not** covered by the MIT licence of this repository. Each keeps the terms
stated in its own header, which is preserved:

- the UBL schema modules (`UBL-*-2.1.xsd` other than the three below): copyright OASIS Open
  2013, under the notice at the end of each file, which allows them to be copied and
  distributed in whole provided the copyright notice and that paragraph are kept, and does not
  allow the documents themselves to be modified;
- `xsd/common/CCTS_CCT_SchemaModule-2.1.xsd`: copyright UN/CEFACT 2006, under the notice in its
  header, with the same conditions;
- `xsd/common/UBL-xmldsig-core-schema-2.1.xsd`: the W3C XML Signature schema (copyright The
  Internet Society and W3C), under the W3C Software License referenced in its header, as
  modified by OASIS (DOCTYPE identifiers removed) and published in the UBL 2.1 package;
- `xsd/common/UBL-XAdESv132-2.1.xsd` and `xsd/common/UBL-XAdESv141-2.1.xsd`: the ETSI XAdES
  schemas as modified by OASIS (import locations changed) and published in the UBL 2.1
  package, with the OASIS header stating their origin.

This `README.md` and `SHA256SUMS` are not upstream files: they record where the files come
from and their checksums.

## Integrity

`SHA256SUMS` lists the SHA-256 of every schema document in this directory. The test
`tests/manifest.rs` of the crate checks it, fails on any file missing from the list or absent
from the directory, and checks that the embedded copies are these files. By hand:

```sh
(cd crates/einvoice-xsd/schemas/oasis-ubl-2.1 && sha256sum -c SHA256SUMS)
```
