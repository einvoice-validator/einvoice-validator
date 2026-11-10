//! The public API on the embedded UBL 2.1 schemas: loading, reachability, error
//! details, byte input, refusals and robustness.
#![cfg(test)]

use einvoice_xsd::{
    CREDIT_NOTE_ENTRY_POINT, DocumentError, ErrorKind, INVOICE_ENTRY_POINT, LoadErrorKind, Schema,
    UBL_2_1_FILES,
};

const CBC: &str = "urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2";

fn ubl() -> Schema {
    Schema::ubl_2_1().unwrap()
}

fn credit_note(body: &str) -> String {
    format!(
        "<CreditNote xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2\" \
         xmlns:cac=\"urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2\" \
         xmlns:cbc=\"{CBC}\" xmlns:ext=\"urn:oasis:names:specification:ubl:schema:xsd:CommonExtensionComponents-2\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">{body}</CreditNote>"
    )
}

const MINIMAL_BODY: &str = "<cbc:ID>1</cbc:ID><cbc:IssueDate>2024-01-01</cbc:IssueDate>\
    <cac:AccountingSupplierParty/><cac:AccountingCustomerParty/>\
    <cac:LegalMonetaryTotal><cbc:PayableAmount currencyID=\"EUR\">0</cbc:PayableAmount></cac:LegalMonetaryTotal>\
    <cac:CreditNoteLine><cbc:ID>1</cbc:ID></cac:CreditNoteLine>";

#[test]
fn every_vendored_file_is_reachable_from_the_entry_points() {
    let schema = ubl();
    let mut loaded = schema.documents().to_vec();
    loaded.sort();
    let mut vendored: Vec<String> = UBL_2_1_FILES.iter().map(|(p, _)| (*p).to_owned()).collect();
    vendored.sort();
    assert_eq!(loaded, vendored);
    assert_eq!(schema.global_element_count(), 1581);
    // 1168 named complex types, 3 named simple types, anyType and the 45 built-in
    // simple types.
    assert_eq!(schema.type_count(), 1168 + 3 + 1 + 45);
}

#[test]
fn only_the_xades_imports_are_omitted() {
    let schema = ubl();
    let omitted: Vec<(&str, u32, &str, &str)> = schema
        .omitted_imports()
        .iter()
        .map(|o| {
            (
                o.file.as_str(),
                o.line,
                o.namespace.as_str(),
                o.location.as_str(),
            )
        })
        .collect();
    let importer = "xsd/common/UBL-SignatureAggregateComponents-2.1.xsd";
    assert_eq!(
        omitted,
        [
            (
                importer,
                32,
                "http://uri.etsi.org/01903/v1.3.2#",
                "xsd/common/UBL-XAdESv132-2.1.xsd"
            ),
            (
                importer,
                34,
                "http://uri.etsi.org/01903/v1.4.1#",
                "xsd/common/UBL-XAdESv141-2.1.xsd"
            ),
        ]
    );
    // Any other missing document is still an error.
    let without_dsig: Vec<(&str, &str)> = UBL_2_1_FILES
        .iter()
        .copied()
        .filter(|(p, _)| !p.ends_with("UBL-xmldsig-core-schema-2.1.xsd"))
        .collect();
    let error = Schema::load(&without_dsig, &[INVOICE_ENTRY_POINT]).unwrap_err();
    assert_eq!(error.kind, LoadErrorKind::MissingFile);
    assert!(
        error.file.ends_with("UBL-xmldsig-core-schema-2.1.xsd"),
        "{error}"
    );
}

/// A credit note whose single extension holds `content`.
fn with_extension(content: &str) -> String {
    credit_note(&format!(
        "<ext:UBLExtensions><ext:UBLExtension><ext:ExtensionContent \
         xmlns:xades=\"http://uri.etsi.org/01903/v1.3.2#\" \
         xmlns:ds=\"http://www.w3.org/2000/09/xmldsig#\">{content}\
         </ext:ExtensionContent></ext:UBLExtension></ext:UBLExtensions>{MINIMAL_BODY}"
    ))
}

#[test]
fn xades_content_is_validated_laxly() {
    let schema = ubl();
    // Content XAdES 1.3.2 would reject (wrong order, a bad xs:dateTime, a missing
    // required attribute) has no declaration here: lax, as xs:anyType.
    let xades = with_extension(
        "<xades:QualifyingProperties><xades:SignedProperties><xades:SignedSignatureProperties>\
         <xades:SigningCertificate/><xades:SigningTime>yesterday</xades:SigningTime>\
         </xades:SignedSignatureProperties></xades:SignedProperties></xades:QualifyingProperties>",
    );
    let report = schema.validate(&xades).unwrap();
    assert!(report.is_valid(), "{:?}", report.errors);
    // Below it, elements with a global declaration are still validated.
    let declared = with_extension(
        "<xades:QualifyingProperties><xades:CertDigest><ds:DigestMethod/>\
         <cbc:IssueDate>2024-02-30</cbc:IssueDate></xades:CertDigest></xades:QualifyingProperties>",
    );
    let kinds: Vec<ErrorKind> = schema
        .validate(&declared)
        .unwrap()
        .errors
        .iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(
        kinds,
        [ErrorKind::MissingAttribute, ErrorKind::InvalidValue]
    );
    // An xsi:type naming a XAdES type does not resolve.
    let typed =
        with_extension("<xades:QualifyingProperties xsi:type=\"xades:QualifyingPropertiesType\"/>");
    let report = schema.validate(&typed).unwrap();
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].kind, ErrorKind::InvalidXsiType);
}

#[test]
fn each_entry_point_alone() {
    let invoice = Schema::load(UBL_2_1_FILES, &[INVOICE_ENTRY_POINT]).unwrap();
    let report = invoice.validate(&credit_note(MINIMAL_BODY)).unwrap();
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].kind, ErrorKind::UnknownRoot);
    let credit = Schema::load(UBL_2_1_FILES, &[CREDIT_NOTE_ENTRY_POINT]).unwrap();
    assert!(
        credit
            .validate(&credit_note(MINIMAL_BODY))
            .unwrap()
            .is_valid()
    );
    // Without the common modules the entry points cannot load.
    let only_entry: Vec<(&str, &str)> = UBL_2_1_FILES
        .iter()
        .copied()
        .filter(|(p, _)| p.contains("maindoc"))
        .collect();
    let error = Schema::load(&only_entry, &[INVOICE_ENTRY_POINT]).unwrap_err();
    assert_eq!(error.kind, LoadErrorKind::MissingFile);
    assert!(
        error
            .to_string()
            .contains("UBL-CommonAggregateComponents-2.1.xsd"),
        "{error}"
    );
}

#[test]
fn error_details() {
    let xml = credit_note(
        "\n<cbc:ID>1</cbc:ID>\n  <cbc:IssueDate>2024-02-30</cbc:IssueDate>\
         <cac:AccountingSupplierParty foo=\"x\"/>\n<cac:AccountingCustomerParty/>\
         <cac:LegalMonetaryTotal><cbc:PayableAmount>1</cbc:PayableAmount></cac:LegalMonetaryTotal>",
    );
    let report = ubl().validate(&xml).unwrap();
    let summary: Vec<(ErrorKind, u32, u32, Option<&str>)> = report
        .errors
        .iter()
        .map(|e| (e.kind, e.line, e.column, e.attribute.as_deref()))
        .collect();
    assert_eq!(
        summary,
        [
            (ErrorKind::MissingElement, 1, 1, None),
            (ErrorKind::InvalidValue, 3, 3, None),
            (ErrorKind::AttributeNotAllowed, 3, 44, Some("foo")),
            (ErrorKind::MissingAttribute, 4, 55, Some("currencyID")),
        ]
    );
    assert_eq!(report.errors[1].element, format!("{{{CBC}}}IssueDate"));
    assert!(
        report.errors[0].message.contains("CreditNoteLine"),
        "{}",
        report.errors[0].message
    );
    let shown = report.errors[2].to_string();
    assert!(
        shown.starts_with("3:44: attribute-not-allowed: element {"),
        "{shown}"
    );
    assert!(shown.contains(", attribute foo: "), "{shown}");
    // Validating again gives the same report.
    assert_eq!(ubl().validate(&xml).unwrap(), report);
}

#[test]
fn bytes_input() {
    let schema = ubl();
    let xml = credit_note(MINIMAL_BODY);
    let with_declaration = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n{xml}");
    assert!(
        schema
            .validate_bytes(with_declaration.as_bytes())
            .unwrap()
            .is_valid()
    );
    let mut with_bom = vec![0xEF, 0xBB, 0xBF];
    with_bom.extend_from_slice(xml.as_bytes());
    assert!(schema.validate_bytes(&with_bom).unwrap().is_valid());
    let latin1 = format!("<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>\n{xml}");
    assert!(schema.validate_bytes(latin1.as_bytes()).unwrap().is_valid());
    let utf16: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain(xml.encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    assert!(schema.validate_bytes(&utf16).unwrap().is_valid());
    let shift_jis = format!("<?xml version=\"1.0\" encoding=\"Shift_JIS\"?>\n{xml}");
    assert!(matches!(
        schema.validate_bytes(shift_jis.as_bytes()),
        Err(DocumentError::Unsupported { .. })
    ));
    assert!(matches!(
        schema.validate_bytes(&[0xFF, 0xFE, b'<', 0, 0x00, 0xD8]),
        Err(DocumentError::Malformed {
            line: 1,
            column: 2,
            ..
        })
    ));
    let mut not_utf8 = xml.into_bytes();
    not_utf8.insert(10, 0xFF);
    assert!(matches!(
        schema.validate_bytes(&not_utf8),
        Err(DocumentError::Malformed { .. })
    ));
}

#[test]
fn malformed_and_unsupported_documents() {
    let schema = ubl();
    let Err(DocumentError::Malformed { line, .. }) = schema.validate("<a>\n<b></a>") else {
        panic!("not malformed");
    };
    assert_eq!(line, 2);
    let xml = credit_note(MINIMAL_BODY).replace(
        "<cbc:ID>1</cbc:ID><cbc:IssueDate>",
        "<cbc:ID xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xsi:type=\"xs:double\">1</cbc:ID><cbc:IssueDate>",
    );
    // Every built-in type resolves; xs:double is not derived from the declared type.
    let report = schema.validate(&xml).unwrap();
    assert_eq!(report.errors.len(), 1, "{report:?}");
    assert_eq!(
        report.errors[0].kind,
        einvoice_xsd::ErrorKind::InvalidXsiType
    );
    assert!(
        report.errors[0].message.contains("xs:double")
            || report.errors[0].message.contains("XMLSchema}double"),
        "{}",
        report.errors[0].message
    );
    // An xs:ENTITY value in a document with a document type declaration, which may
    // declare unparsed entities: refused as unsupported.
    let xml = format!(
        "<!DOCTYPE CreditNote [<!NOTATION gif SYSTEM \"image/gif\"><!ENTITY logo SYSTEM \"logo.gif\" NDATA gif>]>\n{}",
        credit_note(&format!(
            "<ext:UBLExtensions xmlns:ext=\"urn:oasis:names:specification:ubl:schema:xsd:CommonExtensionComponents-2\">\
             <ext:UBLExtension><ext:ExtensionContent>\
             <x:e xmlns:x=\"urn:x\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xsi:type=\"xs:ENTITY\">logo</x:e>\
             </ext:ExtensionContent></ext:UBLExtension></ext:UBLExtensions>{MINIMAL_BODY}"
        ))
    );
    match schema.validate(&xml) {
        Err(DocumentError::Unsupported { message, .. }) => {
            assert!(message.contains("unparsed entity"), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

/// What the reading of the document as a stream of events must preserve from the tree
/// it replaced: text nodes, CDATA sections and their errors, prefixes, the order of
/// errors.
#[test]
fn documents_are_read_as_a_stream() {
    let schema = ubl();
    let body = |date: &str| {
        credit_note(&MINIMAL_BODY.replace(
            "<cbc:IssueDate>2024-01-01</cbc:IssueDate>",
            &format!("<cbc:IssueDate>{date}</cbc:IssueDate>"),
        ))
    };
    // Character data, references and CDATA sections make one value; comments and
    // processing instructions do not split it.
    for date in [
        "2024-<![CDATA[01]]>-01",
        "&#50;024-01-01",
        "2024<!--c-->-01<?p x?>-01",
        " 2024-01-01\n",
    ] {
        let report = schema.validate(&body(date)).unwrap();
        assert!(report.is_valid(), "{date}: {:?}", report.errors);
    }
    let report = schema.validate(&body("2024-<![CDATA[02]]>-30")).unwrap();
    assert_eq!(report.errors.len(), 1);
    assert!(
        report.errors[0]
            .message
            .starts_with("\"2024-02-30\" is not"),
        "{}",
        report.errors[0].message
    );
    // Text in element-only content: once per text node, a CDATA section included; an
    // empty CDATA section is a text node too, which empty content does not allow.
    let xml = credit_note(&format!("x<![CDATA[y]]><!--c-->z{MINIMAL_BODY}"));
    let messages: Vec<String> = schema
        .validate(&xml)
        .unwrap()
        .errors
        .into_iter()
        .map(|e| e.message)
        .collect();
    assert_eq!(
        messages,
        [
            "text \"xy\" in element-only content",
            "text \"z\" in element-only content"
        ]
    );
    let xml = credit_note(&MINIMAL_BODY.replace(
        "<cac:AccountingSupplierParty/>",
        "<cac:AccountingSupplierParty/><cac:AccountingCustomerParty><cac:Party/></cac:AccountingCustomerParty>",
    ));
    assert!(!schema.validate(&xml).unwrap().is_valid());
    // xsi:type: the xml and xmlns prefixes are not declared prefixes, as before.
    for prefix in ["xml", "xmlns", "q"] {
        let xml = credit_note(&MINIMAL_BODY.replace(
            "<cbc:ID>1</cbc:ID><cbc:IssueDate>",
            &format!("<cbc:ID xsi:type=\"{prefix}:x\">1</cbc:ID><cbc:IssueDate>"),
        ));
        let report = schema.validate(&xml).unwrap();
        assert_eq!(
            report.errors[0].message,
            format!("xsi:type \"{prefix}:x\": prefix \"{prefix}\" is not declared")
        );
    }
    // The element a frame reports keeps its namespace when a child rebinds its prefix.
    let xml = credit_note(&MINIMAL_BODY.replace(
        "<cbc:ID>1</cbc:ID>",
        "<cbc:ID>1<cbc:x xmlns:cbc=\"urn:other\"/></cbc:ID>",
    ));
    let report = schema.validate(&xml).unwrap();
    assert_eq!(report.errors[0].kind, ErrorKind::ElementInSimpleContent);
    assert_eq!(report.errors[0].element, format!("{{{CBC}}}ID"));
    assert_eq!(
        report.errors[0].message,
        "child element {urn:other}x in simple content"
    );
    // A document is not well-formed wherever the error is: after an unsupported
    // construct, and after the end of the validated content.
    let unsupported = credit_note(MINIMAL_BODY).replace(
        "<cbc:ID>1</cbc:ID><cbc:IssueDate>",
        "<cbc:ID xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xsi:type=\"xs:double\">1</cbc:ID><cbc:IssueDate>",
    );
    assert!(matches!(
        schema.validate(&format!("{unsupported}<")),
        Err(DocumentError::Malformed { .. })
    ));
    assert!(matches!(
        schema.validate(&format!("<a>{unsupported}")),
        Err(DocumentError::Malformed { .. })
    ));
}

/// Constructs that XML 1.0 or Namespaces in XML 1.0 forbid, which the `roxmltree` crate
/// accepted (until 0.2.0 documents were read with it): the document is not well-formed,
/// with the message of `einvoice-xml`.
#[test]
fn constructs_xml_forbids_are_malformed() {
    let schema = ubl();
    let valid = credit_note(MINIMAL_BODY);
    assert!(schema.validate(&valid).unwrap().is_valid());
    for (find, replace, message) in [
        (
            "<cbc:ID>1",
            "<cbc:ID>&#xD800;1",
            "is not a Unicode scalar value",
        ),
        (
            "<cbc:ID>1",
            "<?XmL x?><cbc:ID>1",
            "the processing instruction target at",
        ),
        (
            "<cbc:ID>1",
            "<cbc:ID xmlns:p=''>1",
            "bound to an empty namespace name",
        ),
        (
            "<cbc:ID>1",
            "<cbc:ID xmlns:xmlns='urn:x'>1",
            "the 'xmlns' prefix is declared",
        ),
        (
            "<cbc:ID>1",
            "<cbc:ID :a='1'>1",
            "a name with an empty prefix",
        ),
        (
            "<cbc:ID>1",
            "<cbc:ID xmlns:p='urn:p' p:xmlns='urn:x'>1",
            "a prefixed attribute named 'xmlns'",
        ),
    ] {
        let xml = valid.replacen(find, replace, 1);
        let Err(DocumentError::Malformed { message: found, .. }) = schema.validate(&xml) else {
            panic!("{replace}: not malformed");
        };
        assert!(found.contains(message), "{replace}: {found}");
    }
    let xml = format!("<?xml version=\"1.0\" encoding=\"-\"?>{valid}");
    let Err(DocumentError::Malformed { message, .. }) = schema.validate(&xml) else {
        panic!("not malformed");
    };
    assert!(message.contains("not an encoding name"), "{message}");
}

/// A document with a document type declaration, which the streaming reader refuses, is
/// read with `roxmltree`: the entities of its internal subset are expanded.
#[test]
fn documents_with_a_document_type_declaration() {
    let schema = ubl();
    let xml = format!(
        "<!DOCTYPE CreditNote [<!ENTITY id \"1\">]>\n{}",
        credit_note(MINIMAL_BODY).replacen("<cbc:ID>1</cbc:ID>", "<cbc:ID>&id;</cbc:ID>", 1)
    );
    assert!(schema.validate(&xml).unwrap().is_valid());
    let xml = xml.replace("2024-01-01", "2024-02-30");
    let report = schema.validate(&xml).unwrap();
    assert_eq!(
        report
            .errors
            .iter()
            .map(|e| (e.kind, e.line))
            .collect::<Vec<_>>(),
        [(ErrorKind::InvalidValue, 2)]
    );
    assert!(matches!(
        schema.validate("<!DOCTYPE a [<!ENTITY e \"&f;\">]><a>&e;</a>"),
        Err(DocumentError::Malformed { .. })
    ));
    // The streaming validation is not given such a document: its reader refuses it.
    let mut reader = einvoice_xml::Reader::new(&xml);
    assert_eq!(
        reader.next_event().unwrap_err(),
        einvoice_xml::Error::DtdDetected
    );
}

/// [`Schema::validation`] given the events of a reader is [`Schema::validate`].
#[test]
fn a_validation_given_events() {
    let schema = ubl();
    for xml in [
        credit_note(MINIMAL_BODY),
        credit_note("<cbc:IssueDate>x</cbc:IssueDate>"),
        "<a/>".to_owned(),
    ] {
        let mut validation = schema.validation(&xml);
        assert!(format!("{validation:?}").starts_with("Validation"));
        let mut reader = einvoice_xml::Reader::new(&xml);
        while let Some(event) = reader.next_event().unwrap() {
            validation.event(event);
        }
        assert_eq!(validation.finish(), schema.validate(&xml));
    }
}

#[test]
fn deep_lax_content_does_not_overflow() {
    let depth = 100_000;
    let open = "<x:a xmlns:x=\"urn:x\">".repeat(depth);
    let close = "</x:a>".repeat(depth);
    let body = format!(
        "<ext:UBLExtensions><ext:UBLExtension><ext:ExtensionContent>{open}<cbc:ID>1</cbc:ID>{close}\
         </ext:ExtensionContent></ext:UBLExtension></ext:UBLExtensions>{MINIMAL_BODY}"
    );
    let report = ubl().validate(&credit_note(&body)).unwrap();
    assert!(report.is_valid(), "{:?}", report.errors.first());
    let bad = body.replacen(
        "<cbc:ID>1</cbc:ID></x:a>",
        "<cbc:IssueDate>x</cbc:IssueDate></x:a>",
        1,
    );
    let report = ubl().validate(&credit_note(&bad)).unwrap();
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].kind, ErrorKind::InvalidValue);
}

#[test]
fn custom_schema_set() {
    let main = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:t="urn:t" targetNamespace="urn:t" elementFormDefault="qualified">
        <xs:include schemaLocation="types/part.xsd"/>
        <xs:element name="root" type="t:Root"/>
    </xs:schema>"#;
    let part = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:t="urn:t" targetNamespace="urn:t" elementFormDefault="qualified">
        <xs:complexType name="Root"><xs:sequence>
            <xs:element name="n" type="xs:integer" maxOccurs="2"/>
        </xs:sequence><xs:attribute name="id" type="xs:ID"/></xs:complexType>
    </xs:schema>"#;
    let schema = Schema::load(
        &[("main.xsd", main), ("types/part.xsd", part)],
        &["main.xsd"],
    )
    .unwrap();
    assert_eq!(schema.documents(), ["main.xsd", "types/part.xsd"]);
    let valid = r#"<root xmlns="urn:t" id="a"><n>1</n><n>-2</n></root>"#;
    assert!(schema.validate(valid).unwrap().is_valid());
    let invalid = r#"<root xmlns="urn:t" id="1a"><n>1</n><n>2</n><n>3</n></root>"#;
    let kinds: Vec<ErrorKind> = schema
        .validate(invalid)
        .unwrap()
        .errors
        .iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            ErrorKind::InvalidAttributeValue,
            ErrorKind::UnexpectedElement
        ]
    );
}

/// A schema set in the style of the CII schemas: fixed and default values, length
/// facets, anonymous complex types.
fn cii_style() -> Schema {
    let schema = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:t="urn:t" targetNamespace="urn:t" elementFormDefault="qualified">
        <xs:simpleType name="Code"><xs:restriction base="xs:token"><xs:minLength value="1"/><xs:maxLength value="3"/></xs:restriction></xs:simpleType>
        <xs:simpleType name="Short"><xs:restriction base="t:Code"><xs:maxLength value="2"/></xs:restriction></xs:simpleType>
        <xs:complexType name="CodeType"><xs:simpleContent><xs:extension base="t:Code">
            <xs:attribute name="listID" type="xs:token" fixed="List"/>
            <xs:attribute name="rate" type="xs:decimal" fixed="1.50"/>
            <xs:attribute name="version" type="xs:token" default="D16B"/>
        </xs:extension></xs:simpleContent></xs:complexType>
        <xs:complexType name="Root"><xs:sequence>
            <xs:element name="code" type="t:CodeType" minOccurs="0" maxOccurs="unbounded"/>
            <xs:element name="amount" type="xs:decimal" fixed="1.50" minOccurs="0" maxOccurs="unbounded"/>
            <xs:element name="count" type="t:Code" default="ABC" minOccurs="0" maxOccurs="unbounded"/>
            <xs:element name="when" minOccurs="0"><xs:complexType><xs:choice>
                <xs:element name="text"><xs:complexType><xs:simpleContent><xs:extension base="xs:string">
                    <xs:attribute name="format" type="xs:token" use="required"/>
                </xs:extension></xs:simpleContent></xs:complexType></xs:element>
                <xs:element name="date" type="xs:date"/>
            </xs:choice></xs:complexType></xs:element>
        </xs:sequence></xs:complexType>
        <xs:element name="doc" type="t:Root"/>
    </xs:schema>"#;
    Schema::load(&[("s.xsd", schema)], &["s.xsd"]).unwrap()
}

#[test]
fn value_constraints_length_facets_and_anonymous_types() {
    let schema = cii_style();
    let errors = |body: &str| -> Vec<(ErrorKind, String, Option<String>, String)> {
        let document = format!(
            r#"<doc xmlns="urn:t" xmlns:t="urn:t" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">{body}</doc>"#
        );
        schema
            .validate(&document)
            .unwrap()
            .errors
            .into_iter()
            .map(|e| (e.kind, e.element, e.attribute, e.message))
            .collect()
    };
    // Equal values with other lexical forms, absent attributes and empty elements
    // with a value constraint are valid.
    let valid = r#"<code listID=" List " rate="+01.5">AB</code><code version="X">A</code>
        <amount>1.5</amount><amount> 1.500 </amount><amount/><amount><!-- c --></amount>
        <count/><count></count><count>  AB  </count>
        <when><text format="102">20240101</text></when>"#;
    assert_eq!(errors(valid), []);
    let code = "{urn:t}code".to_owned();
    assert_eq!(
        errors(r#"<code listID="Other" rate="1.51">AB</code>"#),
        [
            (
                ErrorKind::InvalidAttributeValue,
                code.clone(),
                Some("listID".to_owned()),
                r#""Other" is not the fixed value "List" of the attribute"#.to_owned()
            ),
            (
                ErrorKind::InvalidAttributeValue,
                code.clone(),
                Some("rate".to_owned()),
                r#""1.51" is not the fixed value "1.50" of the attribute"#.to_owned()
            ),
        ]
    );
    assert_eq!(
        errors("<amount>2</amount>"),
        [(
            ErrorKind::InvalidValue,
            "{urn:t}amount".to_owned(),
            None,
            r#""2" is not the fixed value "1.50" of the declaration"#.to_owned()
        )]
    );
    // Whitespace is content: the element takes no value constraint.
    assert_eq!(
        errors("<amount> </amount><count> </count>")
            .iter()
            .map(|e| e.0)
            .collect::<Vec<_>>(),
        [ErrorKind::InvalidValue, ErrorKind::InvalidValue]
    );
    // Length facets, counted in characters after whitespace processing.
    assert_eq!(
        errors("<code>ABCD</code><count>éàü</count><count>éàüö</count>"),
        [
            (
                ErrorKind::InvalidValue,
                code,
                None,
                r#""ABCD" is 4 characters long, more than the maximum length 3 of {urn:t}Code"#
                    .to_owned()
            ),
            (
                ErrorKind::InvalidValue,
                "{urn:t}count".to_owned(),
                None,
                r#""éàüö" is 4 characters long, more than the maximum length 3 of {urn:t}Code"#
                    .to_owned()
            ),
        ]
    );
    // cvc-elt.5.1.1: with xsi:type, the default must be valid for the actual type.
    assert_eq!(
        errors(r#"<count xsi:type="t:Short"/><count xsi:type="t:Short">AB</count>"#),
        [(
            ErrorKind::InvalidValue,
            "{urn:t}count".to_owned(),
            None,
            r#"the default value "ABC" of the declaration is 3 characters long, more than the maximum length 2 of {urn:t}Short"#
                .to_owned()
        )]
    );
    // Anonymous complex types: content models and attributes.
    assert_eq!(
        errors("<when><text>x</text><date>2024-01-01</date></when><when/>")
            .iter()
            .map(|e| e.0)
            .collect::<Vec<_>>(),
        [
            ErrorKind::MissingAttribute,
            ErrorKind::UnexpectedElement,
            ErrorKind::UnexpectedElement
        ]
    );
}

#[test]
fn error_kind_names_are_stable() {
    let names: Vec<&str> = [
        ErrorKind::UnknownRoot,
        ErrorKind::UnexpectedElement,
        ErrorKind::MissingElement,
        ErrorKind::TextNotAllowed,
        ErrorKind::ElementInSimpleContent,
        ErrorKind::ContentNotEmpty,
        ErrorKind::UndeclaredElement,
        ErrorKind::UndeclaredAttribute,
        ErrorKind::AttributeNotAllowed,
        ErrorKind::MissingAttribute,
        ErrorKind::InvalidValue,
        ErrorKind::InvalidAttributeValue,
        ErrorKind::DuplicateId,
        ErrorKind::InvalidXsiType,
        ErrorKind::AbstractType,
        ErrorKind::NotNillable,
        ErrorKind::InvalidXsiNil,
    ]
    .iter()
    .map(|k| k.as_str())
    .collect();
    let mut unique = names.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), names.len());
    assert_eq!(ErrorKind::DuplicateId.to_string(), "duplicate-id");
}
