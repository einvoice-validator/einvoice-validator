//! The synthetic Schematron files of `tests/data/sch/`, compiled by ph-schematron 10.3.1
//! (`ph-schematron-maven-plugin:convert`, engine iso-schematron; goldens in
//! `tests/data/expected/`, written by the `ph-schematron-oracle` example): from the result
//! tree of the last step (`NAME.step3.xml`), the DOM document is the one ph-schematron
//! builds (`NAME.dom`), node by node, and the stylesheet written is the plugin's
//! (`NAME.xslt`), byte for byte.

#![cfg(test)]

mod common;

use einvoice_schematron::{Document, write_stylesheet};

/// The fixtures, by the name of their Schematron file.
const FIXTURES: [&str; 7] = [
    "abstract",
    "basic",
    "characters",
    "foreign",
    "include",
    "markup",
    "minimal",
];

#[test]
fn every_fixture_has_its_goldens() {
    let mut names: Vec<String> = std::fs::read_dir(common::data().join("sch"))
        .unwrap()
        .filter_map(|e| {
            let name = e.unwrap().file_name().into_string().unwrap();
            name.strip_suffix(".sch").map(str::to_owned)
        })
        .collect();
    names.sort();
    assert_eq!(names, FIXTURES);
}

#[test]
fn the_dom_and_the_stylesheet_are_ph_schematron_s() {
    let expected = common::data().join("expected");
    let mut failures = Vec::new();
    for name in FIXTURES {
        let result = common::read(&expected.join(format!("{name}.step3.xml")));
        let document = Document::from_result(&result).unwrap_or_else(|e| panic!("{name}: {e}"));
        let dom = common::read(&expected.join(format!("{name}.dom")));
        let ours = common::dom_text(&document);
        if ours != dom {
            failures.push(format!(
                "{name}.dom: {}",
                common::first_difference(&dom, &ours)
            ));
        }
        let xslt = common::read(&expected.join(format!("{name}.xslt")));
        let written = write_stylesheet(&document).unwrap_or_else(|e| panic!("{name}: {e}"));
        if written != xslt {
            failures.push(format!(
                "{name}.xslt: {}",
                common::first_difference(&xslt, &written)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
