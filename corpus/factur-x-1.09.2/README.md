# Test corpus: Factur-X 1.09.2 (ZUGFeRD 2.5.2), CII

Not committed. FeRD publishes the official Factur-X package behind a registration form; the
stylesheets used here are those of the Mustang validator 2.26.0 on Maven Central
(`org.mustangproject:validator:2.26.0`), which runs them for each profile, and the example
documents are those of the annexes of the French standards XP Z12-012 and XP Z12-014, published
by AFNOR with all rights reserved. None of these files, and nothing derived from them (extracted
documents, goldens, whose SVRL holds the rule messages), is in this repository. (The FNFE-MPE
package of French rules ships its own build of three profiles, BASIC WL, EN16931 and EXTENDED,
the last with French corrections; its stylesheets differ from these and are tested in
`../fr-ctc-cii-1.4.0.04/`.) This directory holds only files of this project (MIT):

- `disagreements.tsv`: the cases where the goldens and the expectations disagree, each with its
  explanation (none for this release).

## The corpus, built locally

`scripts/check.sh facturx` builds the corpus in `target/facturx/`, outside the tracked tree,
then checks it:

1. `scripts/fetch-facturx.sh` downloads, into `target/downloads/`, the Mustang validator jar and
   the two annex archives from `fnfe-mpe.org`, and checks the SHA-256 of each. It extracts the
   stylesheets of the five profiles (`xslt/ZF_250/FACTUR-X_<PROFILE>.xslt`), the code lists they
   read with `document()` (`FACTUR-X_<PROFILE>_codedb.xml`), their Schematron sources (each must
   state Factur-X 1.09.2) and XML schemas, and checks the SHA-256 of every stylesheet and code
   list; then the Factur-X CII documents of the annexes (`*_FX_CII_Commentee.xml`). It copies
   the CEN CII examples of `artefacts/cen-en16931-cii-1.3.17/cii/examples/`, the base documents
   of `../mutations-cii.toml`, and writes one rule-set bundle per profile
   (`target/facturx/factur-x-1.09.2/<profile>/bundle.toml`, with the stylesheet and its code
   list). Every pin is at the top of the script.
2. `cargo xtask facturx` extracts the cases into `target/facturx/factur-x-1.09.2/corpus/` (same
   layout and `cases.tsv` format as the committed corpora, see `../README.md`), applies the
   mutation catalogue `../mutations-cii.toml` to the CEN examples `CII_example5.xml`,
   `CII_example3.xml`, `CII_example2.xml`, `CII_business_example_Z.xml` and `XRechnung-O.xml`
   (808 mutation cases), runs Saxon-HE with each of the five stylesheets on every case
   (`golden-minimum/`, `golden-basic-wl/`, `golden-basic/`, `golden-en16931/`,
   `golden-extended/`), cross-checks the goldens with the expectations against
   `disagreements.tsv`, writes `acceptance.tsv` (whether the findings of each stylesheet reject the case as the
   reference validator reads them, see `../README.md`) and `complete.txt`.
3. The equivalence test of the engine, `tests/facturx_equivalence.rs` of `crates/einvoice-xslt-exec`
   (required by `scripts/check.sh facturx`), runs every stylesheet on every case and mutation
   case: the engine gives the Saxon-HE result on all of them (the same canonical SVRL, or the
   error code Saxon-HE stops with).

| Cases | Source | Count |
|---|---|---|
| `z12-012/<file>` | the Factur-X CII examples of XP Z12-012 annex B, version 1.4 | 12 |
| `z12-014/<file>` | the Factur-X CII examples of XP Z12-014 annex B, version 1.4 | 15 |
| `cen-examples/<file>` | the CEN EN 16931 CII 1.3.17 examples (vendored in this repository) | 15 |
| total | | 42 |

## Cross-check

The annex examples are documents the profile they declare accepts: for each, the stylesheet of
the profile named by its specification identifier (BT-24), as the Mustang validator maps them
(`urn:cen.eu:en16931:2017` to EN16931, `...#conformant#urn:factur-x.eu:1p0:extended` to
EXTENDED, and so on, with the ZUGFeRD 2 identifiers), must report no failed assert other than
one flagged `warning` or `information` (most Factur-X rules have no flag: an unflagged failed
assert rejects the document, as the Mustang validator reads it). The 27 examples are 16 of the
EN16931 profile and 11 of EXTENDED; all 27 are accepted.
