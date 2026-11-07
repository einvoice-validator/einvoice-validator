//! Every XPath expression, pattern, sequence type and attribute value template of the
//! vendored CEN EN 16931 UBL 1.3.17 stylesheet parses, and prints back to text that parses
//! to the same tree.
//!
//! The stylesheet is walked with `roxmltree` directly (not with the XSLT loader), and every
//! attribute of every element is classified, so that nothing XPath-bearing can be skipped:
//! an attribute the table below does not know fails the test.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use einvoice_xpath::{
    AvtPart, Namespaces, parse_avt, parse_expr, parse_pattern, parse_sequence_type,
};

const STYLESHEET: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../artefacts/cen-en16931-ubl-1.3.17/ubl/xslt/EN16931-UBL-validation.xslt"
);
const XSL: &str = "http://www.w3.org/1999/XSL/Transform";

/// What an attribute holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Role {
    Expr,
    Pattern,
    SequenceType,
    Avt,
    /// Names, modes, priorities, output settings: no XPath.
    Plain,
}

/// The role of `attribute` on the XSLT element `element`.
fn xsl_role(element: &str, attribute: &str) -> Option<Role> {
    Some(match (element, attribute) {
        (
            "apply-templates" | "for-each" | "value-of" | "variable" | "param" | "with-param"
            | "sequence" | "attribute",
            "select",
        )
        | ("if" | "when", "test") => Role::Expr,
        ("template", "match") | ("number", "count") => Role::Pattern,
        ("variable" | "param" | "with-param" | "function" | "template", "as") => Role::SequenceType,
        ("attribute", "name") | ("value-of", "separator") => Role::Avt,
        ("stylesheet" | "transform", "version")
        | ("template", "mode" | "priority" | "name")
        | ("apply-templates", "mode")
        | ("variable" | "param" | "with-param" | "function" | "call-template", "name")
        | ("number", "level")
        | ("output", "indent" | "method" | "omit-xml-declaration" | "standalone") => Role::Plain,
        _ => return None,
    })
}

struct InScope<'a, 'input>(roxmltree::Node<'a, 'input>);

impl Namespaces for InScope<'_, '_> {
    fn namespace_uri(&self, prefix: &str) -> Option<String> {
        self.0.lookup_namespace_uri(Some(prefix)).map(str::to_owned)
    }
}

/// Whether `node` is inside a top-level element outside the XSLT namespace (user-defined
/// data, ignored by XSLT processors).
fn in_top_level_data(node: roxmltree::Node<'_, '_>) -> bool {
    node.ancestors()
        .filter(roxmltree::Node::is_element)
        .any(|a| {
            a.parent()
                .is_some_and(|p| p.parent().is_some_and(|r| r.is_root()))
                && a.tag_name().namespace() != Some(XSL)
        })
}

#[test]
fn every_xpath_in_the_cen_stylesheet_parses() {
    let text = std::fs::read_to_string(STYLESHEET).unwrap();
    let doc = roxmltree::Document::parse(&text).unwrap();
    let mut counts: BTreeMap<Role, usize> = BTreeMap::new();
    let mut avt_expressions = 0;
    let mut distinct: BTreeSet<&str> = BTreeSet::new();
    let mut failures = Vec::new();
    // Byte offsets of the line starts (`Document::text_pos_at` rescans the text per call).
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    for node in doc.descendants().filter(roxmltree::Node::is_element) {
        if in_top_level_data(node) {
            continue;
        }
        let ns = InScope(node);
        let line = line_starts.partition_point(|&start| start <= node.range().start);
        let is_xsl = node.tag_name().namespace() == Some(XSL);
        for attribute in node.attributes() {
            let role = if is_xsl {
                assert!(
                    attribute.namespace().is_none(),
                    "line {line}: attribute {attribute:?}"
                );
                xsl_role(node.tag_name().name(), attribute.name())
                    .unwrap_or_else(|| panic!("line {line}: unclassified attribute {attribute:?}"))
            } else {
                Role::Avt
            };
            *counts.entry(role).or_default() += 1;
            let src = attribute.value();
            let printed = match role {
                Role::Expr => {
                    distinct.insert(src);
                    parse_expr(src, &ns).map(|tree| {
                        (
                            tree.to_string(),
                            parse_expr(&tree.to_string(), &ns) == Ok(tree),
                        )
                    })
                }
                Role::Pattern => {
                    distinct.insert(src);
                    parse_pattern(src, &ns).map(|tree| {
                        (
                            tree.to_string(),
                            parse_pattern(&tree.to_string(), &ns) == Ok(tree),
                        )
                    })
                }
                Role::SequenceType => parse_sequence_type(src, &ns).map(|tree| {
                    (
                        tree.to_string(),
                        parse_sequence_type(&tree.to_string(), &ns) == Ok(tree),
                    )
                }),
                Role::Avt => parse_avt(src, &ns).map(|tree| {
                    avt_expressions += tree
                        .parts
                        .iter()
                        .filter(|p| matches!(p, AvtPart::Expr(_)))
                        .count();
                    (
                        tree.to_string(),
                        parse_avt(&tree.to_string(), &ns) == Ok(tree),
                    )
                }),
                Role::Plain => continue,
            };
            match printed {
                Ok((_, true)) => {}
                Ok((again, false)) => failures.push(format!(
                    "line {line} {role:?} {src}: printed {again} parses differently"
                )),
                Err(e) => failures.push(format!("line {line} {role:?} {src}: {e}")),
            }
        }
    }
    for failure in &failures {
        eprintln!("{failure}");
    }
    assert!(failures.is_empty(), "{} failures", failures.len());
    eprintln!(
        "{counts:?}; expressions in attribute value templates: {avt_expressions}; distinct expressions and patterns: {}",
        distinct.len()
    );
    // The counts of the published stylesheet: every select/test (2119), every match/count
    // pattern (131), no `as`, and attribute value templates (2955 xsl:attribute/@name and
    // 1104 literal result element attributes) without a single `{expression}`.
    assert_eq!(counts.get(&Role::Expr), Some(&2119));
    assert_eq!(counts.get(&Role::Pattern), Some(&131));
    assert_eq!(counts.get(&Role::SequenceType), None);
    assert_eq!(counts.get(&Role::Avt), Some(&4059));
    assert_eq!(avt_expressions, 0);
    assert_eq!(distinct.len(), 1041);
}
