//! The public API on every case of the CEN EN 16931 UBL 1.3.17 corpus: the findings
//! (kind, id, flag, location, in order) and the Schematron verdict agree with the
//! Saxon-HE golden, the canonical SVRL equals it, and the overall verdict is invalid
//! exactly when there is a schema error or a fatal finding, the document given as a
//! buffer or borrowed. The documents are spread over threads sharing one `Validator`.

#![cfg(test)]
#![cfg(feature = "cen-ubl")]

mod common;

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use einvoice_validator::{Severity, ValidateOptions, Validator, Verdict, file_uri};

#[derive(Debug, Default)]
struct Tally {
    checked: usize,
    schema_invalid: usize,
    invalid: usize,
    problems: Vec<String>,
}

#[test]
fn every_case_agrees_with_the_saxon_golden() {
    let validator = Validator::cen_ubl().unwrap();
    let cases = common::cases();
    assert_eq!(cases.len(), 1179, "corpus size");
    let next = AtomicUsize::new(0);
    let tally = Mutex::new(Tally::default());
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(16));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(case) = cases.get(index) else { break };
                    let outcome = check(&validator, case);
                    let mut tally = tally.lock().unwrap();
                    tally.checked += 1;
                    match outcome {
                        Ok((schema_invalid, verdict)) => {
                            tally.schema_invalid += usize::from(schema_invalid);
                            tally.invalid += usize::from(verdict == Verdict::Invalid);
                        }
                        Err(problem) => tally.problems.push(format!("{}: {problem}", case.id)),
                    }
                }
            });
        }
    });
    let tally = tally.into_inner().unwrap();
    eprintln!(
        "{} cases: {} agree, {} disagree; {} invalid, {} of them with schema errors",
        tally.checked,
        tally.checked - tally.problems.len(),
        tally.problems.len(),
        tally.invalid,
        tally.schema_invalid
    );
    for problem in tally.problems.iter().take(20) {
        eprintln!("{problem}");
    }
    assert_eq!(tally.checked, 1179);
    assert!(
        tally.problems.is_empty(),
        "{} disagree",
        tally.problems.len()
    );
}

/// Checks one case; returns whether it has schema errors, and its verdict.
fn check(validator: &Validator, case: &common::Case) -> Result<(bool, Verdict), String> {
    let path = std::path::absolute(case.path()).map_err(|e| e.to_string())?;
    let uri = file_uri(&path).ok_or("no file URI")?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let golden = std::fs::read_to_string(case.golden()).map_err(|e| e.to_string())?;
    let options = ValidateOptions::new().document_uri(&uri);
    let report = validator
        .validate_with(&bytes, &options)
        .map_err(|e| format!("error: {e}"))?;
    // The buffer given to the validation instead of borrowed: the same report.
    let from_buffer = validator
        .validate_owned_with(bytes, &options)
        .map_err(|e| format!("error from a buffer: {e}"))?;
    if from_buffer != report {
        return Err("the report from a buffer differs".to_owned());
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
        return Err(format!(
            "findings differ:\n  saxon: {expected:?}\n  ours:  {ours:?}"
        ));
    }
    let saxon_fatal = expected.iter().any(|e| e.2.as_deref() == Some("fatal"));
    if saxon_fatal != (report.count(Severity::Fatal) > 0) {
        return Err("fatal findings differ".to_owned());
    }
    let canonical = report.stylesheets[0]
        .canonical_svrl()
        .map_err(|e| e.to_string())?;
    if canonical != golden {
        return Err("canonical SVRL differs from the golden".to_owned());
    }
    let schema_invalid = !report.schema_errors.is_empty();
    let expected_verdict = if schema_invalid || saxon_fatal {
        Verdict::Invalid
    } else {
        Verdict::Valid
    };
    if report.verdict() != expected_verdict {
        return Err(format!("verdict {}", report.verdict()));
    }
    Ok((schema_invalid, report.verdict()))
}
