# einvoice-xml

A streaming (pull) parser of XML 1.0 documents with namespaces, in pure Rust (no `unsafe`,
no C): it builds no tree, so that well-formedness checking and validation can run in one
pass over a document in memory that does not grow with the document. Names, and text and
attribute values that need no reference replacement or normalisation, are slices of the
input; the reader keeps only the open elements, the namespace declarations in scope and
the attributes of the current start tag.

```rust
use einvoice_xml::{Event, Reader};

fn main() -> Result<(), einvoice_xml::Error> {
    let mut reader = Reader::new(r#"<i:Invoice xmlns:i="urn:i"><i:ID>a &amp; b</i:ID></i:Invoice>"#);
    while let Some(event) = reader.next_event()? {
        if let Event::Text(text) = event {
            assert_eq!(text.value, "a & b");
        }
    }
    assert!(einvoice_xml::check("<a><b></a>").is_err());
    Ok(())
}
```

Supported: UTF-8 text (a byte order mark is skipped), the XML declaration, elements,
attributes, namespaces, comments, processing instructions, CDATA sections, character
references and the five predefined entities, line-end and attribute-value normalisation.
Document type declarations are refused, and so is any other entity.

The reader accepts the documents that the `roxmltree` crate (0.21, document type
declarations refused) accepts and that are namespace-well-formed, and reports
`roxmltree`'s error (message and position) on the documents it refuses, except on the few
constructs that `roxmltree` accepts against XML 1.0 or Namespaces in XML, which the crate
documentation lists. The test suite checks this on every XML file of the repository, on
generated and damaged documents, and checks the documented differences against Saxon-HE.
[`Limits`](https://docs.rs/einvoice-xml/latest/einvoice_xml/struct.Limits.html) bounds
nodes, depth, attributes and namespace declarations; time is linear in the input.

This crate is part of [einvoice-validator](https://github.com/einvoice-validator/einvoice-validator),
which validates EN 16931 and Peppol BIS Billing 3.0 e-invoices against the official
validation artefacts in pure Rust. Documentation: <https://docs.rs/einvoice-xml>.

## Licence

MIT (`LICENSES/MIT.txt`).
