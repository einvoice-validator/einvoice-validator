# einvoice-pdf

Extract the embedded e-invoice XML from a Factur-X, ZUGFeRD or Order-X PDF (a PDF/A-3 file
with the invoice as an associated file), with its XMP metadata, in pure Rust: no C library,
no `unsafe` code, two dependencies (`miniz_oxide` for Flate, `roxmltree` for the XMP
packet), and it builds for `wasm32-unknown-unknown`.

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pdf = std::fs::read("invoice.pdf")?;
    let invoice = einvoice_pdf::extract(&pdf)?;
    println!("{}: {} bytes", invoice.xml.file_name, invoice.xml.data.len());
    println!("relationship: {:?}", invoice.xml.relationship);
    if let Some(fx) = invoice.metadata.as_ref().and_then(|m| m.invoice.as_ref()) {
        println!("{:?} {:?} {:?}", fx.document_type, fx.version, fx.conformance_level);
    }
    for attachment in einvoice_pdf::attachments(&pdf)? {
        println!("embedded: {} ({:?})", attachment.file_name, attachment.mime_type);
    }
    Ok(())
}
```

`extract` returns the embedded file whose name is one of `factur-x.xml`, `zugferd-invoice.xml`,
`ZUGFeRD-invoice.xml`, `xrechnung.xml` or `order-x.xml` (`INVOICE_FILE_NAMES`), byte for
byte, with its file specification (`/AFRelationship`, `/Desc`, whether it is listed in the
catalog's `/AF`, its key in the `/EmbeddedFiles` name tree), its stream's `/Subtype` and
`/Params`, and the XMP metadata of the document catalog: the packet as text, `pdfaid:part`,
`pdfaid:conformance` and the `DocumentType`, `DocumentFileName`, `Version` and
`ConformanceLevel` properties of the Factur-X / ZUGFeRD / Order-X extension schema
(any `urn:<scheme>:pdfa:CrossIndustryDocument:…` namespace, as elements or as attributes).
No invoice name, or several, is an error. `attachments` returns every embedded file.

## What is read, and what is refused

Only the part of ISO 32000-2 on the path to embedded files is read: the header (at byte 0),
the last `startxref`, classic cross-reference tables and cross-reference streams with their
trailers, incremental updates (`/Prev`) and hybrid files (`/XRefStm`), indirect objects,
object streams, the object syntax (literal and hexadecimal strings with their escapes, names
with `#xx`, numbers, arrays, dictionaries, comments), stream `/Length` direct or indirect,
`FlateDecode` with the PNG predictors, text strings (`PDFDocEncoding`, UTF-16BE, UTF-8), the
catalog's `/AF`, `/Names /EmbeddedFiles` (`/Kids`, `/Names`, `/Limits`), file specification
dictionaries (`/F`, `/UF`, `/EF`, `/AFRelationship`, `/Desc`) and embedded file streams
(`/Subtype`, `/Params`: `/Size` and the MD5 `/CheckSum` are verified against the content).
Page content is never read.

Anything else met on that path is refused with `Error::Unsupported` (another filter, the TIFF
predictor, external stream data, a file specification string, …), and a file that breaks the
syntax with `Error::Malformed`, never repaired: an offset that does not point to the object
it should, a `/Length` that does not end at `endstream`, a key repeated in a dictionary, keys
outside their name tree node's `/Limits`, overlapping objects or sections. Encrypted files
are refused (`Error::Encrypted`): PDF/A forbids encryption. Two common deviations are
read, because they are unambiguous: a `/CheckSum` written as the 32 hexadecimal digits of the
MD5 digest instead of its 16 bytes (still verified), and a generation number above 65535 on a
free cross-reference entry. In a hybrid section, an object the table lists, even as free,
is taken from the table, as poppler and MuPDF read it.

The input is untrusted. `Limits` bounds the nesting of arrays and dictionaries, chains of
references, the depth of the name tree, the number of cross-reference sections and of
embedded files, the decoded size of one stream and of all streams, and the size of the XMP
tree; each object and object stream is parsed at most once; reference cycles, `/Prev` loops
and shared or cyclic name tree nodes are errors.

## Tests

The committed tests build their PDF files in code (the module `pdf` of `einvoice-testkit`, an
unpublished crate of the repository, shared with the crates that read PDF files through this
one): every layout above, incremental updates, name trees, the five invoice names and the
malformed and hostile cases, with property tests on arbitrary bytes and random edits. Published sample files can be checked
locally (`tests/samples.rs`): `EINVOICE_PDF_SAMPLES` lists manifest files whose lines are
`PDF<TAB>EXPECTED`, where `EXPECTED` is the published XML (the extracted invoice must be
byte-identical) or `error:<Kind>`; without the variable the test prints SKIPPED.

This crate is part of [einvoice-validator](https://github.com/einvoice-validator/einvoice-validator), which validates EN 16931 and Peppol BIS
Billing 3.0 e-invoices against the official validation artefacts in pure Rust. Documentation:
<https://docs.rs/einvoice-pdf>.

## Licence

MIT (`LICENSES/MIT.txt`).
