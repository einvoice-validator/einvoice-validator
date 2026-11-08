//! A smoke test that runs on any target, `wasm32-unknown-unknown` included (no file
//! system, no clock, no output): builds two small Factur-X-like PDF files in memory (a
//! classic cross-reference table with a Flate-compressed attachment, then a
//! cross-reference stream with the PNG Up predictor and an object stream), extracts the
//! invoice from each, and returns 0 when every check passes, else the number of the
//! first failing check.
//!
//! ```text
//! cargo run -p einvoice-pdf --example wasm-extract; echo $?
//! cargo build --release -p einvoice-pdf --example wasm-extract --target wasm32-unknown-unknown
//! wasmtime run --invoke main target/wasm32-unknown-unknown/release/examples/wasm-extract.wasm 0 0
//! ```

use std::process::ExitCode;

use miniz_oxide::deflate::compress_to_vec_zlib;

const XML: &[u8] = b"<?xml version=\"1.0\"?><rsm:CrossIndustryInvoice xmlns:rsm=\"urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100\"/>";

const XMP: &str = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
    <rdf:Description rdf:about=\"\" xmlns:fx=\"urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#\" \
    fx:DocumentType=\"INVOICE\" fx:DocumentFileName=\"factur-x.xml\" fx:Version=\"1.0\" fx:ConformanceLevel=\"MINIMUM\"/>\
    </rdf:RDF></x:xmpmeta>";

fn main() -> ExitCode {
    ExitCode::from(run())
}

/// `num 0 obj` BODY `endobj`, recording its offset.
fn object(out: &mut Vec<u8>, offsets: &mut Vec<(u32, usize)>, num: u32, body: &[u8]) {
    offsets.push((num, out.len()));
    out.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
    out.extend_from_slice(body);
    out.extend_from_slice(b"\nendobj\n");
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut body = format!("<< /Length {} {dict} >>\nstream\n", data.len()).into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(b"\nendstream");
    body
}

const CATALOG: &str = "<< /Type /Catalog /AF [5 0 R] /Metadata 6 0 R /Names << /EmbeddedFiles << /Names [(factur-x.xml) 5 0 R] >> >> >>";
const SPEC: &str = "<< /Type /Filespec /F (factur-x.xml) /UF (factur-x.xml) /AFRelationship /Alternative /EF << /F 4 0 R /UF 4 0 R >> >>";

fn header() -> Vec<u8> {
    b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec()
}

/// Objects 1, 4, 5 and 6, a classic table.
fn with_table() -> Vec<u8> {
    let mut out = header();
    let mut offsets = Vec::new();
    object(&mut out, &mut offsets, 1, CATALOG.as_bytes());
    let file = stream(
        "/Type /EmbeddedFile /Subtype /text#2Fxml /Filter /FlateDecode",
        &compress_to_vec_zlib(XML, 6),
    );
    object(&mut out, &mut offsets, 4, &file);
    object(&mut out, &mut offsets, 5, SPEC.as_bytes());
    let xmp = stream("/Type /Metadata /Subtype /XML", XMP.as_bytes());
    object(&mut out, &mut offsets, 6, &xmp);
    let xref = out.len();
    let mut lines = vec!["xref\n0 7\n0000000000 65535 f \n".to_owned()];
    for num in 1..7 {
        lines.push(match offsets.iter().find(|(n, _)| *n == num) {
            Some((_, at)) => format!("{at:010} 00000 n \n"),
            None => "0000000000 00000 f \n".to_owned(),
        });
    }
    lines.push(format!(
        "trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
    ));
    out.extend_from_slice(lines.concat().as_bytes());
    out
}

/// Objects 1 and 5 in object stream 7, objects 4 and 6 uncompressed, cross-reference
/// stream 8 with the PNG Up predictor.
fn with_xref_stream() -> Vec<u8> {
    let mut out = header();
    let mut offsets = Vec::new();
    object(
        &mut out,
        &mut offsets,
        4,
        &stream("/Type /EmbeddedFile", XML),
    );
    let xmp = stream("/Type /Metadata /Subtype /XML", XMP.as_bytes());
    object(&mut out, &mut offsets, 6, &xmp);
    let objects = format!("{CATALOG}\n{SPEC}\n");
    let pairs = format!("1 0 5 {} ", CATALOG.len() + 1);
    let data = compress_to_vec_zlib(format!("{pairs}{objects}").as_bytes(), 6);
    let objstm = stream(
        &format!(
            "/Type /ObjStm /N 2 /First {} /Filter /FlateDecode",
            pairs.len()
        ),
        &data,
    );
    object(&mut out, &mut offsets, 7, &objstm);
    let xref = out.len();
    offsets.push((8, xref));
    // Rows (type, offset or stream: 4 bytes, generation or index: 2 bytes) for 0 to 8.
    let mut rows: Vec<[u8; 7]> = Vec::new();
    for num in 0..9u32 {
        let mut row = [0u8; 7];
        if let Some((_, at)) = offsets.iter().find(|(n, _)| *n == num) {
            row[0] = 1;
            row[1..5].copy_from_slice(&u32::try_from(*at).unwrap_or(0).to_be_bytes());
        } else if num == 1 || num == 5 {
            row[0] = 2;
            row[1..5].copy_from_slice(&7u32.to_be_bytes());
            row[6] = u8::from(num == 5);
        }
        rows.push(row);
    }
    let mut predicted = Vec::new();
    let mut previous = [0u8; 7];
    for row in &rows {
        predicted.push(2);
        predicted.extend(row.iter().zip(previous).map(|(b, a)| b.wrapping_sub(a)));
        previous = *row;
    }
    let body = stream(
        "/Type /XRef /Size 9 /W [1 4 2] /Root 1 0 R /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 7 >>",
        &compress_to_vec_zlib(&predicted, 6),
    );
    let mut ignored = Vec::new();
    object(&mut out, &mut ignored, 8, &body);
    out.extend_from_slice(format!("startxref\n{xref}\n%%EOF\n").as_bytes());
    out
}

/// 0 when every check passes, else the number of the first failing one (from 1).
fn run() -> u8 {
    let mut number = 0;
    for (pdf, relationship) in [
        (with_table(), "Alternative"),
        (with_xref_stream(), "Alternative"),
    ] {
        let Ok(embedded) = einvoice_pdf::extract(&pdf) else {
            return number + 1;
        };
        let invoice = embedded.metadata.as_ref().and_then(|m| m.invoice.as_ref());
        let checks = [
            embedded.xml.data == XML,
            embedded.xml.file_name == "factur-x.xml",
            embedded
                .xml
                .relationship
                .as_ref()
                .map(einvoice_pdf::AfRelationship::as_str)
                == Some(relationship),
            embedded.xml.in_associated_files,
            invoice.and_then(|i| i.conformance_level.as_deref()) == Some("MINIMUM"),
            einvoice_pdf::attachments(&pdf).is_ok_and(|all| all.len() == 1),
        ];
        for ok in checks {
            number += 1;
            if !ok {
                return number;
            }
        }
    }
    0
}
