# Test corpus: XRechnung 3.0.2, CII

Not committed. KoSIT publishes the XRechnung artefacts under the Apache License 2.0, but the
XRechnung Schematron includes rules derived from Peppol BIS Billing 3.0, which publishes no
licence; so, as for Peppol, none of the files, and nothing derived from them (extracted
documents, goldens, whose SVRL holds the rule messages), is in this repository. This directory
holds only files of this project (MIT):

- `disagreements.tsv`: the cases where the goldens and the expectations disagree, each with its
  explanation (none for this release).

## The corpus, built locally

`scripts/check.sh xrechnung` builds the UBL and CII corpora in `target/xrechnung/`, outside the
tracked tree, then checks them:

1. `scripts/fetch-xrechnung.sh` downloads from KoSIT's GitLab (`projekte.kosit.org`), into
   `target/downloads/`, the validator configuration 2026-08-31, the test suite 2026-08-31, the
   source archive of the XRechnung Schematron 2.6.0 (tag `v2.6.0`, commit checked) and XML Mutate
   0.6, and checks the SHA-256 of each. It checks the SHA-256 of the two stylesheets the
   configuration's CII scenarios run, `resources/ubl/2.1/xsl/EN16931-UBL-validation.xsl` or
   `resources/cii/16b/xsl/EN16931-CII-validation.xsl` (the CEN EN 16931 rules 1.3.16, compiled
   by KoSIT with SchXslt) and `resources/xrechnung/3.0.2/xsl/XRechnung-CII-validation.xsl`
   (XRechnung Schematron 2.6.0), sorts the test-suite instances by syntax, and runs XML Mutate on
   the unit tests of the XRechnung Schematron (`test/instances/cii/`), as its build does: the
   mutants are checked against the pinned SHA-256 of their listing. XML Mutate numbers the mutants
   in the order it reads the tests and writes the number into the name and the content of each, so
   the script gives it the tests one by one, sorted by name, not the directory, which it would read
   in the order of the file system. It copies the CEN examples
   that are the base documents of the mutation catalogue, and writes the rule-set bundle
   `target/xrechnung/xrechnung-3.0.2/cii/bundle.toml`. Every pin is at the top of the
   script.
2. `cargo xtask xrechnung` extracts the cases into
   `target/xrechnung/xrechnung-3.0.2/corpus-cii/` (same layout and `cases.tsv` format as
   the committed corpora, see `../README.md`), applies `../mutations-cii.toml` to the CEN examples `CII_example5.xml`, `CII_example3.xml`, `CII_example2.xml`, `CII_business_example_Z.xml` and `XRechnung-O.xml` (808 mutation cases), runs
   Saxon-HE with both stylesheets on every case (`golden-cen/`, `golden-xrechnung/`),
   cross-checks the goldens with the expectations against `disagreements.tsv`, writes
   `acceptance.tsv` (whether the findings of each stylesheet reject the case as the KoSIT
   validator reads them, see `../README.md`), checks the levels of the bundle (its
   `[[scenario]]` tables) against `source/configuration/scenarios.xml`, and writes
   `complete.txt`.
3. The equivalence test of the engine, `tests/xrechnung_equivalence.rs` of `crates/einvoice-xslt-exec`
   (required by `scripts/check.sh xrechnung`), runs every stylesheet on every case and mutation
   case: the engine gives the Saxon-HE result on all of them (the same canonical SVRL, or the
   error code Saxon-HE stops with).

| Cases | Source | Count |
|---|---|---|
| `testsuite/<directory>/<file>` | the CII instances of the test suite: 33 `standard`, 1 `extension`, 6 `technical-cases/cius`, 1 `technical-cases/cvd` | 41 |
| `cen-examples/<file>` | the CEN EN 16931 CII 1.3.17 examples (vendored in this repository) | 15 |
| `xmute/<file>` | the mutants XML Mutate generates from the unit tests of the XRechnung Schematron (635, less the 5 listed below) | 630 |
| total | | 686 |

Not used: the 5 mutants of a `code` mutation of a negative value, whose name contains `--`:
XML Mutate 0.6 writes that name in a comment, so the document is not well-formed XML (listed in
`xtask/src/ruleset.rs`).

## Reports with `svrl:metadata`

Both stylesheets are compiled with SchXslt: each report starts with an `svrl:metadata` element
(the processor, the compiler, the times of the compilation and of the run). The goldens are in
version 2 of the canonical form (`../SVRL-CANONICAL.md`), which represents it, the time of the
run written `{run-time}`. The oracle runs Saxon-HE with a fixed clock
(`-now:2000-01-01T00:00:00Z`) and checks that the report records that time; the equivalence
test gives the engine the same clock and checks it too. Two runs give the same goldens, byte for
byte.

## Cross-checks

- XML Mutate cases: the active `<?xmute?>` instruction of each mutant names, for the
  stylesheet `xrcii` (XRechnung-CII), the rules that must fail (`schematron-invalid`) and those
  that must not (`schematron-valid`); `cases.tsv` records them as kind `xmute` (`scope`: the
  stylesheet, `error`: must fail, `success`: must not). The goldens agree with all of them.
- Test-suite instances: documents the configuration accepts. The scenario of
  `scenarios.xml` that matches the root element and the specification identifier (BT-24) of
  each names the stylesheets, and its `customLevel` elements override the level of some rule
  ids; no finding may have the level `error` (a failed assert flagged `fatal` or `error`, or
  unflagged, unless overridden). All 41 are accepted.
