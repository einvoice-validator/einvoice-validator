//! The reference oracle: Saxon-HE runs each compiled XSLT of a rule set on
//! every case; its SVRL, in canonical form, is the golden result.
//!
//! Saxon is not part of the repository: `scripts/fetch-saxon.sh` downloads
//! it from Maven Central into `target/oracle/saxon/`, with the two jars of the
//! XML Resolver it needs at run time, and this module refuses to run if any of
//! the three jars does not have the pinned SHA-256.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::corpus::{self, Case, GOLDEN_SUFFIX};
use crate::ruleset::{RuleSet, Stylesheet};
use crate::svrl;
use crate::util::{self, Error, Result};

/// Saxon-HE 12.10 (Maven Central `net.sf.saxon:Saxon-HE:12.10`).
pub(crate) const SAXON_VERSION: &str = "12.10";
/// The class path of the oracle, file name and SHA-256 of each jar: Saxon-HE
/// first, then the XML Resolver 5.3.3 (`org.xmlresolver:xmlresolver:5.3.3`,
/// plain and with the `data` classifier), a dependency in Saxon-HE 12.10's POM
/// that `net.sf.saxon.Transform` loads at start-up. Saxon's jar is the one
/// given (`--saxon`, `$SAXON_JAR`), the other two are taken from its directory.
pub(crate) const SAXON_JARS: [(&str, &str); 3] = [
    (
        "Saxon-HE-12.10.jar",
        "b571af282f25d7301059f788b9a149aab8b5cdc14ef3d212dc5425d3dcbb9a97",
    ),
    (
        "xmlresolver-5.3.3.jar",
        "1fe4d5b92f708dcdb82dbce12919e0171e6b5ca62c6dca6220483625098feb5f",
    ),
    (
        "xmlresolver-5.3.3-data.jar",
        "b0c487ad2f3e558be8d829c916d2458d10aca6a5bafa7a4d0524b70845e48a5c",
    ),
];
/// Where `scripts/fetch-saxon.sh` puts Saxon's jar, relative to the repository root.
pub(crate) const SAXON_DEFAULT: &str = "target/oracle/saxon/Saxon-HE-12.10.jar";
/// Scratch directory of the oracle runs, relative to the repository root.
const WORK_DIR: &str = "target/oracle/runs";

/// How to run the oracle.
#[derive(Debug)]
pub(crate) struct Options {
    pub(crate) java: OsString,
    pub(crate) saxon: PathBuf,
    /// Write the goldens instead of comparing with them.
    pub(crate) write: bool,
}

/// What a run of one stylesheet found.
#[derive(Debug, Default)]
pub(crate) struct Outcome {
    pub(crate) stylesheet: &'static str,
    pub(crate) cases: usize,
    pub(crate) failed_asserts: usize,
    pub(crate) successful_reports: usize,
    pub(crate) cases_with_findings: usize,
    /// Goldens that differ, are missing or are stale (empty when writing).
    pub(crate) problems: Vec<String>,
}

/// Runs Saxon with each stylesheet of the rule set on every case, then
/// compares the canonical SVRL with the goldens (or writes them). One outcome
/// per stylesheet.
pub(crate) fn run(root: &Path, rule_set: &RuleSet, options: &Options) -> Result<Vec<Outcome>> {
    let class_path = saxon_class_path(&options.saxon)?;
    // The cases come from the index, which `corpus::verify` checks against the
    // upstream files (or `corpus::write` has just written).
    let cases = corpus::read_index(root, rule_set)?;
    let base = rule_set.base(root);

    let work = util::join(root, &format!("{WORK_DIR}/{}", rule_set.name));
    util::remove_dir(&work)?;
    let input_dir = work.join("in");
    // Saxon processes a whole directory in one JVM. The cases are copied there
    // under plain names (c00001.xml, ...), so that names with spaces or other
    // special characters never reach the command line or a URI.
    for (index, case) in cases.iter().enumerate() {
        let bytes = util::read(&util::join(&base, &case.input))?;
        util::write(&input_dir.join(staged_name(index)), &bytes)?;
    }
    let mut outcomes = Vec::new();
    for stylesheet in rule_set.stylesheets {
        let output_dir = work.join(format!("out-{}", stylesheet.name));
        std::fs::create_dir_all(&output_dir)
            .map_err(|e| Error::new(format!("cannot create {}: {e}", output_dir.display())))?;
        outcomes.push(run_stylesheet(
            root,
            rule_set,
            stylesheet,
            &cases,
            options,
            &class_path,
            &work,
        )?);
    }
    Ok(outcomes)
}

/// Runs Saxon with one stylesheet on the cases staged in `work/in`.
fn run_stylesheet(
    root: &Path,
    rule_set: &RuleSet,
    stylesheet: &Stylesheet,
    cases: &[Case],
    options: &Options,
    class_path: &OsString,
    work: &Path,
) -> Result<Outcome> {
    let base = rule_set.base(root);
    let input_dir = work.join("in");
    let output_dir = work.join(format!("out-{}", stylesheet.name));
    let xslt = util::join(&base, stylesheet.xslt);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
    let mut command = Command::new(&options.java);
    command
        .arg("-cp")
        .arg(class_path)
        .arg("net.sf.saxon.Transform")
        .arg(arg("-xsl:", &xslt))
        .arg(arg("-s:", &input_dir))
        .arg(arg("-o:", &output_dir))
        .arg(format!("-threads:{threads}"));
    eprintln!(
        "oracle: {} {}: Saxon-HE {SAXON_VERSION} on {} cases ({threads} threads)",
        rule_set.name,
        stylesheet.name,
        cases.len()
    );
    let output = command
        .output()
        .map_err(|e| Error::new(format!("cannot run {}: {e}", options.java.display())))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !stderr.trim().is_empty() || !stdout.trim().is_empty() {
        return Err(Error::new(format!(
            "Saxon failed ({}); a clean run prints nothing.\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status
        )));
    }

    let produced = util::list_files(&output_dir)?;
    let expected: Vec<String> = (0..cases.len()).map(staged_name).collect();
    if produced != expected {
        return Err(Error::new(format!(
            "Saxon wrote {} reports for {} cases",
            produced.len(),
            cases.len()
        )));
    }

    let mut outcome = Outcome {
        stylesheet: stylesheet.name,
        cases: cases.len(),
        ..Outcome::default()
    };
    let work_text = work.to_string_lossy().replace('\\', "/");
    if options.write {
        // Rewritten from scratch, so that no stale golden survives.
        util::remove_dir(&util::join(
            &base,
            &format!("{}/{}", rule_set.corpus_dir(), stylesheet.golden_dir),
        ))?;
    }
    for (index, case) in cases.iter().enumerate() {
        let staged = absolute(&input_dir.join(staged_name(index)))?;
        let raw = util::read_string(&output_dir.join(staged_name(index)))?;
        let canonical = svrl::canonicalize(&raw, &|uri| svrl::is_document_uri(uri, &staged))
            .map_err(|e| Error::new(format!("{}: {e}", case.id)))?;
        if canonical.contains(&work_text) {
            return Err(Error::new(format!(
                "{}: the canonical SVRL still contains the local scratch path",
                case.id
            )));
        }
        count(&canonical, &mut outcome)?;
        compare_or_write(
            &base,
            &case.golden(rule_set, stylesheet),
            &canonical,
            options.write,
            &mut outcome.problems,
        )?;
    }
    if !options.write {
        stale_goldens(&base, rule_set, stylesheet, cases, &mut outcome.problems)?;
    }
    Ok(outcome)
}

fn staged_name(index: usize) -> String {
    format!("c{:05}.xml", index + 1)
}

fn arg(flag: &str, path: &Path) -> OsString {
    let mut s = OsString::from(flag);
    s.push(path.as_os_str());
    s
}

fn absolute(path: &Path) -> Result<String> {
    let absolute =
        std::path::absolute(path).map_err(|e| Error::new(format!("{}: {e}", path.display())))?;
    absolute
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::new(format!("{}: not UTF-8", absolute.display())))
}

/// The class path of the oracle ([`SAXON_JARS`]), from Saxon's jar and its
/// directory. Refuses a missing jar or one that is not the pinned release.
fn saxon_class_path(saxon: &Path) -> Result<OsString> {
    let dir = saxon.parent().unwrap_or_else(|| Path::new(""));
    let mut jars = Vec::with_capacity(SAXON_JARS.len());
    for (index, (name, sha256)) in SAXON_JARS.iter().enumerate() {
        let jar = if index == 0 {
            saxon.to_owned()
        } else {
            dir.join(name)
        };
        check_jar(&jar, sha256)?;
        jars.push(jar);
    }
    std::env::join_paths(&jars).map_err(|e| Error::new(format!("class path: {e}")))
}

/// Refuses a missing jar or one whose SHA-256 is not `sha256`.
fn check_jar(jar: &Path, sha256: &str) -> Result<()> {
    if !jar.is_file() {
        return Err(Error::new(format!(
            "{} not found: run scripts/fetch-saxon.sh (or pass --saxon / set SAXON_JAR)",
            jar.display()
        )));
    }
    let hash = util::sha256_hex(&util::read(jar)?);
    if hash == sha256 {
        Ok(())
    } else {
        Err(Error::new(format!(
            "{}: SHA-256 {hash}, expected {sha256} (Saxon-HE {SAXON_VERSION})",
            jar.display()
        )))
    }
}

fn count(canonical: &str, outcome: &mut Outcome) -> Result<()> {
    let lines = svrl::parse(canonical)?;
    let asserts = lines.iter().filter(|l| l.kind == "failed-assert").count();
    let reports = lines
        .iter()
        .filter(|l| l.kind == "successful-report")
        .count();
    outcome.failed_asserts += asserts;
    outcome.successful_reports += reports;
    if asserts + reports > 0 {
        outcome.cases_with_findings += 1;
    }
    Ok(())
}

/// Writes the golden `path` (relative to `base`), or compares it with `canonical`.
fn compare_or_write(
    base: &Path,
    path: &str,
    canonical: &str,
    write: bool,
    problems: &mut Vec<String>,
) -> Result<()> {
    let golden = util::join(base, path);
    if write {
        return util::write(&golden, canonical.as_bytes());
    }
    if !golden.exists() {
        problems.push(format!("missing golden: {path}"));
    } else if util::read_string(&golden)? != canonical {
        problems.push(format!("differs: {path}"));
    }
    Ok(())
}

fn stale_goldens(
    base: &Path,
    rule_set: &RuleSet,
    stylesheet: &Stylesheet,
    cases: &[Case],
    problems: &mut Vec<String>,
) -> Result<()> {
    let relative = format!("{}/{}", rule_set.corpus_dir(), stylesheet.golden_dir);
    for file in util::list_files(&util::join(base, &relative))? {
        let known = file
            .strip_suffix(GOLDEN_SUFFIX)
            .is_some_and(|id| cases.iter().any(|c| c.id == id));
        if !known {
            problems.push(format!("stale golden: {relative}/{file}"));
        }
    }
    Ok(())
}
