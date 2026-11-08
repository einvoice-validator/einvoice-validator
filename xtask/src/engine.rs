//! The state of the engine on the corpora of the fetched rule sets: for each
//! stylesheet, `einvoice-validate` (a release build, given with `--cli`, by
//! default `target/release/einvoice-validate`) runs a bundle of that one
//! stylesheet (and the files it reads with `document()`) on every case, with
//! `--format svrl`; its report, in canonical form, is compared with the
//! Saxon-HE golden. A case deeper than the default maximum depth runs with
//! the maximum raised (`--max-depth`), as in the equivalence tests. This is a
//! report, not a check: it never fails on a difference. It is written to
//! `<corpus>/engine-status.txt` in the cache and summarised on standard
//! output.
//!
//! Each case ends in one of: `pass` (the canonical report equals the golden,
//! or the engine stops with the error code Saxon-HE stopped with), `differ`
//! (the first differing line is shown), `refused` (the bundle does not load:
//! a construct outside what the engine supports, for every case), or `failed`
//! (the engine stops on the document where Saxon-HE does not, or with another
//! error).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::corpus::{self, Case};
use crate::fetched;
use crate::ruleset::{Family, RuleSet, Stylesheet};
use crate::svrl;
use crate::util::{self, Error, Result};

/// The default command-line tool, relative to the repository root.
pub(crate) const CLI_DEFAULT: &str = "target/release/einvoice-validate";
/// The report, relative to the corpus directory.
pub(crate) const REPORT: &str = "engine-status.txt";

/// What happened to one case.
#[derive(Debug, Clone)]
enum CaseResult {
    Pass,
    Differ(String),
    Failed(String),
}

/// The outcome of one stylesheet.
#[derive(Debug, Default)]
struct Outcome {
    cases: usize,
    pass: usize,
    refused: Option<String>,
    differ: Vec<(String, String)>,
    failed: Vec<(String, String)>,
}

/// Runs the engine on every corpus of the family and writes the reports.
pub(crate) fn run(root: &Path, family: &Family, cli: &Path) -> Result<String> {
    if !cli.is_file() {
        return Err(Error::new(format!(
            "{} not found: build it with `cargo build --release -p einvoice-cli` (or pass --cli)",
            cli.display()
        )));
    }
    let cli =
        std::path::absolute(cli).map_err(|e| Error::new(format!("{}: {e}", cli.display())))?;
    let mut summary = String::new();
    for rule_set in family.rule_sets {
        if !fetched::stamp(root, rule_set).exists() {
            return Err(Error::new(format!(
                "{}: no complete corpus: run `cargo xtask {}` first",
                rule_set.name, family.name
            )));
        }
        fetched::check_cache(root, family, rule_set)?;
        let cases = corpus::read_all(root, rule_set)?;
        let base = rule_set.base(root);
        let corpus_dir = util::join(&base, &rule_set.corpus_dir());
        let mut report = format!(
            "Engine status of {} ({} cases): {} --format svrl, one bundle per stylesheet\n",
            rule_set.name,
            cases.len(),
            cli.display()
        );
        for stylesheet in rule_set.stylesheets {
            eprintln!(
                "engine-status: {} {} on {} cases",
                rule_set.name,
                stylesheet.name,
                cases.len()
            );
            let bundle = write_bundle(&corpus_dir, &base, rule_set, stylesheet)?;
            let outcome = run_stylesheet(&cli, &bundle, &base, rule_set, stylesheet, &cases)?;
            let line = describe(rule_set, stylesheet, &outcome);
            summary.push_str(&line);
            report.push_str(&line);
            details(&outcome, &mut report);
        }
        util::write(&corpus_dir.join(REPORT), report.as_bytes())?;
        let _ = writeln!(summary, "  details: {}", corpus_dir.join(REPORT).display());
    }
    Ok(summary)
}

/// `<corpus>/engine/<stylesheet>/`: a bundle of the stylesheet alone, without
/// an XML schema, with copies of the stylesheet and of its side files, the
/// resources of the bundle.
fn write_bundle(
    corpus_dir: &Path,
    base: &Path,
    rule_set: &RuleSet,
    stylesheet: &Stylesheet,
) -> Result<PathBuf> {
    let dir = corpus_dir.join("engine").join(stylesheet.name);
    util::remove_dir(&dir)?;
    let (source_dir, file) = stylesheet
        .xslt
        .rsplit_once('/')
        .unwrap_or(("", stylesheet.xslt));
    let bytes = util::read(&util::join(base, stylesheet.xslt))?;
    let sha256 = util::sha256_hex(&bytes);
    util::write(&dir.join(file), &bytes)?;
    let mut manifest = format!(
        "# One stylesheet of {}, written by `cargo xtask engine-status`.\nformat = 2\nname = \"engine-status\"\ntitle = \"{} {}\"\nversion = \"0\"\n\n[[stylesheet]]\nname = \"{}\"\npath = \"{file}\"\nsha256 = \"{sha256}\"\n",
        rule_set.name, rule_set.name, stylesheet.name, stylesheet.name
    );
    for (side, _) in stylesheet.side_files {
        let bytes = util::read(&util::join(base, &format!("{source_dir}/{side}")))?;
        util::write(&util::join(&dir, side), &bytes)?;
        let _ = write!(
            manifest,
            "\n[[resource]]\npath = \"{side}\"\nsha256 = \"{}\"\n",
            util::sha256_hex(&bytes)
        );
    }
    util::write(&dir.join("bundle.toml"), manifest.as_bytes())?;
    Ok(dir)
}

/// What `einvoice-validate` printed.
struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// The maximum element depth of the engine (`--max-depth`), unless a case is
/// deeper: then [`RAISED_MAX_DEPTH`], as the equivalence tests raise it to
/// compare such a case with Saxon-HE, which has no limit.
const DEFAULT_MAX_DEPTH: usize = 256;
const RAISED_MAX_DEPTH: usize = 1024;

/// The element depth of a document (the document element is at depth 1); 0
/// for text that is not XML.
fn depth(text: &str) -> usize {
    let options = roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: u32::MAX,
        ..roxmltree::ParsingOptions::default()
    };
    let Ok(doc) = roxmltree::Document::parse_with_options(text, options) else {
        return 0;
    };
    doc.descendants()
        .filter(roxmltree::Node::is_element)
        .map(|n| n.ancestors().filter(roxmltree::Node::is_element).count())
        .max()
        .unwrap_or(0)
}

fn invoke(cli: &Path, bundle: &Path, input: &Path) -> Result<Run> {
    let deep = std::fs::read_to_string(input).is_ok_and(|t| depth(&t) > DEFAULT_MAX_DEPTH);
    let mut command = Command::new(cli);
    if deep {
        command.arg("--max-depth").arg(RAISED_MAX_DEPTH.to_string());
    }
    let output = command
        .arg("--bundle")
        .arg(bundle)
        .arg("--format")
        .arg("svrl")
        .arg(input)
        .output()
        .map_err(|e| Error::new(format!("cannot run {}: {e}", cli.display())))?;
    Ok(Run {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn run_stylesheet(
    cli: &Path,
    bundle: &Path,
    base: &Path,
    rule_set: &RuleSet,
    stylesheet: &Stylesheet,
    cases: &[Case],
) -> Result<Outcome> {
    let mut outcome = Outcome {
        cases: cases.len(),
        ..Outcome::default()
    };
    let Some(first) = cases.first() else {
        return Ok(outcome);
    };
    // A bundle that does not load fails before any document, with a message
    // of the tool itself (not prefixed by the document's path).
    let probe = invoke(cli, bundle, &absolute(&util::join(base, &first.input))?)?;
    if matches!(probe.code, Some(2 | 3))
        && probe.stdout.is_empty()
        && probe.stderr.starts_with("einvoice-validate: ")
    {
        outcome.refused = Some(probe.stderr.trim().to_owned());
        return Ok(outcome);
    }
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, CaseResult)>> = Mutex::new(Vec::new());
    let errors: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(case) = cases.get(index) else {
                        break;
                    };
                    match judge(cli, bundle, base, rule_set, stylesheet, case) {
                        Ok(r) => {
                            if let Ok(mut v) = results.lock() {
                                v.push((index, r));
                            }
                        }
                        Err(e) => {
                            if let Ok(mut v) = errors.lock() {
                                v.push(format!("{}: {e}", case.id));
                            }
                        }
                    }
                }
            });
        }
    });
    let errors = errors
        .into_inner()
        .map_err(|_| Error::new("poisoned lock"))?;
    if let Some(e) = errors.first() {
        return Err(Error::new(e.clone()));
    }
    let mut results = results
        .into_inner()
        .map_err(|_| Error::new("poisoned lock"))?;
    results.sort_by_key(|(i, _)| *i);
    for (index, result) in results {
        let id = cases[index].id.clone();
        match result {
            CaseResult::Pass => outcome.pass += 1,
            CaseResult::Differ(d) => outcome.differ.push((id, d)),
            CaseResult::Failed(f) => outcome.failed.push((id, f)),
        }
    }
    Ok(outcome)
}

fn absolute(path: &Path) -> Result<PathBuf> {
    std::path::absolute(path).map_err(|e| Error::new(format!("{}: {e}", path.display())))
}

/// Runs the engine on one case and compares with the golden.
fn judge(
    cli: &Path,
    bundle: &Path,
    base: &Path,
    rule_set: &RuleSet,
    stylesheet: &Stylesheet,
    case: &Case,
) -> Result<CaseResult> {
    let input = absolute(&util::join(base, &case.input))?;
    let run = invoke(cli, bundle, &input)?;
    let error_golden = util::join(base, &case.error_golden(rule_set, stylesheet));
    let saxon_error = if error_golden.is_file() {
        let text = util::read_string(&error_golden)?;
        text.lines()
            .next()
            .and_then(|l| l.strip_prefix("error "))
            .map(str::to_owned)
    } else {
        None
    };
    let first_error = run.stderr.lines().next().unwrap_or("").to_owned();
    if run.stdout.trim().is_empty() {
        let message = first_error
            .strip_prefix(&format!("{}: ", input.display()))
            .unwrap_or(&first_error)
            .to_owned();
        // "<file>: stylesheet NAME stopped: CODE: message" or
        // "<file>: error (transform): stylesheet NAME, line N: CODE: message"
        let code = error_code(&message);
        return Ok(match (code, saxon_error) {
            (Some(code), Some(expected)) if code == expected => CaseResult::Pass,
            (_, Some(expected)) => {
                CaseResult::Failed(format!("Saxon-HE stops with {expected}; engine: {message}"))
            }
            (_, None) => CaseResult::Failed(message),
        });
    }
    if let Some(expected) = saxon_error {
        return Ok(CaseResult::Differ(format!(
            "Saxon-HE stops with {expected}; the engine completes"
        )));
    }
    let path = input
        .to_str()
        .ok_or_else(|| Error::new(format!("{}: not UTF-8", input.display())))?
        .to_owned();
    let canonical = match svrl::check(
        &run.stdout,
        stylesheet.generator,
        &|uri| svrl::is_document_uri(uri, &path),
        false,
    ) {
        Ok(canonical) => canonical,
        Err(e) => {
            return Ok(CaseResult::Differ(format!(
                "report not canonicalisable: {e}"
            )));
        }
    };
    let golden = util::read_string(&util::join(base, &case.golden(rule_set, stylesheet)))?;
    if canonical == golden {
        return Ok(CaseResult::Pass);
    }
    let (mut expected, mut actual) = (golden.lines(), canonical.lines());
    let mut line = 1;
    loop {
        match (expected.next(), actual.next()) {
            (Some(e), Some(a)) if e == a => line += 1,
            (e, a) => {
                return Ok(CaseResult::Differ(format!(
                    "line {line}: Saxon-HE {} / engine {}",
                    shorten(e.unwrap_or("(end)")),
                    shorten(a.unwrap_or("(end)"))
                )));
            }
        }
    }
}

/// The first error code of a message: four upper-case letters and four
/// digits, followed by `:`.
fn error_code(message: &str) -> Option<String> {
    message
        .split(' ')
        .filter_map(|w| w.strip_suffix(':'))
        .find(|w| {
            let b = w.as_bytes();
            b.len() == 8
                && b[..4].iter().all(u8::is_ascii_uppercase)
                && b[4..].iter().all(u8::is_ascii_digit)
        })
        .map(str::to_owned)
}

fn shorten(text: &str) -> String {
    let mut out: String = text.chars().take(240).collect();
    if out.len() < text.len() {
        out.push_str("...");
    }
    format!("`{out}`")
}

/// The message with its digits replaced, to group failures of one kind.
fn kind_of(message: &str) -> String {
    let mut out = String::new();
    let mut digits = false;
    for c in message.chars() {
        if c.is_ascii_digit() {
            if !digits {
                out.push('N');
            }
            digits = true;
        } else {
            digits = false;
            out.push(c);
        }
    }
    out
}

fn describe(rule_set: &RuleSet, stylesheet: &Stylesheet, outcome: &Outcome) -> String {
    if let Some(refused) = &outcome.refused {
        return format!(
            "{} {}: refused at load, {} cases not run: {refused}\n",
            rule_set.name, stylesheet.name, outcome.cases
        );
    }
    let mut line = format!(
        "{} {}: {} cases, {} pass, {} differ, {} failed\n",
        rule_set.name,
        stylesheet.name,
        outcome.cases,
        outcome.pass,
        outcome.differ.len(),
        outcome.failed.len()
    );
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for (_, message) in &outcome.failed {
        *kinds.entry(kind_of(message)).or_default() += 1;
    }
    let mut kinds: Vec<(String, usize)> = kinds.into_iter().collect();
    kinds.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (kind, count) in kinds.iter().take(8) {
        let _ = writeln!(line, "  failed {count}x: {}", shorten(kind));
    }
    if kinds.len() > 8 {
        let _ = writeln!(line, "  ... {} other kinds of failure", kinds.len() - 8);
    }
    line
}

fn details(outcome: &Outcome, out: &mut String) {
    for (id, d) in &outcome.differ {
        let _ = writeln!(out, "  differ {id}: {d}");
    }
    for (id, f) in &outcome.failed {
        let _ = writeln!(out, "  failed {id}: {f}");
    }
}

#[cfg(test)]
mod tests {
    use super::{error_code, kind_of};

    #[test]
    fn error_codes_of_messages() {
        assert_eq!(
            error_code("error (transform): stylesheet X, line 5: XPTY0004: A sequence"),
            Some("XPTY0004".to_owned())
        );
        assert_eq!(
            error_code("stylesheet X stopped: FORG0001: Cannot convert"),
            Some("FORG0001".to_owned())
        );
        assert_eq!(error_code("error (unsupported): limit exceeded"), None);
    }

    #[test]
    fn failures_are_grouped_without_their_numbers() {
        assert_eq!(
            kind_of("stylesheet X, line 5111: FODC0002: cannot retrieve"),
            "stylesheet X, line N: FODCN: cannot retrieve"
        );
    }
}
