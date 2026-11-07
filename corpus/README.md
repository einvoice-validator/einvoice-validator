# Test corpus

Documents to validate and the results the reference engine gives for them, one directory per
official rule set. The data in the directory of a vendored rule set is derived from the official
artefacts under `artefacts/` and shares their licence (see the README of each directory); the
Peppol directory holds only files of the project, its corpus is built locally (see below). This
README, `SVRL-CANONICAL.md` and the mutation catalogue `mutations.toml` are part of the project
(MIT); the documents the catalogue derives from the official examples share the licence of
those examples.

| Directory | Rule set | Cases | Mutation cases |
|---|---|---|---|
| `cen-en16931-ubl-1.3.17/` | CEN EN 16931 UBL, release 1.3.17 | 1179 | 504 |
| `peppol-bis-billing-ubl-3.0.22/` | Peppol BIS Billing 3.0.22, UBL: its own rules and the CEN EN 16931 rules it embeds (two stylesheets); built locally, not committed (see below) | 707 | 505 |

## Cases

`<rule set>/cases.tsv` lists every case, one per line after a header, tab-separated:

| Column | Content |
|---|---|
| `case` | the case id: `<prefix>/<file>` for a whole document (`examples`, `testfiles`, `snippets`, `national-examples`) or `vefa/<test set directory>/<test set>.<NNN>.xml` |
| `input` | the document, relative to the repository root |
| `kind` | `document` (a whole upstream document) or `vefa` (extracted from a VEFA test set) |
| `scope` | VEFA only: the rule ids the test set is about (`<testSet><assert><scope>`) |
| `success` | VEFA only: rule ids that must not fire (`<success>`) |
| `error` | VEFA only: rule ids that must fire with flag `fatal` (`<error>`) |
| `warning` | VEFA only: rule ids that must fire with flag `warning` (`<warning>`) |
| `description` | VEFA only: the test's description, whitespace normalised |

Rule ids are separated by single spaces. Whole documents have only the first three columns.

### Extraction of VEFA tests

The NNN-th `<test>` (1-based, three digits) of `test/<dir>/<set>.xml` becomes
`vefa/<dir>/<set>.<NNN>.xml`: the XML declaration, then the test's document element copied byte
for byte from the test set. When that element uses namespaces declared by its ancestors in the
test set, those declarations are added to its start tag, so the document has the same
namespaces in scope as in the test set. Each extracted document is parsed back and compared
node by node with the original element. Any structure the extractor does not know (an unknown
VEFA element, a test with no document or two) is an error.

## Mutation cases

`mutations.toml` is a catalogue of edits of a few official documents, each targeting what the
rules check and where engines can diverge: sums and totals off by a cent, rounding of halves,
decimal scale, precision and size, lexical forms of numbers, booleans and dates, VAT categories
and rates, allowances and charges, prices and base quantities, code lists (case, whitespace,
unknown codes), identifiers with check digits (GLN, organisation numbers, IBAN), missing and
repeated elements, credit notes, namespaces and prefixes, comments, CDATA and references in
values, very long values, national rules, and the elements and attributes EN 16931 does not
use. Each `[[mutation]]` has an `id`, a `base` (`invoice`, `credit-note` or `allowances`), a
one-line `reason`, the `edits` and, for the few that apply to one rule set only, `only`. The
edits, their paths and their checks are described in `xtask/src/mutation.rs`; the base
documents of each rule set are named in `xtask/src/ruleset.rs`. `cargo xtask corpus` applies
the catalogue to the bases of each rule set: the result is deterministic, and the documents
keep every byte of their base that an edit does not touch.

`<rule set>/mutations.tsv` lists the mutation cases, one per line after a header,
tab-separated: `case` (`mutations/<id>.xml`, also the document's path in the corpus
directory), `input` (relative to the base directory, as in `cases.tsv`), `base` (the id of the
base case) and `reason`. `cases.tsv` lists only the upstream cases.

Saxon-HE stops on some mutation cases with a dynamic error (an amount that is not a decimal, a
field given twice where a rule expects one value). Their golden is then
`<rule set>/golden/<case id>.error.txt`, two lines: `error <code>` and `message <message>`
(whitespace normalised); an engine is expected to stop with the same error code. Saxon-HE runs
the mutation cases on one thread with `-t`, so that each diagnostic follows the document it is
about; besides those errors, the only diagnostic accepted is the warning Saxon-HE prints when a
template pattern raises an error, which XSLT 3.0 treats as a pattern that does not match.

## Goldens

`<rule set>/golden/<case id>.svrl.txt` is the SVRL that Saxon-HE 12.10 produces when it runs
the rule set's compiled XSLT on the case, in the canonical form of `SVRL-CANONICAL.md` (or, for a
mutation case on which Saxon-HE stops, `<case id>.error.txt`, see above). The invocation, with
all cases of `cases.tsv` copied into one input directory:

```sh
java -cp Saxon-HE-12.10.jar:xmlresolver-5.3.3.jar:xmlresolver-5.3.3-data.jar net.sf.saxon.Transform \
    -xsl:<compiled XSLT> -s:<input dir> -o:<output dir> -threads:<n>
```

Saxon's defaults apply otherwise (no whitespace stripping of the source documents); the output
does not depend on the number of threads. A run that prints anything is treated as a failure.
The mutation cases are copied into another directory and run with `-t` instead of `-threads`.

## Cross-check with the VEFA expectations

For each VEFA case, the failed asserts and successful reports of the golden must match the
expectations of the test: every `error` id reported with flag `fatal`, every `warning` id with
flag `warning`, no `success` id reported, and no other id of the `scope` reported. Each
disagreement is listed with its explanation in `<rule set>/vefa-disagreements.tsv`
(`case`, `kind`, `rule`, `saxon`, `explanation`); a test fails when the observed disagreements
differ from that list in either direction.

## Peppol BIS Billing 3.0.22: built locally, not committed

The Peppol BIS Billing 3.0 repository publishes no licence, so its files, and everything derived
from them (compiled stylesheets, extracted documents, goldens), stay out of the repository:
`corpus/peppol-bis-billing-ubl-3.0.22/` holds only `vefa-disagreements.tsv` and a README. The
corpus is rebuilt from pinned inputs in `target/peppol/` by `scripts/check.sh peppol`, which CI
runs too (`.github/workflows/peppol.yml`):

1. `scripts/fetch-peppol.sh` downloads the GitHub archive of the release tag and checks its
   SHA-256 and the commit it records, then compiles `rules/sch/PEPPOL-EN16931-UBL.sch` and
   `rules/sch/CEN-EN16931-UBL.sch` with the ISO Schematron skeleton as packaged in ph-schematron
   (ph-schematron-maven-plugin 10.3.1, goal `convert`, engine `iso-schematron`, run by
   Apache Maven 3.9.16 with strict checksums and a private local repository). Both results must
   have the SHA-256 pinned in the script.
2. `cargo xtask peppol` extracts the cases as above into
   `target/peppol/peppol-bis-billing-ubl-3.0.22/corpus/`, applies the mutation catalogue to the
   Peppol examples there, runs Saxon-HE with each stylesheet (goldens in `golden-peppol/` and
   `golden-cen/`), and cross-checks the findings of both stylesheets together with the VEFA
   expectations, against the committed `vefa-disagreements.tsv`.
3. The equivalence test of `crates/einvoice-xslt-exec` (`tests/peppol_equivalence.rs`) runs
   both stylesheets with the engine on every case; each canonical SVRL must equal its golden.
   Without the cache it prints SKIPPED and passes, unless `EINVOICE_PEPPOL_REQUIRED` is set, as
   `scripts/check.sh peppol` does.

Without Java or network access, `scripts/check.sh peppol` prints SKIPPED, except in CI, where it
fails. Its cases are listed in `peppol-bis-billing-ubl-3.0.22/README.md`.

## Commands

```sh
cargo xtask manifest            # vendored files match artefacts/*/SHA256SUMS
cargo xtask corpus              # cases.tsv, vefa/, LICENSE.txt, mutations.tsv and mutations/ match what extraction and the catalogue produce
scripts/fetch-saxon.sh          # Saxon-HE and the XML Resolver from Maven Central, checksums verified, into target/oracle/saxon/
cargo xtask oracle              # Saxon on every case; the goldens must match (needs Java)
cargo xtask crosscheck          # goldens against the VEFA expectations
scripts/fetch-peppol.sh         # Peppol release and its compiled stylesheets into target/peppol/, checksums verified
cargo xtask peppol              # Peppol corpus, Saxon goldens and cross-check, in target/peppol/ (needs Java)
scripts/check.sh peppol         # the two above, then the Peppol equivalence test
```

`--write` regenerates instead of checking (`manifest`, `corpus`, `oracle`); these four commands
work on the committed corpora. `cargo test` runs the manifest, extraction, mutation and
cross-check checks
without Java, and the equivalence tests of `crates/einvoice-xslt-exec`: `tests/cen_equivalence.rs`
runs the vendored stylesheet on every case and its canonical SVRL must equal the golden;
`tests/peppol_equivalence.rs` does the same on the Peppol corpus when it has been built.
