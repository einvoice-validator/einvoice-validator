# einvoice-xpath-eval

Node model and evaluator of the XPath 2.0 subset that the official e-invoicing validation
stylesheets use, with the behaviour of Saxon-HE 12.10: immutable XML trees (parsed with the
streaming reader [`einvoice-xml`](https://crates.io/crates/einvoice-xml), in an arena sized
exactly before it is filled), expressions
parsed by [`einvoice-xpath`](https://crates.io/crates/einvoice-xpath) compiled against a
static context (unknown functions and variables are errors at compile time), then evaluated
lazily with the atomic values of [`einvoice-xdm`](https://crates.io/crates/einvoice-xdm).
The evaluator reads no clock: the current date and time are given by the caller, so it also
runs on `wasm32-unknown-unknown`. A host that evaluates expressions inside a recursion of its
own bounds the stack of the whole with a `StackLimit`: the evaluation then ends with a
`LIMIT` error rather than overflowing the stack.

This crate is part of [einvoice-validator](https://github.com/einvoice-validator/einvoice-validator), which validates EN 16931 and Peppol BIS
Billing 3.0 e-invoices against the official validation artefacts in pure Rust. It is checked
against Saxon-HE's value of every distinct expression of the CEN EN 16931 stylesheet at nodes
of corpus documents. Documentation: <https://docs.rs/einvoice-xpath-eval>.

## Licence

MIT (`LICENSES/MIT.txt`).
