//! The reference oracle: Saxon-HE runs each compiled XSLT of a rule set on
//! every case; its SVRL, in canonical form, is the golden result.
//!
//! Saxon is not part of the repository: `scripts/fetch-saxon.sh` downloads
//! it from Maven Central into `target/oracle/saxon/`, with the two jars of the
//! XML Resolver it needs at run time, and this module refuses to run if any of
//! the three jars does not have the pinned SHA-256.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::corpus::{self, Case};
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
    /// Cases on which Saxon stopped with an error (mutation cases only).
    pub(crate) errors: usize,
    /// Cases on which Saxon warned that a template pattern raised an error
    /// (mutation cases only).
    pub(crate) warned: usize,
    /// Goldens that differ, are missing or are stale (empty when writing).
    pub(crate) problems: Vec<String>,
}

/// Runs Saxon with each stylesheet of the rule set on every case, then
/// compares the canonical SVRL with the goldens (or writes them). One outcome
/// per stylesheet.
pub(crate) fn run(root: &Path, rule_set: &RuleSet, options: &Options) -> Result<Vec<Outcome>> {
    let class_path = saxon_class_path(&options.saxon)?;
    // The cases come from the indexes, which `corpus::verify` checks against
    // the upstream files and the mutation catalogue (or `corpus::write` has
    // just written).
    let cases = corpus::read_all(root, rule_set)?;
    let base = rule_set.base(root);

    let work = util::join(root, &format!("{WORK_DIR}/{}", rule_set.name));
    util::remove_dir(&work)?;
    // Saxon processes a whole directory in one JVM. The cases are copied there
    // under plain names (c00001.xml, ...), so that names with spaces or other
    // special characters never reach the command line or a URI. The cases of
    // `cases.tsv` and the mutation cases go to separate directories: they are
    // run differently (see `run_stylesheet`).
    for (index, case) in cases.iter().enumerate() {
        let bytes = util::read(&util::join(&base, &case.input))?;
        util::write(
            &work
                .join(input_dir(case.may_stop()))
                .join(staged_name(index)),
            &bytes,
        )?;
    }
    let mut outcomes = Vec::new();
    for stylesheet in rule_set.stylesheets {
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

/// The staging directory of the cases of `cases.tsv` or of the mutation cases,
/// relative to the work directory.
fn input_dir(mutation: bool) -> &'static str {
    if mutation { "in-mutations" } else { "in" }
}

/// Runs Saxon with one stylesheet on the staged cases.
///
/// The cases of `cases.tsv` run on several threads and the run must be clean:
/// Saxon prints nothing. The mutation cases run on one thread with `-t`, so
/// that every diagnostic follows the `Processing file:` line of its document:
/// a mutation case may stop Saxon with a dynamic error (its golden is then the
/// error, see [`Diagnostics`]) or make it warn that a template pattern raised
/// an error, which XSLT 3.0 treats as a pattern that does not match (counted,
/// the golden is the report). Anything else Saxon prints is a failure.
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
    let diagnostics = run_saxon(
        &base, rule_set, stylesheet, cases, options, class_path, work,
    )?;

    let mut outcome = Outcome {
        stylesheet: stylesheet.name,
        cases: cases.len(),
        ..Outcome::default()
    };
    let work_text = work.to_string_lossy().replace('\\', "/");
    let golden_dir = format!("{}/{}", rule_set.corpus_dir(), stylesheet.golden_dir);
    if options.write {
        // Rewritten from scratch, so that no stale golden survives.
        util::remove_dir(&util::join(&base, &golden_dir))?;
    }
    let mut goldens = BTreeSet::new();
    for (index, case) in cases.iter().enumerate() {
        let staged = work
            .join(input_dir(case.may_stop()))
            .join(staged_name(index));
        let staged = absolute(&staged)?;
        let found = diagnostics.get(&index).cloned().unwrap_or_default();
        outcome.warned += usize::from(!found.warnings.is_empty());
        let (golden, text) = if let Some((code, message)) = &found.error {
            outcome.errors += 1;
            (
                case.error_golden(rule_set, stylesheet),
                format!("error {code}\nmessage {message}\n"),
            )
        } else {
            let report = work
                .join(format!(
                    "{}-{}",
                    input_dir(case.may_stop()),
                    stylesheet.name
                ))
                .join(staged_name(index));
            let raw = util::read_string(&report)?;
            let canonical = svrl::check(
                &raw,
                stylesheet.generator,
                &|uri| svrl::is_document_uri(uri, &staged),
                true,
            )
            .map_err(|e| Error::new(format!("{}: {e}", case.id)))?;
            count(&canonical, &mut outcome)?;
            (case.golden(rule_set, stylesheet), canonical)
        };
        if text.contains(&work_text) || text.contains(&staged_name(index)) {
            return Err(Error::new(format!(
                "{}: the golden would contain the local scratch path or file name",
                case.id
            )));
        }
        compare_or_write(&base, &golden, &text, options.write, &mut outcome.problems)?;
        goldens.insert(golden);
    }
    if !options.write {
        for file in util::list_files(&util::join(&base, &golden_dir))? {
            let path = format!("{golden_dir}/{file}");
            if !goldens.contains(&path) {
                outcome.problems.push(format!("stale golden: {path}"));
            }
        }
    }
    Ok(outcome)
}

/// Runs Saxon with one stylesheet on the staged cases, writing the reports
/// into the work directory: the cases of `cases.tsv` on several threads, the
/// mutation cases on one with `-t`. Returns the diagnostics of the mutation
/// cases, by case index.
fn run_saxon(
    base: &Path,
    rule_set: &RuleSet,
    stylesheet: &Stylesheet,
    cases: &[Case],
    options: &Options,
    class_path: &OsString,
    work: &Path,
) -> Result<BTreeMap<usize, Diagnostics>> {
    let xslt = util::join(base, stylesheet.xslt);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
    let (mutations, upstream): (Vec<usize>, Vec<usize>) =
        (0..cases.len()).partition(|i| cases[*i].may_stop());
    eprintln!(
        "oracle: {} {}: Saxon-HE {SAXON_VERSION} on {} cases ({threads} threads) and {} mutation cases (1 thread)",
        rule_set.name,
        stylesheet.name,
        upstream.len(),
        mutations.len()
    );
    let mut diagnostics = BTreeMap::new();
    for (indexes, mutation) in [(&upstream, false), (&mutations, true)] {
        if indexes.is_empty() {
            continue;
        }
        let input = work.join(input_dir(mutation));
        let output = work.join(format!("{}-{}", input_dir(mutation), stylesheet.name));
        std::fs::create_dir_all(&output)
            .map_err(|e| Error::new(format!("cannot create {}: {e}", output.display())))?;
        let run = if mutation {
            saxon(options, class_path, &xslt, &input, &output, &["-t"])?
        } else {
            saxon(
                options,
                class_path,
                &xslt,
                &input,
                &output,
                &[&format!("-threads:{threads}")],
            )?
        };
        if mutation {
            diagnostics = run.diagnostics(indexes)?;
            // Saxon-HE keeps, in the compiled stylesheet, what it learns of the
            // variables it evaluates (`LearningEvaluator`), over every document
            // of the run: a variable it has seen read 20 times is evaluated
            // eagerly from then on, so a later document can fail on a value
            // that nothing reads, where alone it would not. A golden is what
            // Saxon-HE gives on the document alone: a case that failed is run
            // again, alone.
            let failed: Vec<usize> = diagnostics
                .iter()
                .filter(|(_, d)| d.error.is_some())
                .map(|(i, _)| *i)
                .collect();
            for index in failed {
                let alone = run_alone(options, class_path, &xslt, work, &input, &output, index)?;
                diagnostics.insert(index, alone);
            }
        } else if !run.success || !run.stdout.trim().is_empty() || !run.stderr.trim().is_empty() {
            return Err(run.failure("a clean run prints nothing"));
        }
    }
    Ok(diagnostics)
}

/// Runs Saxon-HE with `xslt` on the staged case `index` of `input` alone, with
/// `-t`, and returns its diagnostics; when it does not fail, its report
/// replaces the one of the run of all the cases in `output`.
fn run_alone(
    options: &Options,
    class_path: &OsString,
    xslt: &Path,
    work: &Path,
    input: &Path,
    output: &Path,
    index: usize,
) -> Result<Diagnostics> {
    let dir = work.join(format!("alone-{index}"));
    util::remove_dir(&dir)?;
    let (alone_in, alone_out) = (dir.join("in"), dir.join("out"));
    for d in [&alone_in, &alone_out] {
        std::fs::create_dir_all(d)
            .map_err(|e| Error::new(format!("cannot create {}: {e}", d.display())))?;
    }
    let name = staged_name(index);
    util::write(&alone_in.join(&name), &util::read(&input.join(&name))?)?;
    let run = saxon(options, class_path, xslt, &alone_in, &alone_out, &["-t"])?;
    let diagnostics = run
        .diagnostics(&[index])?
        .remove(&index)
        .unwrap_or_default();
    if diagnostics.error.is_none() {
        util::write(&output.join(&name), &util::read(&alone_out.join(&name))?)?;
    }
    util::remove_dir(&dir)?;
    Ok(diagnostics)
}

/// Runs Saxon-HE with `xslt` on the directory `source`, writing to `target`,
/// with the extra options `extra`.
fn saxon(
    options: &Options,
    class_path: &OsString,
    xslt: &Path,
    source: &Path,
    target: &Path,
    extra: &[&str],
) -> Result<Output> {
    let mut command = Command::new(&options.java);
    command
        .arg("-cp")
        .arg(class_path)
        .arg("net.sf.saxon.Transform")
        // The clock of current-dateTime(), which the SchXslt reports record.
        .arg(format!("-now:{}", svrl::FIXED_NOW))
        .args(extra)
        .arg(arg("-xsl:", xslt))
        .arg(arg("-s:", source))
        .arg(arg("-o:", target));
    let output = command
        .output()
        .map_err(|e| Error::new(format!("cannot run {}: {e}", options.java.display())))?;
    Ok(Output {
        success: output.status.success(),
        status: output.status.to_string(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// What a Saxon run printed.
#[derive(Debug)]
struct Output {
    success: bool,
    status: String,
    stdout: String,
    stderr: String,
}

/// What Saxon reported about one document of a `-t` run.
#[derive(Debug, Clone, Default)]
struct Diagnostics {
    /// The dynamic error that stopped the transformation: its code (as
    /// `FORG0001`) and its message, from the `While processing` line.
    error: Option<(String, String)>,
    /// The codes of the warnings (recoverable errors in template patterns).
    warnings: Vec<String>,
}

/// Lines that continue a diagnostic block, as prefixes: Saxon's note on where
/// a lazily evaluated value came from, and the first frame of the stack trace
/// of an error raised in a stylesheet function or a named template.
const NOTE_LINES: [&str; 3] = ["During lazy evaluation of ", "In function ", "In template "];

/// Lines `-t` prints about a document or the run, as prefixes.
const TIMING_LINES: [&str; 8] = [
    "SaxonJ-HE ",
    "Java version ",
    "Using parser ",
    "Stylesheet compilation time: ",
    "Building tree for ",
    "Tree built in ",
    "Tree size: ",
    "Execution time: ",
];

impl Output {
    fn failure(&self, why: &str) -> Error {
        Error::new(format!(
            "Saxon failed ({}): {why}.\nstdout:\n{}\nstderr:\n{}",
            self.status, self.stdout, self.stderr
        ))
    }

    /// The diagnostics of a single-threaded `-t` run on the cases `indexes`,
    /// by case index. Every line must be a timing line, a `Processing file:`
    /// line, a warning or error block (a header line, then indented lines, the
    /// first of which holding the code as `  <CODE>  <message>`, and possibly
    /// `During lazy evaluation of ...` notes), the
    /// `While processing <file>: <message>` line of a failed document, or the
    /// final `<n> transformation(s) failed`; the number of failed documents and
    /// the exit status must agree.
    fn diagnostics(&self, indexes: &[usize]) -> Result<BTreeMap<usize, Diagnostics>> {
        if !self.stdout.trim().is_empty() {
            return Err(self.failure("nothing is expected on standard output"));
        }
        let mut found: BTreeMap<usize, Diagnostics> = BTreeMap::new();
        let mut current: Option<usize> = None;
        // The block being read: whether it is a warning, and its code.
        let mut block: Option<(bool, Option<String>)> = None;
        let mut failed_summary = None;
        let close = |block: &mut Option<(bool, Option<String>)>,
                     current: Option<usize>,
                     found: &mut BTreeMap<usize, Diagnostics>|
         -> std::result::Result<(), String> {
            let Some((warning, code)) = block.take() else {
                return Ok(());
            };
            let code = code.ok_or("a diagnostic without a code")?;
            let index = current.ok_or("a diagnostic before the first document")?;
            let entry = found.entry(index).or_default();
            if warning {
                entry.warnings.push(code);
            } else if entry.error.is_some() {
                return Err("two errors for one document".to_owned());
            } else {
                entry.error = Some((code, String::new()));
            }
            Ok(())
        };
        for line in self.stderr.lines() {
            let fail = |why: &str| self.failure(&format!("{why}: {line:?}"));
            if let Some(rest) = line.strip_prefix("  ") {
                let Some((_, code)) = &mut block else {
                    return Err(fail("an indented line outside a diagnostic"));
                };
                if code.is_none() {
                    *code = Some(error_code(rest).ok_or_else(|| fail("no error code"))?);
                }
                continue;
            }
            if NOTE_LINES.iter().any(|p| line.starts_with(p)) {
                // Part of the block it follows.
                if block.is_none() {
                    return Err(fail("a note outside a diagnostic"));
                }
                continue;
            }
            close(&mut block, current, &mut found).map_err(|e| fail(&e))?;
            if line.is_empty() || TIMING_LINES.iter().any(|p| line.starts_with(p)) {
                continue;
            }
            if let Some(memory) = line.strip_prefix("Memory used: ") {
                if !memory.ends_with("Mb") {
                    return Err(fail("unexpected line"));
                }
            } else if let Some(uri) = line.strip_prefix("Processing file:") {
                let name = uri.rsplit('/').next().unwrap_or(uri);
                let index = indexes
                    .iter()
                    .copied()
                    .find(|i| staged_name(*i) == name)
                    .ok_or_else(|| fail("unknown document"))?;
                current = Some(index);
            } else if let Some(rest) = line.strip_prefix("While processing ") {
                let (name, message) = rest
                    .split_once(": ")
                    .ok_or_else(|| fail("unexpected line"))?;
                let entry = current
                    .filter(|i| staged_name(*i) == name)
                    .and_then(|i| found.get_mut(&i))
                    .and_then(|d| d.error.as_mut())
                    .ok_or_else(|| fail("a failed document without its error"))?;
                if !entry.1.is_empty() {
                    return Err(fail("two failures for one document"));
                }
                entry.1 = util::normalize_space(message);
            } else if line.ends_with(" transformations failed")
                || line.ends_with(" transformation failed")
            {
                failed_summary = line.split(' ').next().and_then(|n| n.parse::<usize>().ok());
                if failed_summary.is_none() {
                    return Err(fail("unexpected line"));
                }
            } else if line.ends_with(':') && line.contains("line ") {
                block = Some((line.starts_with("Warning"), None));
            } else {
                return Err(fail("unexpected line"));
            }
        }
        close(&mut block, current, &mut found).map_err(|e| self.failure(&e))?;
        let errors = found.values().filter(|d| d.error.is_some()).count();
        if found
            .values()
            .any(|d| d.error.as_ref().is_some_and(|(_, m)| m.is_empty()))
        {
            return Err(self.failure("an error without its `While processing` line"));
        }
        let expected_summary = (errors > 0).then_some(errors);
        if failed_summary != expected_summary || self.success != (errors == 0) {
            return Err(self.failure("the failed documents and the exit status disagree"));
        }
        Ok(found)
    }
}

/// The error code at the start of `text` (`FORG0001  message`): four
/// upper-case letters and four digits, followed by two spaces.
fn error_code(text: &str) -> Option<String> {
    let (code, _) = text.split_once("  ")?;
    let bytes = code.as_bytes();
    (bytes.len() == 8
        && bytes[..4].iter().all(u8::is_ascii_uppercase)
        && bytes[4..].iter().all(u8::is_ascii_digit))
    .then(|| code.to_owned())
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
pub(crate) fn saxon_class_path(saxon: &Path) -> Result<OsString> {
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

#[cfg(test)]
mod tests {
    use super::{Output, error_code};

    /// What Saxon-HE 12.10 prints with `-t` on one thread: c00002.xml is fine,
    /// c00003.xml raises a recoverable error in a pattern, c00004.xml stops it.
    const STDERR: &str = "SaxonJ-HE 12.10 from Saxonica
Java version 21.0.12.1
Using parser com.sun.org.apache.xerces.internal.jaxp.SAXParserImpl$JAXPSAXParser
Stylesheet compilation time: 404.939413ms
Processing file:/w/in-mutations/c00002.xml
Using parser com.sun.org.apache.xerces.internal.jaxp.SAXParserImpl$JAXPSAXParser
Building tree for file:/w/in-mutations/c00002.xml using class net.sf.saxon.tree.tiny.TinyBuilder
Tree built in 1.007468ms
Tree size: 391 nodes, 847 characters, 25 attributes
Execution time: 45.15241ms
Memory used: 49Mb
Processing file:/w/in-mutations/c00003.xml
Building tree for file:/w/in-mutations/c00003.xml using class net.sf.saxon.tree.tiny.TinyBuilder
Tree built in 0.713702ms
Tree size: 392 nodes, 853 characters, 25 attributes
Warning at char 26 in xsl:template/@match on line 1296 column 152 of X.xslt:
  XPTY0004  An error occurred matching pattern {cac:PaymentMeans[x]}: A sequence of more
  than one item is not allowed
Warning on line 2438 column 128 of X.xslt:
  XPTY0004  An error occurred matching pattern
  {cac:Party[$v]}: A sequence of more than one item is not allowed
Execution time: 12.90763ms
Memory used: 13Mb
Processing file:/w/in-mutations/c00004.xml
Building tree for file:/w/in-mutations/c00004.xml using class net.sf.saxon.tree.tiny.TinyBuilder
Tree built in 0.298389ms
Tree size: 391 nodes, 851 characters, 25 attributes
Type error at char 226 in expression in xsl:when/@test on line 1044 column 372 of X.xslt:
  XPTY0004  A sequence of more than one item { (1, 1) } is not allowed as the first
  operand of '+'
During lazy evaluation of LegalMonetaryTotal/xs:decimal(TaxExclusiveAmount) on line 1044 of file:/w/X.xslt
  In template rule with match=\"/\" on line 155 of X.xslt
While processing c00004.xml: A sequence of more than one item { (1, 1) } is not allowed as the first operand of '+'
Processing file:/w/in-mutations/c00005.xml
Building tree for file:/w/in-mutations/c00005.xml using class net.sf.saxon.tree.tiny.TinyBuilder
Tree built in 0.298389ms
Tree size: 391 nodes, 851 characters, 25 attributes
Error at char 22 in expression in xsl:variable/@select on line 50 column 83 of X.xslt:
  FORG0001  Cannot convert string \" 201\" to xs:decimal: invalid character ' '
In function custom:is-valid-date-format on line 42 column 34 of X.xslt:
     invoked by function call at file:/w/X.xslt#1004
In template d17e40 on line 118 column 32 of X.xslt:
     invoked by xsl:call-template at file:/w/X.xslt#76
  In template rule with match=\"udt:DateTimeString\" on line 999 of X.xslt
     invoked by xsl:apply-templates (tail calls omitted) at file:/w/X.xslt#1306
  In template rule with match=\"/\" on line 304 of X.xslt
While processing c00005.xml: Cannot convert string \" 201\" to xs:decimal: invalid character ' '

2 transformations failed
";

    fn output(stderr: &str, success: bool) -> Output {
        Output {
            success,
            status: String::new(),
            stdout: String::new(),
            stderr: stderr.to_owned(),
        }
    }

    #[test]
    fn diagnostics_are_attributed_to_their_documents() {
        let found = output(STDERR, false).diagnostics(&[1, 2, 3, 4]).unwrap();
        assert!(!found.contains_key(&1));
        assert_eq!(found[&2].warnings, ["XPTY0004", "XPTY0004"]);
        assert_eq!(found[&2].error, None);
        assert_eq!(
            found[&3].error,
            Some((
                "XPTY0004".to_owned(),
                "A sequence of more than one item { (1, 1) } is not allowed as the first operand of '+'"
                    .to_owned()
            ))
        );
        assert_eq!(
            found[&4].error,
            Some((
                "FORG0001".to_owned(),
                "Cannot convert string \" 201\" to xs:decimal: invalid character ' '".to_owned()
            ))
        );
    }

    #[test]
    fn anything_unexpected_is_a_failure() {
        for (stderr, success, why) in [
            (STDERR, true, "exit status disagree"),
            (
                &STDERR.replace("2 transformations failed", "3 transformations failed"),
                false,
                "disagree",
            ),
            (
                &STDERR.replace("Memory used: 13Mb", "Something else"),
                false,
                "unexpected line",
            ),
            (
                &STDERR.replace(
                    "While processing c00004.xml: ",
                    "While processing c00003.xml: ",
                ),
                false,
                "without its error",
            ),
            (
                &STDERR.replace("c00002.xml", "c00009.xml"),
                false,
                "unknown document",
            ),
            (
                &STDERR.replace(
                    "  XPTY0004  An error occurred matching pattern {cac",
                    "  bad",
                ),
                false,
                "no error code",
            ),
        ] {
            let error = output(stderr, success)
                .diagnostics(&[1, 2, 3, 4])
                .unwrap_err()
                .to_string();
            assert!(error.contains(why), "{why}: {error}");
        }
    }

    #[test]
    fn error_codes() {
        assert_eq!(
            error_code("FORG0001  Cannot convert"),
            Some("FORG0001".to_owned())
        );
        assert_eq!(error_code("In template rule"), None);
        assert_eq!(error_code("forg0001  x"), None);
    }
}
