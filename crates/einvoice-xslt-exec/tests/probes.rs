//! Every probe of `tests/probes/` gives the result Saxon-HE gave (`expected/`, written by
//! the `saxon-probes` example): the same result tree, or the same error code, the same
//! number of distinct ambiguous-rule warnings, the same errors recovered while matching
//! patterns (line and code), and the same messages. No Java is needed. A probe may import
//! the stylesheet modules of `tests/probes/modules/` and read with `document()` the files
//! of `tests/probes/`, itself included.
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

/// The files of `tests/probes/`, by their `file:` URI: the modules a probe imports and
/// the documents it reads.
struct Files(std::path::PathBuf);

impl Files {
    fn read(&self, uri: &str) -> Option<String> {
        let dir = einvoice_xpath_eval::file_uri(&self.0)?;
        let relative = uri.strip_prefix(&dir)?.strip_prefix('/')?;
        if relative.split('/').any(|part| part == "..") {
            return None;
        }
        std::fs::read_to_string(self.0.join(relative)).ok()
    }
}

impl einvoice_xslt::ModuleResolver for Files {
    fn resolve(&self, uri: &str) -> Option<String> {
        self.read(uri)
    }
}

impl einvoice_xslt_exec::DocumentResolver for Files {
    fn resolve(&self, uri: &str) -> Option<String> {
        self.read(uri)
    }
}

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

/// Compares the warnings of a transformation with Saxon's: the number of distinct
/// XTDE0540 warnings (Saxon-HE 12.10's standard error reporter prints each distinct
/// warning once), and the distinct `(line, code)` of the errors recovered while matching
/// patterns. Any other warning is a failure.
fn check_warnings(
    name: &str,
    warnings: &[String],
    expected: &probes::Expected,
    failures: &mut Vec<String>,
) {
    let distinct: std::collections::BTreeSet<&String> = warnings.iter().collect();
    if distinct.len() != warnings.len() {
        failures.push(format!("{name}: a warning is repeated: {warnings:?}"));
    }
    let ambiguities = distinct
        .iter()
        .filter(|w| w.starts_with("XTDE0540: "))
        .count();
    let pattern_errors: std::collections::BTreeSet<(u32, String)> = distinct
        .iter()
        .filter_map(|w| probes::pattern_error(w))
        .collect();
    if distinct
        .iter()
        .any(|w| !w.starts_with("XTDE0540: ") && probes::pattern_error(w).is_none())
    {
        failures.push(format!("{name}: an unknown warning: {warnings:?}"));
    }
    if ambiguities != expected.warnings {
        failures.push(format!(
            "{name}: {ambiguities} distinct XTDE0540 warnings, Saxon {}: {warnings:?}",
            expected.warnings
        ));
    }
    let pattern_errors: Vec<(u32, String)> = pattern_errors.into_iter().collect();
    if pattern_errors != expected.pattern_errors {
        failures.push(format!(
            "{name}: errors recovered in patterns {pattern_errors:?}, Saxon {:?}: {warnings:?}",
            expected.pattern_errors
        ));
    }
}

#[test]
fn every_probe_gives_the_saxon_result() {
    let dir = std::path::absolute(probes::probes_dir()).unwrap();
    let files = Files(dir.clone());
    let input_path = dir.join("input.xml");
    let input = std::fs::read_to_string(&input_path).unwrap();
    let uri = einvoice_xpath_eval::file_uri(&std::path::absolute(&input_path).unwrap());
    let names = probes::probes().unwrap();
    assert!(names.len() >= 25, "{names:?}");
    let mut failures = Vec::new();
    for name in &names {
        let path = dir.join(format!("{name}.xsl"));
        let stylesheet = std::fs::read_to_string(&path).unwrap();
        let expected =
            probes::Expected::parse(&std::fs::read_to_string(probes::expected_path(name)).unwrap())
                .unwrap();
        let module_uri = einvoice_xpath_eval::file_uri(&path).unwrap();
        let sheet = match einvoice_xslt::load_with_imports(&stylesheet, &module_uri, &files) {
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
        let options = einvoice_xslt_exec::TransformOptions {
            resolver: Some(&files),
            ..einvoice_xslt_exec::TransformOptions::default()
        };
        let result = executable.transform_with(input.as_str(), uri.as_deref(), &options);
        // The source given as an owned String (kept by the tree, and as a copy only when
        // document() may load it again) gives the same result.
        let from_owned = executable.transform_with(input.clone(), uri.as_deref(), &options);
        if from_owned != result {
            failures.push(format!(
                "{name}: from an owned String: {from_owned:?}\n  from a &str: {result:?}"
            ));
        }
        match (result, &expected.error) {
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
                check_warnings(name, &ours.warnings, &expected, &mut failures);
                let messages: String = ours.messages.iter().fold(String::new(), |mut all, m| {
                    all.push_str(m);
                    all.push('\n');
                    all
                });
                if messages != expected.messages {
                    failures.push(format!(
                        "{name}: messages {messages:?}, Saxon {:?}",
                        expected.messages
                    ));
                }
            }
            (Err(e), Some(code)) if e.code() == code => {
                // The warnings reported before the error are not compared: the
                // transformation gives none back when it fails.
            }
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
