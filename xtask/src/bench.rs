//! `cargo xtask bench`: the time and memory of a whole validation job,
//! `einvoice-validate` against Saxon-HE, on the corpus of each rule set.
//!
//! A job is a rule set as `einvoice-validate --rules NAME` validates with it
//! (`--families`, by default all of them): a built-in one (`cen-ubl`,
//! `cen-cii`) or a fetched bundle of the cache (`target/rules-cache`, the
//! cache of the equivalence tests of `einvoice-validate --rules`, filled from
//! the downloads of the fetch scripts with `einvoice-validate rules fetch
//! --from` when it lacks the bundle), on every document of the corpus its
//! stylesheets have goldens for (the cases of `cases.tsv` and the mutation
//! cases, staged under plain names in `target/bench/<rule set>/in/`, shared by
//! the jobs on the same corpus). The tool tells which stylesheets the rule set
//! runs and its XML schema (the `rule_set` object of its JSON report); each
//! stylesheet must be, byte for byte, one of the stylesheets the corpus has
//! goldens for. Three commands:
//!
//! - `einvoice-validate` (the release build, `--cli`, by default
//!   `target/release/einvoice-validate`), one process for the whole job:
//!   `--rules NAME --format json --jobs N`, the command its users run, which
//!   loads the rule set as it does for them, selects the scenario of each
//!   document (XRechnung), validates against the rule set's XML schema and
//!   runs its stylesheets with its verdict rules. In that run, a stylesheet
//!   that stops with a dynamic error on a document without schema errors ends
//!   the document's validation: the stylesheets after it do not run on it.
//!   Its verdicts must be the reference ones (below), or the run fails;
//! - `einvoice-validate` doing exactly the work of Saxon-HE, when that run
//!   does not (a rule set with an XML schema, or several stylesheets): one
//!   process per stylesheet, each with a bundle of that stylesheet alone, no
//!   schema and no scenarios, so that every stylesheet runs on every
//!   document;
//! - Saxon-HE 12.10 (the pinned jars of `scripts/fetch-saxon.sh`), one JVM
//!   per stylesheet of the rule set (`net.sf.saxon.Transform` runs one
//!   stylesheet), on the directory of the documents with `-threads:N` and
//!   `-t`: the stylesheets only, on every document, as the second command.
//!   Saxon-HE has no XML schema validation, and the command line neither
//!   selects scenarios nor reads the reports. On one thread, the `-t`
//!   timings (the compilation of the stylesheet and the execution of each
//!   document) also give the time Saxon-HE spends without the start of the
//!   JVM and the loading of its classes; on several threads the documents'
//!   times overlap, and that figure is not given.
//!
//! The reference verdicts (`crate::acceptance::references`): for a fetched
//! rule set, the acceptance table of its corpus (`acceptance.tsv`, which
//! `cargo xtask <family>` writes from the Saxon-HE goldens and xmllint): the
//! scenario of each document (XRechnung), the verdict of the stylesheets that
//! run on it, and that of xmllint on the rule set's XML schema when the table
//! has it (XRechnung, Factur-X and the CII and CDAR rule sets of the French
//! rules); for the built-in rule sets, the goldens. Where xmllint is the
//! reference, the schema step of `einvoice-validate` must agree with it on
//! every document, but for the differences listed with their cause in
//! `crates/einvoice-cli/tests/schema-disagreements.tsv`, each of which must
//! occur; elsewhere (the UBL 2.1 and CII D16B schemas outside XRechnung) the
//! corpus records no schema verdict and the tool's own is taken. A document
//! is then invalid without a scenario or with schema errors, without verdict
//! when a stylesheet stopped, invalid when one rejects it, valid otherwise.
//!
//! Each command runs once to warm up (and to check its outcome), then
//! `--runs` times (default 5), the tools taking turns; the thread counts
//! come from `--threads` (default `1,8`). Each run is measured by a helper
//! process (`cargo xtask bench-measure`, the same binary) whose only child is
//! the command: the wall time from just before the spawn to just after the
//! child is reaped, and from `getrusage(RUSAGE_CHILDREN)` once it is (what
//! `wait4` returns for the child): the user and system CPU time of the
//! process and its waited-for descendants, and the peak resident set size
//! of the largest of them. Then the latency of one small invoice, a new
//! process each time, with each tool.
//!
//! A run must have done the whole job: `einvoice-validate` must report every
//! document (exit status 0, 1 or 3) with the reference verdicts and the same
//! counts on every run, and Saxon-HE must have processed every document (any
//! failed transformation is counted and reported).
//!
//! The results go to standard output as markdown, to
//! `target/bench/benchmarks.md` and `target/bench/benchmarks.json`, and with
//! `--write` to `docs/benchmarks.json` and between the markers of
//! `docs/benchmarks.md`.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use crate::acceptance::{self, Reference, Status};
use crate::bench_report::{
    self, Exit, Latency, Machine, Outcome, Report, Sample, Summary, TargetRun, Tool, ToolRun,
};
use crate::corpus::{self, Case};
use crate::engine;
use crate::fetched;
use crate::json;
use crate::oracle;
use crate::ruleset::{self, Family, Profiles, RuleSet, Stylesheet};
use crate::util::{self, Error, Result};

pub(crate) const USAGE: &str = "usage: cargo xtask bench [--families NAME,...] [--runs N] [--threads N,...] [--write] [--cli PATH] [--java JAVA] [--saxon JAR]\n  names: all, the groups cen, peppol, facturx, xrechnung, fr-ctc, fr-ctc-cii, fr-ctc-ubl, or a rule set as --rules names it (cen-ubl, factur-x-en16931, fr-ctc-cdar, ...)";

/// The scratch directory, relative to the repository root.
const WORK_DIR: &str = "target/bench";
/// The cache of the fetched rule sets, relative to the repository root: the
/// one of the equivalence tests of `einvoice-validate --rules`.
const RULES_CACHE: &str = "target/rules-cache";
/// The directories `einvoice-validate rules fetch --from` takes the published
/// files from, relative to the repository root: the downloads of the fetch
/// scripts.
const DOWNLOADS: &[&str] = &["target/downloads", "target/peppol/downloads"];
/// The cases on which the XML schema step of a bundle and xmllint disagree,
/// with their cause, relative to the repository root.
const SCHEMA_DISAGREEMENTS: &str = "crates/einvoice-cli/tests/schema-disagreements.tsv";
/// The committed results, relative to the repository root.
pub(crate) const DOC: &str = "docs/benchmarks.md";
pub(crate) const DATA: &str = "docs/benchmarks.json";

/// The document of the single-document latency, relative to the repository
/// root: a small invoice of the CEN examples.
const LATENCY_DOCUMENT: &str =
    "artefacts/cen-en16931-ubl-1.3.17/ubl/examples/ubl-tc434-example9.xml";

/// A job: a rule set as `einvoice-validate --rules` names it, on the corpus
/// of a rule set of `crate::ruleset`.
#[derive(Debug)]
pub(crate) struct Target {
    /// The name `--rules` takes.
    pub(crate) key: &'static str,
    pub(crate) title: &'static str,
    /// The rule set whose corpus and goldens the job runs on.
    pub(crate) rule_set: &'static RuleSet,
    /// The family of a fetched rule set; `None` when built in.
    family: Option<&'static Family>,
}

macro_rules! target {
    ($key:literal, $title:literal, $rule_set:expr) => {
        Target {
            key: $key,
            title: $title,
            rule_set: &$rule_set,
            family: None,
        }
    };
    ($key:literal, $title:literal, $rule_set:expr, $family:expr) => {
        Target {
            key: $key,
            title: $title,
            rule_set: &$rule_set,
            family: Some(&$family),
        }
    };
}

pub(crate) const TARGETS: &[Target] = &[
    target!("cen-ubl", "CEN EN 16931 UBL 1.3.17", ruleset::CEN_UBL),
    target!("cen-cii", "CEN EN 16931 CII 1.3.17", ruleset::CEN_CII),
    target!(
        "peppol-bis-billing-ubl",
        "Peppol BIS Billing 3.0.22 (UBL)",
        ruleset::PEPPOL_UBL,
        ruleset::PEPPOL
    ),
    target!(
        "factur-x-minimum",
        "Factur-X 1.09.2 MINIMUM",
        ruleset::FACTURX,
        ruleset::FACTURX_FAMILY
    ),
    target!(
        "factur-x-basic-wl",
        "Factur-X 1.09.2 BASIC WL",
        ruleset::FACTURX,
        ruleset::FACTURX_FAMILY
    ),
    target!(
        "factur-x-basic",
        "Factur-X 1.09.2 BASIC",
        ruleset::FACTURX,
        ruleset::FACTURX_FAMILY
    ),
    target!(
        "factur-x-en16931",
        "Factur-X 1.09.2 EN16931",
        ruleset::FACTURX,
        ruleset::FACTURX_FAMILY
    ),
    target!(
        "factur-x-extended",
        "Factur-X 1.09.2 EXTENDED",
        ruleset::FACTURX,
        ruleset::FACTURX_FAMILY
    ),
    target!(
        "xrechnung-ubl",
        "XRechnung 3.0.2 UBL",
        ruleset::XRECHNUNG_UBL,
        ruleset::XRECHNUNG
    ),
    target!(
        "xrechnung-cii",
        "XRechnung 3.0.2 CII",
        ruleset::XRECHNUNG_CII,
        ruleset::XRECHNUNG
    ),
    target!(
        "fr-ctc-cii-en16931",
        "French rules 1.4.0.04, EN 16931 CII",
        ruleset::FR_CTC_CII,
        ruleset::FR_CTC
    ),
    target!(
        "fr-ctc-cii-en16931-warning",
        "French rules 1.4.0.04, EN 16931 CII (WARNING version)",
        ruleset::FR_CTC_CII,
        ruleset::FR_CTC
    ),
    target!(
        "fr-ctc-cii-extended-ctc-fr",
        "French rules 1.4.0.04, EXTENDED-CTC-FR CII",
        ruleset::FR_CTC_CII,
        ruleset::FR_CTC
    ),
    target!(
        "fr-ctc-cii-extended-ctc-fr-warning",
        "French rules 1.4.0.04, EXTENDED-CTC-FR CII (WARNING version)",
        ruleset::FR_CTC_CII,
        ruleset::FR_CTC
    ),
    target!(
        "fr-ctc-factur-x-basic-wl",
        "French rules 1.4.0.04, Factur-X BASIC WL",
        ruleset::FR_CTC_CII,
        ruleset::FR_CTC
    ),
    target!(
        "fr-ctc-factur-x-en16931",
        "French rules 1.4.0.04, Factur-X EN16931",
        ruleset::FR_CTC_CII,
        ruleset::FR_CTC
    ),
    target!(
        "fr-ctc-factur-x-extended",
        "French rules 1.4.0.04, Factur-X EXTENDED",
        ruleset::FR_CTC_CII,
        ruleset::FR_CTC
    ),
    target!(
        "fr-ctc-ubl-en16931",
        "French rules 1.4.0.04, EN 16931 UBL",
        ruleset::FR_CTC_UBL,
        ruleset::FR_CTC
    ),
    target!(
        "fr-ctc-ubl-en16931-warning",
        "French rules 1.4.0.04, EN 16931 UBL (WARNING version)",
        ruleset::FR_CTC_UBL,
        ruleset::FR_CTC
    ),
    target!(
        "fr-ctc-ubl-extended-ctc-fr",
        "French rules 1.4.0.04, EXTENDED-CTC-FR UBL",
        ruleset::FR_CTC_UBL,
        ruleset::FR_CTC
    ),
    target!(
        "fr-ctc-ubl-extended-ctc-fr-warning",
        "French rules 1.4.0.04, EXTENDED-CTC-FR UBL (WARNING version)",
        ruleset::FR_CTC_UBL,
        ruleset::FR_CTC
    ),
    target!(
        "fr-ctc-cdar",
        "French rules 1.4.0.04, life-cycle statuses (CDAR)",
        ruleset::FR_CTC_CDAR,
        ruleset::FR_CTC
    ),
    target!(
        "fr-ctc-cdar-warning",
        "French rules 1.4.0.04, life-cycle statuses (CDAR, WARNING version)",
        ruleset::FR_CTC_CDAR,
        ruleset::FR_CTC
    ),
];

/// The names `--families` takes besides the keys of [`TARGETS`]: groups of
/// them, by family or by corpus.
const GROUPS: &[(&str, Group)] = &[
    ("cen", Group::BuiltIn),
    ("peppol", Group::Family("peppol")),
    ("facturx", Group::Family("facturx")),
    ("xrechnung", Group::Family("xrechnung")),
    ("fr-ctc", Group::Family("fr-ctc")),
    ("fr-ctc-cii", Group::Corpus(&ruleset::FR_CTC_CII)),
    ("fr-ctc-ubl", Group::Corpus(&ruleset::FR_CTC_UBL)),
];

/// The jobs of a group.
#[derive(Debug)]
enum Group {
    /// The built-in rule sets.
    BuiltIn,
    /// The rule sets of a family of fetched ones, by its name.
    Family(&'static str),
    /// The jobs on the corpus of a rule set.
    Corpus(&'static RuleSet),
}

impl Group {
    fn contains(&self, target: &Target) -> bool {
        match self {
            Self::BuiltIn => target.family.is_none(),
            Self::Family(name) => target.family.is_some_and(|f| f.name == *name),
            Self::Corpus(rule_set) => target.rule_set.name == rule_set.name,
        }
    }
}

/// The command line of `cargo xtask bench`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Args {
    pub(crate) targets: Vec<&'static str>,
    pub(crate) runs: usize,
    pub(crate) threads: Vec<usize>,
    pub(crate) write: bool,
    pub(crate) cli: Option<PathBuf>,
    pub(crate) java: Option<OsString>,
    pub(crate) saxon: Option<PathBuf>,
}

/// Parses the arguments after `bench`.
pub(crate) fn parse_args(args: &[OsString]) -> Result<Args> {
    let mut parsed = Args {
        targets: TARGETS.iter().map(|t| t.key).collect(),
        runs: 5,
        threads: vec![1, 8],
        write: false,
        cli: None,
        java: None,
        saxon: None,
    };
    let mut seen: Vec<&str> = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let flag = arg
            .to_str()
            .ok_or_else(|| Error::new(format!("unexpected argument {}\n{USAGE}", arg.display())))?;
        if flag == "--write" {
            parsed.write = true;
            continue;
        }
        let known = [
            "--families",
            "--runs",
            "--threads",
            "--cli",
            "--java",
            "--saxon",
        ];
        let Some(flag) = known.iter().copied().find(|k| *k == flag) else {
            return Err(Error::new(format!("unexpected argument {flag}\n{USAGE}")));
        };
        if seen.contains(&flag) {
            return Err(Error::new(format!("{flag} given twice\n{USAGE}")));
        }
        seen.push(flag);
        let value = args
            .next()
            .ok_or_else(|| Error::new(format!("{flag} needs a value\n{USAGE}")))?;
        let text = || {
            value
                .to_str()
                .ok_or_else(|| Error::new(format!("{flag}: not UTF-8\n{USAGE}")))
        };
        match flag {
            "--families" => parsed.targets = select(text()?)?,
            "--runs" => parsed.runs = positive(flag, text()?)?,
            "--threads" => {
                let mut threads = Vec::new();
                for part in text()?.split(',') {
                    let n = positive(flag, part)?;
                    if threads.contains(&n) {
                        return Err(Error::new(format!("{flag}: {n} given twice")));
                    }
                    threads.push(n);
                }
                parsed.threads = threads;
            }
            "--cli" => parsed.cli = Some(PathBuf::from(value)),
            "--java" => parsed.java = Some(value.clone()),
            _ => parsed.saxon = Some(PathBuf::from(value)),
        }
    }
    Ok(parsed)
}

fn positive(flag: &str, text: &str) -> Result<usize> {
    text.parse::<usize>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| Error::new(format!("{flag}: expected a positive integer, not {text:?}")))
}

/// The keys of the jobs that a comma-separated list of names selects, in
/// the order of [`TARGETS`].
fn select(names: &str) -> Result<Vec<&'static str>> {
    let mut wanted: Vec<&str> = Vec::new();
    for name in names.split(',') {
        if name == "all" {
            wanted.extend(TARGETS.iter().map(|t| t.key));
        } else if let Some((_, group)) = GROUPS.iter().find(|(g, _)| *g == name) {
            wanted.extend(TARGETS.iter().filter(|t| group.contains(t)).map(|t| t.key));
        } else if let Some(target) = TARGETS.iter().find(|t| t.key == name) {
            wanted.push(target.key);
        } else {
            return Err(Error::new(format!(
                "--families: unknown name {name:?}\n{USAGE}"
            )));
        }
    }
    Ok(TARGETS
        .iter()
        .map(|t| t.key)
        .filter(|k| wanted.contains(k))
        .collect())
}

/// What the run needs, resolved.
#[derive(Debug)]
pub(crate) struct Options {
    pub(crate) args: Args,
    pub(crate) cli: PathBuf,
    pub(crate) java: OsString,
    pub(crate) saxon: PathBuf,
}

/// The tools, ready to be run.
struct Tools {
    cli: PathBuf,
    java: OsString,
    class_path: OsString,
    /// This binary, which measures each run (`bench-measure`).
    helper: PathBuf,
    /// The cache of the fetched rule sets ([`RULES_CACHE`], absolute), given
    /// to `einvoice-validate` as `EINVOICE_VALIDATOR_CACHE`.
    cache: PathBuf,
}

/// Runs the benchmark and writes its results.
pub(crate) fn run(root: &Path, options: &Options) -> Result<()> {
    if !options.cli.is_file() {
        return Err(Error::new(format!(
            "{} not found: build it with `cargo build --release --locked -p einvoice-cli` (or pass --cli)",
            options.cli.display()
        )));
    }
    let tools = Tools {
        cli: absolute(&options.cli)?,
        java: options.java.clone(),
        class_path: oracle::saxon_class_path(&options.saxon)?,
        helper: std::env::current_exe()
            .map_err(|e| Error::new(format!("cannot find this program: {e}")))?,
        cache: absolute(&util::join(root, RULES_CACHE))?,
    };
    let args = &options.args;
    let targets: Vec<&Target> = TARGETS
        .iter()
        .filter(|t| args.targets.contains(&t.key))
        .collect();
    // Every corpus and every rule set is checked before anything is measured.
    for target in &targets {
        check_corpus(root, target)?;
        ensure_cached(root, &tools, target)?;
    }
    let work = util::join(root, WORK_DIR);
    let mut machine = machine(root, &tools)?;
    let mut results = Vec::new();
    let mut corpora: Vec<Corpus> = Vec::new();
    for target in targets {
        let staged = corpora
            .iter()
            .position(|c| c.rule_set == target.rule_set.name);
        let index = if let Some(index) = staged {
            index
        } else {
            eprintln!("bench: {}: staging the corpus", target.rule_set.name);
            corpora.push(stage_corpus(root, &work, target.rule_set)?);
            corpora.len() - 1
        };
        let corpus = &corpora[index];
        results.push(bench_target(root, &tools, corpus, target, args)?);
    }
    let latency = latency(root, &work, &tools, args.runs)?;
    machine.load_after = load_average();
    let report = Report {
        machine,
        runs: args.runs,
        threads: args.threads.clone(),
        targets: results,
        latency: Some(latency),
    };
    let section = bench_report::markdown(&report);
    let data = bench_report::json(&report);
    util::write(&work.join("benchmarks.md"), section.as_bytes())?;
    util::write(&work.join("benchmarks.json"), data.as_bytes())?;
    print!("{section}");
    if args.write {
        let doc = util::join(root, DOC);
        let text = util::read_string(&doc)?;
        let updated = bench_report::replace_section(&text, &section)
            .map_err(|e| Error::new(format!("{}: {e}", doc.display())))?;
        util::write(&doc, updated.as_bytes())?;
        util::write(&util::join(root, DATA), data.as_bytes())?;
        eprintln!("bench: written {DOC} and {DATA}");
    }
    Ok(())
}

/// Refuses a rule set whose corpus is not there (the fetched ones are built
/// by `cargo xtask <family>`, with their acceptance table).
fn check_corpus(root: &Path, target: &Target) -> Result<()> {
    let rule_set = target.rule_set;
    if let Some(family) = target.family {
        if !fetched::stamp(root, rule_set).exists() {
            return Err(Error::new(format!(
                "{}: no complete corpus: run `scripts/check.sh {}` (or `{}` then `cargo xtask {}`) first, or leave it out with --families",
                rule_set.name, family.name, family.script, family.name
            )));
        }
        fetched::check_cache(root, family, rule_set)?;
    }
    Ok(())
}

/// A command of `einvoice-validate`, with the cache of the fetched rule sets.
fn cli_command(tools: &Tools) -> Command {
    let mut command = Command::new(&tools.cli);
    command.env("EINVOICE_VALIDATOR_CACHE", &tools.cache);
    command
}

/// Makes sure the cache holds the rule set of a fetched job, as the
/// equivalence tests of `einvoice-validate --rules` do: when it does not,
/// `einvoice-validate rules fetch NAME --from` builds its family there from
/// the downloads of the fetch scripts, with no mirror reachable, so that
/// nothing is downloaded.
fn ensure_cached(root: &Path, tools: &Tools, target: &Target) -> Result<()> {
    if target.family.is_none() {
        return Ok(());
    }
    let path = cli_command(tools)
        .args(["rules", "path", target.key])
        .output()
        .map_err(|e| Error::new(format!("cannot run {}: {e}", tools.cli.display())))?;
    if path.status.success() {
        return Ok(());
    }
    eprintln!(
        "bench: {}: not in {}, fetching it from the downloads of the fetch scripts",
        target.key,
        tools.cache.display()
    );
    let mut command = cli_command(tools);
    command
        .args(["rules", "fetch", target.key])
        .env("EINVOICE_VALIDATOR_MIRROR", "http://127.0.0.1:9");
    for dir in DOWNLOADS {
        command.arg("--from").arg(util::join(root, dir));
    }
    let fetch = command
        .output()
        .map_err(|e| Error::new(format!("cannot run {}: {e}", tools.cli.display())))?;
    if !fetch.status.success() {
        return Err(Error::new(format!(
            "{}: not in {} and cannot be fetched there from {}: {}",
            target.key,
            tools.cache.display(),
            DOWNLOADS.join(" and "),
            String::from_utf8_lossy(&fetch.stderr).trim()
        )));
    }
    Ok(())
}

/// The corpus of a rule set, staged for the jobs that run on it.
struct Corpus {
    /// The name of the rule set.
    rule_set: &'static str,
    /// The directory the documents are relative to.
    dir: PathBuf,
    cases: Vec<Case>,
    /// The documents, relative to `dir`, in the order of `cases`.
    files: Vec<String>,
    max_depth: Option<usize>,
    /// The SHA-256 of each stylesheet of the rule set, in its order.
    sha256: Vec<String>,
}

fn stage_corpus(root: &Path, work: &Path, rule_set: &'static RuleSet) -> Result<Corpus> {
    let base = rule_set.base(root);
    let dir = work.join(rule_set.name);
    util::remove_dir(&dir)?;
    let cases = corpus::read_all(root, rule_set)?;
    let mut files = Vec::with_capacity(cases.len());
    let mut deepest = 0;
    for (index, case) in cases.iter().enumerate() {
        let bytes = util::read(&util::join(&base, &case.input))?;
        if let Ok(text) = std::str::from_utf8(&bytes) {
            deepest = deepest.max(engine::depth(text));
        }
        let name = format!("in/c{:05}.xml", index + 1);
        util::write(&util::join(&dir, &name), &bytes)?;
        files.push(name);
    }
    let sha256 = rule_set
        .stylesheets
        .iter()
        .map(|s| Ok(util::sha256_hex(&util::read(&util::join(&base, s.xslt))?)))
        .collect::<Result<_>>()?;
    Ok(Corpus {
        rule_set: rule_set.name,
        dir: absolute(&dir)?,
        cases,
        files,
        max_depth: (deepest > engine::DEFAULT_MAX_DEPTH).then_some(engine::RAISED_MAX_DEPTH),
        sha256,
    })
}

/// A job ready to be measured.
struct Staged<'c> {
    corpus: &'c Corpus,
    /// Where the outputs of the job go.
    out: PathBuf,
    /// Absolute paths of the stylesheets the rule set runs, for Saxon-HE.
    stylesheets: Vec<PathBuf>,
    /// The rule set as `einvoice-validate` validates with it.
    rule_set: RuleSource,
    /// For [`Tool::EngineSameWork`]: one bundle per stylesheet, without
    /// schema and scenarios; empty when [`Tool::Engine`] already does that
    /// work.
    same_work: Vec<PathBuf>,
    /// The reference verdicts, with the cases on which the schema step of
    /// `einvoice-validate` differs from xmllint for a documented cause.
    verdicts: Option<Verdicts>,
}

/// How `einvoice-validate` gets the rule set.
enum RuleSource {
    /// `--rules NAME`.
    Named(&'static str),
    /// `--bundle DIR`.
    Bundle(PathBuf),
}

/// The rule set as `einvoice-validate --rules NAME` reports it: the `schema`
/// of its manifest and the SHA-256 of its stylesheets.
struct Described {
    schema: Option<String>,
    stylesheets: Vec<(String, String)>,
}

/// Asks `einvoice-validate --rules NAME` what the rule set runs: the
/// `rule_set` object of its JSON report on the first document.
fn describe(tools: &Tools, corpus: &Corpus, key: &str) -> Result<Described> {
    let first = corpus
        .files
        .first()
        .ok_or_else(|| Error::new(format!("{}: no documents", corpus.rule_set)))?;
    let output = cli_command(tools)
        .args(["--rules", key, "--format", "json", "--max-depth"])
        .arg(engine::RAISED_MAX_DEPTH.to_string())
        .arg("--")
        .arg(first)
        .current_dir(&corpus.dir)
        .output()
        .map_err(|e| Error::new(format!("cannot run {}: {e}", tools.cli.display())))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let report = json::parse(&text).map_err(|e| {
        Error::new(format!(
            "einvoice-validate --rules {key}: {e}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    })?;
    let rule_set = report
        .get("rule_set")
        .ok_or_else(|| Error::new(format!("einvoice-validate --rules {key}: no rule_set")))?;
    if rule_set.get("name").and_then(json::Value::as_str) != Some(key) {
        return Err(Error::new(format!(
            "einvoice-validate --rules {key}: the report names another rule set"
        )));
    }
    let stylesheets = rule_set
        .get("stylesheets")
        .and_then(json::Value::as_array)
        .unwrap_or_default()
        .iter()
        .map(|s| {
            match (
                s.get("name").and_then(json::Value::as_str),
                s.get("sha256").and_then(json::Value::as_str),
            ) {
                (Some(name), Some(sha256)) => Ok((name.to_owned(), sha256.to_owned())),
                _ => Err(Error::new(format!(
                    "einvoice-validate --rules {key}: a stylesheet without name or sha256"
                ))),
            }
        })
        .collect::<Result<Vec<_>>>()?;
    if stylesheets.is_empty() {
        return Err(Error::new(format!(
            "einvoice-validate --rules {key}: no stylesheets"
        )));
    }
    Ok(Described {
        schema: rule_set
            .get("schema")
            .and_then(json::Value::as_str)
            .map(str::to_owned),
        stylesheets,
    })
}

/// Stages a job: what the rule set runs, its stylesheets among the corpus's,
/// the bundles of the same work and the reference verdicts.
fn stage<'c>(
    root: &Path,
    tools: &Tools,
    corpus: &'c Corpus,
    target: &Target,
) -> Result<(Staged<'c>, Option<String>)> {
    let rule_set = target.rule_set;
    let base = rule_set.base(root);
    let out = corpus.dir.join(target.key);
    util::remove_dir(&out)?;
    let described = describe(tools, corpus, target.key)?;
    // Each stylesheet of the rule set, byte for byte one of the corpus's.
    let mut runs: Vec<&Stylesheet> = Vec::new();
    for (name, sha256) in &described.stylesheets {
        let index = corpus.sha256.iter().position(|s| s == sha256).ok_or_else(|| {
            Error::new(format!(
                "{}: its stylesheet {name} (SHA-256 {sha256}) is none of the stylesheets of {}, whose goldens the corpus holds: the cache {} and the corpus differ",
                target.key,
                rule_set.name,
                tools.cache.display()
            ))
        })?;
        runs.push(&rule_set.stylesheets[index]);
    }
    let stylesheets = runs
        .iter()
        .map(|s| absolute(&util::join(&base, s.xslt)))
        .collect::<Result<Vec<_>>>()?;
    let mut same_work = Vec::new();
    let scenarios = matches!(
        rule_set.acceptance.as_ref().map(|a| &a.profiles),
        Some(Profiles::KositScenarios { .. })
    );
    if described.schema.is_some() || scenarios || runs.len() > 1 {
        for (index, stylesheet) in runs.iter().enumerate() {
            same_work.push(write_bundle(
                &out.join(format!("bundle-{index}")),
                &base,
                target,
                stylesheet,
            )?);
        }
    }
    let references = acceptance::references(
        root,
        rule_set,
        &corpus.cases,
        &runs,
        described.schema.as_deref(),
    )?;
    let documented = documented_schema_differences(root, target.key)?;
    Ok((
        Staged {
            corpus,
            out,
            stylesheets,
            rule_set: RuleSource::Named(target.key),
            same_work,
            verdicts: Some(Verdicts {
                references,
                documented,
            }),
        },
        described.schema,
    ))
}

/// Writes a bundle of `stylesheet` alone, without schema and scenarios, and
/// of the files it reads with `document()`, at their paths relative to the
/// base directory.
fn write_bundle(
    dir: &Path,
    base: &Path,
    target: &Target,
    stylesheet: &Stylesheet,
) -> Result<PathBuf> {
    util::remove_dir(dir)?;
    let mut manifest = format!(
        "# {}, {}, written by `cargo xtask bench`.\nformat = 2\nname = \"bench-{}\"\ntitle = \"{}\"\nversion = \"0\"\n",
        target.title, stylesheet.name, target.key, target.title
    );
    let bytes = util::read(&util::join(base, stylesheet.xslt))?;
    util::write(&util::join(dir, stylesheet.xslt), &bytes)?;
    let _ = write!(
        manifest,
        "\n[[stylesheet]]\nname = \"{}\"\npath = \"{}\"\nsha256 = \"{}\"\n",
        stylesheet.name,
        stylesheet.xslt,
        util::sha256_hex(&bytes)
    );
    let source_dir = stylesheet.xslt.rsplit_once('/').map_or("", |(d, _)| d);
    for (side, _) in stylesheet.side_files {
        let path = if source_dir.is_empty() {
            (*side).to_owned()
        } else {
            format!("{source_dir}/{side}")
        };
        let bytes = util::read(&util::join(base, &path))?;
        util::write(&util::join(dir, &path), &bytes)?;
        let _ = write!(
            manifest,
            "\n[[resource]]\npath = \"{path}\"\nsha256 = \"{}\"\n",
            util::sha256_hex(&bytes)
        );
    }
    util::write(&dir.join("bundle.toml"), manifest.as_bytes())?;
    absolute(dir)
}

/// The cases of [`SCHEMA_DISAGREEMENTS`] for the rule set `key`: where its
/// XML schema step and xmllint disagree, for a documented cause.
fn documented_schema_differences(root: &Path, key: &str) -> Result<HashSet<String>> {
    let text = util::read_string(&util::join(root, SCHEMA_DISAGREEMENTS))?;
    let mut lines = text.lines().filter(|l| !l.starts_with('#'));
    if lines.next() != Some("bundle\tcase\texplanation") {
        return Err(Error::new(format!(
            "{SCHEMA_DISAGREEMENTS}: not the header bundle, case, explanation"
        )));
    }
    let mut out = HashSet::new();
    for line in lines {
        let columns: Vec<&str> = line.split('\t').collect();
        let [bundle, case, _] = columns[..] else {
            return Err(Error::new(format!("{SCHEMA_DISAGREEMENTS}: {line:?}")));
        };
        if bundle == key && !out.insert(case.to_owned()) {
            return Err(Error::new(format!(
                "{SCHEMA_DISAGREEMENTS}: {bundle} {case} listed twice"
            )));
        }
    }
    Ok(out)
}

/// The reference verdicts of a job ([`acceptance::references`]), with the
/// cases of [`SCHEMA_DISAGREEMENTS`].
struct Verdicts {
    references: Vec<Reference>,
    documented: HashSet<String>,
}

impl Verdicts {
    /// Checks the JSON report of `einvoice-validate` on the documents of the
    /// corpus: in order, each with the reference status (on the schema
    /// verdict of xmllint where it is the reference, the documented
    /// differences reversing it, each of which must occur; on the tool's
    /// own elsewhere). Returns the counts of the reference.
    fn check(&self, corpus: &Corpus, key: &str, report: &str) -> Result<Summary> {
        let report = json::parse(report)?;
        let documents = report
            .get("documents")
            .and_then(json::Value::as_array)
            .ok_or_else(|| Error::new("einvoice-validate: no documents in the JSON report"))?;
        if documents.len() != corpus.cases.len() {
            return Err(Error::new(format!(
                "{key}: {} documents reported of {}",
                documents.len(),
                corpus.cases.len()
            )));
        }
        let mut problems = Vec::new();
        let mut expected = Summary {
            documents: documents.len(),
            valid: 0,
            invalid: 0,
            errors: 0,
        };
        for (((document, case), file), reference) in documents
            .iter()
            .zip(&corpus.cases)
            .zip(&corpus.files)
            .zip(&self.references)
        {
            let id = case.id.as_str();
            if document.get("path").and_then(json::Value::as_str) != Some(file.as_str()) {
                return Err(Error::new(format!(
                    "{key}: the report of {file} ({id}) is not in its place"
                )));
            }
            let schema_errors = document
                .get("counts")
                .and_then(|c| c.get("schema_errors"))
                .and_then(json::Value::as_count)
                .ok_or_else(|| Error::new(format!("{key}: {id}: no counts.schema_errors")))?;
            let ours = schema_errors == 0;
            let documented = self.documented.contains(id);
            let schema_valid = if let Some(xmllint) = &reference.schema {
                let expected = (xmllint == "valid") != documented;
                if ours != expected {
                    problems.push(format!(
                        "{id}: {schema_errors} schema errors, xmllint {xmllint}{}",
                        if documented {
                            format!(", listed in {SCHEMA_DISAGREEMENTS} but not different")
                        } else {
                            String::new()
                        }
                    ));
                }
                expected
            } else {
                if documented {
                    problems.push(format!(
                        "{id}: listed in {SCHEMA_DISAGREEMENTS}, but the corpus records no xmllint verdict for it"
                    ));
                }
                ours
            };
            let status = reference.status(schema_valid);
            match status {
                Status::Valid => expected.valid += 1,
                Status::Invalid => expected.invalid += 1,
                Status::Error => expected.errors += 1,
            }
            let actual = document
                .get("status")
                .and_then(json::Value::as_str)
                .unwrap_or("");
            if actual != status.as_str() {
                problems.push(format!(
                    "{id}: {actual}, the reference {} ({schema_errors} schema errors, stylesheets {:?}{})",
                    status.as_str(),
                    reference.stylesheets,
                    if reference.unmatched {
                        ", no scenario"
                    } else {
                        ""
                    }
                ));
            }
        }
        if !problems.is_empty() {
            let shown: Vec<&str> = problems.iter().take(20).map(String::as_str).collect();
            return Err(Error::new(format!(
                "{key}: the verdicts of einvoice-validate are not the reference's ({} valid / {} invalid / {} no verdict): {} differences:\n{}",
                expected.valid,
                expected.invalid,
                expected.errors,
                problems.len(),
                shown.join("\n")
            )));
        }
        Ok(expected)
    }
}

fn absolute(path: &Path) -> Result<PathBuf> {
    std::path::absolute(path).map_err(|e| Error::new(format!("{}: {e}", path.display())))
}

fn threads_text(threads: usize) -> String {
    if threads == 1 {
        "1 thread".to_owned()
    } else {
        format!("{threads} threads")
    }
}

fn bench_target(
    root: &Path,
    tools: &Tools,
    corpus: &Corpus,
    target: &Target,
    args: &Args,
) -> Result<TargetRun> {
    let (staged, schema) = stage(root, tools, corpus, target)?;
    let documents = corpus.files.len();
    eprintln!(
        "bench: {}: {documents} documents, {} stylesheets, schema {}",
        target.key,
        staged.stylesheets.len(),
        schema.as_deref().unwrap_or("none")
    );
    let mut kinds = vec![Tool::Engine];
    if !staged.same_work.is_empty() {
        kinds.push(Tool::EngineSameWork);
    }
    kinds.push(Tool::Saxon);
    let mut runs = Vec::new();
    for &threads in &args.threads {
        let mut measured: Vec<ToolRun> = Vec::new();
        for round in 0..=args.runs {
            for &tool in &kinds {
                let (sample, outcome) = match tool {
                    Tool::Engine => engine_run(tools, &staged, &staged.rule_set, threads)?,
                    Tool::EngineSameWork => same_work_run(tools, &staged, threads)?,
                    Tool::Saxon => saxon_run(tools, &staged, threads)?,
                };
                let label = if round == 0 {
                    "warm-up".to_owned()
                } else {
                    format!("run {round}/{}", args.runs)
                };
                eprintln!(
                    "bench: {} {} {}, {label}: {:.3} s, {:.3} s CPU, {} KiB",
                    target.key,
                    tool.key(),
                    threads_text(threads),
                    sample.wall.as_secs_f64(),
                    sample.cpu().as_secs_f64(),
                    sample.peak_rss_kib
                );
                if round == 0
                    && let Outcome::Engine(s) = outcome
                    && tool == Tool::Engine
                {
                    eprintln!(
                        "bench: {} {}: {} valid / {} invalid / {} no verdict, the reference verdicts",
                        target.key,
                        threads_text(threads),
                        s.valid,
                        s.invalid,
                        s.errors
                    );
                }
                record(&mut measured, tool, threads, round, sample, outcome)?;
            }
        }
        runs.extend(measured);
    }
    Ok(TargetRun {
        key: target.key,
        title: target.title,
        documents,
        stylesheets: staged.stylesheets.len(),
        schema,
        max_depth: corpus.max_depth,
        runs,
    })
}

/// Adds a sample (none for the warm-up round 0), checking that the outcome
/// does not change between runs.
fn record(
    measured: &mut Vec<ToolRun>,
    tool: Tool,
    threads: usize,
    round: usize,
    sample: Sample,
    outcome: Outcome,
) -> Result<()> {
    let index = if let Some(i) = measured.iter().position(|r| r.tool == tool) {
        i
    } else {
        measured.push(ToolRun {
            tool,
            threads,
            samples: Vec::new(),
            outcome,
        });
        measured.len() - 1
    };
    let run = &mut measured[index];
    match (run.outcome, outcome) {
        (Outcome::Saxon { failed: a }, Outcome::Saxon { failed: b }) if a != b => {
            // Saxon-HE learns, within one JVM, which variables to evaluate
            // eagerly; on several threads the documents come in another order
            // each time, and a mutation case may then fail or not. The
            // largest count is kept.
            eprintln!(
                "bench: {} {}: {a} then {b} failed transformations",
                tool.key(),
                threads_text(threads)
            );
            run.outcome = Outcome::Saxon { failed: a.max(b) };
        }
        (a, b) if a != b => {
            return Err(Error::new(format!(
                "{} {}: the outcome changed between runs: {a:?} then {b:?}",
                tool.key(),
                threads_text(threads)
            )));
        }
        _ => {}
    }
    if round > 0 {
        run.samples.push(sample);
    }
    Ok(())
}

/// Runs `program` with `args` in `dir` under the helper, standard output and
/// standard error going to the given files.
fn measure(
    tools: &Tools,
    dir: &Path,
    stdout: &Path,
    stderr: &Path,
    program: &OsStr,
    args: &[OsString],
) -> Result<(Sample, Exit)> {
    let output = Command::new(&tools.helper)
        .arg("bench-measure")
        .arg(stdout)
        .arg(stderr)
        .arg(dir)
        .arg(program)
        .args(args)
        .env("EINVOICE_VALIDATOR_CACHE", &tools.cache)
        .output()
        .map_err(|e| Error::new(format!("cannot run {}: {e}", tools.helper.display())))?;
    if !output.status.success() {
        return Err(Error::new(format!(
            "bench-measure failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    bench_report::parse_measure_line(String::from_utf8_lossy(&output.stdout).trim())
}

/// The first lines of a file the measured tool wrote, for an error message.
fn head(path: &Path) -> String {
    std::fs::read_to_string(path).map_or_else(
        |e| format!("({}: {e})", path.display()),
        |t| t.lines().take(10).collect::<Vec<_>>().join("\n"),
    )
}

fn engine_args(
    source: &RuleSource,
    threads: usize,
    max_depth: Option<usize>,
    files: &[String],
) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::with_capacity(files.len() + 8);
    match source {
        RuleSource::Named(name) => {
            args.push("--rules".into());
            args.push((*name).into());
        }
        RuleSource::Bundle(dir) => {
            args.push("--bundle".into());
            args.push(dir.clone().into_os_string());
        }
    }
    args.extend(["--format".into(), "json".into(), "--jobs".into()]);
    args.push(threads.to_string().into());
    if let Some(depth) = max_depth {
        args.push("--max-depth".into());
        args.push(depth.to_string().into());
    }
    args.push("--".into());
    args.extend(files.iter().map(OsString::from));
    args
}

/// One run of `einvoice-validate` on every staged document; with
/// [`RuleSource::Named`], its verdicts must be the reference ones.
fn engine_run(
    tools: &Tools,
    staged: &Staged<'_>,
    source: &RuleSource,
    threads: usize,
) -> Result<(Sample, Outcome)> {
    let corpus = staged.corpus;
    let stdout = staged.out.join("engine.json");
    let stderr = staged.out.join("engine.stderr");
    let args = engine_args(source, threads, corpus.max_depth, &corpus.files);
    let (sample, exit) = measure(
        tools,
        &corpus.dir,
        &stdout,
        &stderr,
        tools.cli.as_os_str(),
        &args,
    )?;
    let outcome = engine_outcome(&stdout, &stderr, exit, corpus.files.len())?;
    if let (RuleSource::Named(key), Some(verdicts), Outcome::Engine(summary)) =
        (source, &staged.verdicts, outcome)
    {
        let expected = verdicts.check(corpus, key, &util::read_string(&stdout)?)?;
        if expected != summary {
            return Err(Error::new(format!(
                "{key}: the summary of einvoice-validate, {summary:?}, is not the reference's, {expected:?}"
            )));
        }
    }
    Ok((sample, outcome))
}

/// One run of `einvoice-validate` per stylesheet, each with the bundle of
/// that stylesheet alone, on every staged document: the counts of the
/// outcome are of transformations.
fn same_work_run(tools: &Tools, staged: &Staged<'_>, threads: usize) -> Result<(Sample, Outcome)> {
    let mut total: Option<(Sample, Summary)> = None;
    for bundle in &staged.same_work {
        let (sample, outcome) =
            engine_run(tools, staged, &RuleSource::Bundle(bundle.clone()), threads)?;
        let Outcome::Engine(summary) = outcome else {
            return Err(Error::new("einvoice-validate: no summary"));
        };
        total = Some(match total {
            None => (sample, summary),
            Some((s, t)) => (
                s.then(sample),
                Summary {
                    documents: t.documents + summary.documents,
                    valid: t.valid + summary.valid,
                    invalid: t.invalid + summary.invalid,
                    errors: t.errors + summary.errors,
                },
            ),
        });
    }
    let (sample, summary) = total.ok_or_else(|| Error::new("no bundle for the same work"))?;
    Ok((sample, Outcome::Engine(summary)))
}

/// Checks that `einvoice-validate` reported every document.
fn engine_outcome(stdout: &Path, stderr: &Path, exit: Exit, documents: usize) -> Result<Outcome> {
    if !matches!(exit, Some(0 | 1 | 3)) {
        return Err(Error::new(format!(
            "einvoice-validate ended with {exit:?}:\n{}",
            head(stderr)
        )));
    }
    let summary = bench_report::parse_summary(&util::read_string(stdout)?)?;
    if summary.documents != documents {
        return Err(Error::new(format!(
            "einvoice-validate reported {} documents of {documents}",
            summary.documents
        )));
    }
    Ok(Outcome::Engine(summary))
}

/// One run of Saxon-HE, each stylesheet in turn, on the staged documents.
fn saxon_run(tools: &Tools, staged: &Staged<'_>, threads: usize) -> Result<(Sample, Outcome)> {
    let corpus = staged.corpus;
    let input = corpus.dir.join("in");
    let mut total: Option<Sample> = None;
    let mut failed = 0;
    for (index, xslt) in staged.stylesheets.iter().enumerate() {
        let output = staged.out.join(format!("saxon-{index}"));
        util::remove_dir(&output)?;
        std::fs::create_dir_all(&output)
            .map_err(|e| Error::new(format!("cannot create {}: {e}", output.display())))?;
        let stderr = staged.out.join(format!("saxon-{index}.stderr"));
        let args = saxon_args(tools, xslt, &input, &output, Some(threads));
        let (mut sample, exit) = measure(
            tools,
            &corpus.dir,
            &staged.out.join(format!("saxon-{index}.stdout")),
            &stderr,
            &tools.java,
            &args,
        )?;
        let text = util::read_string(&stderr)?;
        let timings = bench_report::parse_saxon_timings(&text)
            .map_err(|e| Error::new(format!("{e}:\n{}", head(&stderr))))?;
        if timings.processed != corpus.files.len() {
            return Err(Error::new(format!(
                "Saxon-HE processed {} documents of {} with {}:\n{}",
                timings.processed,
                corpus.files.len(),
                xslt.display(),
                head(&stderr)
            )));
        }
        // With -threads, Saxon-HE exits with status 0 even when documents fail
        // (see `parse_saxon_timings`); any other status needs a failed document.
        if exit != Some(0) && timings.failed == 0 {
            return Err(Error::new(format!(
                "Saxon-HE ended with {exit:?} after {} failed transformations with {}:\n{}",
                timings.failed,
                xslt.display(),
                head(&stderr)
            )));
        }
        failed += timings.failed;
        sample.in_jvm = (threads == 1).then_some(timings.compile + timings.execution);
        total = Some(total.map_or(sample, |t| t.then(sample)));
    }
    let total = total.ok_or_else(|| Error::new("a rule set without stylesheets"))?;
    Ok((total, Outcome::Saxon { failed }))
}

fn saxon_args(
    tools: &Tools,
    xslt: &Path,
    source: &Path,
    output: &Path,
    threads: Option<usize>,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "-cp".into(),
        tools.class_path.clone(),
        "net.sf.saxon.Transform".into(),
        "-t".into(),
    ];
    if let Some(threads) = threads {
        args.push(format!("-threads:{threads}").into());
    }
    for (flag, path) in [("-xsl:", xslt), ("-s:", source), ("-o:", output)] {
        let mut arg = OsString::from(flag);
        arg.push(path.as_os_str());
        args.push(arg);
    }
    args
}

/// The latency of one small invoice, a new process each time.
fn latency(root: &Path, work: &Path, tools: &Tools, runs: usize) -> Result<Latency> {
    let dir = work.join("latency");
    util::remove_dir(&dir)?;
    let source = util::join(root, LATENCY_DOCUMENT);
    let bytes = util::read(&source)?;
    let name = LATENCY_DOCUMENT
        .rsplit('/')
        .next()
        .unwrap_or(LATENCY_DOCUMENT)
        .to_owned();
    util::write(&dir.join(&name), &bytes)?;
    let target = TARGETS
        .iter()
        .find(|t| t.key == "cen-ubl")
        .ok_or_else(|| Error::new("no cen-ubl target"))?;
    let stylesheet = target
        .rule_set
        .stylesheets
        .first()
        .ok_or_else(|| Error::new("cen-ubl has no stylesheet"))?;
    let no_schema = write_bundle(&dir.join("bundle-no-schema"), root, target, stylesheet)?;
    let dir = absolute(&dir)?;
    let xslt = absolute(&util::join(root, stylesheet.xslt))?;
    let corpus = Corpus {
        rule_set: target.rule_set.name,
        dir: dir.clone(),
        cases: Vec::new(),
        files: vec![name.clone()],
        max_depth: None,
        sha256: Vec::new(),
    };
    let staged = Staged {
        corpus: &corpus,
        out: dir.clone(),
        stylesheets: vec![xslt.clone()],
        rule_set: RuleSource::Named("cen-ubl"),
        same_work: vec![no_schema],
        verdicts: None,
    };
    let mut measured = Vec::new();
    for round in 0..=runs {
        for tool in [Tool::Engine, Tool::EngineSameWork, Tool::Saxon] {
            let (sample, outcome) = match tool {
                Tool::Engine => engine_run(tools, &staged, &staged.rule_set, 1)?,
                Tool::EngineSameWork => same_work_run(tools, &staged, 1)?,
                Tool::Saxon => {
                    let stderr = dir.join("saxon.stderr");
                    let args =
                        saxon_args(tools, &xslt, &dir.join(&name), &dir.join("out.svrl"), None);
                    let (mut sample, exit) = measure(
                        tools,
                        &dir,
                        &dir.join("saxon.stdout"),
                        &stderr,
                        &tools.java,
                        &args,
                    )?;
                    if exit != Some(0) {
                        return Err(Error::new(format!(
                            "Saxon-HE ended with {exit:?}:\n{}",
                            head(&stderr)
                        )));
                    }
                    let timings = bench_report::parse_saxon_timings(&util::read_string(&stderr)?)?;
                    sample.in_jvm = Some(timings.compile + timings.execution);
                    (sample, Outcome::Saxon { failed: 0 })
                }
            };
            eprintln!(
                "bench: one document, {} {}: {:.1} ms",
                tool.key(),
                if round == 0 {
                    "warm-up".to_owned()
                } else {
                    format!("run {round}/{runs}")
                },
                sample.wall.as_secs_f64() * 1000.0
            );
            record(&mut measured, tool, 1, round, sample, outcome)?;
        }
    }
    Ok(Latency {
        document: LATENCY_DOCUMENT.to_owned(),
        bytes: bytes.len(),
        runs: measured,
    })
}

/// The output of a command, standard output then standard error, or `None`
/// when it cannot run.
fn command_output(program: &OsStr, args: &[&str], dir: &Path) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Some(text)
}

fn first_line(text: Option<String>) -> String {
    text.and_then(|t| t.lines().next().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// `/proc/loadavg`: the load averages over 1, 5 and 15 minutes.
fn load_average() -> String {
    std::fs::read_to_string("/proc/loadavg").map_or_else(
        |_| "unknown".to_owned(),
        |t| t.split_whitespace().take(3).collect::<Vec<_>>().join(" "),
    )
}

fn machine(root: &Path, tools: &Tools) -> Result<Machine> {
    let epoch = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    let git = |args: &[&str]| command_output(OsStr::new("git"), args, root);
    let mut commit = first_line(git(&["rev-parse", "--short=12", "HEAD"]));
    if git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|t| !t.trim().is_empty())
    {
        commit.push_str(" (with uncommitted changes)");
    }
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let (cpu, logical, cores) = parse_cpuinfo(&cpuinfo);
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let os = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|t| os_name(&t))
        .unwrap_or_else(|| std::env::consts::OS.to_owned());
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map_or_else(|_| "unknown".to_owned(), |t| t.trim().to_owned());
    let java = command_output(&tools.java, &["-version"], root)
        .map(|t| t.lines().take(2).collect::<Vec<_>>().join("; "))
        .ok_or_else(|| Error::new(format!("cannot run {}", tools.java.display())))?;
    Ok(Machine {
        date: bench_report::utc_date(epoch),
        commit,
        cpu: cpu.unwrap_or_else(|| "unknown".to_owned()),
        logical_cpus: if logical > 0 {
            logical
        } else {
            std::thread::available_parallelism().map_or(0, std::num::NonZero::get)
        },
        cores,
        memory_kib: meminfo
            .lines()
            .find_map(|l| l.strip_prefix("MemTotal:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
            .unwrap_or(0),
        os,
        kernel,
        rustc: first_line(command_output(OsStr::new("rustc"), &["-V"], root)),
        java,
        saxon: format!(
            "{} (Maven Central net.sf.saxon:Saxon-HE)",
            oracle::SAXON_VERSION
        ),
        cli: first_line(command_output(tools.cli.as_os_str(), &["--version"], root)),
        load_before: load_average(),
        load_after: String::new(),
    })
}

/// The model name, the number of logical CPUs and of physical cores (distinct
/// `physical id` / `core id` pairs, when listed) of `/proc/cpuinfo`.
fn parse_cpuinfo(text: &str) -> (Option<String>, usize, Option<usize>) {
    let mut model = None;
    let mut logical = 0;
    let mut cores: Vec<(String, String)> = Vec::new();
    let mut physical = String::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "processor" => logical += 1,
            "model name" if model.is_none() => model = Some(value.to_owned()),
            "physical id" => value.clone_into(&mut physical),
            "core id" => {
                let pair = (physical.clone(), value.to_owned());
                if !cores.contains(&pair) {
                    cores.push(pair);
                }
            }
            _ => {}
        }
    }
    (model, logical, (!cores.is_empty()).then_some(cores.len()))
}

/// `PRETTY_NAME` of `/etc/os-release`.
fn os_name(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim().trim_matches('"').to_owned())
}

/// `cargo xtask bench-measure STDOUT STDERR DIR PROGRAM [ARG...]`: runs
/// PROGRAM in DIR, its standard output and error going to the two files, and
/// prints the line of [`bench_report::measure_line`].
#[cfg(unix)]
pub(crate) fn measure_main(args: &[OsString]) -> Result<()> {
    use nix::sys::resource::{UsageWho, getrusage};
    use nix::sys::time::TimeValLike as _;
    use std::process::Stdio;

    let [stdout, stderr, dir, program, rest @ ..] = args else {
        return Err(Error::new(
            "usage: cargo xtask bench-measure STDOUT STDERR DIR PROGRAM [ARG...]",
        ));
    };
    let create = |path: &OsString| {
        std::fs::File::create(path)
            .map_err(|e| Error::new(format!("cannot create {}: {e}", path.display())))
    };
    let (out, err) = (create(stdout)?, create(stderr)?);
    let mut command = Command::new(program);
    command
        .args(rest)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err);
    let start = std::time::Instant::now();
    let status = command
        .status()
        .map_err(|e| Error::new(format!("cannot run {}: {e}", program.display())))?;
    let wall = start.elapsed();
    let usage =
        getrusage(UsageWho::RUSAGE_CHILDREN).map_err(|e| Error::new(format!("getrusage: {e}")))?;
    let micros = |t: nix::sys::time::TimeVal| {
        Duration::from_micros(u64::try_from(t.num_microseconds()).unwrap_or(0))
    };
    let rss = u64::try_from(usage.max_rss()).unwrap_or(0);
    // ru_maxrss is in bytes on Apple systems, in KiB elsewhere.
    let rss_kib = if cfg!(target_vendor = "apple") {
        rss / 1024
    } else {
        rss
    };
    let sample = Sample {
        wall,
        user: micros(usage.user_time()),
        system: micros(usage.system_time()),
        peak_rss_kib: rss_kib,
        in_jvm: None,
    };
    println!("{}", bench_report::measure_line(&sample, status.code()));
    Ok(())
}

/// Without `getrusage`, nothing is measured.
#[cfg(not(unix))]
pub(crate) fn measure_main(_args: &[OsString]) -> Result<()> {
    Err(Error::new(
        "cargo xtask bench needs a Unix system (getrusage of the measured process)",
    ))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use std::collections::HashSet;

    use super::{
        Args, Corpus, GROUPS, RuleSource, TARGETS, Verdicts, documented_schema_differences,
        engine_args, os_name, parse_args, parse_cpuinfo,
    };
    use crate::acceptance::Reference;
    use crate::bench_report::Summary;
    use crate::corpus::Case;
    use crate::util::repo_root;

    fn args(list: &[&str]) -> crate::util::Result<Args> {
        parse_args(&list.iter().map(OsString::from).collect::<Vec<_>>())
    }

    #[test]
    fn defaults() {
        let parsed = args(&[]).unwrap();
        assert_eq!(parsed.targets.len(), 23);
        assert_eq!(
            (parsed.runs, parsed.threads, parsed.write),
            (5, vec![1, 8], false)
        );
        assert_eq!((parsed.cli, parsed.java, parsed.saxon), (None, None, None));
    }

    #[test]
    fn options() {
        let parsed = args(&[
            "--families",
            "xrechnung,cen-ubl,cen",
            "--runs",
            "3",
            "--threads",
            "1,4,16",
            "--write",
            "--cli",
            "x/cli",
            "--java",
            "j",
            "--saxon",
            "s.jar",
        ])
        .unwrap();
        assert_eq!(
            parsed.targets,
            ["cen-ubl", "cen-cii", "xrechnung-ubl", "xrechnung-cii"]
        );
        assert_eq!(
            (parsed.runs, parsed.threads, parsed.write),
            (3, vec![1, 4, 16], true)
        );
        assert_eq!(parsed.cli, Some(PathBuf::from("x/cli")));
        assert_eq!(parsed.java, Some(OsString::from("j")));
        assert_eq!(parsed.saxon, Some(PathBuf::from("s.jar")));
        assert_eq!(args(&["--families", "all"]).unwrap().targets.len(), 23);
        assert_eq!(args(&["--families", "fr-ctc"]).unwrap().targets.len(), 13);
        assert_eq!(
            args(&["--families", "fr-ctc-cii"]).unwrap().targets.len(),
            7
        );
        assert_eq!(
            args(&["--families", "fr-ctc-ubl,peppol"]).unwrap().targets,
            [
                "peppol-bis-billing-ubl",
                "fr-ctc-ubl-en16931",
                "fr-ctc-ubl-en16931-warning",
                "fr-ctc-ubl-extended-ctc-fr",
                "fr-ctc-ubl-extended-ctc-fr-warning"
            ]
        );
        assert_eq!(
            args(&["--families", "fr-ctc-cdar,facturx"])
                .unwrap()
                .targets,
            [
                "factur-x-minimum",
                "factur-x-basic-wl",
                "factur-x-basic",
                "factur-x-en16931",
                "factur-x-extended",
                "fr-ctc-cdar"
            ]
        );
    }

    #[test]
    fn bad_options() {
        for (bad, message) in [
            (
                &["--families", "cen,nope"][..],
                "--families: unknown name \"nope\"",
            ),
            (
                &["--runs", "0"],
                "--runs: expected a positive integer, not \"0\"",
            ),
            (
                &["--runs", "x"],
                "--runs: expected a positive integer, not \"x\"",
            ),
            (
                &["--threads", "1,,8"],
                "--threads: expected a positive integer, not \"\"",
            ),
            (&["--threads", "8,8"], "--threads: 8 given twice"),
            (&["--runs", "1", "--runs", "2"], "--runs given twice"),
            (&["--runs"], "--runs needs a value"),
            (&["--bogus"], "unexpected argument --bogus"),
        ] {
            let error = args(bad).unwrap_err().to_string();
            assert!(error.starts_with(message), "{bad:?}: {error}");
        }
    }

    #[test]
    fn engine_command_lines() {
        let files = ["in/c00001.xml".to_owned(), "in/c00002.xml".to_owned()];
        let line = |source, depth| -> Vec<String> {
            engine_args(&source, 8, depth, &files)
                .into_iter()
                .map(|a| a.into_string().unwrap())
                .collect()
        };
        assert_eq!(
            line(RuleSource::Named("cen-ubl"), None),
            [
                "--rules",
                "cen-ubl",
                "--format",
                "json",
                "--jobs",
                "8",
                "--",
                "in/c00001.xml",
                "in/c00002.xml"
            ]
        );
        assert_eq!(
            line(RuleSource::Bundle(PathBuf::from("/b")), Some(1024)),
            [
                "--bundle",
                "/b",
                "--format",
                "json",
                "--jobs",
                "8",
                "--max-depth",
                "1024",
                "--",
                "in/c00001.xml",
                "in/c00002.xml"
            ]
        );
    }

    #[test]
    fn machine_files() {
        let cpuinfo = "processor\t: 0\nmodel name\t: Some CPU\nphysical id\t: 0\ncore id\t\t: 0\n\nprocessor\t: 1\nmodel name\t: Some CPU\nphysical id\t: 0\ncore id\t\t: 0\n\nprocessor\t: 2\nmodel name\t: Some CPU\nphysical id\t: 0\ncore id\t\t: 1\n";
        assert_eq!(
            parse_cpuinfo(cpuinfo),
            (Some("Some CPU".to_owned()), 3, Some(2))
        );
        assert_eq!(parse_cpuinfo("processor : 0\n"), (None, 1, None));
        assert_eq!(
            os_name("NAME=\"Debian\"\nPRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\n"),
            Some("Debian GNU/Linux 13 (trixie)".to_owned())
        );
        assert_eq!(os_name("NAME=x\n"), None);
    }

    /// Every rule set has jobs on its corpus, the job names are unique, and
    /// no group has the name of a job.
    #[test]
    fn every_rule_set_is_measured() {
        for rule_set in crate::ruleset::ALL {
            assert!(
                TARGETS.iter().any(|t| t.rule_set.name == rule_set.name),
                "{}",
                rule_set.name
            );
        }
        let mut keys: Vec<&str> = TARGETS.iter().map(|t| t.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), TARGETS.len());
        for (group, _) in GROUPS {
            assert!(!keys.contains(group), "{group}");
            assert_ne!(
                args(&["--families", group]).unwrap().targets,
                Vec::<&str>::new()
            );
        }
    }

    /// The JSON report of four documents: path, status, schema errors.
    fn report(documents: &[(&str, &str, usize)]) -> String {
        let documents: Vec<String> = documents
            .iter()
            .map(|(path, status, schema_errors)| {
                format!(
                    "{{\"path\": \"{path}\", \"status\": \"{status}\", \"counts\": {{\"schema_errors\": {schema_errors}, \"fatal\": 0}}}}"
                )
            })
            .collect();
        format!(
            "{{\"documents\": [{}], \"summary\": {{}}}}",
            documents.join(", ")
        )
    }

    /// The verdicts of the tool are checked document by document against the
    /// reference: xmllint's schema verdict where there is one, reversed for
    /// a documented difference, the tool's own otherwise.
    #[test]
    fn verdicts_are_the_reference_ones() {
        let case = |id: &str| Case {
            id: id.to_owned(),
            input: id.to_owned(),
            expectation: None,
            mutation: false,
        };
        let corpus = Corpus {
            rule_set: "synthetic",
            dir: PathBuf::from("/nowhere"),
            cases: ["a", "b", "c", "d"].map(case).to_vec(),
            files: ["in/1", "in/2", "in/3", "in/4"].map(str::to_owned).to_vec(),
            max_depth: None,
            sha256: Vec::new(),
        };
        let reference = |unmatched, schema: Option<&str>, stylesheets: &[&'static str]| Reference {
            unmatched,
            schema: schema.map(str::to_owned),
            stylesheets: stylesheets.to_vec(),
        };
        let verdicts = Verdicts {
            references: vec![
                reference(false, Some("valid"), &["accept"]),
                // xmllint finds it invalid, a documented difference: the
                // stylesheet decides, and it stopped.
                reference(false, Some("invalid"), &["error"]),
                // No recorded schema verdict: the tool's own.
                reference(false, None, &["accept"]),
                reference(true, None, &[]),
            ],
            documented: HashSet::from(["b".to_owned()]),
        };
        let good = [
            ("in/1", "valid", 0),
            ("in/2", "error", 0),
            ("in/3", "invalid", 2),
            ("in/4", "invalid", 0),
        ];
        assert_eq!(
            verdicts.check(&corpus, "x", &report(&good)).unwrap(),
            Summary {
                documents: 4,
                valid: 1,
                invalid: 2,
                errors: 1
            }
        );
        for (bad, message) in [
            (
                [("in/1", "invalid", 0), good[1], good[2], good[3]],
                "a: invalid, the reference valid (0 schema errors, stylesheets [\"accept\"])",
            ),
            (
                [("in/1", "invalid", 1), good[1], good[2], good[3]],
                "a: 1 schema errors, xmllint valid",
            ),
            (
                [good[0], ("in/2", "invalid", 3), good[2], good[3]],
                "b: 3 schema errors, xmllint invalid, listed in crates/einvoice-cli/tests/schema-disagreements.tsv but not different",
            ),
            (
                [good[0], good[1], good[2], ("in/4", "valid", 0)],
                "d: valid, the reference invalid (0 schema errors, stylesheets [], no scenario)",
            ),
        ] {
            let error = verdicts
                .check(&corpus, "x", &report(&bad))
                .unwrap_err()
                .to_string();
            assert!(error.starts_with("x: the verdicts of einvoice-validate are not the reference's (1 valid / 2 invalid / 1 no verdict): "), "{error}");
            assert!(error.contains(message), "{message}: {error}");
        }
        let error = verdicts
            .check(&corpus, "x", &report(&[good[1], good[0], good[2], good[3]]))
            .unwrap_err()
            .to_string();
        assert_eq!(error, "x: the report of in/1 (a) is not in its place");
        let error = verdicts
            .check(&corpus, "x", &report(&good[..3]))
            .unwrap_err()
            .to_string();
        assert_eq!(error, "x: 3 documents reported of 4");
    }

    /// The documented differences of the schema step with xmllint are read
    /// for the rule set named, and only for it.
    #[test]
    fn documented_schema_differences_by_rule_set() {
        let root = repo_root();
        let listed = documented_schema_differences(&root, "factur-x-en16931").unwrap();
        assert!(listed.contains("mutations/long/huge-decimal.xml"));
        assert!(
            documented_schema_differences(&root, "cen-ubl")
                .unwrap()
                .is_empty()
        );
    }
}
