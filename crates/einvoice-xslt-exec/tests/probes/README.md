# Saxon-HE probes of `einvoice-xslt-exec`

Synthetic stylesheets for the XSLT behaviour the official rule sets do not all exercise:
rule selection and ties, `xsl:next-match`, modes, built-in rules, simple and complex content,
variables and parameters, the static types Saxon gives their references (`static-types`),
`xsl:function`, `xsl:number`, patterns (the XSLT 3.0 parenthesized ones in
`patterns-parenthesized`), namespaces (also in a temporary tree, as the stylesheets compiled by
SchXslt build their report: `temporary-tree-namespaces`), `cast as`, nodes constructed in a
sequence as the stylesheets compiled by SchXslt construct them (`sequences`), when Saxon
evaluates variables and
parameters (`lazy`, and the `error-learned-*`, `error-eager-*` and `error-stale-references`
probes, where a value nothing reads raises an error), paths whose nodes Saxon reads unsorted and
lazily where neither their order nor their duplicates matter (`unordered-paths`, and the
`error-unordered-*` probes, where they matter), dynamic errors, and dynamic errors raised
while matching a pattern (`pattern-error*`, which Saxon recovers from with a warning). Each runs on `input.xml`. Every expected result in
`expected/` is produced by Saxon-HE, the reference engine, never written by hand (format in
`../common/probes.rs`: the result, or the error code, the number of distinct XTDE0540
warnings, as Saxon's error reporter prints each distinct warning once, and the line and
code of each error recovered while matching a pattern).

The test `../probes.rs` runs each probe with this crate and compares the result trees node by
node (names with prefixes, attributes, namespaces in scope, text, comments); it needs neither
Java nor Saxon.

Regenerating or checking the expected results needs Java and Saxon-HE, fetched by
`scripts/fetch-saxon.sh`:

```sh
scripts/fetch-saxon.sh
cargo run -p einvoice-xslt-exec --example saxon-probes -- --write   # rewrite expected/
cargo run -p einvoice-xslt-exec --example saxon-probes              # check expected/ against Saxon
```
