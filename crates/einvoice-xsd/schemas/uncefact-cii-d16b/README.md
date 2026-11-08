# UN/CEFACT Cross Industry Invoice D16B schemas (uncoupled code lists)

Unmodified copies of the W3C XML Schema documents of the UN/CEFACT Cross Industry Invoice,
release D16B (Supply Chain Reference Data Model, subset), with uncoupled code lists, that are
reachable from the entry point `uncefact/data/standard/CrossIndustryInvoice_100pD16B.xsd`
through `xsd:import`: the entry point and the three modules it imports, its qualified data
types, its reusable aggregate business information entities and its unqualified data types.
None of them imports a code-list module, and none is in this directory. The bytes are those of
the files UNECE publishes, CRLF line ends included (the repository's `.gitattributes` marks the
directory `-text`, so that git does not convert them). Paths are the ones in the published
package below its `CII/` directory. `einvoice-xsd` embeds them (`Schema::cii_d16b`).

These are the schemas the CEN EN 16931 CII validation artefacts are built against: the CEN
source archive carries the same files, with LF line ends, under `cii/schema/D16B SCRDM
(Subset)/uncoupled clm/CII/uncefact/data/standard/`.

| | |
|---|---|
| Publisher | UN/CEFACT (UNECE) |
| Release | D16B SCRDM (Subset) CII, uncoupled code lists, schema version 100.D16B, 10 October 2016 |
| Package | `D16B_SCRDM__Subset__CII_uncoupled.zip`, published at <https://unece.org/fileadmin/DAM/cefact/Standards/SupplyChain/SCRDM/D16B_SCRDM__Subset__CII_uncoupled.zip> |
| Package SHA-256 | `313955894bacc617fc4b359e572ca7732496aa0807d7898de025a8f35b9ee9ad` |
| Path in the package | `D16B SCRDM (Subset) CII uncoupled/uncoupled clm/CII/uncefact/data/standard/` |

| File | SHA-256 |
|---|---|
| `CrossIndustryInvoice_100pD16B.xsd` | `3baf143b90f289d62c5dfb086aa51fa183490a2d0b38a40c6b31dcae0760f2ba` |
| `CrossIndustryInvoice_QualifiedDataType_100pD16B.xsd` | `6649215914728652e883662bec1d3eab008c215e86a59f3bb417db9723ee8c5c` |
| `CrossIndustryInvoice_ReusableAggregateBusinessInformationEntity_100pD16B.xsd` | `1c42244298a863bcc0b5196983ff0a5d084fcabe58aea77ef82b242e7cd0ddfd` |
| `CrossIndustryInvoice_UnqualifiedDataType_100pD16B.xsd` | `04fce6f6f28521886731294be18a4e6bb5cb3003456457d99ad5c4d3e652f497` |

## CII D22B is not included

The UN/CEFACT CII D22B schemas (`CII_D22B.zip`, published by UNECE in 2023) carry no copyright
notice and no licence in any of their files. Without a notice of their own, the terms of use of
the United Nations web sites apply to them, which allow personal, non-commercial use only,
without a right to redistribute. They are therefore not vendored and not embedded. `Schema::load`
loads them from files the user has.

## Licence

These files are **not** covered by the MIT licence of this repository. Each keeps the terms
stated in its own header, which is preserved: copyright UN/CEFACT 2016, under a notice that
allows the document to be copied and furnished to others, without restriction of any kind,
provided the copyright notice and that paragraph are included on all copies, and does not allow
the document itself to be modified. The permission is stated to be perpetual. The notice is,
word for word, the one of the UN/CEFACT module of the UBL 2.1 schemas
(`../oasis-ubl-2.1/xsd/common/CCTS_CCT_SchemaModule-2.1.xsd`), with the year 2016 instead of
2006. The crate's licence expression therefore names it with the same term,
`LicenseRef-UN-CEFACT-Copyright-Notice`, whose text in the crate's `LICENSES/` directory carries
both copyright lines:

```text
Copyright (C) UN/CEFACT (2016). All Rights Reserved.
```

This `README.md` and `SHA256SUMS` are not upstream files: they record where the files come from
and their checksums.

## Integrity

`SHA256SUMS` lists the SHA-256 of every schema document in this directory. The test
`tests/manifest.rs` of the crate checks it, fails on any file missing from the list or absent
from the directory, and checks that the embedded copies are these files and that each carries
the UN/CEFACT 2016 notice. By hand:

```sh
(cd crates/einvoice-xsd/schemas/uncefact-cii-d16b && sha256sum -c SHA256SUMS)
```
