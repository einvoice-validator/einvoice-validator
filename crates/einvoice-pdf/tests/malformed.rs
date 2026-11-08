//! Files outside the supported subset, broken files and hostile files: each is refused
//! with the right error, never read approximately, and quickly.

#![cfg(test)]

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use einvoice_pdf::{Error, Limits, attachments, extract, extract_with};
use einvoice_testkit::pdf::{
    Fixture, INVOICE, Layout, Writer, deflate, embedded, filespec, with_catalog,
};

fn malformed(result: Result<impl std::fmt::Debug, Error>, needle: &str) {
    match result {
        Err(Error::Malformed { message, .. }) => {
            assert!(message.contains(needle), "{message:?} lacks {needle:?}");
        }
        other => panic!("expected a malformed-file error with {needle:?}, got {other:?}"),
    }
}

fn unsupported(result: Result<impl std::fmt::Debug, Error>, needle: &str) {
    match result {
        Err(Error::Unsupported { feature, .. }) => {
            assert!(feature.contains(needle), "{feature:?} lacks {needle:?}");
        }
        other => panic!("expected an unsupported-feature error with {needle:?}, got {other:?}"),
    }
}

fn limit(result: Result<impl std::fmt::Debug, Error>, name: &str) {
    match result {
        Err(Error::LimitExceeded { limit, .. }) => assert_eq!(limit, name),
        other => panic!("expected limit {name}, got {other:?}"),
    }
}

/// `pdf` with every occurrence of `from` replaced by `to` (same length, so that the
/// offsets stay right).
fn patch(pdf: &[u8], from: &str, to: &str) -> Vec<u8> {
    assert_eq!(from.len(), to.len(), "patches keep offsets");
    let (from, to) = (from.as_bytes(), to.as_bytes());
    let mut out = pdf.to_vec();
    let mut found = false;
    let mut i = 0;
    while i + from.len() <= out.len() {
        if &out[i..i + from.len()] == from {
            out[i..i + from.len()].copy_from_slice(to);
            found = true;
            i += from.len();
        } else {
            i += 1;
        }
    }
    assert!(found, "{} not found", String::from_utf8_lossy(from));
    out
}

fn invoice_catalog() -> String {
    "<< /Type /Catalog /AF [5 0 R] >>".to_owned()
}

#[test]
fn not_pdf() {
    assert_eq!(extract(b""), Err(Error::NotPdf));
    assert_eq!(extract(b"hello"), Err(Error::NotPdf));
    assert_eq!(extract(b"%PDF-"), Err(Error::NotPdf));
    assert_eq!(extract(b"%PDF-1.9\n"), Err(Error::NotPdf));
    let pdf = Fixture::default().build();
    let mut shifted = b"junk".to_vec();
    shifted.extend_from_slice(&pdf);
    assert_eq!(extract(&shifted), Err(Error::NotPdf));
    malformed(extract(b"%PDF-1.7\n"), "%%EOF");
    malformed(extract(b"%PDF-1.7\n%%EOF"), "startxref");
    malformed(extract(b"%PDF-1.7\nstartxref\n%%EOF"), "unsigned integer");
    malformed(extract(b"%PDF-1.7\nstartxref\n9 x\n%%EOF"), "%%EOF");
    malformed(
        extract(b"%PDF-1.7\nstartxref\n999\n%%EOF"),
        "beyond the end",
    );
    malformed(extract(b"%PDF-1.7\nstartxref\n3\n%%EOF"), "neither `xref`");
}

#[test]
fn data_after_eof() {
    let mut pdf = Fixture::default().build();
    pdf.extend_from_slice(b"garbage");
    malformed(extract(&pdf), "%%EOF");
}

#[test]
fn encrypted() {
    let mut w = Writer::new("1.7");
    w.object(1, invoice_catalog().as_bytes());
    w.object(
        2,
        b"<< /Filter /Standard /V 2 /R 3 /O <00> /U <00> /P -4 >>",
    );
    w.table("/Root 1 0 R /Encrypt 2 0 R");
    assert_eq!(extract(&w.finish()), Err(Error::Encrypted));
    // In a cross-reference stream dictionary too.
    let mut w = Writer::new("1.7");
    w.object(1, invoice_catalog().as_bytes());
    w.xref_stream(2, "/Root 1 0 R /Encrypt << /Filter /Standard >>", false);
    assert_eq!(extract(&w.finish()), Err(Error::Encrypted));
}

#[test]
fn no_invoice_and_ambiguous() {
    let pdf = with_catalog(
        "<< /Type /Catalog /AF [5 0 R 6 0 R] >>",
        &[
            (4, &embedded(b"x")),
            (5, filespec("invoice.xml", 4).as_bytes()),
            (6, filespec("Factur-X.xml", 4).as_bytes()),
        ],
    );
    assert_eq!(
        extract(&pdf),
        Err(Error::NoInvoice {
            attachments: vec!["invoice.xml".to_owned(), "Factur-X.xml".to_owned()]
        })
    );
    let pdf = with_catalog(
        "<< /Type /Catalog /AF [5 0 R 6 0 R] >>",
        &[
            (4, &embedded(INVOICE)),
            (5, filespec("factur-x.xml", 4).as_bytes()),
            (6, filespec("zugferd-invoice.xml", 4).as_bytes()),
        ],
    );
    assert_eq!(
        extract(&pdf),
        Err(Error::Ambiguous {
            names: vec!["factur-x.xml".to_owned(), "zugferd-invoice.xml".to_owned()]
        })
    );
    // Two different streams under one invoice name are ambiguous too.
    let pdf = with_catalog(
        "<< /Type /Catalog /AF [5 0 R 6 0 R] >>",
        &[
            (4, &embedded(INVOICE)),
            (7, &embedded(b"other")),
            (5, filespec("factur-x.xml", 4).as_bytes()),
            (6, filespec("factur-x.xml", 7).as_bytes()),
        ],
    );
    assert!(matches!(extract(&pdf), Err(Error::Ambiguous { .. })));
}

#[test]
fn catalog_problems() {
    let mut w = Writer::new("1.7");
    w.object(1, invoice_catalog().as_bytes());
    w.table("/Info 1 0 R");
    malformed(extract(&w.finish()), "no /Root");
    malformed(
        extract(&with_catalog("<< /Type /Pages /AF [] >>", &[])),
        "/Type /Catalog",
    );
    malformed(extract(&with_catalog("[1 2 3]", &[])), "not a dictionary");
    malformed(
        extract(&with_catalog("<< /Type /Catalog /AF 7 >>", &[])),
        "not an array",
    );
    unsupported(
        extract(&with_catalog("<< /Type /Catalog /AF [(x)] >>", &[])),
        "file specification string",
    );
    malformed(
        extract(&with_catalog("<< /Type /Catalog /Names 5 >>", &[])),
        "not a dictionary",
    );
    malformed(
        extract(&with_catalog(
            "<< /Type /Catalog /Metadata 4 0 R /AF [5 0 R] >>",
            &[
                (4, b"<< /Type /Metadata >>"),
                (5, filespec("factur-x.xml", 6).as_bytes()),
                (6, &embedded(INVOICE)),
            ],
        )),
        "not a stream",
    );
}

#[test]
fn file_specification_problems() {
    let with_spec = |spec: &str| {
        with_catalog(
            &invoice_catalog(),
            &[(4, &embedded(INVOICE)), (5, spec.as_bytes())],
        )
    };
    malformed(
        extract(&with_spec(
            "<< /Type /Font /F (factur-x.xml) /EF << /F 4 0 R >> >>",
        )),
        "/Filespec",
    );
    malformed(
        extract(&with_spec("<< /F (factur-x.xml) /EF << >> >>")),
        "neither /F nor /UF",
    );
    malformed(
        extract(&with_spec("<< /F (factur-x.xml) /EF << /F 4 >> >>")),
        "indirect reference",
    );
    unsupported(
        extract(&with_spec(
            "<< /F (factur-x.xml) /EF << /F 4 0 R /UF 6 0 R >> >>",
        )),
        "different embedded files",
    );
    malformed(
        extract(&with_spec("<< /F 12 /EF << /F 4 0 R >> >>")),
        "not a string",
    );
    malformed(
        extract(&with_spec(
            "<< /F (factur-x.xml) /AFRelationship (Data) /EF << /F 4 0 R >> >>",
        )),
        "not a name",
    );
    malformed(
        extract(&with_spec("<< /UF <FEFF00> /EF << /F 4 0 R >> >>")),
        "odd number",
    );
    malformed(
        extract(&with_spec("<< /F (\\237) /EF << /F 4 0 R >> >>")),
        "PDFDocEncoding",
    );
    malformed(extract(&with_spec("<< /EF << /F 4 0 R >> >>")), "no name");
    // A name tree entry without /EF.
    let pdf = with_catalog(
        "<< /Type /Catalog /Names << /EmbeddedFiles << /Names [(factur-x.xml) 5 0 R] >> >> >>",
        &[(5, b"<< /Type /Filespec /F (factur-x.xml) >>")],
    );
    malformed(extract(&pdf), "no /EF");
    unsupported(
        extract(&with_catalog(
            "<< /Type /Catalog /Names << /EmbeddedFiles << /Names [(factur-x.xml) (factur-x.xml)] >> >> >>",
            &[],
        )),
        "file specification string",
    );
}

#[test]
fn embedded_file_problems() {
    let with_file = |file: &[u8]| {
        with_catalog(
            &invoice_catalog(),
            &[(4, file), (5, filespec("factur-x.xml", 4).as_bytes())],
        )
    };
    let stream = |dict: &str, data: &[u8]| {
        let mut body = format!("<< /Length {} {dict} >>\nstream\n", data.len()).into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        body
    };
    malformed(extract(&with_file(b"(not a stream)")), "not a stream");
    malformed(
        extract(&with_file(&stream("/Type /XObject", INVOICE))),
        "/EmbeddedFile",
    );
    malformed(
        extract(&with_file(&stream("/Params << /Size 3 >>", INVOICE))),
        "/Size says 3",
    );
    malformed(
        extract(&with_file(&stream(
            "/Params << /CheckSum <00112233445566778899AABBCCDDEEFF> >>",
            INVOICE,
        ))),
        "MD5",
    );
    malformed(
        extract(&with_file(&stream(
            "/Params << /CheckSum (abc) >>",
            INVOICE,
        ))),
        "neither 16 nor 32",
    );
    malformed(
        extract(&with_file(&stream(
            "/Params << /CheckSum (zz112233445566778899aabbccddeeff) >>",
            INVOICE,
        ))),
        "neither 16 nor 32",
    );
    malformed(
        extract(&with_file(&stream("/Params << /Size -1 >>", INVOICE))),
        "negative",
    );
    malformed(
        extract(&with_file(&stream("/Params 7", INVOICE))),
        "not a dictionary",
    );
    malformed(
        extract(&with_file(&stream("/Subtype (text/xml)", INVOICE))),
        "not a name",
    );
    unsupported(
        extract(&with_file(&stream("/Filter /LZWDecode", INVOICE))),
        "/LZWDecode",
    );
    unsupported(
        extract(&with_file(&stream("/Filter /ASCII85Decode", INVOICE))),
        "/ASCII85Decode",
    );
    unsupported(
        extract(&with_file(&stream("/F (external.xml)", b""))),
        "external file",
    );
    malformed(
        extract(&with_file(&stream("/Filter /FlateDecode", INVOICE))),
        "FlateDecode",
    );
    malformed(
        extract(&with_file(&stream("/Filter 3", INVOICE))),
        "not a name or an array",
    );
    malformed(
        extract(&with_file(&stream(
            "/Filter [/FlateDecode /FlateDecode] /DecodeParms << /Predictor 12 >>",
            INVOICE,
        ))),
        "several filters",
    );
    malformed(
        extract(&with_file(&stream(
            "/Filter [/FlateDecode] /DecodeParms [null null]",
            INVOICE,
        ))),
        "different lengths",
    );
    let truncated = deflate(INVOICE);
    malformed(
        extract(&with_file(&stream(
            "/Filter /FlateDecode",
            &truncated[..truncated.len() / 2],
        ))),
        "truncated",
    );
}

#[test]
fn stream_length_problems() {
    let catalog = invoice_catalog();
    let spec = filespec("factur-x.xml", 4);
    let with_body = |body: &[u8], extra: &[(u32, &[u8])]| {
        let mut objects: Vec<(u32, &[u8])> = vec![(4, body), (5, spec.as_bytes())];
        objects.extend_from_slice(extra);
        with_catalog(&catalog, &objects)
    };
    // A /Length one byte short, or two bytes long: no `endstream` where the data should
    // end. (One byte long takes the end-of-line marker into the data: indistinguishable
    // from a stream without the marker.)
    for wrong in [INVOICE.len() - 1, INVOICE.len() + 2] {
        let mut body = format!("<< /Length {wrong} >>\nstream\n").into_bytes();
        body.extend_from_slice(INVOICE);
        body.extend_from_slice(b"\nendstream");
        malformed(extract(&with_body(&body, &[])), "endstream");
    }
    let mut body = b"<< /Length 99999999 >>\nstream\n".to_vec();
    body.extend_from_slice(INVOICE);
    body.extend_from_slice(b"\nendstream");
    malformed(extract(&with_body(&body, &[])), "beyond the end");
    let mut body = b"<< >>\nstream\n".to_vec();
    body.extend_from_slice(INVOICE);
    body.extend_from_slice(b"\nendstream");
    malformed(extract(&with_body(&body, &[])), "without /Length");
    // An indirect /Length that is not an integer, or that points to the stream itself.
    let mut body = b"<< /Length 6 0 R >>\nstream\n".to_vec();
    body.extend_from_slice(INVOICE);
    body.extend_from_slice(b"\nendstream");
    malformed(
        extract(&with_body(&body, &[(6, b"(12)")])),
        "not an integer",
    );
    let mut body = b"<< /Length 4 0 R >>\nstream\n".to_vec();
    body.extend_from_slice(INVOICE);
    body.extend_from_slice(b"\nendstream");
    malformed(extract(&with_body(&body, &[])), "depends on itself");
    // `stream` followed by CR alone.
    let mut body = format!("<< /Length {} >>\nstream\r", INVOICE.len()).into_bytes();
    body.extend_from_slice(INVOICE);
    body.extend_from_slice(b"\nendstream");
    malformed(extract(&with_body(&body, &[])), "CRLF or LF");
}

#[test]
fn reference_problems() {
    // A chain of references that loops.
    let pdf = with_catalog(
        "<< /Type /Catalog /AF 6 0 R >>",
        &[(6, b"7 0 R"), (7, b"6 0 R")],
    );
    malformed(extract(&pdf), "loops");
    // A long chain of references.
    let objects: Vec<(u32, Vec<u8>)> = (6..60)
        .map(|n| (n, format!("{} 0 R", n + 1).into_bytes()))
        .chain(std::iter::once((60, b"[]".to_vec())))
        .collect();
    let extra: Vec<(u32, &[u8])> = objects.iter().map(|(n, b)| (*n, b.as_slice())).collect();
    let pdf = with_catalog("<< /Type /Catalog /AF 6 0 R >>", &extra);
    limit(extract(&pdf), "max_resolution_depth");
    // A cross-reference entry pointing at the wrong object.
    let pdf = with_catalog(
        &invoice_catalog(),
        &[
            (4, &embedded(INVOICE)),
            (5, filespec("factur-x.xml", 4).as_bytes()),
        ],
    );
    let swapped = patch(&pdf, "5 0 obj", "4 0 obj");
    malformed(extract(&swapped), "points to object 4 0");
    // An object missing `endobj`.
    let pdf = with_catalog(
        &invoice_catalog(),
        &[
            (4, &embedded(INVOICE)),
            (5, filespec("factur-x.xml", 4).as_bytes()),
        ],
    );
    let broken = patch(&pdf, "endstream\nendobj\n5", "endstream\nendobx\n5");
    malformed(extract(&broken), "expected `endobj`");
}

#[test]
fn xref_table_problems() {
    let pdf = with_catalog(
        &invoice_catalog(),
        &[
            (4, &embedded(INVOICE)),
            (5, filespec("factur-x.xml", 4).as_bytes()),
        ],
    );
    malformed(
        extract(&patch(&pdf, " n \n", " x \n")),
        "neither `n` nor `f`",
    );
    malformed(
        extract(&patch(&pdf, "trailer\n<<", "trailer\n[ ")),
        "expected",
    );
    malformed(
        extract(&patch(&pdf, "trailer", "trailex")),
        "unsigned integer",
    );
    // A subsection claiming far more entries than the file holds stops at the end.
    let start = Instant::now();
    let huge = patch(&pdf, "xref\n0 6", "xref\n0 9");
    assert!(extract(&huge).is_err());
    let text = String::from_utf8_lossy(&pdf).into_owned();
    let count = text
        .split("xref\n0 ")
        .nth(1)
        .unwrap()
        .split('\n')
        .next()
        .unwrap()
        .to_owned();
    let replaced = text.replacen(&format!("xref\n0 {count}\n"), "xref\n0 4294967295\n", 1);
    let start_huge = Instant::now();
    assert!(extract(replaced.as_bytes()).is_err());
    assert!(start_huge.elapsed() < Duration::from_secs(1));
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn prev_chain_problems() {
    let pdf = Fixture::default().build();
    let at = pdf.windows(9).rposition(|w| w == b"startxref").unwrap();
    let offset: usize = std::str::from_utf8(&pdf[at + 10..])
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    // An update whose /Prev is itself.
    let mut looped = pdf.clone();
    let here = looped.len();
    looped.extend_from_slice(
        format!(
            "xref\n0 0\ntrailer\n<< /Size 7 /Root 1 0 R /Prev {here} >>\nstartxref\n{here}\n%%EOF\n"
        )
        .as_bytes(),
    );
    malformed(extract(&looped), "loops");
    // Two updates pointing at each other (offsets written with ten digits, so that the
    // lengths are known in advance).
    let mut pair = pdf.clone();
    let first = pair.len();
    let first_len = "xref\n0 0\ntrailer\n<< /Size 7 /Root 1 0 R /Prev 0000000000 >>\n".len();
    let second = first + first_len;
    pair.extend_from_slice(
        format!("xref\n0 0\ntrailer\n<< /Size 7 /Root 1 0 R /Prev {second:010} >>\n").as_bytes(),
    );
    pair.extend_from_slice(
        format!("xref\n0 0\ntrailer\n<< /Size 7 /Root 1 0 R /Prev {first:010} >>\nstartxref\n{second}\n%%EOF\n")
            .as_bytes(),
    );
    malformed(extract(&pair), "loops");
    // /Prev beyond the file, not an integer, negative.
    for prev in ["99999999", "(1)", "-5"] {
        let mut bad = pdf.clone();
        let here = bad.len();
        bad.extend_from_slice(
            format!("xref\n0 0\ntrailer\n<< /Size 7 /Root 1 0 R /Prev {prev} >>\nstartxref\n{here}\n%%EOF\n")
                .as_bytes(),
        );
        assert!(
            matches!(extract(&bad), Err(Error::Malformed { .. })),
            "/Prev {prev}"
        );
    }
    // Many sections.
    let mut many = pdf.clone();
    let mut prev = offset;
    for _ in 0..40 {
        let here = many.len();
        many.extend_from_slice(
            format!("xref\n0 0\ntrailer\n<< /Size 7 /Root 1 0 R /Prev {prev} >>\nstartxref\n{here}\n%%EOF\n")
                .as_bytes(),
        );
        prev = here;
    }
    assert_eq!(extract(&many).unwrap().xml.data, INVOICE);
    let mut limits = Limits::default();
    limits.max_xref_sections = 10;
    limit(extract_with(&many, &limits), "max_xref_sections");
    // A section nested inside another's trailer (to have it parsed twice).
    let mut nested = pdf.clone();
    let outer = nested.len();
    let inner = outer + "xref\n0 0\ntrailer\n<< /Size 7 /Root 1 0 R /X (".len();
    nested.extend_from_slice(
        format!(
            "xref\n0 0\ntrailer\n<< /Size 7 /Root 1 0 R /X (xref 0 0 trailer << /Prev {offset} >>) /Prev {inner} >>\nstartxref\n{outer}\n%%EOF\n"
        )
        .as_bytes(),
    );
    malformed(extract(&nested), "overlaps");
}

#[test]
fn xref_stream_problems() {
    let build = |dict: &str, data: &[u8]| {
        let mut w = Writer::new("1.7");
        w.object(1, invoice_catalog().as_bytes());
        let offset = w.out.len();
        w.stream(2, dict, data);
        w.raw(format!("startxref\n{offset}\n%%EOF\n").as_bytes());
        w.finish()
    };
    let rows = [0u8, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    malformed(
        extract(&build("/Type /XRef /Size 3 /W [0 0 0]", b"")),
        "all zero",
    );
    malformed(
        extract(&build("/Type /XRef /Size 3 /W [1 9 1]", &rows)),
        "not 0 to 8",
    );
    malformed(
        extract(&build("/Type /XRef /Size 3 /W [1 2]", &rows)),
        "three",
    );
    malformed(
        extract(&build(
            "/Type /XRef /Size 3 /W [1 4 1] /Index [0 3 5]",
            &rows,
        )),
        "odd number",
    );
    malformed(
        extract(&build("/Type /XRef /Size 4000000000 /W [1 4 1]", &rows)),
        "rows",
    );
    malformed(
        extract(&build(
            "/Type /XRef /W [1 4 1] /Size 3 /Index [0 3000000000]",
            &rows,
        )),
        "rows",
    );
    malformed(
        extract(&build("/Type /XRef /W [1 4 1] /Index [0 3]", &rows)),
        "no /Size",
    );
    malformed(
        extract(&build("/Type /ObjStm /Size 3 /W [1 4 1]", &rows)),
        "/XRef",
    );
    let mut bad_type = rows;
    bad_type[6] = 7;
    unsupported(
        extract(&build("/Type /XRef /Size 3 /W [1 4 1]", &bad_type)),
        "entry type 7",
    );
    malformed(
        extract(&build(
            "/Type /XRef /Size 3 /W [1 4 1] /Filter 4 0 R",
            &rows,
        )),
        "indirect reference",
    );
    unsupported(
        extract(&build(
            "/Type /XRef /Size 3 /W [1 4 1] /Filter /FlateDecode /DecodeParms << /Predictor 2 /Columns 6 >>",
            &deflate(&rows),
        )),
        "TIFF predictor",
    );
    // Indirect /Length of a cross-reference stream.
    let mut w = Writer::new("1.7");
    let offset = w.out.len();
    w.raw(b"2 0 obj\n<< /Type /XRef /Size 3 /W [1 4 1] /Length 3 0 R >>\nstream\n");
    w.raw(&rows);
    w.raw(format!("\nendstream\nendobj\nstartxref\n{offset}\n%%EOF\n").as_bytes());
    malformed(extract(&w.finish()), "direct integer");
}

#[test]
fn object_stream_problems() {
    // An object stream whose /Length is an object it contains.
    let mut w = Writer::new("1.7");
    let data = "1 0 2 2 99 9";
    w.object(
        3,
        format!("<< /Type /ObjStm /N 2 /First 8 /Length 2 0 R >>\nstream\n{data}\nendstream")
            .as_bytes(),
    );
    w.compressed_for_tests(&[(1, 3, 0), (2, 3, 1)]);
    w.xref_stream(4, "/Root 1 0 R", false);
    malformed(extract(&w.finish()), "depends on itself");

    let objstm = |dict: &str, data: &str, entries: &[(u32, u32, u32)]| {
        let mut w = Writer::new("1.7");
        w.stream(3, dict, data.as_bytes());
        w.compressed_for_tests(entries);
        w.xref_stream(4, "/Root 1 0 R", false);
        w.finish()
    };
    let catalog = "<< /Type /Catalog >>";
    let good = format!("1 0 {catalog}");
    assert_eq!(
        extract(&objstm("/Type /ObjStm /N 1 /First 4", &good, &[(1, 3, 0)])),
        Err(Error::NoInvoice {
            attachments: Vec::new()
        })
    );
    malformed(
        extract(&objstm("/Type /XObject /N 1 /First 4", &good, &[(1, 3, 0)])),
        "/Type /ObjStm",
    );
    malformed(
        extract(&objstm("/Type /ObjStm /First 4", &good, &[(1, 3, 0)])),
        "no /N",
    );
    malformed(
        extract(&objstm(
            "/Type /ObjStm /N 1 /First 400",
            &good,
            &[(1, 3, 0)],
        )),
        "/First beyond",
    );
    malformed(
        extract(&objstm("/Type /ObjStm /N 1 /First 4", &good, &[(1, 3, 1)])),
        "index 1 beyond",
    );
    malformed(
        extract(&objstm(
            "/Type /ObjStm /N 1 /First 4",
            &format!("7 0 {catalog}"),
            &[(1, 3, 0)],
        )),
        "holds object 7",
    );
    malformed(
        extract(&objstm(
            "/Type /ObjStm /N 1000000 /First 4",
            &good,
            &[(1, 3, 0)],
        )),
        "too large",
    );
    malformed(
        extract(&objstm(
            "/Type /ObjStm /N 1 /First 5",
            &format!("1 99 {catalog}"),
            &[(1, 3, 0)],
        )),
        "beyond the decoded data",
    );
    // Two objects at one offset.
    malformed(
        extract(&objstm(
            "/Type /ObjStm /N 2 /First 8",
            &format!("1 0 2 0 {catalog}"),
            &[(1, 3, 0), (2, 3, 1)],
        )),
        "two objects at offset 0",
    );
    // An object stream in an object stream.
    malformed(
        extract(&objstm(
            "/Type /ObjStm /N 1 /First 4",
            &good,
            &[(1, 5, 0), (5, 3, 0)],
        )),
        "itself in an object stream",
    );
    malformed(
        extract(&objstm("/Type /ObjStm /N 1 /First 4", &good, &[(1, 9, 0)])),
        "object stream not found",
    );
    // Each object is parsed within its own bytes, up to the next object: here object 2
    // starts inside the catalog, which is therefore cut short.
    let data = "1 0 2 5 << /Type /Catalog >>";
    malformed(
        extract(&objstm(
            "/Type /ObjStm /N 2 /First 8",
            data,
            &[(1, 3, 0), (2, 3, 1)],
        )),
        "unexpected end of data",
    );
}

#[test]
fn name_tree_problems() {
    let tree = |root: &str, extra: &[(u32, &[u8])]| {
        let mut objects: Vec<(u32, &[u8])> = vec![(10, root.as_bytes())];
        objects.extend_from_slice(extra);
        with_catalog(
            "<< /Type /Catalog /Names << /EmbeddedFiles 10 0 R >> >>",
            &objects,
        )
    };
    let spec = filespec("factur-x.xml", 4);
    let file = embedded(INVOICE);
    malformed(
        extract(&tree("<< /Kids [10 0 R] >>", &[])),
        "cycle or a shared node",
    );
    malformed(
        extract(&tree(
            "<< /Kids [11 0 R 11 0 R] >>",
            &[(11, b"<< /Names [] >>")],
        )),
        "cycle or a shared node",
    );
    malformed(
        extract(&tree("<< /Kids [] /Names [] >>", &[])),
        "both /Kids and /Names",
    );
    malformed(extract(&tree("<< /Kids 5 >>", &[])), "not an array");
    malformed(extract(&tree("<< /Names [(a)] >>", &[])), "odd number");
    malformed(
        extract(&tree("<< /Names [/a 5 0 R] >>", &[])),
        "not a string",
    );
    malformed(
        extract(&tree(
            "<< /Kids [11 0 R] >>",
            &[
                (11, b"<< /Limits [(a) (b)] /Names [(factur-x.xml) 5 0 R] >>"),
                (5, spec.as_bytes()),
                (4, &file),
            ],
        )),
        "outside the node's /Limits",
    );
    malformed(
        extract(&tree("<< /Limits [(b) (a)] /Names [] >>", &[])),
        "decreasing",
    );
    malformed(
        extract(&tree("<< /Limits [(a)] /Names [] >>", &[])),
        "two strings",
    );
    malformed(extract(&tree("(x)", &[])), "not a dictionary");
    // A deep chain of kids.
    let objects: Vec<(u32, Vec<u8>)> = (10..60)
        .map(|n| (n, format!("<< /Kids [{} 0 R] >>", n + 1).into_bytes()))
        .chain(std::iter::once((60, b"<< /Names [] >>".to_vec())))
        .collect();
    let extra: Vec<(u32, &[u8])> = objects.iter().map(|(n, b)| (*n, b.as_slice())).collect();
    let pdf = with_catalog(
        "<< /Type /Catalog /Names << /EmbeddedFiles 10 0 R >> >>",
        &extra,
    );
    limit(extract(&pdf), "max_name_tree_depth");
}

#[test]
fn nesting_and_size_limits() {
    let deep = format!(
        "<< /Type /Catalog /X {}{} >>",
        "[".repeat(100),
        "]".repeat(100)
    );
    limit(extract(&with_catalog(&deep, &[])), "max_depth");
    let very_deep = format!("<< /Type /Catalog /X {} >>", "[".repeat(1_000_000));
    let start = Instant::now();
    limit(extract(&with_catalog(&very_deep, &[])), "max_depth");
    assert!(start.elapsed() < Duration::from_secs(1));

    // A Flate bomb: 16 MiB of zeros in about 16 KiB.
    let zeros = vec![0u8; 16 << 20];
    let bomb = deflate(&zeros);
    drop(zeros);
    let mut body = format!(
        "<< /Length {} /Filter /FlateDecode >>\nstream\n",
        bomb.len()
    )
    .into_bytes();
    body.extend_from_slice(&bomb);
    body.extend_from_slice(b"\nendstream");
    let pdf = with_catalog(
        &invoice_catalog(),
        &[(4, &body), (5, filespec("factur-x.xml", 4).as_bytes())],
    );
    let mut limits = Limits::default();
    limits.max_stream_size = 1 << 20;
    let start = Instant::now();
    limit(extract_with(&pdf, &limits), "max_stream_size");
    assert!(start.elapsed() < Duration::from_secs(1));

    // Many attachments.
    let names = (0..50).fold(String::new(), |mut out, i| {
        write!(
            out,
            "(f{i:03}.txt) << /F (f{i:03}.txt) /EF << /F 4 0 R >> >> "
        )
        .unwrap();
        out
    });
    let pdf = with_catalog(
        &format!("<< /Type /Catalog /Names << /EmbeddedFiles << /Names [{names}] >> >> >>"),
        &[(4, &embedded(b"x"))],
    );
    assert_eq!(attachments(&pdf).unwrap().len(), 50);
    let mut limits = Limits::default();
    limits.max_attachments = 20;
    limit(
        einvoice_pdf::attachments_with(&pdf, &limits),
        "max_attachments",
    );
}

#[test]
fn overlapping_objects() {
    // Object 5's cross-reference offset points inside object 4, a string, and object 5
    // refers to object 4: the same bytes would be parsed for both.
    let mut w = Writer::new("1.7");
    w.object(1, invoice_catalog().as_bytes());
    let four = w.out.len();
    let inner = "5 0 obj << /F (factur-x.xml) /Desc 4 0 R /EF << /F 6 0 R >> >> endobj";
    w.object(4, format!("({inner})").as_bytes());
    w.object(6, &embedded(INVOICE));
    let pdf = w.finish_with_table_override("/Root 1 0 R", &[(5, four + "4 0 obj\n(".len())]);
    malformed(extract(&pdf), "overlaps");
}

#[test]
fn xmp_problems() {
    let bad = |xmp: &str| {
        Fixture {
            xmp: Some(xmp.to_owned()),
            ..Fixture::default()
        }
        .build()
    };
    assert!(matches!(
        extract(&bad("<x:xmpmeta")),
        Err(Error::Metadata(_))
    ));
    assert!(matches!(
        extract(&bad(
            "<?xml version=\"1.0\"?><!DOCTYPE x [<!ENTITY a \"b\">]><x>&a;</x>"
        )),
        Err(Error::Metadata(_))
    ));
    let mut pdf = Fixture::default().build();
    // Latin-1 bytes in the packet: not UTF-8.
    let at = pdf.windows(9).position(|w| w == b"INVOICE</").unwrap();
    pdf[at] = 0xC9;
    assert!(matches!(extract(&pdf), Err(Error::Metadata(_))));
    let metadata = with_catalog(
        "<< /Type /Catalog /Metadata 4 0 R /AF [5 0 R] >>",
        &[
            (
                4,
                b"<< /Length 4 /Subtype /Text >>\nstream\n<a/>\nendstream",
            ),
            (5, filespec("factur-x.xml", 6).as_bytes()),
            (6, &embedded(INVOICE)),
        ],
    );
    malformed(extract(&metadata), "/Subtype other than /XML");
}

#[test]
fn every_single_byte_edit() {
    // Deleting any one byte, or overwriting it with a delimiter, a digit or a byte
    // outside ASCII, never panics; every result is an error or an invoice.
    for layout in [Layout::Table, Layout::ObjectStreams, Layout::Hybrid] {
        let pdf = Fixture {
            layout,
            compress: false,
            ..Fixture::default()
        }
        .build();
        let start = Instant::now();
        let mut errors = 0;
        for i in 0..pdf.len() {
            let mut deleted = pdf.clone();
            deleted.remove(i);
            errors += usize::from(extract(&deleted).is_err());
            for byte in [b'0', b'(', b'<', b'[', b'/', 0xFF] {
                let mut edited = pdf.clone();
                edited[i] = byte;
                if let Ok(embedded) = extract(&edited) {
                    assert_ne!(embedded.xml.data.len(), 0);
                }
            }
        }
        // Most deletions shift the offsets after them.
        assert!(errors > pdf.len() / 2, "{layout:?}: {errors} errors");
        assert!(start.elapsed() < Duration::from_secs(60), "{layout:?}");
    }
}

#[test]
fn every_truncation_is_refused() {
    // Each prefix of a valid file cut before its last cross-reference section, then
    // ended by its own startxref (which points to where that section was, or beyond),
    // is an error, never a panic.
    for layout in [Layout::Table, Layout::XrefStreamPredictor, Layout::Hybrid] {
        let pdf = Fixture {
            layout,
            ..Fixture::default()
        }
        .build();
        let at = pdf.windows(9).rposition(|w| w == b"startxref").unwrap();
        let tail = pdf[at..].to_vec();
        let section: usize = std::str::from_utf8(&tail[10..])
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        for len in 0..section {
            let mut cut = pdf[..len].to_vec();
            cut.push(b'\n');
            cut.extend_from_slice(&tail);
            assert!(extract(&cut).is_err(), "{layout:?} cut at {len}");
        }
    }
}
