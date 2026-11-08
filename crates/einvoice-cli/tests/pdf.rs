//! PDF files on the command line: a file that starts with `%PDF-` is validated through the
//! invoice XML it embeds (Factur-X, `ZUGFeRD`, Order-X), with the rule set its root element
//! names unless `--rules` or `--bundle` is given; the text and JSON outputs say what the
//! PDF states about it; a PDF the invoice cannot be read from is an error of kind `io`
//! (exit 2), an encrypted one or one beyond the PDF subset read an error of kind
//! `unsupported` (exit 3). The PDF files are synthetic (einvoice-testkit), embedding
//! documents of the corpus.

#![cfg(test)]

mod common;

use std::path::{Path, PathBuf};

use common::json::{self, Value};
use einvoice_testkit::pdf::{Fixture, LAYOUTS, Layout, Writer, xmp};

const CII_VALID: &str = "artefacts/cen-en16931-cii-1.3.17/cii/examples/CII_example1.xml";
const CII_INVALID: &str = "corpus/cen-en16931-cii-1.3.17/vefa/cii/BR-CO-15-2.002.xml";
const UBL_VALID: &str = "artefacts/cen-en16931-ubl-1.3.17/ubl/examples/ubl-tc434-example1.xml";
const UBL_INVALID: &str = "corpus/cen-en16931-ubl-1.3.17/vefa/Invoice-unit-UBL/BR-02.002.xml";

fn read(path: &str) -> Vec<u8> {
    std::fs::read(common::join(&common::workspace_root(), path)).unwrap()
}

/// A Factur-X-like PDF embedding `data` as `name`, with the default fixture otherwise
/// (no `/CheckSum`: the corpus documents have none written for them).
fn pdf(name: &str, data: &[u8], layout: Layout) -> Vec<u8> {
    Fixture {
        layout,
        name,
        data,
        checksum: None,
        xmp: Some(xmp(name, "EN 16931")),
        ..Fixture::default()
    }
    .build()
}

/// Writes `bytes` to `dir/name`; returns the path as a string.
fn write(dir: &Path, name: &str, bytes: &[u8]) -> String {
    let path: PathBuf = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_str().unwrap().to_owned()
}

fn documents(r: &common::Run) -> Vec<Value> {
    json::parse(&r.stdout)
        .unwrap()
        .get("documents")
        .array()
        .to_vec()
}

/// The members of a document that do not depend on how the XML was given.
fn verdict(document: &Value) -> (Option<String>, Option<String>, Vec<common::Finding>) {
    (
        document.get("rule_set").str().map(str::to_owned),
        document.get("status").str().map(str::to_owned),
        common::json_findings(document),
    )
}

#[test]
fn a_pdf_is_validated_through_its_embedded_xml_with_the_rule_set_of_its_root() {
    let dir = common::temp_dir("pdf-verdicts");
    let mut pdfs = Vec::new();
    let mut xmls = Vec::new();
    for (i, (xml, name)) in [
        (CII_VALID, "factur-x.xml"),
        (CII_INVALID, "zugferd-invoice.xml"),
        (UBL_VALID, "xrechnung.xml"),
        (UBL_INVALID, "factur-x.xml"),
    ]
    .into_iter()
    .enumerate()
    {
        pdfs.push(write(
            &dir,
            &format!("{i}.pdf"),
            &pdf(name, &read(xml), Layout::ObjectStreams),
        ));
        xmls.push(xml);
    }
    let pdf_args: Vec<&str> = ["-f", "json"]
        .into_iter()
        .chain(pdfs.iter().map(String::as_str))
        .collect();
    let from_pdf = common::run(&pdf_args, None);
    assert_eq!(from_pdf.code, 1, "{}", from_pdf.stderr);
    assert_eq!(from_pdf.stderr, "");
    let xml_args: Vec<&str> = ["-f", "json"].into_iter().chain(xmls).collect();
    let from_xml = common::run(&xml_args, None);
    let pdf_documents = documents(&from_pdf);
    let xml_documents = documents(&from_xml);
    let statuses: Vec<Option<&str>> = pdf_documents
        .iter()
        .map(|d| d.get("status").str())
        .collect();
    assert_eq!(
        statuses,
        [
            Some("valid"),
            Some("invalid"),
            Some("valid"),
            Some("invalid")
        ]
    );
    let rule_sets: Vec<Option<&str>> = pdf_documents
        .iter()
        .map(|d| d.get("rule_set").str())
        .collect();
    assert_eq!(
        rule_sets,
        [
            Some("cen-cii"),
            Some("cen-cii"),
            Some("cen-ubl"),
            Some("cen-ubl")
        ]
    );
    for (p, x) in pdf_documents.iter().zip(&xml_documents) {
        assert_eq!(verdict(p), verdict(x));
        assert_eq!(p.get("schema_errors"), x.get("schema_errors"));
        assert_eq!(p.get("counts"), x.get("counts"));
        assert_eq!(x.get("pdf"), &Value::Null);
    }
    check_pdf_member(pdf_documents[0].get("pdf"));
    assert_eq!(
        pdf_documents[2]
            .get("pdf")
            .get("attachment")
            .get("file_name")
            .str(),
        Some("xrechnung.xml")
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// The `pdf` member of a document embedded with the default fixture as `factur-x.xml`.
fn check_pdf_member(first: &Value) {
    assert_eq!(first.keys(), ["version", "attachment", "xmp"]);
    assert_eq!(first.get("version").str(), Some("1.7"));
    let attachment = first.get("attachment");
    assert_eq!(
        attachment.keys(),
        [
            "file_name",
            "relationship",
            "in_associated_files",
            "description",
            "mime_type"
        ]
    );
    assert_eq!(attachment.get("file_name").str(), Some("factur-x.xml"));
    assert_eq!(attachment.get("relationship").str(), Some("Data"));
    assert_eq!(attachment.get("in_associated_files"), &Value::Bool(true));
    assert_eq!(attachment.get("description").str(), Some("Invoice"));
    assert_eq!(attachment.get("mime_type").str(), Some("text/xml"));
    let xmp = first.get("xmp");
    assert_eq!(xmp.keys(), ["pdfa_part", "pdfa_conformance", "invoice"]);
    assert_eq!(xmp.get("pdfa_part").str(), Some("3"));
    assert_eq!(xmp.get("pdfa_conformance").str(), Some("B"));
    let invoice = xmp.get("invoice");
    assert_eq!(
        invoice.get("namespace").str(),
        Some("urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#")
    );
    assert_eq!(invoice.get("document_type").str(), Some("INVOICE"));
    assert_eq!(
        invoice.get("document_file_name").str(),
        Some("factur-x.xml")
    );
    assert_eq!(invoice.get("version").str(), Some("1.0"));
    assert_eq!(invoice.get("conformance_level").str(), Some("EN 16931"));
}

#[test]
fn every_layout_of_the_pdf_reader_gives_the_same_report() {
    let dir = common::temp_dir("pdf-layouts");
    let xml = read(CII_INVALID);
    let paths: Vec<String> = LAYOUTS
        .iter()
        .enumerate()
        .map(|(i, layout)| {
            write(
                &dir,
                &format!("{i}.pdf"),
                &pdf("factur-x.xml", &xml, *layout),
            )
        })
        .collect();
    let args: Vec<&str> = ["-f", "json"]
        .into_iter()
        .chain(paths.iter().map(String::as_str))
        .collect();
    let r = common::run(&args, None);
    assert_eq!(r.code, 1, "{}", r.stderr);
    let expected = documents(&common::run(&["-f", "json", CII_INVALID], None));
    for document in documents(&r) {
        assert_eq!(verdict(&document), verdict(&expected[0]));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_text_output_says_what_the_pdf_states() {
    let dir = common::temp_dir("pdf-text");
    let with_xmp = write(
        &dir,
        "a.pdf",
        &pdf("factur-x.xml", &read(CII_VALID), Layout::Table),
    );
    let without = Fixture {
        name: "factur-x.xml",
        data: &read(UBL_INVALID),
        checksum: None,
        xmp: None,
        relationship: "/Alternative",
        ..Fixture::default()
    }
    .build();
    let without = write(&dir, "b.pdf", &without);
    let r = common::run(&[&with_xmp, &without], None);
    assert_eq!(r.code, 1, "{}", r.stderr);
    let lines: Vec<&str> = r.stdout.lines().collect();
    assert_eq!(lines[0], format!("{with_xmp}: valid"));
    assert_eq!(
        lines[1],
        "  pdf      factur-x.xml embedded in a PDF 1.7: AFRelationship Data; PDF/A-3B; \
         document type INVOICE, file name factur-x.xml, version 1.0, conformance level EN 16931"
    );
    assert!(
        lines[2].starts_with(&format!("{without}: invalid: ")),
        "{}",
        lines[2]
    );
    assert_eq!(
        lines[3],
        "  pdf      factur-x.xml embedded in a PDF 1.7: AFRelationship Alternative; no XMP metadata"
    );
    assert!(lines[4].starts_with("  schema   "), "{}", lines[4]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rules_and_standard_input_and_svrl() {
    let dir = common::temp_dir("pdf-rules");
    let bytes = pdf(
        "factur-x.xml",
        &read(CII_VALID),
        Layout::XrefStreamPredictor,
    );
    let path = write(&dir, "cii.pdf", &bytes);
    // --rules wins over the root element.
    let r = common::run(&["-f", "json", "--rules", "cen-ubl", &path], None);
    assert_eq!(r.code, 1);
    let document = &documents(&r)[0];
    assert_eq!(document.get("rule_set").str(), Some("cen-ubl"));
    assert_eq!(document.get("status").str(), Some("invalid"));
    // Standard input.
    let r = common::run(&["-f", "json", "-"], Some(&bytes));
    assert_eq!(r.code, 0, "{}", r.stdout);
    let document = &documents(&r)[0];
    assert_eq!(document.get("path").str(), Some("-"));
    assert_eq!(document.get("rule_set").str(), Some("cen-cii"));
    assert_eq!(
        document.get("pdf").get("attachment").get("file_name").str(),
        Some("factur-x.xml")
    );
    // The SVRL of the embedded XML is that of the XML given directly.
    let from_pdf = common::run(&["-f", "svrl", &path], None);
    let from_xml = common::run(&["-f", "svrl", CII_VALID], None);
    assert_eq!(from_pdf.code, 0);
    assert_eq!(from_pdf.stdout, from_xml.stdout);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_pdf_without_one_readable_invoice_is_an_error() {
    let dir = common::temp_dir("pdf-errors");
    let cii = read(CII_VALID);
    let no_invoice = write(&dir, "none.pdf", &pdf("invoice.xml", &cii, Layout::Table));
    let malformed = write(&dir, "malformed.pdf", b"%PDF-1.7\nnot a PDF body\n");
    let mut encrypted = Writer::new("1.7");
    encrypted.object(1, b"<< /Type /Catalog >>");
    encrypted.object(2, b"<< /Filter /Standard /V 2 /R 3 >>");
    encrypted.table("/Root 1 0 R /Encrypt 2 0 R");
    let encrypted = write(&dir, "encrypted.pdf", &encrypted.finish());
    let not_xml = write(
        &dir,
        "not-xml.pdf",
        &pdf("factur-x.xml", b"<Invoice>\n<a></Invoice>", Layout::Table),
    );
    let r = common::run(
        &["-f", "json", &no_invoice, &malformed, &encrypted, &not_xml],
        None,
    );
    assert_eq!(r.code, 3, "{}", r.stdout);
    let documents = documents(&r);
    let kinds: Vec<(Option<&str>, Option<&str>)> = documents
        .iter()
        .map(|d| (d.get("status").str(), d.get("error").get("kind").str()))
        .collect();
    assert_eq!(
        kinds,
        [
            (Some("error"), Some("io")),
            (Some("error"), Some("io")),
            (Some("error"), Some("unsupported")),
            (Some("invalid"), Some("not-xml")),
        ]
    );
    let message = |i: usize| {
        documents[i]
            .get("error")
            .get("message")
            .str()
            .unwrap()
            .to_owned()
    };
    assert!(
        message(0).starts_with(
            "no embedded invoice: no embedded file has an invoice name; embedded files: invoice.xml"
        ),
        "{}",
        message(0)
    );
    assert!(message(1).starts_with("malformed PDF: "), "{}", message(1));
    assert!(message(2).starts_with("encrypted PDF"), "{}", message(2));
    // No invoice was extracted: no pdf member; the XML that is not well-formed was.
    for document in &documents[..3] {
        assert_eq!(document.get("pdf"), &Value::Null);
    }
    assert_eq!(
        documents[3]
            .get("pdf")
            .get("attachment")
            .get("file_name")
            .str(),
        Some("factur-x.xml")
    );
    assert_eq!(documents[3].get("error").get("line").number(), 2);
    // Each alone: its exit code.
    for (path, code) in [
        (&no_invoice, 2),
        (&malformed, 2),
        (&encrypted, 3),
        (&not_xml, 1),
    ] {
        let r = common::run(&[path.as_str()], None);
        assert_eq!(r.code, code, "{path}: {}", r.stdout);
    }
    let r = common::run(&[&no_invoice], None);
    assert_eq!(
        r.stdout,
        format!(
            "{no_invoice}: error (io): no embedded invoice: no embedded file has an invoice name; embedded files: invoice.xml\n"
        )
    );
    std::fs::remove_dir_all(dir).unwrap();
}
