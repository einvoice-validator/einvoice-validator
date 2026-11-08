# einvoice-schematron

Schematron compiled to XSLT exactly as ph-schematron 10.3.1 writes it
(`ph-schematron-maven-plugin:convert`, engine `iso-schematron`), in pure Rust: the
stages of that compiler that are not XSLT.

ph-schematron runs the three stylesheets of the ISO Schematron skeleton (include,
abstract patterns, compilation to XSLT) on Saxon-HE, each into a DOM document of the JDK,
and writes the last one with the XML writer of ph-commons. This crate builds that DOM
document from the result tree of the last step, as Saxon's `DOMWriter` builds it (the
attribute order and text normalization of the JDK's DOM, the names it refuses), and
writes it as ph-commons does, byte for byte: namespace prefixes, indentation, escaping,
the characters dropped.

```rust
let stylesheet = einvoice_schematron::compile_result(result_tree_as_xml)?;
```

The tests compare, on synthetic Schematron files, the DOM document and the stylesheet
with those of ph-schematron itself (goldens written by the `ph-schematron-oracle`
example, which runs ph-schematron on the JVM), and, locally, the stylesheets of Peppol
BIS Billing 3.0 with their pinned SHA-256.

Documentation: <https://docs.rs/einvoice-schematron>.

Licence: MIT.
