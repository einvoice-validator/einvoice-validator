# Saxon-HE oracle of `einvoice-xpath-eval`

Every expected result here is produced by Saxon-HE 9.9.1-8, the reference engine of the
official validation stylesheets, never written by hand.

Two sets of probes, in the same format (described in `common.rs`):

| file | content |
|---|---|
| `cases.tsv` | hand-written edge cases: an expression, the synthetic document it runs on and the nodes it runs at |
| `docs/` | the synthetic documents of the edge cases |
| `expected.tsv` | Saxon's results for the edge cases |
| `../../../../corpus/cen-en16931-ubl-1.3.17/xpath/` | every distinct expression of the vendored CEN stylesheet at nodes of corpus documents, and the list of those documents (derived from the CEN artefacts, EUPL-1.2) |
| `common.rs` | the stylesheet walker, the file formats and node paths, shared by the example and the test |

The CEN probes take each template of `EN16931-UBL-validation.xslt` (and each step of a
match pattern that has predicates) as a group: its expressions (`xsl:when`/`xsl:if` tests,
`select`s, pattern predicates) are evaluated at every node its pattern matches, inside the
template's `xsl:for-each`s and after its local variables, as in the stylesheet. Tests are
probed twice: their value, and their effective boolean value as `xsl:when` computes it. The
goldens keep up to six contexts per expression over all documents, contexts with different
results first. `documents.tsv` lists the corpus documents used: a fixed starting set, then
documents added until every group that some corpus document reaches is reached.

The test `tests/oracle.rs` replays every golden line with this crate and needs neither Java
nor Saxon. A line that disagrees fails the test unless it is listed, with its reason, in
`KNOWN_DIFFERENCES` there.

Regenerating the goldens needs Java 21 and Saxon-HE, fetched by `scripts/fetch-saxon.sh`:

```sh
scripts/fetch-saxon.sh
cargo run -p einvoice-xpath-eval --example saxon-oracle -- select-documents  # rewrite documents.tsv (whole corpus)
cargo run -p einvoice-xpath-eval --example saxon-oracle -- --write           # rewrite the goldens
cargo run -p einvoice-xpath-eval --example saxon-oracle                      # check the goldens against Saxon
```

Saxon refuses statically some expressions that are only dynamic errors here (its type
inference sees them); such edge cases pass their operand through `p:any`, an identity
function that hides the static type, and some probe only the value (`v`) because Saxon
refuses their effective boolean value statically.
