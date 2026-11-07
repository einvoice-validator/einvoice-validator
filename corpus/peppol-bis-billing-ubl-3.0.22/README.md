# Test corpus: Peppol BIS Billing 3.0.22, UBL

The Peppol BIS Billing 3.0 repository (`OpenPeppol/peppol-bis-invoice-3`) publishes no licence,
so none of its files, and nothing derived from them (compiled stylesheets, extracted documents,
goldens, whose SVRL holds the rule messages), is in this repository. This directory holds only
files of this project (MIT):

- `vefa-disagreements.tsv`: the cases where the goldens and the VEFA expectations disagree, each
  with its explanation (none for this release).

## The corpus, built locally

`scripts/check.sh peppol` builds the corpus in `target/peppol/`, outside the tracked tree, then
checks it:

1. `scripts/fetch-peppol.sh` downloads the GitHub archive of tag `3.0.22` (commit
   `38d693d71d28d813781323a6ca97324edcc4d239`), checks its SHA-256 and extracts it into
   `target/peppol/peppol-bis-billing-ubl-3.0.22/source/`. It compiles
   `rules/sch/PEPPOL-EN16931-UBL.sch` and `rules/sch/CEN-EN16931-UBL.sch` (the CEN EN 16931 rules
   that Peppol BIS Billing 3.0.22 embeds, release 1.3.16) to XSLT with the ISO Schematron skeleton
   as packaged in ph-schematron, and checks the SHA-256 of both results:
   `target/peppol/peppol-bis-billing-ubl-3.0.22/xslt/`. Every pin is at the top of the script.
2. `cargo xtask peppol` extracts the cases into `target/peppol/peppol-bis-billing-ubl-3.0.22/corpus/`
   (same layout and `cases.tsv` format as the committed corpora, see `../README.md`), applies
   the mutation catalogue `../mutations.toml` to `rules/examples/base-example.xml`,
   `base-creditnote-correction.xml` and `Allowance-example.xml` (505 mutation cases,
   `mutations.tsv` and `mutations/`), runs
   Saxon-HE with each stylesheet on every case (`golden-peppol/` and `golden-cen/`), cross-checks
   the goldens of both stylesheets together with the VEFA expectations against
   `vefa-disagreements.tsv`, and writes `complete.txt` (number of cases and of mutation cases,
   checksums of the stylesheets).
3. `crates/einvoice-xslt-exec/tests/peppol_equivalence.rs`: the engine runs both stylesheets on
   every case; each canonical SVRL must equal its golden.

| Cases | Source in the release | Count |
|---|---|---|
| `examples/<file>` | `rules/examples/` | 10 |
| `snippets/<file>` | `rules/snippets/` | 16 |
| `national-examples/<country>/<file>` | `rules/national-examples/` | 3 |
| `vefa/<dir>/<set>.<NNN>.xml` | the UBL VEFA unit test sets: `rules/unit-UBL-PEPPOL/` (340), `unit-UBL-DE` (65), `unit-UBL-DK` (137), `unit-UBL-GR` (41), `unit-UBL-IT` (12), `unit-UBL-NL` (21), `unit-UBL-NO` (14), `unit-UBL-SE` (48) | 678 |
| total | | 707 |

Not used: `rules/national-examples/README.adoc`, and `rules/unit-UBL-PEPPOL/PEPPOL-COMMON-R048.xm`,
whose tests are all commented out (the upstream build runs only the `*.xml` files of a test
folder). The CII and SBDH test sets are for other syntaxes.
