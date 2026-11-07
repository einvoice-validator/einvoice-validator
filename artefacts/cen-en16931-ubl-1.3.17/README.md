# CEN EN 16931 validation artefacts for UBL, release 1.3.17

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
| Release asset | <https://github.com/ConnectingEurope/eInvoicing-EN16931/releases/download/validation-1.3.17/en16931-ubl-1.3.17.zip> |
| Release asset SHA-256 | `bf02a0e3f6cbe692a4a28e70be7bdcaded5612b4169b543e13d6cb4f33c29e92` |

Every file comes from the source archive; the files under `ubl/` are also byte-identical to the
ones in the release asset.

## Contents

- `ubl/xslt/EN16931-UBL-validation.xslt`: the compiled XSLT 2.0 rule set.
- `ubl/schematron/`: the Schematron sources it was compiled from.
- `ubl/examples/`: the 19 example documents (18 invoices, 1 credit note).
- `test/Invoice-unit-UBL/`, `test/CreditNote-unit-UBL/`: the VEFA unit test sets, one
  `<testSet>` per file, each `<test>` holding a document and the rule ids expected to succeed
  or fail.
- `test/testfiles/`: further whole test documents.
- `LICENSE.txt`: the licence.

## Licence

These files are licensed under the **European Union Public Licence (EUPL) version 1.2**; see
`LICENSE.txt`. This directory is EUPL-1.2, as is `corpus/cen-en16931-ubl-1.3.17/`, which is
derived from it. The rest of the repository is MIT.

This `README.md` and `SHA256SUMS` are not upstream files: they record where the files come from
and their checksums.

## Integrity

`SHA256SUMS` lists the SHA-256 of every upstream file in this directory. Check it with either
of:

```sh
cargo xtask manifest                 # also fails on files missing from or absent in the list
(cd artefacts/cen-en16931-ubl-1.3.17 && sha256sum -c SHA256SUMS)
```
