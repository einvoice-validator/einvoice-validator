//! Equivalence with xmllint (libxml2 2.9.14): every case of the oracle (the CEN
//! EN 16931 UBL corpus, `tests/oracle/documents/` and the mutations of
//! `tests/oracle/mutations.tsv`) is validated with this crate and compared with what
//! xmllint said (`tests/oracle/expected.tsv`, written by the `xmllint-oracle`
//! example). Every difference must be documented, with its cause, in
//! `tests/oracle/disagreements.tsv`, and every documented disagreement must still
//! occur. Needs neither xmllint nor libxml2.
#![cfg(test)]

#[path = "oracle/format.rs"]
mod format;

use std::collections::BTreeMap;

fn load() -> (
    Vec<format::Case>,
    BTreeMap<String, format::Expected>,
    Vec<format::Disagreement>,
) {
    let cases = format::cases().unwrap();
    let expected_text = format::read(&format::oracle_dir().join("expected.tsv")).unwrap();
    let expected = format::parse_expected(&expected_text).unwrap();
    let documented = format::parse_disagreements(
        &format::read(&format::oracle_dir().join("disagreements.tsv")).unwrap(),
    )
    .unwrap();
    (cases, expected, documented)
}

#[test]
fn agrees_with_xmllint_except_documented_disagreements() {
    let (cases, expected, documented) = load();
    let schema = einvoice_xsd::Schema::ubl_2_1().unwrap();
    let differences = format::differences(&schema, &cases, &expected).unwrap();
    let problems = format::unexplained(&differences, &documented);
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
    let verdicts = differences.iter().filter(|d| d.kind == "verdict").count();
    assert!(
        verdicts * 20 < cases.len(),
        "{verdicts} verdict disagreements out of {} cases",
        cases.len()
    );
}

#[test]
fn every_corpus_verdict_agrees() {
    // The official examples and test documents: no disagreement is tolerated there.
    let (cases, expected, _) = load();
    let schema = einvoice_xsd::Schema::ubl_2_1().unwrap();
    let corpus: Vec<format::Case> = cases
        .into_iter()
        .filter(|c| c.id.starts_with("corpus/"))
        .collect();
    assert!(corpus.len() > 1000, "{} corpus cases", corpus.len());
    for case in &corpus {
        let ours = format::observe(&schema, case);
        assert_eq!(Some(&ours), expected.get(&case.id), "{}", case.id);
    }
}

#[test]
fn xades_disagreements_concern_xades_content() {
    // A disagreement put down to the XAdES schemas not being shipped is on a mutation
    // whose document has XAdES content; none is on a corpus case.
    let (cases, _, documented) = load();
    let xades: Vec<&format::Disagreement> = documented
        .iter()
        .filter(|x| format::caused_by_unshipped_xades(x))
        .collect();
    assert!(xades.len() > 40, "{} XAdES disagreements", xades.len());
    for x in xades {
        assert!(x.case.starts_with("mutation/"), "{}", x.case);
        let case = cases.iter().find(|c| c.id == x.case).unwrap();
        assert!(
            einvoice_xsd::XADES_NAMESPACES
                .iter()
                .any(|ns| case.text.contains(ns)),
            "{}",
            x.case
        );
        assert!(
            x.explanation.len() > format::XADES_NOT_SHIPPED.len() + 20,
            "{}: the explanation must say what XAdES would check",
            x.case
        );
    }
}

#[test]
fn expected_file_is_canonical() {
    let (_, expected, _) = load();
    let text = format::read(&format::oracle_dir().join("expected.tsv")).unwrap();
    let rewritten = format::format_expected(
        &expected
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Vec<_>>(),
    );
    // expected.tsv lists cases in case order, the map in key order: compare sets.
    let mut original: Vec<&str> = text.lines().collect();
    let mut again: Vec<&str> = rewritten.lines().collect();
    original.sort_unstable();
    again.sort_unstable();
    assert_eq!(original, again);
    assert_eq!(format::EXPECTED_HEADER, "case\tverdict\terrors");
}

#[test]
fn mutations_apply_exactly_once() {
    // `format::cases` fails on a find text that occurs zero or several times.
    let cases = format::cases().unwrap();
    let mutations = cases
        .iter()
        .filter(|c| c.id.starts_with("mutation/"))
        .count();
    assert!(mutations > 300, "{mutations} mutations");
    assert_eq!(
        format::apply("a b a", &[("b".to_owned(), "c".to_owned())]).unwrap(),
        "a c a"
    );
    assert!(format::apply("a b a", &[("a".to_owned(), "c".to_owned())]).is_err());
    assert!(format::apply("a b a", &[("x".to_owned(), "c".to_owned())]).is_err());
    let documents = format::workspace_root().join(format::CORPUS);
    assert!(documents.is_dir());
    assert!(
        format::crate_dir()
            .join("examples/xmllint-oracle.rs")
            .is_file()
    );
}
