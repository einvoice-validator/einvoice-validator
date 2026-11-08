//! The DOM operations of `tests/data/xerces-cases.txt` give on
//! `einvoice_schematron::dom` what they give on the JDK's DOM (Xerces), as the
//! `ph-schematron-oracle` example recorded them in `tests/data/xerces-expected.txt`: the
//! order of attributes, `normalize`, and the names refused.

#![cfg(test)]

mod common;

use std::fmt::Write as _;

use einvoice_schematron::dom::{Element, Node, check_name};

fn result(element: &Element, out: &mut String) {
    for attribute in element.attributes() {
        let _ = writeln!(
            out,
            "A\t{}\t{}",
            common::escape(&attribute.name),
            common::escape(&attribute.value)
        );
    }
    for child in &element.children {
        node(child, 0, out);
    }
}

fn node(n: &Node, depth: usize, out: &mut String) {
    match n {
        Node::Text(data) => {
            let _ = writeln!(out, "T\t{depth}\t{}", common::escape(data));
        }
        Node::CData(data) => {
            let _ = writeln!(out, "D\t{depth}\t{}", common::escape(data));
        }
        Node::Comment(data) => {
            let _ = writeln!(out, "C\t{depth}\t{}", common::escape(data));
        }
        Node::Element(element) => {
            let _ = writeln!(out, "E\t{depth}\t{}", common::escape(element.name()));
            for child in &element.children {
                node(child, depth + 1, out);
            }
        }
        Node::ProcessingInstruction { .. } => panic!("no processing instruction in the cases"),
    }
}

fn namespace(field: &str) -> Option<&str> {
    (field != "-").then_some(field)
}

fn outcome(ok: bool) -> &'static str {
    if ok { "ok" } else { "error" }
}

#[test]
fn dom_operations_are_xerces_ones() {
    let data = common::data();
    let cases = common::read(&data.join("xerces-cases.txt"));
    let expected = common::read(&data.join("xerces-expected.txt"));
    let mut out = String::new();
    let mut element: Option<Element> = None;
    for line in cases.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields: Vec<&str> = line.split('\t').collect();
        // Missing trailing fields are empty, as the oracle reads them.
        fields.resize(4, "");
        let e = element.as_mut();
        match fields[0] {
            "case" => {
                if let Some(e) = element.take() {
                    result(&e, &mut out);
                }
                element = Some(Element::new(None, "e").unwrap());
                let _ = writeln!(out, "case\t{}", fields[1]);
            }
            "set" => e
                .unwrap()
                .set_attribute_ns(namespace(fields[1]), fields[2], fields[3])
                .unwrap(),
            "text" => e.unwrap().children.push(Node::Text(fields[1].to_owned())),
            "cdata" => e.unwrap().children.push(Node::CData(fields[1].to_owned())),
            "comment" => e
                .unwrap()
                .children
                .push(Node::Comment(fields[1].to_owned())),
            "child" => e
                .unwrap()
                .children
                .push(Node::Element(Element::new(None, fields[1]).unwrap())),
            "child-text" => {
                let last = e
                    .unwrap()
                    .children
                    .iter_mut()
                    .rev()
                    .find_map(|n| match n {
                        Node::Element(child) => Some(child),
                        _ => None,
                    })
                    .unwrap();
                last.children.push(Node::Text(fields[1].to_owned()));
            }
            "normalize" => e.unwrap().normalize(),
            "create" => {
                let ok = Element::new(namespace(fields[1]), fields[2]).is_ok();
                let _ = writeln!(
                    out,
                    "create\t{}\t{}\t{}",
                    fields[1],
                    common::escape(fields[2]),
                    outcome(ok)
                );
            }
            "attr" => {
                let ok = check_name(namespace(fields[1]), fields[2]).is_ok();
                let _ = writeln!(
                    out,
                    "attr\t{}\t{}\t{}",
                    fields[1],
                    common::escape(fields[2]),
                    outcome(ok)
                );
            }
            "target" => {
                let ok = Node::processing_instruction(fields[1], "d").is_ok();
                let _ = writeln!(
                    out,
                    "target\t{}\t{}",
                    common::escape(fields[1]),
                    outcome(ok)
                );
            }
            other => panic!("unknown operation {other}"),
        }
    }
    if let Some(e) = element.take() {
        result(&e, &mut out);
    }
    assert!(
        out == expected,
        "{}",
        common::first_difference(&expected, &out)
    );
}
