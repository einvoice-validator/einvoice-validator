# Saxon-HE probes of `einvoice-xslt-exec`

Synthetic stylesheets for the XSLT behaviour the official rule sets do not all exercise:
rule selection and ties, `xsl:next-match`, modes, built-in rules, simple and complex content,
variables and parameters, `xsl:function`, `xsl:number`, patterns, namespaces, `cast as`, dynamic
errors, and dynamic errors raised while matching a pattern (`pattern-error*`, which Saxon
recovers from with a warning). Each runs on `input.xml`. Every expected result in
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
