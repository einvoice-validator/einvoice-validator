# Test corpus: French e-invoicing rules (FNFE-MPE package 1.4.0.04), UBL

Not committed. The licence statements of the FNFE-MPE package "Schematrons FR CTC" disagree
(the Apache License 2.0 in its documentation, "European Union Public Licence (EUPL) version
1.4.0" in its Schematron files), and the example documents of the French standards XP Z12-012
and XP Z12-014 are published by AFNOR with all rights reserved; so none of these files, and
nothing derived from them (extracted documents, goldens, whose SVRL holds the rule messages), is
in this repository. This directory holds only files of this project (MIT):

- `disagreements.tsv`: the cases where the goldens and the expectations disagree, each with its
  explanation (none for this release).

## The corpus, built locally

`scripts/check.sh fr-ctc` builds the CII, UBL and CDAR corpora in `target/fr-ctc/`, outside the
tracked tree, then checks them:

1. `scripts/fetch-fr-ctc.sh` downloads from `fnfe-mpe.org`, into `target/downloads/`, the
   package `2026_09_04_FNFE_SCHEMATRONS_FR_CTC_V1.4.0.04.zip` and the annex archives of XP
   Z12-012 and XP Z12-014 (version 1.4), and checks the SHA-256 of each. Every compiled
   stylesheet and code list of the package must have one of the SHA-256 pinned in the script.
   It sorts the example documents by syntax (`*_CII_Commentee.xml`, `*_UBL_Commentee.xml`,
   `*-CDV-*.xml`), copies the CEN examples that are the base documents of the mutation
   catalogues, and writes the rule-set bundles of the package's profiles
   (`target/fr-ctc/fr-ctc-1.4.0.04/<bundle>/bundle.toml`: `cii-en16931`, `cii-extended-ctc-fr`,
   `ubl-en16931`, `ubl-extended-ctc-fr`, `cdar`, each also as `<bundle>-warning` with the
   WARNING version of the French rules, `factur-x-basic-wl`, `factur-x-en16931`,
   `factur-x-extended` and `flux10`). Every pin is at the top of the script.
2. `cargo xtask fr-ctc` extracts the cases into `target/fr-ctc/fr-ctc-1.4.0.04/corpus-ubl/` (same
   layout and `cases.tsv` format as the committed corpora, see `../README.md`), applies the mutation catalogue `../mutations.toml` to the CEN documents `Invoice-Max_content.xml`, `CreditNote-Max_content.xml` and `ubl-tc434-example2.xml` (504 mutation cases), runs
   Saxon-HE with each stylesheet on every case, cross-checks the goldens with the expectations
   against `disagreements.tsv`, and writes `complete.txt`.
3. The equivalence test of the engine, `tests/fr_ctc_equivalence.rs` of `crates/einvoice-xslt-exec`
   (required by `scripts/check.sh fr-ctc`), runs every stylesheet on every case and mutation
   case: the engine gives the Saxon-HE result on all of them (the same canonical SVRL, or the
   error code Saxon-HE stops with).

| Stylesheet (package path) | Goldens |
|---|---|
| `UBL/EN16931/2xslt/EN16931-UBL-validation.xslt` (the CEN EN 16931 UBL rules 1.3.16 as the package ships them) | `golden-en16931/` |
| `UBL/EXTENDED-CTC-FR/2xslt/EXTENDED-CTC-FR-UBL.xslt` | `golden-extended-ctc-fr/` |
| `UBL/EN16931/2xslt/BR-FR-Flux2-Schematron-UBL.xslt` (the French rules) | `golden-br-fr/` |
| `UBL/EN16931/2xslt/BR-FR-Flux2-Schematron-UBL_WARNING.xslt` (their WARNING version) | `golden-br-fr-warning/` |

| Cases | Source | Count |
|---|---|---|
| `z12-012/<file>` | the UBL examples of XP Z12-012 annex B, version 1.4 | 12 |
| `z12-014/<file>` | the UBL examples of XP Z12-014 annex B, version 1.4 | 15 |
| `cen-examples/<file>` | the CEN EN 16931 UBL 1.3.17 examples (vendored in this repository) | 19 |
| `cen-testfiles/<file>` | the CEN EN 16931 UBL 1.3.17 test files (vendored in this repository) | 29 |
| total | | 75 |

## Cross-check

The annex examples are documents the package's profiles accept: for each, the stylesheets of
the profile named by its specification identifier (BT-24) must report no failed assert other
than one flagged `warning` or `information`, and no successful report flagged `fatal` or
`error`. `urn:cen.eu:en16931:2017` (16 examples): EN16931-UBL and the French rules;
`...#conformant#urn.cpro.gouv.fr:1p0:extended-ctc-fr` (11): EXTENDED-CTC-FR-UBL and the French
rules (both versions in each case). All 27 are accepted.
