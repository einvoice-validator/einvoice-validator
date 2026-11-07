//! Every probe of `tests/probes/` gives the result Saxon-HE gave (`expected/`, written by
//! the `saxon-probes` example): the same result tree, or the same error code, and the
//! same number of distinct ambiguous-rule warnings. No Java is needed.
//!
//! Result trees are compared node by node, not as text: the serializations of two
//! engines may differ (attribute order, quoting, where namespaces are declared) without
//! the trees differing. Compared: node kinds and order, element and attribute names with
//! their prefixes, attribute values, the namespaces in scope on each element, text,
//! comments and processing instructions, character by character.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

#[path = "common/probes.rs"]
mod probes;

use roxmltree::{Document, Node, NodeType};

/// The qualified name of an element or attribute as written.
fn written(text: &str, range: std::ops::Range<usize>) -> String {
    let source = &text[range];
    source
        .trim_start_matches('<')
        .chars()
        .take_while(|&c| c == ':' || c == '_' || c == '-' || c == '.' || c.is_alphanumeric())
        .collect()
}

fn describe(text: &str, node: Node<'_, '_>, out: &mut Vec<String>) {
    match node.node_type() {
        NodeType::Root => {
            for child in node.children() {
                describe(text, child, out);
            }
        }
        NodeType::Element => {
            let mut namespaces: Vec<String> = node
                .namespaces()
                .map(|ns| format!("{}={}", ns.name().unwrap_or(""), ns.uri()))
                .collect();
            namespaces.sort();
            let mut attributes: Vec<String> = node
                .attributes()
                .map(|a| {
                    format!(
                        "{}{{{}}}={:?}",
                        written(text, a.range_qname()),
                        a.namespace().unwrap_or(""),
                        a.value()
                    )
                })
                .collect();
            attributes.sort();
            out.push(format!(
                "<{}{{{}}} {} ns[{}]",
                written(text, node.range()),
                node.tag_name().namespace().unwrap_or(""),
                attributes.join(" "),
                namespaces.join(" ")
            ));
            for child in node.children() {
                describe(text, child, out);
            }
            out.push(">".to_owned());
        }
        NodeType::Text => out.push(format!("text {:?}", node.text().unwrap_or(""))),
        NodeType::Comment => out.push(format!("comment {:?}", node.text().unwrap_or(""))),
        NodeType::PI => out.push(format!("pi {:?}", node.pi())),
    }
}

/// The result tree as lines, for a readable comparison.
fn tree(xml: &str) -> Result<Vec<String>, String> {
    let doc = Document::parse(xml).map_err(|e| format!("{e}\n{xml}"))?;
    let mut out = Vec::new();
    describe(xml, doc.root(), &mut out);
    Ok(out)
}

#[test]
fn every_probe_gives_the_saxon_result() {
    let dir = probes::probes_dir();
    let input_path = dir.join("input.xml");
    let input = std::fs::read_to_string(&input_path).unwrap();
    let uri = einvoice_xpath_eval::file_uri(&std::path::absolute(&input_path).unwrap());
    let names = probes::probes().unwrap();
    assert!(names.len() >= 17, "{names:?}");
    let mut failures = Vec::new();
    for name in &names {
        let stylesheet = std::fs::read_to_string(dir.join(format!("{name}.xsl"))).unwrap();
        let expected =
            probes::Expected::parse(&std::fs::read_to_string(probes::expected_path(name)).unwrap())
                .unwrap();
        let sheet = match einvoice_xslt::load(&stylesheet) {
            Ok(sheet) => sheet,
            Err(e) => {
                failures.push(format!("{name}: does not load: {e}"));
                continue;
            }
        };
        let executable = match einvoice_xslt_exec::Executable::new(&sheet) {
            Ok(executable) => executable,
            Err(e) => {
                failures.push(format!("{name}: does not prepare: {e}"));
                continue;
            }
        };
        match (
            executable.transform(&input, uri.as_deref()),
            &expected.error,
        ) {
            (Ok(ours), None) => {
                let (a, b) = (tree(&expected.output).unwrap(), tree(&ours.output).unwrap());
                if a != b {
                    let i = a
                        .iter()
                        .zip(&b)
                        .position(|(x, y)| x != y)
                        .unwrap_or(a.len().min(b.len()));
                    failures.push(format!(
                        "{name}: result differs at node {i}\n  saxon: {}\n  ours:  {}\n  ours in full: {}",
                        a.get(i).map_or("<end>", String::as_str),
                        b.get(i).map_or("<end>", String::as_str),
                        ours.output
                    ));
                }
                // Saxon-HE 12.10's standard error reporter prints each distinct warning
                // once.
                let distinct: std::collections::BTreeSet<&String> = ours.warnings.iter().collect();
                if distinct.len() != expected.warnings {
                    failures.push(format!(
                        "{name}: {} distinct warnings, Saxon {}: {:?}",
                        distinct.len(),
                        expected.warnings,
                        ours.warnings
                    ));
                }
            }
            (Err(e), Some(code)) if e.code() == code => {}
            (Err(e), _) => failures.push(format!("{name}: error {e}, Saxon: {:?}", expected.error)),
            (Ok(ours), Some(code)) => {
                failures.push(format!(
                    "{name}: no error, Saxon {code}; ours: {}",
                    ours.output
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
