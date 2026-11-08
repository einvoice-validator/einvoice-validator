# Test corpus: French e-invoicing rules (FNFE-MPE package 1.4.0.04), CDAR (life-cycle statuses)

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
2. `cargo xtask fr-ctc` extracts the cases into `target/fr-ctc/fr-ctc-1.4.0.04/corpus-cdar/` (same
   layout and `cases.tsv` format as the committed corpora, see `../README.md`) (there is no mutation catalogue for this syntax), runs
   Saxon-HE with each stylesheet on every case, cross-checks the goldens with the expectations
   against `disagreements.tsv`, and writes `complete.txt`.

| Stylesheet (package path) | Goldens |
|---|---|
| `CDAR/2xslt/BR-FR-CDV-Schematron-CDAR.xslt` | `golden/` |
| `CDAR/2xslt/BR-FR-CDV-Schematron-CDAR_WARNING.xslt` (its WARNING version) | `golden-warning/` |

| Cases | Source | Count |
|---|---|---|
| `z12-012/<file>` | the life-cycle status examples of XP Z12-012 annex B, version 1.4 | 11 |
| `z12-014/<file>` | the life-cycle status examples of XP Z12-014 annex B, version 1.4 | 59 |
| total | | 70 |

No golden has a finding: every example satisfies every rule, so this corpus exercises the
rules' contexts and fired rules, not their failures.

## Cross-check

The annex examples are documents the package accepts: both stylesheets must report no failed
assert other than one flagged `warning` or `information` (identifiers
`urn.cpro.gouv.fr:1p0:CDV:invoice`, 53 examples, and `urn.cpro.gouv.fr:1p0:CDV:einvoicingF2`,
17). All 70 are accepted.

## E-reporting (Flux 10)

The package's `Flux10/Flux10.xslt` is fetched and has its bundle (`flux10`), but neither the
package nor the annexes include an e-reporting document: there is no corpus for it.
