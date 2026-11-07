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
    assert_eq!(schema.global_element_count(), 1622);
    // 1219 named complex types, 4 named simple types, anyType and the 16
    // implemented built-in simple types.
    assert_eq!(schema.type_count(), 1219 + 4 + 1 + 16);
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
    assert!(matches!(
        schema.validate_bytes(latin1.as_bytes()),
        Err(DocumentError::Unsupported { .. })
    ));
    assert!(matches!(
        schema.validate_bytes(&[0xFF, 0xFE, b'<', 0]),
        Err(DocumentError::Unsupported { .. })
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
    let Err(DocumentError::Unsupported { message, .. }) = schema.validate(&xml) else {
        panic!("not refused");
    };
    assert!(message.contains("xs:double"), "{message}");
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
