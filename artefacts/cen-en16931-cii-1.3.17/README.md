# CEN EN 16931 validation artefacts for UN/CEFACT CII D16B, release 1.3.17

Unmodified copies of files published by CEN/TC 434 in the official EN 16931 validation
artefacts. No file is edited or renamed; paths are the upstream ones, relative to the root of
the source archive. Files are stored without the executable bit some of them carry upstream.

| | |
|---|---|
| Repository | <https://github.com/ConnectingEurope/eInvoicing-EN16931> |
| Tag | `validation-1.3.17` |
| Commit | `6e35e0ab8b09eb183f7d84b282e24ef95f9f5715` |
| Source archive | <https://github.com/ConnectingEurope/eInvoicing-EN16931/archive/refs/tags/validation-1.3.17.tar.gz> |
| Source archive SHA-256 | `44b3a8ffd4b0d0b88e9da0d5f6beae065bbf7b28ae398df8044506635b5e19f7` |
| Release asset | <https://github.com/ConnectingEurope/eInvoicing-EN16931/releases/download/validation-1.3.17/en16931-cii-1.3.17.zip> |
| Release asset SHA-256 | `e5f3235824b6b38454b1d54cf3f14e01efb5b33971fbefc4e64997d0ef619e89` |

Every file comes from the source archive; the files under `cii/` are also byte-identical to the
ones in the release asset, which holds exactly `cii/examples/`, `cii/schematron/` and
`cii/xslt/`.

## Contents

- `cii/xslt/EN16931-CII-validation.xslt`: the compiled XSLT 2.0 rule set.
- `cii/schematron/`: the Schematron sources it was compiled from.
- `cii/examples/`: the 15 example documents (`rsm:CrossIndustryInvoice`, all of type 380).
- `test/cii/`: the VEFA unit test sets of the CII syntax, one `<testSet>` per file, each
  `<test>` holding a document and the rule ids expected to succeed or fail. With
  `cii/examples/`, they are the test folders the upstream build runs for CII
  (`buildconfig-cii.xml`).
- `LICENSE.txt`: the licence (the same file as in `../cen-en16931-ubl-1.3.17/`).

Not vendored: `cii/schema/`, the UN/CEFACT CII D16B XML schemas the source archive carries
(the release asset does not). They are UN/CEFACT works under their own terms, not the EUPL;
the rule set runs without XML schema validation.

## Licence

These files are licensed under the **European Union Public Licence (EUPL) version 1.2**; see
`LICENSE.txt`. This directory is EUPL-1.2, as is `corpus/cen-en16931-cii-1.3.17/`, which is
derived from it. The rest of the repository is MIT.

This `README.md` and `SHA256SUMS` are not upstream files: they record where the files come from
and their checksums.

## Integrity

`SHA256SUMS` lists the SHA-256 of every upstream file in this directory. Check it with either
of:

```sh
cargo xtask manifest                 # also fails on files missing from or absent in the list
(cd artefacts/cen-en16931-cii-1.3.17 && sha256sum -c SHA256SUMS)
```
