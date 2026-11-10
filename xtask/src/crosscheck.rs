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
//! For an XML Mutate case (`crate::xmute`), the findings are those of the one
//! stylesheet the expectation names: every `error` id must be reported (any
//! flag), no `success` id may be; and for a document the reference pipeline
//! accepts, nothing of its profile's stylesheets may reject it
//! (`crate::acceptance`).
//!
//! Every disagreement is recorded, with its explanation, in the record of the
//! rule set, `corpus/<rule set>/vefa-disagreements.tsv` or
//! `corpus/<rule set>/disagreements.tsv` (committed, also for a fetched rule
//! set); the check fails when the observed disagreements differ from the
//! recorded ones, either way.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use crate::acceptance;
use crate::corpus::{self, Case};
use crate::ruleset::{RuleSet, Stylesheet};
use crate::svrl;
use crate::util::{self, Error, Result};
use crate::vefa::ExpectationKind;

const RECORD_HEADER: &str = "case\tkind\trule\tsaxon\texplanation";

/// One way a golden disagrees with its VEFA expectation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Disagreement {
    pub(crate) case: String,
    /// `missing-error`, `missing-warning`, `success-failed` or
    /// `unexpected-in-scope` (VEFA); `invalid-not-reported`, `valid-reported`
    /// or `stopped` (XML Mutate: Saxon-HE stopped with a dynamic error);
    /// `rejected` (an accepted document).
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
    pub(crate) xmute_cases: usize,
    pub(crate) agreeing_xmute_cases: usize,
    /// Accepted documents checked, and those no stylesheet rejects.
    pub(crate) accepted_cases: usize,
    pub(crate) agreeing_accepted_cases: usize,
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
        if expectation.kind == ExpectationKind::Xmute {
            let found = xmute_disagreements(root, rule_set, case)?;
            summary.xmute_cases += 1;
            let checks = expectation.error.len() + expectation.success.len();
            summary.expectations += checks;
            summary.agreeing_expectations += checks.saturating_sub(found.len());
            if found.is_empty() {
                summary.agreeing_xmute_cases += 1;
            }
            summary.disagreements.extend(found);
            continue;
        }
        summary.vefa_cases += 1;
        let findings = findings(root, rule_set, rule_set.stylesheets.iter(), case)?;
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
    if rule_set.acceptance.is_some() {
        let accepted = acceptance::run(root, rule_set, &cases)?;
        summary.accepted_cases = accepted.cases;
        summary.agreeing_accepted_cases = accepted.agreeing;
        summary.disagreements.extend(accepted.disagreements);
    }
    summary.disagreements.sort();
    Ok(summary)
}

/// The disagreements of an XML Mutate case with its expectation.
fn xmute_disagreements(root: &Path, rule_set: &RuleSet, case: &Case) -> Result<Vec<Disagreement>> {
    let Some(expectation) = &case.expectation else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    let Some(name) = expectation.scope.first() else {
        return Ok(found);
    };
    let stylesheet = rule_set
        .stylesheet(name)
        .ok_or_else(|| Error::new(format!("{}: unknown stylesheet {name}", case.id)))?;
    let error_golden = util::join(
        &rule_set.base(root),
        &case.error_golden(rule_set, stylesheet),
    );
    if error_golden.is_file() {
        // Saxon-HE stopped with a dynamic error: no expectation can hold.
        let text = util::read_string(&error_golden)?;
        let code = text
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("error "))
            .unwrap_or("?");
        found.push(Disagreement {
            case: case.id.clone(),
            kind: "stopped",
            rule: "-".to_owned(),
            saxon: format!("error {code}"),
        });
        return Ok(found);
    }
    let findings = findings(root, rule_set, std::iter::once(stylesheet), case)?;
    let flags_of = |rule: &str| -> BTreeSet<&str> {
        findings
            .iter()
            .filter(|(id, _)| id == rule)
            .map(|(_, flag)| flag.as_str())
            .collect()
    };
    // (rule ids, whether each must be reported, the kind of a disagreement)
    for (rules, must_be_reported, kind) in [
        (&expectation.error, true, "invalid-not-reported"),
        (&expectation.success, false, "valid-reported"),
    ] {
        for rule in rules {
            let flags = flags_of(rule);
            if flags.is_empty() != must_be_reported {
                continue;
            }
            found.push(Disagreement {
                case: case.id.clone(),
                kind,
                rule: rule.clone(),
                saxon: describe(&flags),
            });
        }
    }
    Ok(found)
}

/// (rule id, flag) of every failed assert and successful report of the
/// goldens of a case, the given stylesheets together.
pub(crate) fn findings<'a>(
    root: &Path,
    rule_set: &RuleSet,
    stylesheets: impl Iterator<Item = &'a Stylesheet>,
    case: &Case,
) -> Result<Vec<(String, String)>> {
    Ok(findings_with_kind(root, rule_set, stylesheets, case)?
        .into_iter()
        .map(|(_, id, flag)| (id, flag))
        .collect())
}

/// (kind, rule id, flag) of every failed assert and successful report of the
/// goldens of a case, the given stylesheets together; the flag is `none` when
/// there is none.
pub(crate) fn findings_with_kind<'a>(
    root: &Path,
    rule_set: &RuleSet,
    stylesheets: impl Iterator<Item = &'a Stylesheet>,
    case: &Case,
) -> Result<Vec<(&'static str, String, String)>> {
    let base = rule_set.base(root);
    let mut found = Vec::new();
    for stylesheet in stylesheets {
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
            let kind = if l.kind == "failed-assert" {
                "failed-assert"
            } else {
                "successful-report"
            };
            found.push((kind, id.to_owned(), flag.to_owned()));
        }
    }
    Ok(found)
}

pub(crate) fn describe(flags: &BTreeSet<&str>) -> String {
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
    let path = format!("{}/{}", rule_set.record_dir(), rule_set.record);
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
    let mut parts = Vec::new();
    if summary.vefa_cases > 0 || summary.xmute_cases + summary.accepted_cases == 0 {
        parts.push(format!(
            "{} VEFA cases, {} agree with Saxon, {} disagree",
            summary.vefa_cases,
            summary.agreeing_cases,
            summary.vefa_cases - summary.agreeing_cases
        ));
    }
    if summary.xmute_cases > 0 {
        parts.push(format!(
            "{} XML Mutate cases, {} agree with Saxon, {} disagree",
            summary.xmute_cases,
            summary.agreeing_xmute_cases,
            summary.xmute_cases - summary.agreeing_xmute_cases
        ));
    }
    if summary.vefa_cases + summary.xmute_cases > 0 || summary.accepted_cases == 0 {
        parts.push(format!(
            "{} expectations, {} agree, {} disagree",
            summary.expectations,
            summary.agreeing_expectations,
            summary.expectations - summary.agreeing_expectations
        ));
    }
    if rule_set.acceptance.is_some() {
        // With KoSIT scenarios, the schema of the scenario (xmllint) must accept
        // them too.
        let by = match rule_set.acceptance.as_ref().map(|a| &a.profiles) {
            Some(crate::ruleset::Profiles::KositScenarios { .. }) => "Saxon and xmllint",
            _ => "Saxon",
        };
        parts.push(format!(
            "{} accepted documents, {} accepted by {by}, {} rejected",
            summary.accepted_cases,
            summary.agreeing_accepted_cases,
            summary.accepted_cases - summary.agreeing_accepted_cases
        ));
    }
    let _ = writeln!(
        out,
        "{}: {}; {} disagreements in total",
        rule_set.name,
        parts.join("; "),
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
    use std::fmt::Write as _;

    use super::{Disagreement, report, run};
    use crate::ruleset::{Generator, Origin, RuleSet, Stylesheet, VEFA_RECORD, VENDORED};
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
                side_files: &[],
                generator: Generator::IsoSkeleton,
                schema: None,
            }],
            document_dirs: &[],
            vefa_dirs: &[],
            xmute_dirs: &[],
            xmute_schematrons: &[],
            ignored: &[],
            mutation_catalogue: None,
            mutation_bases: &[],
            acceptance: None,
            record: VEFA_RECORD,
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

    /// The XML Mutate expectations are checked on the stylesheet they name
    /// only, and a dynamic error of Saxon-HE is a disagreement.
    #[test]
    fn detects_xmute_disagreements() {
        const SET: RuleSet = RuleSet {
            name: "synthetic-xmute",
            origin: Origin::Vendored { licence: "LICENSE" },
            source: "src",
            stylesheets: &[
                Stylesheet {
                    name: "S",
                    xslt: "s.xslt",
                    golden_dir: "golden-s",
                    sha256_pin: None,
                    side_files: &[],
                    generator: Generator::IsoSkeleton,
                    schema: None,
                },
                Stylesheet {
                    name: "T",
                    xslt: "t.xslt",
                    golden_dir: "golden-t",
                    sha256_pin: None,
                    side_files: &[],
                    generator: Generator::IsoSkeleton,
                    schema: None,
                },
            ],
            document_dirs: &[],
            vefa_dirs: &[],
            xmute_dirs: &[],
            xmute_schematrons: &[],
            ignored: &[],
            mutation_catalogue: None,
            mutation_bases: &[],
            acceptance: None,
            record: VEFA_RECORD,
        };
        let root = std::env::temp_dir().join(format!("xtask-xmute-{}", std::process::id()));
        // a: R1 must not fail but does, R2 must fail but only T reports it,
        // R3 must fail and does; b: Saxon-HE stopped.
        let index = "case\tinput\tkind\tscope\tsuccess\terror\twarning\tdescription\n\
            xmute/a.xml\tx\txmute\tS\tR1\tR2 R3\t\td\n\
            xmute/b.xml\tx\txmute\tS\t\tR1\t\td\n";
        let dir = "corpus/synthetic-xmute";
        util::write(&root.join(format!("{dir}/cases.tsv")), index.as_bytes()).unwrap();
        let golden = |ids: &[&str]| {
            let mut text = "svrl-canonical 1\nschematron-output\n".to_owned();
            for id in ids {
                let _ = write!(
                    text,
                    "failed-assert flag=\"fatal\" id=\"{id}\" location=\"/\" test=\"t\"\n  text \"m\"\n"
                );
            }
            text
        };
        for (path, text) in [
            ("golden-s/xmute/a.xml.svrl.txt", golden(&["R1", "R3"])),
            ("golden-t/xmute/a.xml.svrl.txt", golden(&["R2"])),
            (
                "golden-s/xmute/b.xml.error.txt",
                "error FORG0001\nmessage m\n".to_owned(),
            ),
            ("golden-t/xmute/b.xml.svrl.txt", golden(&[])),
        ] {
            util::write(&root.join(format!("{dir}/{path}")), text.as_bytes()).unwrap();
        }
        let summary = run(&root, &SET).unwrap();
        util::remove_dir(&root).unwrap();
        let d = |case: &str, kind, rule: &str, saxon: &str| Disagreement {
            case: case.to_owned(),
            kind,
            rule: rule.to_owned(),
            saxon: saxon.to_owned(),
        };
        assert_eq!(
            summary.disagreements,
            [
                d("xmute/a.xml", "invalid-not-reported", "R2", "none"),
                d("xmute/a.xml", "valid-reported", "R1", "fatal"),
                d("xmute/b.xml", "stopped", "-", "error FORG0001"),
            ]
        );
        assert_eq!(summary.xmute_cases, 2);
        assert_eq!(summary.agreeing_xmute_cases, 0);
        assert_eq!(summary.expectations, 4);
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
