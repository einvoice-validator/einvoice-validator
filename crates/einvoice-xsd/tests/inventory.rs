//! The inventory of XML Schema constructs in the crate documentation (between
//! `INVENTORY-START` and `INVENTORY-END` in `src/lib.rs`) is what the vendored UBL
//! 2.1 schemas contain. On a mismatch the test prints the table to paste.
#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Write as _;

const XS: &str = "http://www.w3.org/2001/XMLSchema";

/// Attributes whose values are counted.
const COUNTED_VALUES: [&str; 9] = [
    "minOccurs",
    "maxOccurs",
    "use",
    "processContents",
    "namespace",
    "mixed",
    "abstract",
    "elementFormDefault",
    "attributeFormDefault",
];

fn walk(node: roxmltree::Node<'_, '_>, counts: &mut BTreeMap<String, usize>) {
    let mut add = |key: String| *counts.entry(key).or_default() += 1;
    let local = node.tag_name().name();
    let parent = node
        .parent_element()
        .map(|p| p.tag_name().name().to_owned())
        .unwrap_or_default();
    let construct = match local {
        "element" if parent == "schema" => "xs:element (global)".to_owned(),
        "element" if node.has_attribute("ref") => "xs:element (local, ref)".to_owned(),
        "element" => "xs:element (local, name)".to_owned(),
        "restriction" | "extension" => format!("xs:{local} (in xs:{parent})"),
        other => format!("xs:{other}"),
    };
    add(construct.clone());
    if local == "annotation" {
        return;
    }
    for attribute in node.attributes() {
        let name = attribute.name();
        if COUNTED_VALUES.contains(&name) {
            add(format!("{construct} @{name}=\"{}\"", attribute.value()));
        }
        if matches!(name, "type" | "base") {
            let value = attribute.value().trim();
            let (prefix, type_local) = value
                .split_once(':')
                .map_or((None, value), |(p, l)| (Some(p), l));
            if node.lookup_namespace_uri(prefix) == Some(XS) {
                add(format!("built-in type xs:{type_local}"));
            }
        }
    }
    for child in node.children().filter(roxmltree::Node::is_element) {
        assert_eq!(
            child.tag_name().namespace(),
            Some(XS),
            "foreign element outside an annotation"
        );
        walk(child, counts);
    }
}

fn inventory() -> String {
    let mut counts = BTreeMap::new();
    for (_, text) in einvoice_xsd::UBL_2_1_FILES {
        let options = roxmltree::ParsingOptions {
            allow_dtd: true,
            ..roxmltree::ParsingOptions::default()
        };
        let doc = roxmltree::Document::parse_with_options(text, options).unwrap();
        walk(doc.root_element(), &mut counts);
    }
    let mut out = String::from("//! | Construct | Count |\n//! |---|---:|\n");
    for (key, count) in counts {
        writeln!(out, "//! | `{key}` | {count} |").unwrap();
    }
    out
}

#[test]
fn documented_inventory_matches_the_schemas() {
    let lib = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs")).unwrap();
    let start = lib.find("//! INVENTORY-START\n").unwrap() + "//! INVENTORY-START\n".len();
    let end = lib.find("//! INVENTORY-END\n").unwrap();
    let documented = &lib[start..end];
    let actual = inventory();
    assert!(
        documented == actual,
        "the inventory in src/lib.rs is out of date; it should be:\n{actual}"
    );
}

#[test]
fn only_schema_and_annotation_content() {
    // `walk` asserts that no foreign element appears outside annotations; the
    // inventory must also have found every kind of document.
    let table = inventory();
    for construct in [
        "xs:schema",
        "xs:import",
        "xs:include",
        "xs:any",
        "xs:anyAttribute",
        "xs:enumeration",
    ] {
        assert!(table.contains(&format!("`{construct}`")), "{construct}");
    }
}
