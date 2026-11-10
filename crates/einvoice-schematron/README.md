# einvoice-schematron

Schematron compiled to XSLT exactly as ph-schematron 10.3.1 writes it
(`ph-schematron-maven-plugin:convert`, engine `iso-schematron`), in pure Rust, without
Java.

ph-schematron runs the three stylesheets of the ISO Schematron skeleton (include,
abstract patterns, compilation to XSLT) on Saxon-HE, each into a DOM document of the JDK,
and writes the last one with the XML writer of ph-commons. This crate runs the same three
stylesheets, as ph-schematron-isosch 10.3.1 packages them, on this project's XSLT engine
(`einvoice-xslt`, `einvoice-xslt-exec`), passes each result to the next step as
ph-schematron does, builds the DOM document of the last one as Saxon's `DOMWriter` builds
it (the attribute order and text normalization of the JDK's DOM, the names it refuses),
and writes it as ph-commons does, byte for byte: namespace prefixes, indentation,
escaping, the characters dropped.

```rust
// The four stylesheets of the skeleton, by file name, read from the jar of
// ph-schematron-isosch 10.3.1 (each checked against its SHA-256).
let skeleton = einvoice_schematron::Skeleton::new(&skeleton_files)?;
let sources = einvoice_schematron::Sources::new().with(uri, schematron);
let stylesheet = skeleton.compile(schematron, uri, &sources)?.stylesheet;
```

The skeleton is not part of the crate: the caller gives its files (`einvoice-validate
rules fetch peppol` takes them from the pinned jar on Maven Central).

The tests compare, on synthetic Schematron files, the result of each step, the DOM
document and the stylesheet with those of ph-schematron itself (goldens written by the
`ph-schematron-oracle` example, which runs ph-schematron on the JVM), and, locally, the
stylesheets of Peppol BIS Billing 3.0 and of the CEN rules it embeds with their pinned
SHA-256.

Documentation: <https://docs.rs/einvoice-schematron>.

Licence: MIT.
