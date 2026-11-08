//! The public API on every case of the CEN EN 16931 CII 1.3.17 corpus and on every one of
//! its mutation cases (`corpus/README.md`), with the built-in rule set `cen-cii`: the
//! canonical SVRL of the report equals the Saxon-HE golden, the findings agree with it,
//! the verdict is invalid exactly when there is a schema error (CII D16B) or a fatal
//! finding, and where Saxon-HE stops with a dynamic error the stylesheet stops with the
//! same code: the validation fails with it when the document is valid for the schema,
//! and otherwise returns an invalid report holding it. The schema errors are those of
//! `einvoice_xsd::Schema::cii_d16b` on the document.

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

/// What a case gave: whether it has schema errors, whether Saxon-HE found it valid
/// (no fatal finding, no dynamic error), whether Saxon-HE stopped with a dynamic error,
/// and the verdict.
struct Outcome {
    schema_invalid: bool,
    saxon_valid: bool,
    dynamic_error: bool,
    verdict: Verdict,
}

/// Whether the one stylesheet of the report stopped with the error `code`.
fn stopped_with(report: &einvoice_validator::Report, code: &str) -> bool {
    match &report.stylesheets[..] {
        [stylesheet] => stylesheet.failure.as_ref().is_some_and(|f| f.code == code),
        _ => false,
    }
}

fn check(
    validator: &Validator,
    schema: &einvoice_xsd::Schema,
    id: &str,
    input: &str,
) -> Result<Outcome, String> {
    let root = common::workspace_root();
    let path = std::path::absolute(common::join(&root, input)).map_err(|e| e.to_string())?;
    let uri = file_uri(&path).ok_or("no file URI")?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    // The schema alone; a document deeper than the reader's default maximum depth is
    // validated by the schema all the same (the validator raises its maximum for it).
    let expected_schema_errors = schema
        .validate_bytes(&bytes)
        .map_err(|e| format!("schema: {e}"))?
        .errors;
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
            Err(Error::Transform { code: ours, .. })
                if ours == code && expected_schema_errors.is_empty() =>
            {
                Ok(Outcome {
                    schema_invalid: false,
                    saxon_valid: false,
                    dynamic_error: true,
                    verdict: Verdict::Invalid,
                })
            }
            Ok(report)
                if !expected_schema_errors.is_empty()
                    && report.schema_errors == expected_schema_errors
                    && report.verdict() == Verdict::Invalid
                    && stopped_with(&report, code) =>
            {
                Ok(Outcome {
                    schema_invalid: true,
                    saxon_valid: false,
                    dynamic_error: true,
                    verdict: Verdict::Invalid,
                })
            }
            other => Err(format!(
                "saxon: error {code}, {} schema errors, ours: {other:?}",
                expected_schema_errors.len()
            )),
        };
    }
    let golden = std::fs::read_to_string(common::join(&golden_dir, &format!("{id}.svrl.txt")))
        .map_err(|e| e.to_string())?;
    let report = result.map_err(|e| format!("error: {e}"))?;
    if report.schema_errors != expected_schema_errors {
        return Err(format!(
            "schema errors {:?}, the schema alone {expected_schema_errors:?}",
            report.schema_errors
        ));
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
    let schema_invalid = !report.schema_errors.is_empty();
    if fatal != (report.count(Severity::Fatal) > 0)
        || report.verdict()
            != if fatal || schema_invalid {
                Verdict::Invalid
            } else {
                Verdict::Valid
            }
    {
        return Err(format!("verdict {}", report.verdict()));
    }
    Ok(Outcome {
        schema_invalid,
        saxon_valid: !fatal,
        dynamic_error: false,
        verdict: report.verdict(),
    })
}

/// On the default stack of a test thread (2 MiB): the mutation case
/// `long/deep-nesting.xml`, 304 levels deep, is refused under the default maximum depth,
/// then compared with Saxon-HE with the maximum raised (`common::validate_case`).
#[test]
fn every_cii_case_agrees_with_the_saxon_golden() {
    let validator = Validator::cen_cii().unwrap();
    let schema = einvoice_xsd::Schema::cii_d16b().unwrap();
    let cases = index("cases.tsv");
    let mutations = index("mutations.tsv");
    assert_eq!(cases.len(), 24, "cases");
    assert_eq!(mutations.len(), 808, "mutation cases");
    let mut problems = Vec::new();
    // [upstream cases, mutation cases]: (invalid, with schema errors, valid for Saxon-HE
    // but invalid for the schema, dynamic error of the stylesheet with schema errors).
    let mut tally = [(0, 0, 0, 0); 2];
    for (index, (id, input)) in cases.iter().chain(&mutations).enumerate() {
        match check(&validator, &schema, id, input) {
            Ok(outcome) => {
                let counts = &mut tally[usize::from(index >= cases.len())];
                counts.0 += usize::from(outcome.verdict == Verdict::Invalid);
                counts.1 += usize::from(outcome.schema_invalid);
                counts.2 += usize::from(outcome.schema_invalid && outcome.saxon_valid);
                counts.3 += usize::from(outcome.schema_invalid && outcome.dynamic_error);
            }
            Err(e) => problems.push(format!("{id}: {e}")),
        }
    }
    eprintln!(
        "{} CII cases and mutation cases: {} agree, {} disagree",
        cases.len() + mutations.len(),
        cases.len() + mutations.len() - problems.len(),
        problems.len()
    );
    for (name, (invalid, schema_invalid, only_schema, dynamic)) in
        [("cases", tally[0]), ("mutation cases", tally[1])]
    {
        eprintln!(
            "{name}: {invalid} invalid, {schema_invalid} of them with schema errors \
             ({only_schema} valid for the CEN rules, {dynamic} where the stylesheet stops with \
             a dynamic error)"
        );
    }
    for p in problems.iter().take(20) {
        eprintln!("{p}");
    }
    assert!(problems.is_empty(), "{} disagree", problems.len());
    // The schema errors are those xmllint finds (crates/einvoice-xsd/tests/oracle/cii/):
    // the nine VEFA test documents, which are fragments, and 274 mutation cases.
    assert_eq!(tally[0].1, 9, "upstream cases with schema errors");
    assert_eq!(tally[1].1, 274, "mutation cases with schema errors");
}
