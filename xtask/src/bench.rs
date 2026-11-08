//! `cargo xtask bench`: the time and memory of a whole validation job,
//! `einvoice-validate` against Saxon-HE, on the corpus of each rule set.
//!
//! For each rule set selected (`--families`, by default all nine), the job is
//! every document of its corpus (the cases of `cases.tsv` and the mutation
//! cases, staged under plain names in `target/bench/<rule set>/in/`) through
//! every stylesheet of the rule set:
//!
//! - `einvoice-validate` (the release build, `--cli`, by default
//!   `target/release/einvoice-validate`), one process for the whole job:
//!   `--format json --jobs N`, with the built-in rule set (`cen-ubl`,
//!   `cen-cii`) or a bundle of the rule set's stylesheets and the files they
//!   read; the bundle of a UBL rule set names the UBL 2.1 schema, so that the
//!   tool validates as it does for its users, the XML schema included. In
//!   that run, a stylesheet that stops with a dynamic error on a document
//!   without schema errors ends the document's validation: the stylesheets
//!   after it do not run on it;
//! - `einvoice-validate` doing exactly the work of Saxon-HE, when that run
//!   does not (a UBL rule set, or several stylesheets): one process per
//!   stylesheet, each with a bundle of that stylesheet alone and no schema,
//!   so that every stylesheet runs on every document;
//! - Saxon-HE 12.10 (the pinned jars of `scripts/fetch-saxon.sh`), one JVM
//!   per stylesheet (`net.sf.saxon.Transform` runs one stylesheet), on the
//!   directory of the documents with `-threads:N` and `-t`. On one thread,
//!   the `-t` timings (the compilation of the stylesheet and the execution of
//!   each document) also give the time Saxon-HE spends without the start of
//!   the JVM and the loading of its classes; on several threads the
//!   documents' times overlap, and that figure is not given.
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
//! document (exit status 0, 1 or 3) with the same counts on every run, and
//! Saxon-HE must have processed every document (any failed transformation is
//! counted and reported).
//!
//! The results go to standard output as markdown, to
//! `target/bench/benchmarks.md` and `target/bench/benchmarks.json`, and with
//! `--write` to `docs/benchmarks.json` and between the markers of
//! `docs/benchmarks.md`.

use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use crate::bench_report::{
    self, Exit, Latency, Machine, Outcome, Report, Sample, Summary, TargetRun, Tool, ToolRun,
};
use crate::corpus;
use crate::engine;
use crate::fetched;
use crate::oracle;
use crate::ruleset::{self, Family, RuleSet, Stylesheet};
use crate::util::{self, Error, Result};

pub(crate) const USAGE: &str = "usage: cargo xtask bench [--families NAME,...] [--runs N] [--threads N,...] [--write] [--cli PATH] [--java JAVA] [--saxon JAR]\n  names: all, cen, cen-ubl, cen-cii, peppol, facturx, xrechnung, xrechnung-ubl, xrechnung-cii, fr-ctc, fr-ctc-cii, fr-ctc-ubl, fr-ctc-cdar";

/// The scratch directory, relative to the repository root.
const WORK_DIR: &str = "target/bench";
/// The committed results, relative to the repository root.
pub(crate) const DOC: &str = "docs/benchmarks.md";
pub(crate) const DATA: &str = "docs/benchmarks.json";

/// The document of the single-document latency, relative to the repository
/// root: a small invoice of the CEN examples.
const LATENCY_DOCUMENT: &str =
    "artefacts/cen-en16931-ubl-1.3.17/ubl/examples/ubl-tc434-example9.xml";

/// The rule sets measured.
#[derive(Debug)]
pub(crate) struct Target {
    pub(crate) key: &'static str,
    pub(crate) title: &'static str,
    pub(crate) rule_set: &'static RuleSet,
    /// The family of a fetched rule set; `None` when vendored.
    family: Option<&'static Family>,
    /// The built-in rule set of `einvoice-validate` with these stylesheets.
    builtin: Option<&'static str>,
    /// Whether documents are UBL: the bundle then names the UBL 2.1 schema.
    ubl: bool,
}

pub(crate) const TARGETS: &[Target] = &[
    Target {
        key: "cen-ubl",
        title: "CEN EN 16931 UBL 1.3.17",
        rule_set: &ruleset::CEN_UBL,
        family: None,
        builtin: Some("cen-ubl"),
        ubl: true,
    },
    Target {
        key: "cen-cii",
        title: "CEN EN 16931 CII 1.3.17",
        rule_set: &ruleset::CEN_CII,
        family: None,
        builtin: Some("cen-cii"),
        ubl: false,
    },
    Target {
        key: "peppol",
        title: "Peppol BIS Billing 3.0.22 (UBL)",
        rule_set: &ruleset::PEPPOL_UBL,
        family: Some(&ruleset::PEPPOL),
        builtin: None,
        ubl: true,
    },
    Target {
        key: "facturx",
        title: "Factur-X 1.09.2 (CII), five profiles",
        rule_set: &ruleset::FACTURX,
        family: Some(&ruleset::FACTURX_FAMILY),
        builtin: None,
        ubl: false,
    },
    Target {
        key: "xrechnung-ubl",
        title: "XRechnung 3.0.2 UBL",
        rule_set: &ruleset::XRECHNUNG_UBL,
        family: Some(&ruleset::XRECHNUNG),
        builtin: None,
        ubl: true,
    },
    Target {
        key: "xrechnung-cii",
        title: "XRechnung 3.0.2 CII",
        rule_set: &ruleset::XRECHNUNG_CII,
        family: Some(&ruleset::XRECHNUNG),
        builtin: None,
        ubl: false,
    },
    Target {
        key: "fr-ctc-cii",
        title: "French rules 1.4.0.04 CII",
        rule_set: &ruleset::FR_CTC_CII,
        family: Some(&ruleset::FR_CTC),
        builtin: None,
        ubl: false,
    },
    Target {
        key: "fr-ctc-ubl",
        title: "French rules 1.4.0.04 UBL",
        rule_set: &ruleset::FR_CTC_UBL,
        family: Some(&ruleset::FR_CTC),
        builtin: None,
        ubl: true,
    },
    Target {
        key: "fr-ctc-cdar",
        title: "French rules 1.4.0.04 CDAR (life-cycle statuses)",
        rule_set: &ruleset::FR_CTC_CDAR,
        family: Some(&ruleset::FR_CTC),
        builtin: None,
        ubl: false,
    },
];

/// The names `--families` takes besides the keys of [`TARGETS`]: groups of them.
const GROUPS: &[(&str, &[&str])] = &[
    ("cen", &["cen-ubl", "cen-cii"]),
    ("xrechnung", &["xrechnung-ubl", "xrechnung-cii"]),
    ("fr-ctc", &["fr-ctc-cii", "fr-ctc-ubl", "fr-ctc-cdar"]),
];

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

/// The keys of the rule sets that a comma-separated list of names selects, in
/// the order of [`TARGETS`].
fn select(names: &str) -> Result<Vec<&'static str>> {
    let mut wanted: Vec<&str> = Vec::new();
    for name in names.split(',') {
        if name == "all" {
            wanted.extend(TARGETS.iter().map(|t| t.key));
        } else if let Some((_, keys)) = GROUPS.iter().find(|(g, _)| *g == name) {
            wanted.extend(keys.iter().copied());
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
    };
    let args = &options.args;
    let targets: Vec<&Target> = TARGETS
        .iter()
        .filter(|t| args.targets.contains(&t.key))
        .collect();
    // Every cache is checked before anything is measured.
    for target in &targets {
        check_corpus(root, target)?;
    }
    let work = util::join(root, WORK_DIR);
    let mut machine = machine(root, &tools)?;
    let mut results = Vec::new();
    for target in targets {
        results.push(bench_target(root, &work, &tools, target, args)?);
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
/// by `cargo xtask <family>`).
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

/// A rule set ready to be measured: its staged documents and bundles.
struct Staged {
    dir: PathBuf,
    /// The documents, relative to `dir`.
    files: Vec<String>,
    /// Absolute paths of the stylesheets, for Saxon-HE.
    stylesheets: Vec<PathBuf>,
    /// The rule set as `einvoice-validate` validates with it.
    rule_set: RuleSource,
    /// For [`Tool::EngineSameWork`]: one bundle per stylesheet, without
    /// schema; empty when [`Tool::Engine`] already does that work.
    same_work: Vec<PathBuf>,
    max_depth: Option<usize>,
}

/// How `einvoice-validate` gets the rule set.
enum RuleSource {
    Builtin(&'static str),
    Bundle(PathBuf),
}

fn stage(root: &Path, work: &Path, target: &Target) -> Result<Staged> {
    let rule_set = target.rule_set;
    let base = rule_set.base(root);
    let dir = work.join(target.key);
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
    let stylesheets = rule_set
        .stylesheets
        .iter()
        .map(|s| absolute(&util::join(&base, s.xslt)))
        .collect::<Result<Vec<_>>>()?;
    let all: Vec<&Stylesheet> = rule_set.stylesheets.iter().collect();
    let source = match target.builtin {
        Some(name) => RuleSource::Builtin(name),
        None => RuleSource::Bundle(write_bundle(
            &dir.join("bundle"),
            &base,
            target,
            &all,
            target.ubl,
        )?),
    };
    let mut same_work = Vec::new();
    if target.ubl || all.len() > 1 {
        for (index, stylesheet) in all.iter().enumerate() {
            same_work.push(write_bundle(
                &dir.join(format!("bundle-{index}")),
                &base,
                target,
                &[stylesheet],
                false,
            )?);
        }
    }
    Ok(Staged {
        dir: absolute(&dir)?,
        files,
        stylesheets,
        rule_set: source,
        same_work,
        max_depth: (deepest > engine::DEFAULT_MAX_DEPTH).then_some(engine::RAISED_MAX_DEPTH),
    })
}

/// Writes a bundle of `stylesheets` and of the files they read with
/// `document()`, at their paths relative to the base directory, naming the
/// UBL 2.1 schema when `schema`.
fn write_bundle(
    dir: &Path,
    base: &Path,
    target: &Target,
    stylesheets: &[&Stylesheet],
    schema: bool,
) -> Result<PathBuf> {
    util::remove_dir(dir)?;
    let mut manifest = format!(
        "# {}, written by `cargo xtask bench`.\nformat = 2\nname = \"bench-{}\"\ntitle = \"{}\"\nversion = \"0\"\n",
        target.title, target.key, target.title
    );
    if schema {
        manifest.push_str("schema = \"ubl-2.1\"\n");
    }
    let mut resources: Vec<String> = Vec::new();
    for stylesheet in stylesheets {
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
            if !resources.contains(&path) {
                resources.push(path);
            }
        }
    }
    for path in &resources {
        let bytes = util::read(&util::join(base, path))?;
        util::write(&util::join(dir, path), &bytes)?;
        let _ = write!(
            manifest,
            "\n[[resource]]\npath = \"{path}\"\nsha256 = \"{}\"\n",
            util::sha256_hex(&bytes)
        );
    }
    util::write(&dir.join("bundle.toml"), manifest.as_bytes())?;
    absolute(dir)
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
    work: &Path,
    tools: &Tools,
    target: &Target,
    args: &Args,
) -> Result<TargetRun> {
    eprintln!("bench: {}: staging the corpus", target.key);
    let staged = stage(root, work, target)?;
    let documents = staged.files.len();
    eprintln!(
        "bench: {}: {documents} documents, {} stylesheets",
        target.key,
        staged.stylesheets.len()
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
        schema: target.ubl,
        max_depth: staged.max_depth,
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
        RuleSource::Builtin(name) => {
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

/// One run of `einvoice-validate` on every staged document.
fn engine_run(
    tools: &Tools,
    staged: &Staged,
    source: &RuleSource,
    threads: usize,
) -> Result<(Sample, Outcome)> {
    let stdout = staged.dir.join("engine.json");
    let stderr = staged.dir.join("engine.stderr");
    let args = engine_args(source, threads, staged.max_depth, &staged.files);
    let (sample, exit) = measure(
        tools,
        &staged.dir,
        &stdout,
        &stderr,
        tools.cli.as_os_str(),
        &args,
    )?;
    engine_outcome(&stdout, &stderr, exit, staged.files.len()).map(|o| (sample, o))
}

/// One run of `einvoice-validate` per stylesheet, each with the bundle of
/// that stylesheet alone, on every staged document: the counts of the
/// outcome are of transformations.
fn same_work_run(tools: &Tools, staged: &Staged, threads: usize) -> Result<(Sample, Outcome)> {
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
fn saxon_run(tools: &Tools, staged: &Staged, threads: usize) -> Result<(Sample, Outcome)> {
    let input = staged.dir.join("in");
    let mut total: Option<Sample> = None;
    let mut failed = 0;
    for (index, xslt) in staged.stylesheets.iter().enumerate() {
        let output = staged.dir.join(format!("saxon-{index}"));
        util::remove_dir(&output)?;
        std::fs::create_dir_all(&output)
            .map_err(|e| Error::new(format!("cannot create {}: {e}", output.display())))?;
        let stderr = staged.dir.join(format!("saxon-{index}.stderr"));
        let args = saxon_args(tools, xslt, &input, &output, Some(threads));
        let (mut sample, exit) = measure(
            tools,
            &staged.dir,
            &staged.dir.join(format!("saxon-{index}.stdout")),
            &stderr,
            &tools.java,
            &args,
        )?;
        let text = util::read_string(&stderr)?;
        let timings = bench_report::parse_saxon_timings(&text)
            .map_err(|e| Error::new(format!("{e}:\n{}", head(&stderr))))?;
        if timings.processed != staged.files.len() {
            return Err(Error::new(format!(
                "Saxon-HE processed {} documents of {} with {}:\n{}",
                timings.processed,
                staged.files.len(),
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
    let no_schema = write_bundle(
        &dir.join("bundle-no-schema"),
        root,
        target,
        &[stylesheet],
        false,
    )?;
    let dir = absolute(&dir)?;
    let xslt = absolute(&util::join(root, stylesheet.xslt))?;
    let staged = Staged {
        dir: dir.clone(),
        files: vec![name.clone()],
        stylesheets: vec![xslt.clone()],
        rule_set: RuleSource::Builtin("cen-ubl"),
        same_work: vec![no_schema],
        max_depth: None,
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

    use super::{Args, RuleSource, TARGETS, engine_args, os_name, parse_args, parse_cpuinfo};

    fn args(list: &[&str]) -> crate::util::Result<Args> {
        parse_args(&list.iter().map(OsString::from).collect::<Vec<_>>())
    }

    #[test]
    fn defaults() {
        let parsed = args(&[]).unwrap();
        assert_eq!(parsed.targets.len(), 9);
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
        assert_eq!(args(&["--families", "all"]).unwrap().targets.len(), 9);
        assert_eq!(
            args(&["--families", "fr-ctc,peppol"]).unwrap().targets,
            ["peppol", "fr-ctc-cii", "fr-ctc-ubl", "fr-ctc-cdar"]
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
            line(RuleSource::Builtin("cen-ubl"), None),
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

    #[test]
    fn every_rule_set_is_measured_once() {
        let names: Vec<&str> = TARGETS.iter().map(|t| t.rule_set.name).collect();
        for rule_set in crate::ruleset::ALL {
            assert_eq!(
                names.iter().filter(|n| **n == rule_set.name).count(),
                1,
                "{}",
                rule_set.name
            );
        }
        assert_eq!(names.len(), crate::ruleset::ALL.len());
    }
}
