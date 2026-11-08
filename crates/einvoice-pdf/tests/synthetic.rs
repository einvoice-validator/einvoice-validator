//! Extraction from synthetic PDF files covering the supported subset: classic tables,
//! cross-reference streams with and without predictor, object streams, hybrid files,
//! incremental updates, name trees with kids and limits, `/AF`, text string encodings,
//! and the Factur-X / `ZUGFeRD` / Order-X names and XMP properties.

#![cfg(test)]

use einvoice_pdf::{AfRelationship, Error, INVOICE_FILE_NAMES, Limits, attachments, extract};
use einvoice_testkit::pdf::{
    Fixture, INVOICE, INVOICE_MD5, LAYOUTS, Layout, Writer, deflate, embedded, filespec, literal,
    with_catalog, xmp,
};

fn md5_bytes() -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&INVOICE_MD5[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

#[test]
fn every_layout() {
    for layout in LAYOUTS {
        let pdf = Fixture {
            layout,
            ..Fixture::default()
        }
        .build();
        let embedded = extract(&pdf).unwrap_or_else(|e| panic!("{layout:?}: {e}"));
        assert_eq!(embedded.xml.data, INVOICE, "{layout:?}");
        assert_eq!(embedded.xml.file_name, "factur-x.xml");
        assert_eq!(embedded.xml.key.as_deref(), Some("factur-x.xml"));
        assert!(embedded.xml.in_associated_files);
        assert_eq!(embedded.xml.relationship, Some(AfRelationship::Data));
        assert_eq!(embedded.xml.description.as_deref(), Some("Invoice"));
        assert_eq!(embedded.xml.mime_type.as_deref(), Some("text/xml"));
        assert_eq!(
            embedded.xml.size,
            Some(u64::try_from(INVOICE.len()).unwrap())
        );
        assert_eq!(embedded.xml.checksum, Some(md5_bytes()));
        assert_eq!(
            embedded.xml.modification_date.as_deref(),
            Some("D:20260101000000+00'00'")
        );
        assert_eq!(embedded.xml.creation_date, None);
        assert_eq!(embedded.pdf_version, "1.7");
        let metadata = embedded.metadata.unwrap();
        assert_eq!(metadata.xmp, xmp("factur-x.xml", "EN 16931"));
        assert_eq!(metadata.pdfa_part.as_deref(), Some("3"));
        assert_eq!(metadata.pdfa_conformance.as_deref(), Some("B"));
        let invoice = metadata.invoice.unwrap();
        assert_eq!(
            invoice.namespace,
            "urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#"
        );
        assert_eq!(invoice.document_type.as_deref(), Some("INVOICE"));
        assert_eq!(invoice.document_file_name.as_deref(), Some("factur-x.xml"));
        assert_eq!(invoice.version.as_deref(), Some("1.0"));
        assert_eq!(invoice.conformance_level.as_deref(), Some("EN 16931"));

        let all = attachments(&pdf).unwrap();
        assert_eq!(all.len(), 1, "{layout:?}");
        assert_eq!(all[0].data, INVOICE);
    }
}

#[test]
fn every_invoice_name() {
    for name in INVOICE_FILE_NAMES {
        for layout in [Layout::Table, Layout::ObjectStreams] {
            let pdf = Fixture {
                layout,
                name,
                xmp: Some(xmp(name, "BASIC")),
                ..Fixture::default()
            }
            .build();
            let embedded = extract(&pdf).unwrap();
            assert_eq!(embedded.xml.file_name, name);
            assert_eq!(embedded.xml.data, INVOICE);
        }
    }
}

#[test]
fn uncompressed_and_without_checksum_or_metadata() {
    let pdf = Fixture {
        compress: false,
        checksum: None,
        xmp: None,
        ..Fixture::default()
    }
    .build();
    let embedded = extract(&pdf).unwrap();
    assert_eq!(embedded.xml.data, INVOICE);
    assert_eq!(embedded.xml.checksum, None);
    assert!(embedded.metadata.is_none());
}

#[test]
fn checksum_as_hexadecimal_text() {
    // Some producers write the 32 hexadecimal digits of the MD5 as a literal string.
    let pdf = Fixture {
        checksum: Some(format!("({INVOICE_MD5})")),
        ..Fixture::default()
    }
    .build();
    assert_eq!(extract(&pdf).unwrap().xml.checksum, Some(md5_bytes()));
    let upper = Fixture {
        checksum: Some(format!("({})", INVOICE_MD5.to_uppercase())),
        ..Fixture::default()
    }
    .build();
    assert_eq!(extract(&upper).unwrap().xml.checksum, Some(md5_bytes()));
}

#[test]
fn relationships() {
    for (name, expected) in [
        ("/Source", AfRelationship::Source),
        ("/Data", AfRelationship::Data),
        ("/Alternative", AfRelationship::Alternative),
        ("/Supplement", AfRelationship::Supplement),
        ("/Unspecified", AfRelationship::Unspecified),
        ("/EncryptedPayload", AfRelationship::EncryptedPayload),
        ("/FormData", AfRelationship::FormData),
        ("/Schema", AfRelationship::Schema),
        ("/My#20Own", AfRelationship::Other("My Own".to_owned())),
    ] {
        let pdf = Fixture {
            relationship: name,
            ..Fixture::default()
        }
        .build();
        let got = extract(&pdf).unwrap().xml.relationship.unwrap();
        assert_eq!(got, expected);
        assert_eq!(format!("/{got}").replace(' ', "#20"), name);
    }
}

#[test]
fn listed_only_in_af_or_only_in_names() {
    let af_only = Fixture {
        in_names: false,
        ..Fixture::default()
    }
    .build();
    let embedded = extract(&af_only).unwrap();
    assert!(embedded.xml.in_associated_files);
    assert_eq!(embedded.xml.key, None);
    let names_only = Fixture {
        in_af: false,
        ..Fixture::default()
    }
    .build();
    let embedded = extract(&names_only).unwrap();
    assert!(!embedded.xml.in_associated_files);
    assert_eq!(embedded.xml.key.as_deref(), Some("factur-x.xml"));
    let neither = Fixture {
        in_af: false,
        in_names: false,
        ..Fixture::default()
    }
    .build();
    assert_eq!(
        extract(&neither),
        Err(Error::NoInvoice {
            attachments: Vec::new()
        })
    );
    assert_eq!(attachments(&neither).unwrap(), Vec::new());
}

#[test]
fn name_tree_with_kids_and_limits() {
    // Root with two kids; the second kid has two leaves. Keys in byte order.
    let catalog = "<< /Type /Catalog /Names << /EmbeddedFiles 10 0 R >> /AF [21 0 R] >>";
    let objects: Vec<(u32, Vec<u8>)> = vec![
        (10, b"<< /Kids [11 0 R 12 0 R] >>".to_vec()),
        (
            11,
            b"<< /Limits [(a.txt) (b.txt)] /Names [(a.txt) 20 0 R (b.txt) 22 0 R] >>".to_vec(),
        ),
        (
            12,
            b"<< /Limits [(c.png) (z.xml)] /Kids [13 0 R 14 0 R] >>".to_vec(),
        ),
        (
            13,
            b"<< /Limits [(c.png) (c.png)] /Names [(c.png) 24 0 R] >>".to_vec(),
        ),
        (
            14,
            b"<< /Limits [(factur-x.xml) (z.xml)] /Names [(factur-x.xml) 21 0 R (z.xml) 26 0 R] >>"
                .to_vec(),
        ),
        (20, filespec("a.txt", 30).into_bytes()),
        (21, filespec("factur-x.xml", 31).into_bytes()),
        (22, filespec("b.txt", 32).into_bytes()),
        (24, filespec("c.png", 34).into_bytes()),
        (26, filespec("z.xml", 36).into_bytes()),
        (30, embedded(b"a")),
        (31, embedded(INVOICE)),
        (32, embedded(b"bb")),
        (34, embedded(b"\x89PNG")),
        (36, embedded(b"<z/>")),
    ];
    let extra: Vec<(u32, &[u8])> = objects.iter().map(|(n, b)| (*n, b.as_slice())).collect();
    let pdf = with_catalog(catalog, &extra);
    let embedded = extract(&pdf).unwrap();
    assert_eq!(embedded.xml.data, INVOICE);
    assert!(embedded.xml.in_associated_files);
    let all = attachments(&pdf).unwrap();
    let names: Vec<&str> = all.iter().map(|a| a.file_name.as_str()).collect();
    assert_eq!(names, ["a.txt", "b.txt", "c.png", "factur-x.xml", "z.xml"]);
    assert_eq!(all[1].data, b"bb");
    assert_eq!(
        all.iter().filter(|a| a.in_associated_files).count(),
        1,
        "only factur-x.xml is in /AF"
    );
}

#[test]
fn unicode_file_names_and_af_only_files() {
    // /UF in UTF-16BE, /F in PDFDocEncoding; a second file only in /AF, without /EF
    // (an external associated file, skipped).
    let catalog = "<< /Type /Catalog /AF [5 0 R 6 0 R] >>";
    let spec = "<< /Type /Filespec /F (factur-x.xml) /UF <FEFF0066006100630074007500720020002D0078002E0078006D006C> /EF << /UF 4 0 R >> >>";
    let pdf = with_catalog(
        catalog,
        &[
            (4, &embedded(INVOICE)),
            (5, spec.as_bytes()),
            (6, b"<< /Type /Filespec /F (external.xml) >>"),
        ],
    );
    // /UF wins over /F: "factur -x.xml" is not an invoice name.
    assert_eq!(
        extract(&pdf),
        Err(Error::NoInvoice {
            attachments: vec!["factur -x.xml".to_owned()]
        })
    );
    let spec = "<< /Type /Filespec /UF <FEFF0066006100630074007500720020002D0078002E0078006D006C> /F (\\344) /EF << /UF 4 0 R >> >>";
    let pdf = with_catalog(
        "<< /Type /Catalog /AF [5 0 R] >>",
        &[(4, &embedded(INVOICE)), (5, spec.as_bytes())],
    );
    assert_eq!(attachments(&pdf).unwrap()[0].file_name, "factur -x.xml");
    // A name from the name tree key when the file specification has neither /UF nor /F.
    let pdf = with_catalog(
        "<< /Type /Catalog /Names << /EmbeddedFiles << /Names [(order-x.xml) 5 0 R] >> >> >>",
        &[
            (4, &embedded(INVOICE)),
            (5, b"<< /Type /Filespec /EF << /F 4 0 R >> >>"),
        ],
    );
    assert_eq!(extract(&pdf).unwrap().xml.file_name, "order-x.xml");
}

#[test]
fn incremental_update_replaces_the_invoice() {
    let base = Fixture::default().build();
    // Continue the base file: its last startxref is the /Prev.
    let prev = last_startxref(&base);
    let mut update = Writer::new("1.7");
    update.out = base;
    let replacement = b"<invoice>replaced</invoice>";
    update.prev_from(prev, 11);
    update.stream(
        4,
        "/Type /EmbeddedFile /Subtype /text#2Fxml /Filter /FlateDecode",
        &deflate(replacement),
    );
    update.table("/Root 1 0 R");
    let pdf = update.finish();
    let embedded = extract(&pdf).unwrap();
    assert_eq!(embedded.xml.data, replacement);
    // The metadata of the base file is still found through the /Prev chain.
    assert!(embedded.metadata.is_some());
}

#[test]
fn incremental_update_as_xref_stream_over_a_table() {
    let base = Fixture::default().build();
    let prev = last_startxref(&base);
    let mut update = Writer::new("1.7");
    update.out = base;
    update.prev_from(prev, 11);
    // A new catalog in an object stream, naming the file zugferd-invoice.xml.
    update.object_stream(
        12,
        &[
            (
                1,
                "<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles << /Names [(zugferd-invoice.xml) 13 0 R] >> >> >>",
            ),
            (
                13,
                "<< /Type /Filespec /F (zugferd-invoice.xml) /EF << /F 4 0 R >> /AFRelationship /Alternative >>",
            ),
        ],
        true,
    );
    update.xref_stream(14, "/Root 1 0 R", true);
    let pdf = update.finish();
    let embedded = extract(&pdf).unwrap();
    assert_eq!(embedded.xml.file_name, "zugferd-invoice.xml");
    assert_eq!(embedded.xml.data, INVOICE);
    assert_eq!(embedded.xml.relationship, Some(AfRelationship::Alternative));
    assert!(!embedded.xml.in_associated_files);
}

#[test]
fn incremental_update_deleting_the_invoice() {
    // The update frees object 5 (the file specification) and drops it from the catalog.
    let base = Fixture::default().build();
    let prev = last_startxref(&base);
    let mut update = Writer::new("1.7");
    update.out = base;
    update.prev_from(prev, 11);
    update.object(1, b"<< /Type /Catalog /Pages 2 0 R >>");
    let offset = update.out.len();
    let table = format!(
        "xref\n1 1\n{:010} 00000 n \n5 1\n0000000000 00001 f \ntrailer\n<< /Size 11 /Root 1 0 R /Prev {prev} >>\nstartxref\n{offset}\n%%EOF\n",
        update.pending_offset(1)
    );
    update.raw(table.as_bytes());
    let pdf = update.finish();
    assert_eq!(
        extract(&pdf),
        Err(Error::NoInvoice {
            attachments: Vec::new()
        })
    );
    // The old catalog is still in the file but no longer reachable; a reference to a
    // freed object is null.
    let with_dangling = {
        let mut w = Writer::new("1.7");
        w.object(1, b"<< /Type /Catalog /AF [9 0 R] >>");
        w.table("/Root 1 0 R");
        w.finish()
    };
    assert!(extract(&with_dangling).is_err());
}

#[test]
fn generation_numbers() {
    let build = |af: &str| {
        let mut w = Writer::new("1.7");
        w.object(1, format!("<< /Type /Catalog /AF [{af}] >>").as_bytes());
        w.object(4, &embedded(INVOICE));
        w.object_gen(5, 3, filespec("factur-x.xml", 4).as_bytes());
        w.table("/Root 1 0 R");
        w.finish()
    };
    assert_eq!(extract(&build("5 3 R")).unwrap().xml.data, INVOICE);
    // Another generation number: a reference to an object that does not exist (null),
    // which is not a file specification.
    let error = extract(&build("5 0 R")).unwrap_err();
    assert!(
        matches!(&error, Error::Malformed { message, .. } if message.contains("null")),
        "{error}"
    );
}

#[test]
fn zugferd_1_metadata() {
    let packet = "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\
        <x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
        <rdf:Description rdf:about=\"\" xmlns:zf=\"urn:ferd:pdfa:CrossIndustryDocument:invoice:1p0#\" \
        zf:DocumentType=\"INVOICE\" zf:DocumentFileName=\"ZUGFeRD-invoice.xml\" zf:Version=\"1.0\" zf:ConformanceLevel=\"COMFORT\"/>\
        </rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>";
    let pdf = Fixture {
        name: "ZUGFeRD-invoice.xml",
        xmp: Some(packet.to_owned()),
        ..Fixture::default()
    }
    .build();
    let embedded = extract(&pdf).unwrap();
    let invoice = embedded.metadata.unwrap().invoice.unwrap();
    assert_eq!(
        invoice.namespace,
        "urn:ferd:pdfa:CrossIndustryDocument:invoice:1p0#"
    );
    assert_eq!(invoice.conformance_level.as_deref(), Some("COMFORT"));
    assert_eq!(
        invoice.document_file_name.as_deref(),
        Some("ZUGFeRD-invoice.xml")
    );
}

#[test]
fn pdf_2_0_and_crlf() {
    let pdf = Fixture::default().build();
    let mut v2 = b"%PDF-2.0".to_vec();
    v2.extend_from_slice(&pdf[8..]);
    assert_eq!(extract(&v2).unwrap().pdf_version, "2.0");
    // Trailing white space after %%EOF is allowed.
    let mut padded = pdf.clone();
    padded.extend_from_slice(b"\r\n\n  \0");
    assert_eq!(extract(&padded).unwrap().xml.data, INVOICE);
}

#[test]
fn stream_keyword_with_crlf_and_comments() {
    let mut body = format!(
        "<< /Length {} % the length\n /Type /EmbeddedFile >>\r\nstream\r\n",
        INVOICE.len()
    )
    .into_bytes();
    body.extend_from_slice(INVOICE);
    body.extend_from_slice(b"\r\nendstream");
    let pdf = with_catalog(
        "<< /Type /Catalog % comment\r\n /AF [5 0 R] >>",
        &[(4, &body), (5, filespec("factur-x.xml", 4).as_bytes())],
    );
    assert_eq!(extract(&pdf).unwrap().xml.data, INVOICE);
}

#[test]
fn several_attachments_one_invoice() {
    let pdf = with_catalog(
        "<< /Type /Catalog /AF [5 0 R 6 0 R 7 0 R] >>",
        &[
            (4, &embedded(INVOICE)),
            (5, filespec("logo.png", 8).as_bytes()),
            (6, filespec("factur-x.xml", 4).as_bytes()),
            (7, filespec("terms.pdf", 9).as_bytes()),
            (8, &embedded(b"png")),
            (9, &embedded(b"pdf")),
        ],
    );
    assert_eq!(extract(&pdf).unwrap().xml.data, INVOICE);
    assert_eq!(attachments(&pdf).unwrap().len(), 3);
    // The same file specification listed twice is one attachment.
    let pdf = with_catalog(
        "<< /Type /Catalog /AF [5 0 R 5 0 R] /Names << /EmbeddedFiles << /Names [(factur-x.xml) 5 0 R] >> >> >>",
        &[
            (4, &embedded(INVOICE)),
            (5, filespec("factur-x.xml", 4).as_bytes()),
        ],
    );
    let all = attachments(&pdf).unwrap();
    assert_eq!(all.len(), 1);
    assert!(all[0].in_associated_files);
    assert_eq!(all[0].key.as_deref(), Some("factur-x.xml"));
}

#[test]
fn limits_are_configurable() {
    let pdf = Fixture::default().build();
    let mut limits = Limits::default();
    limits.max_stream_size = 100;
    assert_eq!(
        einvoice_pdf::extract_with(&pdf, &limits),
        Err(Error::LimitExceeded {
            limit: "max_stream_size",
            value: 100
        })
    );
    let mut limits = Limits::default();
    limits.max_total_decoded = INVOICE.len() + 10;
    // The invoice fits; the XMP after it does not.
    assert_eq!(
        einvoice_pdf::extract_with(&pdf, &limits),
        Err(Error::LimitExceeded {
            limit: "max_total_decoded",
            value: INVOICE.len() + 10
        })
    );
}

/// With `EINVOICE_PDF_DUMP` set to a directory, writes one fixture per layout there, so
/// that independent PDF readers can confirm the fixtures are valid (for instance
/// `pdfdetach -saveall` of poppler, or `mutool extract`). Without it, does nothing.
#[test]
fn dump_fixtures_for_external_readers() {
    let Some(dir) = std::env::var_os("EINVOICE_PDF_DUMP") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    for layout in LAYOUTS {
        let pdf = Fixture {
            layout,
            ..Fixture::default()
        }
        .build();
        std::fs::write(dir.join(format!("{layout:?}.pdf")), pdf).unwrap();
    }
}

fn last_startxref(pdf: &[u8]) -> usize {
    let at = pdf.windows(9).rposition(|w| w == b"startxref").unwrap();
    let text = std::str::from_utf8(&pdf[at + 9..]).unwrap();
    text.split_whitespace().next().unwrap().parse().unwrap()
}

#[test]
fn literal_helper() {
    assert_eq!(literal("a(b)\\"), "(a\\(b\\)\\\\)");
}
