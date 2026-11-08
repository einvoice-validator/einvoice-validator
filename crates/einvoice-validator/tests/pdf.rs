//! The module `pdf`: the invoice XML a synthetic Factur-X-like PDF embeds (einvoice-testkit)
//! is the corpus document byte for byte, the built-in rule set its root names validates it
//! as it validates the document itself, and the errors of the PDF reader have the kinds
//! the documentation gives them.

#![cfg(test)]
#![cfg(all(feature = "pdf", feature = "cen-ubl", feature = "cen-cii"))]

use std::path::Path;

use einvoice_testkit::pdf::{Fixture, LAYOUTS, Writer, embedded, filespec, with_catalog, xmp};
use einvoice_validator::pdf::{self, AfRelationship, Limits};
use einvoice_validator::{Clock, Error, ErrorKind, Syntax, ValidateOptions, Validator, Verdict};

const CII: &str = "artefacts/cen-en16931-cii-1.3.17/cii/examples/CII_example1.xml";
const UBL_INVALID: &str = "corpus/cen-en16931-ubl-1.3.17/vefa/Invoice-unit-UBL/BR-02.002.xml";

fn read(path: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path),
    )
    .unwrap()
}

fn options() -> ValidateOptions<'static> {
    ValidateOptions::new().clock(Clock::from_unix_millis(0, 1_789_000_123_456).unwrap())
}

#[test]
fn the_embedded_xml_is_validated_with_the_rule_set_of_its_root() {
    for (path, rule_set, verdict) in [
        (CII, "cen-cii", Verdict::Valid),
        (UBL_INVALID, "cen-ubl", Verdict::Invalid),
    ] {
        let xml = read(path);
        let validator = Validator::builtin(rule_set).unwrap();
        let direct = validator.validate_with(&xml, &options()).unwrap();
        assert_eq!(direct.verdict(), verdict, "{path}");
        for layout in LAYOUTS {
            let file = Fixture {
                layout,
                data: &xml,
                checksum: None,
                ..Fixture::default()
            }
            .build();
            assert!(pdf::is_pdf(&file));
            let embedded = pdf::extract(&file).unwrap();
            assert_eq!(embedded.xml.data, xml, "{path} {layout:?}");
            assert_eq!(embedded.xml.file_name, "factur-x.xml");
            assert_eq!(embedded.xml.relationship, Some(AfRelationship::Data));
            let metadata = embedded.metadata.as_ref().unwrap();
            assert_eq!(metadata.pdfa_part.as_deref(), Some("3"));
            assert_eq!(
                metadata
                    .invoice
                    .as_ref()
                    .unwrap()
                    .conformance_level
                    .as_deref(),
                Some("EN 16931")
            );
            let syntax = Syntax::detect(&embedded.xml.data).unwrap();
            assert_eq!(syntax.builtin_rule_set(), rule_set);
            let report = validator
                .validate_with(&embedded.xml.data, &options())
                .unwrap();
            assert_eq!(report, direct, "{path} {layout:?}");
        }
    }
}

#[test]
fn pdf_errors_have_their_kinds() {
    let kind = |file: &[u8]| {
        let e = pdf::extract(file).unwrap_err();
        assert!(matches!(e, Error::Pdf(_)), "{e:?}");
        (e.kind(), e.to_string())
    };
    // Not a PDF, malformed, no invoice, several: the file cannot be read as an invoice.
    assert_eq!(kind(b"<Invoice/>").0, ErrorKind::Io);
    assert_eq!(kind(b"%PDF-1.7\n%%EOF\n").0, ErrorKind::Io);
    let none = Fixture {
        name: "invoice.xml",
        xmp: Some(xmp("invoice.xml", "EN 16931")),
        ..Fixture::default()
    }
    .build();
    let (k, message) = kind(&none);
    assert_eq!(k, ErrorKind::Io);
    assert!(message.starts_with("no embedded invoice"), "{message}");
    let two = with_catalog(
        "<< /Type /Catalog /Names << /EmbeddedFiles << /Names [(factur-x.xml) 2 0 R (xrechnung.xml) 3 0 R] >> >> >>",
        &[
            (2, filespec("factur-x.xml", 4).as_bytes()),
            (3, filespec("xrechnung.xml", 5).as_bytes()),
            (4, &embedded(b"<a/>")),
            (5, &embedded(b"<b/>")),
        ],
    );
    let (k, message) = kind(&two);
    assert_eq!(k, ErrorKind::Io);
    assert!(
        message.starts_with("ambiguous embedded invoice"),
        "{message}"
    );
    // Encrypted, beyond the subset read, beyond a limit: unsupported.
    let mut encrypted = Writer::new("1.7");
    encrypted.object(1, b"<< /Type /Catalog >>");
    encrypted.object(2, b"<< /Filter /Standard /V 2 /R 3 >>");
    encrypted.table("/Root 1 0 R /Encrypt 2 0 R");
    assert_eq!(kind(&encrypted.finish()).0, ErrorKind::Unsupported);
    let lzw: &[u8] =
        b"<< /Length 4 /Type /EmbeddedFile /Filter /LZWDecode >>\nstream\nabcd\nendstream";
    let lzw = with_catalog(
        "<< /Type /Catalog /Names << /EmbeddedFiles << /Names [(factur-x.xml) 2 0 R] >> >> >>",
        &[(2, filespec("factur-x.xml", 3).as_bytes()), (3, lzw)],
    );
    let (k, message) = kind(&lzw);
    assert_eq!(k, ErrorKind::Unsupported, "{message}");
    let mut small = Limits::default();
    small.max_stream_size = 16;
    let e = pdf::extract_with(&Fixture::default().build(), &small).unwrap_err();
    assert!(
        matches!(e, Error::Pdf(einvoice_pdf::Error::LimitExceeded { .. })),
        "{e:?}"
    );
    assert_eq!(e.kind(), ErrorKind::Unsupported);
    // The cause is the PDF reader's error.
    let source = std::error::Error::source(&e).unwrap();
    assert_eq!(source.to_string(), e.to_string());
}
