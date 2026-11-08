//! The result trees of `tests/data/write/` (`NAME.xml`, or `NAME.xsl` applied to itself,
//! whose result Saxon wrote into `tests/data/written/NAME.result.xml`): the DOM document
//! is the one Saxon's `DOMWriter` builds (`NAME.dom`), the stylesheet written is the one
//! ph-commons writes as ph-schematron-maven-plugin calls it (`NAME.xslt`), and a name the
//! JDK's DOM refuses is refused with the same `DOMException` code (`NAME.error`); the
//! goldens are written by the `ph-schematron-oracle` example.

#![cfg(test)]

mod common;

use einvoice_schematron::{Document, Error, write_stylesheet};

#[test]
fn result_trees_are_written_as_ph_commons_writes_them() {
    let data = common::data();
    let written = data.join("written");
    let mut inputs: Vec<std::path::PathBuf> = std::fs::read_dir(data.join("write"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    inputs.sort();
    assert_ne!(inputs.len(), 0, "no result tree in tests/data/write");
    let mut failures = Vec::new();
    for input in &inputs {
        let stem = input.file_stem().unwrap().to_str().unwrap().to_owned();
        let xml = match input.extension().and_then(|e| e.to_str()) {
            Some("xml") => common::read(input),
            Some("xsl") => common::read(&written.join(format!("{stem}.result.xml"))),
            other => panic!("{}: unexpected extension {other:?}", input.display()),
        };
        let error = written.join(format!("{stem}.error"));
        let document = Document::from_result(&xml);
        if error.is_file() {
            let code = common::read(&error).trim().to_owned();
            match document {
                Err(Error::Dom { code: ours, .. }) if ours == code => {}
                other => failures.push(format!("{stem}: expected {code}, got {other:?}")),
            }
            continue;
        }
        let document = document.unwrap_or_else(|e| panic!("{stem}: {e}"));
        let dom = common::read(&written.join(format!("{stem}.dom")));
        let ours = common::dom_text(&document);
        if ours != dom {
            failures.push(format!(
                "{stem}.dom: {}",
                common::first_difference(&dom, &ours)
            ));
        }
        let xslt = common::read(&written.join(format!("{stem}.xslt")));
        let out = write_stylesheet(&document).unwrap_or_else(|e| panic!("{stem}: {e}"));
        if out != xslt {
            failures.push(format!(
                "{stem}.xslt: {}",
                common::first_difference(&xslt, &out)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
