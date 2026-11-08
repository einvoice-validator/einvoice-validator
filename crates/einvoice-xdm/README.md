# einvoice-xdm

Atomic values of the XPath 2.0 data model, as the official EN 16931 and Peppol BIS Billing
3.0 validation stylesheets use them, with the behaviour of Saxon-HE 12.10: exact,
arbitrary-precision `xs:decimal` and `xs:integer` (with Saxon's division precision and
string forms), `xs:double` parsing and formatting, dates, casts, arithmetic, comparisons,
and the XSD regular expressions of `fn:matches`, `fn:replace` and `fn:tokenize`. Behaviour
of the reference engine that is not reproduced is refused rather than approximated.

```rust
use einvoice_xdm::{AtomicType, AtomicValue, ops};

fn main() -> Result<(), einvoice_xdm::XdmError> {
    let price = AtomicValue::UntypedAtomic(" 19.99 ".to_owned()).cast(AtomicType::Decimal)?;
    let total = ops::arithmetic(ops::ArithmeticOp::Multiply, &price, &AtomicValue::from(3.0))?;
    assert_eq!(total.to_string(), "59.97");
    Ok(())
}
```

This crate is part of [einvoice-validator](https://github.com/einvoice-validator/einvoice-validator), which validates EN 16931 and Peppol BIS
Billing 3.0 e-invoices against the official validation artefacts in pure Rust. Its results
are checked against Saxon-HE. Documentation: <https://docs.rs/einvoice-xdm>.

## Licence

MIT (`LICENSES/MIT.txt`).
