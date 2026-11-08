# einvoice-xpath

Parser and syntax tree for the XPath 2.0 subset that the official e-invoicing validation
stylesheets use, and for the XSLT pieces built on it: match patterns, sequence types and
attribute value templates. Only syntax is handled; nothing is evaluated. The subset is closed:
a construct outside it is refused with an error naming it, never parsed as something else.

```rust
use einvoice_xpath::{parse_expr, NamespaceMap, XS_NAMESPACE};

fn main() -> Result<(), einvoice_xpath::Error> {
    let ns = NamespaceMap::new().bind("xs", XS_NAMESPACE);
    let expr = parse_expr("xs:decimal(Amount) * 2 = 10", &ns)?;
    assert_eq!(expr.to_string(), "xs:decimal(Amount) * 2 = 10");
    assert!(parse_expr("a intersect b", &ns).is_err());
    Ok(())
}
```

This crate is part of [einvoice-validator](https://github.com/einvoice-validator/einvoice-validator), which validates EN 16931 and Peppol BIS
Billing 3.0 e-invoices against the official validation artefacts in pure Rust.
Documentation: <https://docs.rs/einvoice-xpath>.

## Licence

MIT (`LICENSES/MIT.txt`).
