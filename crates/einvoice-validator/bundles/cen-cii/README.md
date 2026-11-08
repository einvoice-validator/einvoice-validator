# The built-in rule set `cen-cii`: CEN EN 16931 validation artefacts for UN/CEFACT CII D16B, release 1.3.17

`cii/xslt/EN16931-CII-validation.xslt` is an unmodified copy of the compiled XSLT 2.0 rule set
published by CEN/TC 434 in the official EN 16931 validation artefacts, at the same path as
upstream. `einvoice-validator` embeds it with its default feature `cen-cii`, as the built-in
rule set `cen-cii`, described by `bundle.toml`.

| | |
|---|---|
| Repository | <https://github.com/ConnectingEurope/eInvoicing-EN16931> |
| Tag | `validation-1.3.17` |
| Commit | `6e35e0ab8b09eb183f7d84b282e24ef95f9f5715` |
| Release asset | <https://github.com/ConnectingEurope/eInvoicing-EN16931/releases/download/validation-1.3.17/en16931-cii-1.3.17.zip> |

In the repository, the same file is in `artefacts/cen-en16931-cii-1.3.17/`, with the rest of
the release.

## XML schema

The rule set names the schema `cii-d16b` (`bundle.toml`): a document is checked for well-formed
XML, validated against the UN/CEFACT CII D16B schemas with uncoupled code lists, then by the
stylesheet, which runs whatever the schema reports. The CEN release asset does not ship those
schemas; `einvoice-xsd` embeds UNECE's files, unmodified, under their own UN/CEFACT terms
(`crates/einvoice-xsd/schemas/uncefact-cii-d16b/README.md`).

## Licence

This file is **not** covered by the MIT licence of the crate's code: it is licensed under the
**European Union Public Licence (EUPL) version 1.2**, whose text, as published with the
artefacts (their `LICENSE.txt`, unmodified), is the crate's `LICENSES/EUPL-1.2.txt`. The crate
therefore declares the SPDX expression `MIT AND EUPL-1.2`; built without its default features,
it embeds no EUPL-1.2 file.

`README.md`, `SHA256SUMS` and `bundle.toml` are not upstream files: they record where the file
comes from, its checksum, and the rule set it makes up.

## Integrity

`SHA256SUMS` lists the SHA-256 of the vendored file. The test `tests/vendored.rs` of the crate
checks it, fails on any file missing from the list or absent from the directory, checks that
the file is byte-identical to the copy in `artefacts/` and that the built-in rule set embeds
it. By hand:

```sh
(cd crates/einvoice-validator/bundles/cen-cii && sha256sum -c SHA256SUMS)
```
