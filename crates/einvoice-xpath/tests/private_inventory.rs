//! Local-only check: every distinct expression, pattern, sequence type and attribute value
//! template expression of an inventory file parses, and prints back to text that parses to
//! the same tree.
//!
//! The inventory is a TSV file kept outside the repository, given by the environment
//! variable `EINVOICE_VALIDATOR_PRIVATE_INVENTORY`, with six tab-separated columns: label,
//! line, element, attribute, role (`expr`, `pattern`, `seqtype`, `avt-expr`) and the text.
//! Without the variable the test does nothing.
//!
//! The file carries no namespace declarations, so every prefix is bound to a made-up
//! namespace, except `xs`, which is bound to XML Schema (atomic type names are checked).

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use einvoice_xpath::{Namespaces, XS_NAMESPACE, parse_expr, parse_pattern, parse_sequence_type};

const VARIABLE: &str = "EINVOICE_VALIDATOR_PRIVATE_INVENTORY";

struct AnyPrefix;

impl Namespaces for AnyPrefix {
    fn namespace_uri(&self, prefix: &str) -> Option<String> {
        Some(if prefix == "xs" {
            XS_NAMESPACE.to_owned()
        } else {
            format!("urn:x-inventory:{prefix}")
        })
    }
}

#[test]
fn every_inventoried_expression_parses() {
    let Some(path) = std::env::var_os(VARIABLE) else {
        eprintln!("SKIPPED: {VARIABLE} is not set");
        return;
    };
    let text = std::fs::read_to_string(&path).unwrap();
    let mut rows = 0;
    let mut distinct: BTreeSet<(String, String)> = BTreeSet::new();
    let mut labels: BTreeMap<String, usize> = BTreeMap::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), 6, "malformed inventory line: {line}");
        rows += 1;
        *labels.entry(fields[0].to_owned()).or_default() += 1;
        distinct.insert((fields[4].to_owned(), fields[5].to_owned()));
    }
    let mut per_role: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut failures = Vec::new();
    for (role, src) in &distinct {
        // Parse, print, parse the printed text again: both trees must be equal.
        let result = match role.as_str() {
            "expr" | "avt-expr" => parse_expr(src, &AnyPrefix).and_then(|tree| {
                let printed = tree.to_string();
                parse_expr(&printed, &AnyPrefix).map(|again| (tree == again, printed))
            }),
            "pattern" => parse_pattern(src, &AnyPrefix).and_then(|tree| {
                let printed = tree.to_string();
                parse_pattern(&printed, &AnyPrefix).map(|again| (tree == again, printed))
            }),
            "seqtype" => parse_sequence_type(src, &AnyPrefix).and_then(|tree| {
                let printed = tree.to_string();
                parse_sequence_type(&printed, &AnyPrefix).map(|again| (tree == again, printed))
            }),
            other => panic!("unknown role {other}"),
        };
        let entry = per_role.entry(role.clone()).or_default();
        entry.0 += 1;
        match result {
            Ok((true, _)) => {}
            Ok((false, printed)) => {
                entry.1 += 1;
                failures.push(format!(
                    "{role}: {src}\n    printed as {printed}, which parses differently"
                ));
            }
            Err(e) => {
                entry.1 += 1;
                failures.push(format!("{role}: {src}\n    {e}"));
            }
        }
    }
    let distinct_texts: BTreeSet<&str> = distinct
        .iter()
        .filter(|(role, _)| role == "expr" || role == "pattern")
        .map(|(_, src)| src.as_str())
        .collect();
    eprintln!("inventory rows: {rows} {labels:?}");
    eprintln!(
        "distinct (role, text): {}; distinct expression and pattern texts: {}",
        distinct.len(),
        distinct_texts.len()
    );
    for (role, (total, failed)) in &per_role {
        eprintln!(
            "  {role}: {total} distinct, {} parsed and round-tripped, {failed} failed",
            total - failed
        );
    }
    for failure in &failures {
        eprintln!("FAIL {failure}");
    }
    assert!(
        failures.is_empty(),
        "{} inventoried texts do not parse",
        failures.len()
    );
}
