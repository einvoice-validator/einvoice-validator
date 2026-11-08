//! The public API on every case of the CEN EN 16931 CII 1.3.17 corpus and on every one of
//! its mutation cases (`corpus/README.md`), with the built-in rule set `cen-cii`: the
//! canonical SVRL of the report equals the Saxon-HE golden, the findings and the verdict
//! agree with it, and where Saxon-HE stops with a dynamic error the validation fails with
//! the same code (there is no schema, so there is no schema error to keep a verdict).

#![cfg(test)]
#![cfg(feature = "cen-cii")]

mod common;

use einvoice_validator::{Error, Severity, ValidateOptions, Validator, Verdict, file_uri};

const CORPUS: &str = "corpus/cen-en16931-cii-1.3.17";

/// (case id, input relative to the workspace root) of `cases.tsv` or `mutations.tsv`.
fn index(file: &str) -> Vec<(String, String)> {
    let path = common::join(&common::workspace_root(), &format!("{CORPUS}/{file}"));
    std::fs::read_to_string(path)
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

fn check(validator: &Validator, id: &str, input: &str) -> Result<(), String> {
    let root = common::workspace_root();
    let path = std::path::absolute(common::join(&root, input)).map_err(|e| e.to_string())?;
    let uri = file_uri(&path).ok_or("no file URI")?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let golden_dir = common::join(&root, &format!("{CORPUS}/golden"));
    let error_golden = common::join(&golden_dir, &format!("{id}.error.txt"));
    let result =
        common::validate_case(validator, bytes, &ValidateOptions::new().document_uri(&uri));
    if error_golden.exists() {
        let text = std::fs::read_to_string(&error_golden).map_err(|e| e.to_string())?;
        let code = text
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("error "))
            .ok_or("malformed error golden")?;
        return match result {
            Err(Error::Transform { code: ours, .. }) if ours == code => Ok(()),
            other => Err(format!("saxon: error {code}, ours: {other:?}")),
        };
    }
    let golden = std::fs::read_to_string(common::join(&golden_dir, &format!("{id}.svrl.txt")))
        .map_err(|e| e.to_string())?;
    let report = result.map_err(|e| format!("error: {e}"))?;
    if !report.schema_errors.is_empty() {
        return Err("schema errors without a schema".to_owned());
    }
    let [stylesheet] = &report.stylesheets[..] else {
        return Err("not one stylesheet".to_owned());
    };
    if stylesheet.canonical_svrl().map_err(|e| e.to_string())? != golden {
        return Err("canonical SVRL differs from the golden".to_owned());
    }
    let expected = common::expected_findings(&golden);
    let ours: Vec<common::Expected> = report
        .findings()
        .map(|f| {
            (
                f.kind.as_str().to_owned(),
                f.id.clone(),
                f.flag.clone(),
                f.location.clone(),
            )
        })
        .collect();
    if ours != expected {
        return Err("findings differ".to_owned());
    }
    let fatal = expected.iter().any(|e| e.2.as_deref() == Some("fatal"));
    if fatal != (report.count(Severity::Fatal) > 0)
        || report.verdict()
            != if fatal {
                Verdict::Invalid
            } else {
                Verdict::Valid
            }
    {
        return Err(format!("verdict {}", report.verdict()));
    }
    Ok(())
}

/// On the default stack of a test thread (2 MiB): the mutation case
/// `long/deep-nesting.xml`, 304 levels deep, is refused under the default maximum depth,
/// then compared with Saxon-HE with the maximum raised (`common::validate_case`).
#[test]
fn every_cii_case_agrees_with_the_saxon_golden() {
    let validator = Validator::cen_cii().unwrap();
    let cases = index("cases.tsv");
    let mutations = index("mutations.tsv");
    assert_eq!(cases.len(), 24, "cases");
    assert_eq!(mutations.len(), 808, "mutation cases");
    let problems: Vec<String> = cases
        .iter()
        .chain(&mutations)
        .filter_map(|(id, input)| {
            check(&validator, id, input)
                .err()
                .map(|e| format!("{id}: {e}"))
        })
        .collect();
    eprintln!(
        "{} CII cases and mutation cases: {} agree, {} disagree",
        cases.len() + mutations.len(),
        cases.len() + mutations.len() - problems.len(),
        problems.len()
    );
    for p in problems.iter().take(20) {
        eprintln!("{p}");
    }
    assert!(problems.is_empty(), "{} disagree", problems.len());
}
