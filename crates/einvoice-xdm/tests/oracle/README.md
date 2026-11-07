# Saxon-HE oracle of `einvoice-xdm`

Every expected result in this directory is produced by Saxon-HE 9.9.1-8, the reference
engine of the official validation stylesheets, never written by hand.

| file | content |
|---|---|
| `cases.tsv` | hand-written probe cases: one operation on typed operands per line |
| `generated.tsv` | random cases (doubles to string, decimal and integer arithmetic, rounding, comparisons, lexical forms), from a fixed seed |
| `probe.xsl` | the stylesheet Saxon runs on the cases; operands are built at run time from their lexical forms |
| `expected.tsv` | Saxon's result for every case: `type:value` per item, `error:CODE` or `empty` |
| `format.rs` | the file format, shared by the example and the test |

The test `tests/oracle.rs` evaluates every case with this crate and compares with
`expected.tsv`; it needs neither Java nor Saxon. Cases whose id starts with `refuse-` are
valid in Saxon and refused on purpose by this crate (`ErrorCode::Unsupported`).

Regenerating the goldens needs Java 21 (the JVM provides the digits Saxon writes for
subnormal doubles and the decimal digits it accepts in long integers) and Saxon-HE, fetched
by `scripts/fetch-saxon.sh`:

```sh
scripts/fetch-saxon.sh
cargo run -p einvoice-xdm --example saxon-oracle -- generate   # rewrite generated.tsv
cargo run -p einvoice-xdm --example saxon-oracle -- --write    # rewrite expected.tsv
cargo run -p einvoice-xdm --example saxon-oracle               # check expected.tsv against Saxon
```
