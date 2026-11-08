//! The public API on the embedded UN/CEFACT CII D16B schemas: loading, reachability and
//! a few validity errors on an official example.
#![cfg(test)]

use einvoice_xsd::{CII_D16B_ENTRY_POINT, CII_D16B_FILES, ErrorKind, Schema};

const RSM: &str = "urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100";
const RAM: &str =
    "urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100";
const UDT: &str = "urn:un:unece:uncefact:data:standard:UnqualifiedDataType:100";

fn cii() -> Schema {
    Schema::cii_d16b().unwrap()
}

/// An official example of the CEN CII artefacts, valid for the schemas.
fn example() -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artefacts/cen-en16931-cii-1.3.17/cii/examples/CII_example3.xml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn every_vendored_file_is_reachable_from_the_entry_point() {
    let schema = cii();
    let mut loaded = schema.documents().to_vec();
    loaded.sort();
    let mut vendored: Vec<String> = CII_D16B_FILES
        .iter()
        .map(|(p, _)| (*p).to_owned())
        .collect();
    vendored.sort();
    assert_eq!(loaded, vendored);
    assert_eq!(schema.omitted_imports(), []);
    // One global element, the document element.
    assert_eq!(schema.global_element_count(), 1);
    assert!(
        CII_D16B_FILES
            .iter()
            .any(|(p, _)| *p == CII_D16B_ENTRY_POINT)
    );
}

#[test]
fn an_official_example_is_valid() {
    let report = cii().validate(&example()).unwrap();
    assert!(report.is_valid(), "{:?}", report.errors);
    // The same as bytes, with a byte order mark.
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(example().as_bytes());
    assert!(cii().validate_bytes(&bytes).unwrap().is_valid());
}

#[test]
fn validity_errors_of_cii_documents() {
    let schema = cii();
    // An empty document element: the exchanged document context comes first.
    let report = schema
        .validate(&format!("<rsm:CrossIndustryInvoice xmlns:rsm=\"{RSM}\"/>"))
        .unwrap();
    let kinds: Vec<&str> = report.errors.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, ["missing-element"]);
    // Another root element.
    let report = schema
        .validate(&format!("<rsm:Invoice xmlns:rsm=\"{RSM}\"/>"))
        .unwrap();
    assert_eq!(report.errors[0].kind, ErrorKind::UnknownRoot);

    let text = example();
    // The value of an xs:boolean indicator.
    let edited = replace_once(
        &text,
        "<udt:Indicator>true</udt:Indicator>",
        "<udt:Indicator>yes</udt:Indicator>",
    );
    let report = schema.validate(&edited).unwrap();
    assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
    let error = &report.errors[0];
    assert_eq!(error.kind, ErrorKind::InvalidValue);
    assert_eq!(error.element, format!("{{{UDT}}}Indicator"));
    assert_eq!(error.line, 121);
    // A child element out of order.
    let edited = replace_once(
        &text,
        "<rsm:ExchangedDocumentContext>",
        "<rsm:ExchangedDocument/><rsm:ExchangedDocumentContext>",
    );
    let report = schema.validate(&edited).unwrap();
    assert_eq!(report.errors[0].kind, ErrorKind::UnexpectedElement);
    // An undeclared element in an aggregate, which has no wildcard.
    let edited = replace_once(
        &text,
        "<ram:SellerTradeParty>",
        &format!("<ram:SellerTradeParty><ram:Unknown xmlns:ram=\"{RAM}\"/>"),
    );
    let report = schema.validate(&edited).unwrap();
    assert_eq!(report.errors[0].kind, ErrorKind::UnexpectedElement);
}

fn replace_once(text: &str, find: &str, replace: &str) -> String {
    assert_eq!(text.matches(find).count(), 1, "{find}");
    text.replacen(find, replace, 1)
}
