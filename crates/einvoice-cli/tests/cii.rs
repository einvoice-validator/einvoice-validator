//! The CII rule set from the command line: chosen from the root element of each document
//! when neither `--rules` nor `--bundle` is given, or named with `--rules cen-cii`; every
//! case and mutation case of the CEN EN 16931 CII 1.3.17 corpus agrees with its Saxon-HE
//! golden; runs that mix UBL and CII documents validate each with its own rule set.

#![cfg(test)]

mod common;

use common::json::{self, Value};

const CORPUS: &str = "corpus/cen-en16931-cii-1.3.17";
const CII_VALID: &str = "artefacts/cen-en16931-cii-1.3.17/cii/examples/CII_example1.xml";
const CII_BR_CO_15: &str = "corpus/cen-en16931-cii-1.3.17/vefa/cii/BR-CO-15-2.002.xml";
const UBL_VALID: &str = "artefacts/cen-en16931-ubl-1.3.17/ubl/examples/ubl-tc434-example1.xml";

fn report(run: &common::Run) -> Value {
    json::parse(&run.stdout).unwrap()
}

fn rule_sets(run: &common::Run) -> Vec<String> {
    report(run)
        .get("documents")
        .array()
        .iter()
        .map(|d| d.get("rule_set").str().unwrap().to_owned())
        .collect()
}

fn index(file: &str) -> Vec<(String, String)> {
    std::fs::read_to_string(common::join(
        &common::workspace_root(),
        &format!("{CORPUS}/{file}"),
    ))
    .unwrap()
    .lines()
    .skip(1)
    .map(|line| {
        let mut columns = line.split('\t');
        (
            columns.next().unwrap().to_owned(),
            columns.next().unwrap().to_owned(),
        )
    })
    .collect()
}

#[test]
fn a_cii_document_is_validated_with_cen_cii_by_default() {
    let r = common::run(&[CII_VALID], None);
    assert_eq!((r.code, r.stderr.as_str()), (0, ""));
    assert_eq!(r.stdout, format!("{CII_VALID}: valid\n"));
    let r = common::run(&["--format", "json", CII_VALID, CII_BR_CO_15], None);
    assert_eq!(r.code, 1);
    assert_eq!(rule_sets(&r), ["cen-cii", "cen-cii"]);
    let report = report(&r);
    let rule_set = report.get("rule_set");
    assert_eq!(rule_set.get("name").str(), Some("cen-cii"));
    assert_eq!(rule_set.get("title").str(), Some("CEN EN 16931 CII"));
    assert_eq!(rule_set.get("schema").str(), Some("cii-d16b"));
    let documents = report.get("documents").array();
    assert_eq!(documents[1].get("status").str(), Some("invalid"));
    let ids: Vec<&str> = documents[1]
        .get("findings")
        .array()
        .iter()
        .filter_map(|f| f.get("id").str())
        .collect();
    assert!(ids.contains(&"BR-CO-15"), "{ids:?}");
}

#[test]
fn the_rule_set_can_be_named() {
    let r = common::run(&["--rules", "cen-cii", CII_VALID], None);
    assert_eq!(
        (r.code, r.stdout.as_str()),
        (0, &*format!("{CII_VALID}: valid\n"))
    );
    // The UBL rule set on a CII document: its schema refuses the root element.
    let r = common::run(&["--rules", "cen-ubl", "--format", "json", CII_VALID], None);
    assert_eq!(r.code, 1);
    assert_eq!(rule_sets(&r), ["cen-ubl"]);
    let json = report(&r);
    let document = &json.get("documents").array()[0];
    assert!(document.get("counts").get("schema_errors").number() > 0);
    // And the CII rule set on a UBL document: its schema refuses the root element (no
    // rule context of the stylesheet matches).
    let r = common::run(&["--rules", "cen-cii", "--format", "json", UBL_VALID], None);
    assert_eq!(r.code, 1);
    assert_eq!(rule_sets(&r), ["cen-cii"]);
    let json = report(&r);
    let document = &json.get("documents").array()[0];
    assert_eq!(document.get("counts").get("schema_errors").number(), 1);
    assert_eq!(
        document.get("schema_errors").array()[0].get("kind").str(),
        Some("unknown-root")
    );
    assert_eq!(document.get("findings").array().len(), 0);
}

#[test]
fn mixed_runs_use_the_rule_set_of_each_document() {
    let r = common::run(
        &[
            "--format",
            "json",
            "--jobs",
            "3",
            UBL_VALID,
            CII_VALID,
            CII_BR_CO_15,
            UBL_VALID,
        ],
        None,
    );
    assert_eq!((r.code, r.stderr.as_str()), (1, ""));
    assert_eq!(rule_sets(&r), ["cen-ubl", "cen-cii", "cen-cii", "cen-ubl"]);
    // The rule set of the report is the first document's.
    assert_eq!(
        report(&r).get("rule_set").get("name").str(),
        Some("cen-ubl")
    );
    let r = common::run(&["--format", "json", CII_VALID, UBL_VALID], None);
    assert_eq!(r.code, 0);
    assert_eq!(rule_sets(&r), ["cen-cii", "cen-ubl"]);
    assert_eq!(
        report(&r).get("rule_set").get("name").str(),
        Some("cen-cii")
    );
}

#[test]
fn standard_input_and_unknown_roots() {
    let cii = std::fs::read(common::join(&common::workspace_root(), CII_VALID)).unwrap();
    let r = common::run(&["--format", "json", "-"], Some(&cii));
    assert_eq!(r.code, 0);
    assert_eq!(rule_sets(&r), ["cen-cii"]);
    // Neither UBL nor CII, or not XML: the UBL rule set, as without detection.
    for document in [&b"<a/>"[..], b"not xml", b""] {
        let r = common::run(&["--format", "json", "-"], Some(document));
        assert_eq!(r.code, 1, "{document:?}");
        assert_eq!(rule_sets(&r), ["cen-ubl"], "{document:?}");
    }
    // A file that cannot be read: reported as before, with the UBL rule set.
    let r = common::run(&["--format", "json", "does-not-exist.xml"], None);
    assert_eq!(r.code, 2);
    assert_eq!(rule_sets(&r), ["cen-ubl"]);
}

/// Every case of the CII corpus, without `--rules`: each is detected as CII, its findings
/// agree with the Saxon-HE golden, and it is invalid exactly when the golden has a fatal
/// finding or the document has schema errors.
#[test]
fn the_cii_corpus_agrees_with_the_saxon_goldens() {
    let cases = index("cases.tsv");
    assert_eq!(cases.len(), 24);
    let inputs: Vec<&str> = cases.iter().map(|(_, input)| input.as_str()).collect();
    let runs = common::run_json_batches(&["--format", "json", "--jobs", "4"], &inputs);
    assert_eq!(runs.stderr(), "");
    let documents = runs.documents();
    assert_eq!(documents.len(), cases.len());
    let mut problems = Vec::new();
    for ((id, _), document) in cases.iter().zip(documents) {
        let golden = std::fs::read_to_string(common::join(
            &common::workspace_root(),
            &format!("{CORPUS}/golden/{id}.svrl.txt"),
        ))
        .unwrap();
        let expected = common::golden_findings(&golden);
        let fatal = expected.iter().any(|f| f.2.as_deref() == Some("fatal"));
        let schema_errors = document.get("counts").get("schema_errors").number();
        let status = document.get("status").str().unwrap();
        if status
            != if fatal || schema_errors > 0 {
                "invalid"
            } else {
                "valid"
            }
        {
            problems.push(format!("{id}: {status}"));
        }
        if document.get("rule_set").str() != Some("cen-cii") {
            problems.push(format!("{id}: not validated with cen-cii"));
        }
        if common::json_findings(document) != expected {
            problems.push(format!("{id}: findings differ"));
        }
    }
    assert_eq!(problems, Vec::<String>::new());
}

#[test]
fn the_cii_rule_set_agrees_with_the_saxon_goldens_on_the_mutation_cases() {
    let cases = common::mutation_cases(CORPUS, "");
    assert_eq!(cases.len(), 808);
    let inputs: Vec<&str> = cases.iter().map(|c| c.input.as_str()).collect();
    // Named: two mutation cases have another root element, which detection leaves to the
    // UBL rule set; the goldens are those of the CII stylesheet.
    let runs = common::run_json_batches(
        &["--rules", "cen-cii", "--format", "json", "--jobs", "4"],
        &inputs,
    );
    let documents = runs.documents();
    assert_eq!(documents.len(), cases.len());
    let golden = format!("{CORPUS}/golden");
    let mut problems = Vec::new();
    // The cases deeper than the default maximum depth (256): refused, then compared with
    // Saxon-HE with `--max-depth` raised.
    let mut deep = Vec::new();
    for (case, document) in cases.iter().zip(documents) {
        if document.get("rule_set").str() != Some("cen-cii") {
            problems.push(format!("{}: not validated with cen-cii", case.id));
        }
        let too_deep =
            document.get("status").str() == Some("error")
                && document.get("error").get("message").str().is_some_and(|m| {
                    m.contains("element nested deeper than the maximum depth of 256")
                });
        if too_deep {
            assert_eq!(document.get("error").get("kind").str(), Some("unsupported"));
            deep.push(case);
            continue;
        }
        problems.extend(common::check_mutation_document(
            &case.id,
            document,
            &[("EN16931-CII", golden.as_str())],
        ));
    }
    let ids: Vec<&str> = deep.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["mutations/long/deep-nesting.xml"]);
    let inputs: Vec<&str> = deep.iter().map(|c| c.input.as_str()).collect();
    let raised = common::run_json_batches(
        &[
            "--rules",
            "cen-cii",
            "--format",
            "json",
            "--max-depth",
            "1024",
        ],
        &inputs,
    );
    for (case, document) in deep.iter().zip(raised.documents()) {
        problems.extend(common::check_mutation_document(
            &case.id,
            document,
            &[("EN16931-CII", golden.as_str())],
        ));
    }
    for p in &problems {
        eprintln!("DISAGREE {p}");
    }
    assert_eq!(problems, Vec::<String>::new());
}
