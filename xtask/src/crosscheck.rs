//! Cross-check of the goldens against the VEFA expectations: for every VEFA
//! case, the rule ids Saxon reports must be the ones the test set expects.
//!
//! The findings of a case are the failed asserts and successful reports, with
//! their flag, of its goldens: of every stylesheet of the rule set together
//! (Peppol BIS Billing runs its own rules and the CEN rules on each document).
//! For a case with scope S and expectations error E, warning W, success U, the
//! findings agree when:
//!
//! - every id of E is reported with flag `fatal`;
//! - every id of W is reported with flag `warning`;
//! - no id of U is reported, whatever the flag;
//! - every reported id of S is in E or W (nothing unexpected in scope).
//!
//! Every disagreement is recorded, with its explanation, in
//! `corpus/<rule set>/vefa-disagreements.tsv` (committed, also for a fetched
//! rule set); the check fails when the observed disagreements differ from the
//! recorded ones, either way.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use crate::corpus::{self, Case};
use crate::ruleset::RuleSet;
use crate::svrl;
use crate::util::{self, Error, Result};

/// The record of known disagreements, in the corpus directory.
pub(crate) const RECORD: &str = "vefa-disagreements.tsv";
const RECORD_HEADER: &str = "case\tkind\trule\tsaxon\texplanation";

/// One way a golden disagrees with its VEFA expectation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Disagreement {
    pub(crate) case: String,
    /// `missing-error`, `missing-warning`, `success-failed` or
    /// `unexpected-in-scope`.
    pub(crate) kind: &'static str,
    pub(crate) rule: String,
    /// The flags Saxon reported for the rule (`none` when not reported).
    pub(crate) saxon: String,
}

/// Result of a cross-check.
#[derive(Debug, Default)]
pub(crate) struct Summary {
    pub(crate) vefa_cases: usize,
    pub(crate) agreeing_cases: usize,
    /// Individual expectations (one per expected id) checked and agreeing.
    pub(crate) expectations: usize,
    pub(crate) agreeing_expectations: usize,
    pub(crate) disagreements: Vec<Disagreement>,
}

/// Compares every VEFA case's golden with its expectation.
pub(crate) fn run(root: &Path, rule_set: &RuleSet) -> Result<Summary> {
    let cases = corpus::read_index(root, rule_set)?;
    let mut summary = Summary::default();
    for case in &cases {
        let Some(expectation) = &case.expectation else {
            continue;
        };
        summary.vefa_cases += 1;
        let findings = findings(root, rule_set, case)?;
        let flags_of = |rule: &str| -> BTreeSet<&str> {
            findings
                .iter()
                .filter(|(id, _)| id == rule)
                .map(|(_, flag)| flag.as_str())
                .collect()
        };
        let mut found = Vec::new();
        let mut disagree = |kind: &'static str, rule: &str, flags: &BTreeSet<&str>| {
            found.push(Disagreement {
                case: case.id.clone(),
                kind,
                rule: rule.to_owned(),
                saxon: describe(flags),
            });
        };

        let checks =
            expectation.error.len() + expectation.warning.len() + expectation.success.len();
        let mut failed_checks = 0;
        for (expected, flag, kind) in [
            (&expectation.error, "fatal", "missing-error"),
            (&expectation.warning, "warning", "missing-warning"),
        ] {
            for rule in expected {
                let flags = flags_of(rule);
                if !flags.contains(flag) {
                    failed_checks += 1;
                    disagree(kind, rule, &flags);
                }
            }
        }
        for rule in &expectation.success {
            let flags = flags_of(rule);
            if !flags.is_empty() {
                failed_checks += 1;
                disagree("success-failed", rule, &flags);
            }
        }
        let reported: BTreeSet<&str> = findings.iter().map(|(id, _)| id.as_str()).collect();
        for rule in reported {
            let in_scope = expectation.scope.iter().any(|s| s == rule);
            let expected = expectation
                .error
                .iter()
                .chain(&expectation.warning)
                .any(|e| e == rule);
            let success = expectation.success.iter().any(|s| s == rule);
            if in_scope && !expected && !success {
                disagree("unexpected-in-scope", rule, &flags_of(rule));
            }
        }

        summary.expectations += checks;
        summary.agreeing_expectations += checks - failed_checks;
        if found.is_empty() {
            summary.agreeing_cases += 1;
        }
        summary.disagreements.extend(found);
    }
    summary.disagreements.sort();
    Ok(summary)
}

/// (rule id, flag) of every failed assert and successful report of the
/// goldens of a case, every stylesheet of the rule set together.
fn findings(root: &Path, rule_set: &RuleSet, case: &Case) -> Result<Vec<(String, String)>> {
    let base = rule_set.base(root);
    let mut found = Vec::new();
    for stylesheet in rule_set.stylesheets {
        let path = case.golden(rule_set, stylesheet);
        let text = util::read_string(&util::join(&base, &path))?;
        let lines = svrl::parse(&text).map_err(|e| Error::new(format!("{path}: {e}")))?;
        for l in lines
            .iter()
            .filter(|l| l.kind == "failed-assert" || l.kind == "successful-report")
        {
            let id = l
                .attribute("id")
                .ok_or_else(|| Error::new(format!("{path}: {} without id", l.kind)))?;
            let flag = l.attribute("flag").unwrap_or("none");
            found.push((id.to_owned(), flag.to_owned()));
        }
    }
    Ok(found)
}

fn describe(flags: &BTreeSet<&str>) -> String {
    if flags.is_empty() {
        "none".to_owned()
    } else {
        flags.iter().copied().collect::<Vec<_>>().join(",")
    }
}

/// A recorded disagreement and its explanation.
#[derive(Debug)]
pub(crate) struct Recorded {
    pub(crate) disagreement: (String, String, String, String),
    pub(crate) explanation: String,
}

/// Reads the record of known disagreements.
pub(crate) fn read_record(root: &Path, rule_set: &RuleSet) -> Result<Vec<Recorded>> {
    let path = format!("{}/{RECORD}", rule_set.record_dir());
    let text = util::read_string(&util::join(root, &path))?;
    let mut lines = text.lines();
    if lines.next() != Some(RECORD_HEADER) {
        return Err(Error::new(format!("{path}: unexpected header")));
    }
    lines
        .enumerate()
        .map(
            |(index, line)| match line.split('\t').collect::<Vec<_>>()[..] {
                [case, kind, rule, saxon, explanation] if !explanation.trim().is_empty() => {
                    Ok(Recorded {
                        disagreement: (
                            case.to_owned(),
                            kind.to_owned(),
                            rule.to_owned(),
                            saxon.to_owned(),
                        ),
                        explanation: explanation.to_owned(),
                    })
                }
                _ => Err(Error::new(format!(
                    "{path}: line {}: expected 5 tab-separated fields with a non-empty explanation",
                    index + 2
                ))),
            },
        )
        .collect()
}

fn key(d: &Disagreement) -> (String, String, String, String) {
    (
        d.case.clone(),
        d.kind.to_owned(),
        d.rule.clone(),
        d.saxon.clone(),
    )
}

/// Human-readable report: the counts, every disagreement with its recorded
/// explanation, and the differences between observed and recorded ones.
/// Returns the report and whether observed and recorded agree.
pub(crate) fn report(root: &Path, rule_set: &RuleSet) -> Result<(String, bool)> {
    let summary = run(root, rule_set)?;
    let recorded = read_record(root, rule_set)?;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{}: {} VEFA cases, {} agree with Saxon, {} disagree; {} expectations, {} agree, {} disagree; {} disagreements in total",
        rule_set.name,
        summary.vefa_cases,
        summary.agreeing_cases,
        summary.vefa_cases - summary.agreeing_cases,
        summary.expectations,
        summary.agreeing_expectations,
        summary.expectations - summary.agreeing_expectations,
        summary.disagreements.len()
    );
    let mut consistent = true;
    for d in &summary.disagreements {
        let explanation = recorded
            .iter()
            .find(|r| r.disagreement == key(d))
            .map(|r| r.explanation.as_str());
        if explanation.is_none() {
            consistent = false;
        }
        let _ = writeln!(
            out,
            "  {} {} {} (saxon: {}): {}",
            d.case,
            d.kind,
            d.rule,
            d.saxon,
            explanation.unwrap_or("NOT RECORDED")
        );
    }
    for r in &recorded {
        if !summary
            .disagreements
            .iter()
            .any(|d| key(d) == r.disagreement)
        {
            consistent = false;
            let (case, kind, rule, saxon) = &r.disagreement;
            let _ = writeln!(
                out,
                "  recorded but not observed: {case} {kind} {rule} (saxon: {saxon})"
            );
        }
    }
    Ok((out, consistent))
}

#[cfg(test)]
mod tests {
    use super::{Disagreement, report, run};
    use crate::ruleset::{Origin, RuleSet, Stylesheet, VENDORED};
    use crate::util::{self, repo_root};

    /// The checker finds each kind of disagreement (a negative control for
    /// the real corpus, where it finds none).
    #[test]
    fn detects_every_kind_of_disagreement() {
        const SET: RuleSet = RuleSet {
            name: "synthetic",
            origin: Origin::Vendored { licence: "LICENSE" },
            source: "artefacts/synthetic",
            stylesheets: &[Stylesheet {
                name: "x",
                xslt: "x.xslt",
                golden_dir: "golden",
                sha256_pin: None,
            }],
            document_dirs: &[],
            vefa_dirs: &[],
            ignored: &[],
            mutation_catalogue: "",
            mutation_bases: &[],
        };
        let root = std::env::temp_dir().join(format!("xtask-crosscheck-{}", std::process::id()));
        let index = "case\tinput\tkind\tscope\tsuccess\terror\twarning\tdescription\n\
            vefa/a.xml\tx\tvefa\tR1 R2 R3 R4 R5\tR1\tR2 R3\tR4\td\n\
            vefa/b.xml\tx\tvefa\tR1\tR1\t\t\td\n\
            whole.xml\tx\tdocument\n";
        util::write(&root.join("corpus/synthetic/cases.tsv"), index.as_bytes()).unwrap();
        let finding = |id: &str, flag: &str| {
            format!(
                "failed-assert flag=\"{flag}\" id=\"{id}\" location=\"/\" test=\"t\"\n  text \"m\"\n"
            )
        };
        // a: R1 fails although it should succeed, R2 is missing, R3 comes as a
        // warning instead of fatal, R4 (warning expected) is missing, R5 is
        // in scope but not expected, R9 is out of scope (ignored).
        let a = format!(
            "svrl-canonical 1\nschematron-output\n{}{}{}{}",
            finding("R1", "fatal"),
            finding("R3", "warning"),
            finding("R5", "fatal"),
            finding("R9", "fatal")
        );
        let b = "svrl-canonical 1\nschematron-output\n".to_owned();
        util::write(
            &root.join("corpus/synthetic/golden/vefa/a.xml.svrl.txt"),
            a.as_bytes(),
        )
        .unwrap();
        util::write(
            &root.join("corpus/synthetic/golden/vefa/b.xml.svrl.txt"),
            b.as_bytes(),
        )
        .unwrap();

        let summary = run(&root, &SET).unwrap();
        util::remove_dir(&root).unwrap();
        let d = |kind, rule: &str, saxon: &str| Disagreement {
            case: "vefa/a.xml".to_owned(),
            kind,
            rule: rule.to_owned(),
            saxon: saxon.to_owned(),
        };
        assert_eq!(
            summary.disagreements,
            [
                d("missing-error", "R2", "none"),
                d("missing-error", "R3", "warning"),
                d("missing-warning", "R4", "none"),
                d("success-failed", "R1", "fatal"),
                d("unexpected-in-scope", "R5", "fatal"),
            ]
        );
        assert_eq!(summary.vefa_cases, 2);
        assert_eq!(summary.agreeing_cases, 1);
        assert_eq!(summary.expectations, 5);
        assert_eq!(summary.agreeing_expectations, 1);
    }

    /// The goldens agree with the VEFA expectations, except for the recorded
    /// and explained disagreements, and every recorded one still occurs.
    #[test]
    fn goldens_agree_with_vefa_except_recorded_disagreements() {
        for rule_set in VENDORED {
            let (text, consistent) =
                report(&repo_root(), rule_set).unwrap_or_else(|e| panic!("{e}"));
            println!("{text}");
            assert!(consistent, "{text}");
        }
    }
}
