//! The Peppol BIS Billing 3.0.22 rule set as an external bundle: the directory that
//! `scripts/fetch-peppol.sh` builds (its two compiled stylesheets and the `bundle.toml`
//! that names them), given to `einvoice-validate --bundle`. On every case of the Peppol
//! corpus, the verdict and the findings of each stylesheet (kind, id, flag, location, in
//! order) agree with the Saxon-HE goldens.
//!
//! The Peppol files may not be redistributed, so the bundle, the corpus and its goldens
//! live in a cache outside the tracked tree, built by `scripts/check.sh peppol`
//! (`scripts/fetch-peppol.sh`, then `cargo xtask peppol`; see `corpus/README.md`).
//! Without the cache this test says SKIPPED and passes, unless `EINVOICE_PEPPOL_REQUIRED`
//! is set, as `scripts/check.sh peppol` and the `peppol` CI workflow do: then it fails.

#![cfg(test)]

mod common;

use common::json::{self, Value};
use common::{Finding, join, run, workspace_root};

/// The cache directory of the rule set, relative to the workspace root: the bundle.
const BASE: &str = "target/peppol/peppol-bis-billing-ubl-3.0.22";
const REQUIRED_VAR: &str = "EINVOICE_PEPPOL_REQUIRED";
/// The stylesheets of the bundle, in its order, with their golden directories.
const STYLESHEETS: [(&str, &str); 2] = [
    ("CEN-EN16931-UBL", "golden-cen"),
    ("PEPPOL-EN16931-UBL", "golden-peppol"),
];

/// One case: its id and its input, relative to the workspace root.
struct Case {
    id: String,
    input: String,
}

/// The cases of the cached corpus, `None` when the cache or the bundle is absent or the
/// corpus incomplete (no stamp).
fn cases() -> Option<Vec<Case>> {
    let base = join(&workspace_root(), BASE);
    if !base.join("bundle.toml").is_file() {
        return None;
    }
    let stamp = std::fs::read_to_string(base.join("corpus").join("complete.txt")).ok()?;
    let count: usize = stamp
        .lines()
        .find_map(|l| l.strip_prefix("cases "))
        .unwrap_or_else(|| panic!("{BASE}/corpus/complete.txt: no `cases` line"))
        .parse()
        .unwrap();
    let tsv = std::fs::read_to_string(base.join("corpus").join("cases.tsv")).unwrap();
    let cases: Vec<Case> = tsv
        .lines()
        .skip(1)
        .map(|line| {
            let mut columns = line.split('\t');
            let id = columns.next().unwrap().to_owned();
            let input = format!("{BASE}/{}", columns.next().unwrap());
            Case { id, input }
        })
        .collect();
    assert_eq!(
        cases.len(),
        count,
        "{BASE}/corpus: cases.tsv and the stamp disagree"
    );
    Some(cases)
}

fn golden(directory: &str, case: &Case) -> String {
    std::fs::read_to_string(join(
        &workspace_root(),
        &format!("{BASE}/corpus/{directory}/{}.svrl.txt", case.id),
    ))
    .unwrap()
}

/// The findings of one stylesheet in one document of the JSON report.
fn findings_of(document: &Value, stylesheet: &str) -> Vec<Finding> {
    document
        .get("findings")
        .array()
        .iter()
        .filter(|f| f.get("stylesheet").str() == Some(stylesheet))
        .map(|f| {
            (
                f.get("kind").str().unwrap().to_owned(),
                f.get("id").str().map(str::to_owned),
                f.get("flag").str().map(str::to_owned),
                f.get("location").str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn the_peppol_bundle_agrees_with_the_saxon_goldens() {
    let Some(cases) = cases() else {
        let message = format!(
            "no Peppol bundle and corpus in {BASE}: build them with scripts/check.sh peppol (needs Java and network access)"
        );
        assert!(
            std::env::var_os(REQUIRED_VAR).is_none_or(|v| v.is_empty()),
            "{message}; {REQUIRED_VAR} is set"
        );
        eprintln!("SKIPPED: {message}");
        return;
    };
    assert!(!cases.is_empty(), "empty Peppol corpus");
    let mut args = vec!["--bundle", BASE, "--format", "json", "--jobs", "4"];
    args.extend(cases.iter().map(|c| c.input.as_str()));
    let r = run(&args, None);
    assert_eq!(r.stderr, "");
    let report = json::parse(&r.stdout).unwrap();
    let rule_set = report.get("rule_set");
    assert_eq!(rule_set.get("name").str(), Some("peppol-bis-billing-ubl"));
    // The version of the release, as the directory name gives it.
    assert_eq!(rule_set.get("version").str(), BASE.rsplit('-').next());
    let names: Vec<&str> = rule_set
        .get("stylesheets")
        .array()
        .iter()
        .map(|s| s.get("name").str().unwrap())
        .collect();
    assert_eq!(names, STYLESHEETS.map(|(name, _)| name));
    let documents = report.get("documents").array();
    assert_eq!(documents.len(), cases.len());
    let mut problems = Vec::new();
    let (mut valid, mut invalid) = (0, 0);
    for (case, document) in cases.iter().zip(documents) {
        assert_eq!(document.get("path").str(), Some(case.input.as_str()));
        let mut fatal = false;
        for (stylesheet, directory) in STYLESHEETS {
            let expected = common::golden_findings(&golden(directory, case));
            fatal |= expected.iter().any(|f| f.2.as_deref() == Some("fatal"));
            if findings_of(document, stylesheet) != expected {
                problems.push(format!("{}: findings of {stylesheet} differ", case.id));
            }
        }
        let schema_errors = document.get("counts").get("schema_errors").number();
        let expected_status = if fatal || schema_errors > 0 {
            "invalid"
        } else {
            "valid"
        };
        let status = document.get("status").str().unwrap();
        if status != expected_status {
            problems.push(format!("{}: {status}, expected {expected_status}", case.id));
        }
        valid += usize::from(status == "valid");
        invalid += usize::from(status == "invalid");
    }
    eprintln!(
        "{} documents: {valid} valid, {invalid} invalid, {} disagreements",
        cases.len(),
        problems.len()
    );
    assert_eq!(problems, Vec::<String>::new());
    assert_eq!(valid + invalid, cases.len());
    let summary = report.get("summary");
    assert_eq!(summary.get("errors").number(), 0);
    assert_eq!(r.code, i32::from(invalid > 0), "exit status");
}

#[test]
fn the_peppol_bundle_agrees_with_the_saxon_goldens_on_the_mutation_cases() {
    if cases().is_none() {
        assert!(
            std::env::var_os(REQUIRED_VAR).is_none_or(|v| v.is_empty()),
            "no Peppol bundle and corpus in {BASE}; {REQUIRED_VAR} is set"
        );
        eprintln!("SKIPPED: no Peppol bundle and corpus in {BASE}");
        return;
    }
    let cases = common::mutation_cases(&format!("{BASE}/corpus"), BASE);
    assert!(!cases.is_empty());
    let mut args = vec!["--bundle", BASE, "--format", "json", "--jobs", "4"];
    args.extend(cases.iter().map(|c| c.input.as_str()));
    let r = run(&args, None);
    let report = json::parse(&r.stdout).unwrap();
    let documents = report.get("documents").array();
    assert_eq!(documents.len(), cases.len());
    let stylesheets: Vec<(&str, String)> = STYLESHEETS
        .iter()
        .map(|(name, dir)| (*name, format!("{BASE}/corpus/{dir}")))
        .collect();
    let stylesheets: Vec<(&str, &str)> =
        stylesheets.iter().map(|(n, d)| (*n, d.as_str())).collect();
    let mut problems = Vec::new();
    for (case, document) in cases.iter().zip(documents) {
        assert_eq!(document.get("path").str(), Some(case.input.as_str()));
        problems.extend(common::check_mutation_document(
            &case.id,
            document,
            &stylesheets,
        ));
    }
    eprintln!(
        "{} mutation documents, {} disagreements",
        cases.len(),
        problems.len()
    );
    for p in &problems {
        eprintln!("DISAGREE {p}");
    }
    assert_eq!(problems, Vec::<String>::new());
}
