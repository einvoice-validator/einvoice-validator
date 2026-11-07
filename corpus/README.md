# Test corpus

Documents to validate and the results the reference engine gives for them, one directory per
official rule set. The data in each rule-set directory is derived from the official artefacts
vendored under `artefacts/` and shares their licence (see the README of each directory); this
README and `SVRL-CANONICAL.md` are part of the project (MIT).

| Directory | Rule set | Cases |
|---|---|---|
| `cen-en16931-ubl-1.3.17/` | CEN EN 16931 UBL, release 1.3.17 | 1179 |

## Cases

`<rule set>/cases.tsv` lists every case, one per line after a header, tab-separated:

| Column | Content |
|---|---|
| `case` | the case id: `examples/<file>`, `testfiles/<file>` or `vefa/<test set directory>/<test set>.<NNN>.xml` |
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

## Goldens

`<rule set>/golden/<case id>.svrl.txt` is the SVRL that Saxon-HE 12.10 produces when it runs
the rule set's compiled XSLT on the case, in the canonical form of `SVRL-CANONICAL.md`. The
invocation, with all cases copied into one input directory:

```sh
java -cp Saxon-HE-12.10.jar:xmlresolver-5.3.3.jar:xmlresolver-5.3.3-data.jar net.sf.saxon.Transform \
    -xsl:<compiled XSLT> -s:<input dir> -o:<output dir> -threads:<n>
```

Saxon's defaults apply otherwise (no whitespace stripping of the source documents); the output
does not depend on the number of threads. A run that prints anything is treated as a failure.

## Cross-check with the VEFA expectations

For each VEFA case, the failed asserts and successful reports of the golden must match the
expectations of the test: every `error` id reported with flag `fatal`, every `warning` id with
flag `warning`, no `success` id reported, and no other id of the `scope` reported. Each
disagreement is listed with its explanation in `<rule set>/vefa-disagreements.tsv`
(`case`, `kind`, `rule`, `saxon`, `explanation`); a test fails when the observed disagreements
differ from that list in either direction.

## Commands

```sh
cargo xtask manifest            # vendored files match artefacts/*/SHA256SUMS
cargo xtask corpus              # cases.tsv, vefa/ and LICENSE.txt match what extraction produces
scripts/fetch-saxon.sh          # Saxon-HE and the XML Resolver from Maven Central, checksums verified, into target/oracle/saxon/
cargo xtask oracle              # Saxon on every case; the goldens must match (needs Java)
cargo xtask crosscheck          # goldens against the VEFA expectations
```

`--write` regenerates instead of checking (`manifest`, `corpus`, `oracle`). `cargo test`
runs the manifest, extraction and cross-check checks without Java, and the equivalence test
of `crates/einvoice-xslt-exec` (`tests/cen_equivalence.rs`): the engine runs the vendored
stylesheet on every case and its canonical SVRL must equal the golden.
