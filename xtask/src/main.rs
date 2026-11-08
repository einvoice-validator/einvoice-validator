//! Repository tooling, run with `cargo xtask <command>`:
//!
//! ```text
//! cargo xtask manifest [--write]    check (or rewrite) SHA256SUMS of the vendored artefacts
//! cargo xtask corpus [--write]      check (or regenerate) the cases extracted from VEFA test sets
//!                                   and the documents of the mutation catalogue
//! cargo xtask oracle [--write] [--java JAVA] [--saxon JAR]
//!                                   run Saxon-HE on every case and compare with (or write) the goldens
//! cargo xtask crosscheck            compare the goldens with the VEFA expectations
//! cargo xtask peppol [--java JAVA] [--saxon JAR]
//! cargo xtask facturx [--java JAVA] [--saxon JAR]
//! cargo xtask xrechnung [--java JAVA] [--saxon JAR]
//! cargo xtask fr-ctc [--java JAVA] [--saxon JAR]
//!                                   build the corpora of a family of fetched rule sets and their
//!                                   goldens in the cache of its fetch script (scripts/fetch-peppol.sh,
//!                                   fetch-facturx.sh, fetch-xrechnung.sh, fetch-fr-ctc.sh), then
//!                                   cross-check them
//! cargo xtask engine-status FAMILY [--cli PATH]
//!                                   run einvoice-validate on those corpora, one stylesheet at a
//!                                   time, and report what passes, differs, fails or is refused
//! cargo xtask bench [--families NAME,...] [--runs N] [--threads N,...] [--write]
//!                   [--cli PATH] [--java JAVA] [--saxon JAR]
//!                                   time and memory of the whole job on every corpus,
//!                                   einvoice-validate against Saxon-HE (docs/benchmarks.md)
//! ```
//!
//! `manifest`, `corpus`, `oracle` and `crosscheck` work on the vendored rule
//! sets, whose corpus is committed. Without `--write` they only check, and
//! exit with status 1 on any difference. The fetched families always write,
//! into the cache under `target/` (their files may not be redistributed), and
//! fail when a cross-check differs from the committed record.
//! `oracle` and the families take Java from `--java`, else `$JAVA`, else `java` on
//! the PATH, and the Saxon-HE jar from `--saxon`, else `$SAXON_JAR`, else
//! where `scripts/fetch-saxon.sh` puts it; the two XML Resolver jars that
//! Saxon-HE 12.10 needs are taken from the directory of Saxon's jar.
//! `engine-status` and `bench` take the tool from `--cli`, else
//! `target/release/einvoice-validate`; `bench` takes Java and Saxon-HE as
//! `oracle` does (see the `bench` module). `bench-measure` is its helper.

mod acceptance;
mod bench;
mod bench_report;
mod corpus;
mod crosscheck;
mod engine;
mod fetched;
mod manifest;
mod mutation;
mod oracle;
mod ruleset;
mod svrl;
mod toml;
mod util;
mod vefa;
mod xmute;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use util::{Error, Result};

const USAGE: &str = "usage: cargo xtask (manifest [--write] | corpus [--write] | oracle [--write] [--java JAVA] [--saxon JAR] | crosscheck | (peppol | facturx | xrechnung | fr-ctc) [--java JAVA] [--saxon JAR] | engine-status (peppol | facturx | xrechnung | fr-ctc) [--cli PATH] | bench [--families NAME,...] [--runs N] [--threads N,...] [--write] [--cli PATH] [--java JAVA] [--saxon JAR])";

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match dispatch(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(args: &[OsString]) -> Result<()> {
    let (command, rest) = args.split_first().ok_or_else(|| Error::new(USAGE))?;
    let command = command.to_str().ok_or_else(|| Error::new(USAGE))?;
    match command {
        "bench" => return run_bench(rest),
        "bench-measure" => return bench::measure_main(rest),
        _ => {}
    }
    let family = |name: &str| ruleset::FAMILIES.iter().copied().find(|f| f.name == name);
    let mut write = false;
    let mut java = None;
    let mut saxon = None;
    let mut cli = None;
    let mut target = None;
    let runs_saxon = command == "oracle" || family(command).is_some();
    let mut rest = rest.iter();
    while let Some(arg) = rest.next() {
        match arg.to_str() {
            Some("--write") => write = true,
            Some(flag @ ("--java" | "--saxon")) if runs_saxon => {
                let value = rest
                    .next()
                    .ok_or_else(|| Error::new(format!("{flag} needs a value\n{USAGE}")))?;
                if flag == "--java" {
                    java = Some(value.clone());
                } else {
                    saxon = Some(PathBuf::from(value));
                }
            }
            Some("--cli") if command == "engine-status" => {
                let value = rest
                    .next()
                    .ok_or_else(|| Error::new(format!("--cli needs a value\n{USAGE}")))?;
                cli = Some(PathBuf::from(value));
            }
            Some(name)
                if command == "engine-status" && target.is_none() && family(name).is_some() =>
            {
                target = family(name);
            }
            _ => {
                return Err(Error::new(format!(
                    "unexpected argument {}\n{USAGE}",
                    arg.display()
                )));
            }
        }
    }
    let root = util::repo_root();
    let options = oracle::Options {
        java: java
            .or_else(|| std::env::var_os("JAVA"))
            .unwrap_or_else(|| OsString::from("java")),
        saxon: saxon
            .or_else(|| std::env::var_os("SAXON_JAR").map(PathBuf::from))
            .unwrap_or_else(|| util::join(&root, oracle::SAXON_DEFAULT)),
        write,
    };
    match command {
        "manifest" => run_manifest(&root, write),
        "corpus" => run_corpus(&root, write),
        "oracle" => run_oracle(&root, &options),
        "crosscheck" if !write => run_crosscheck(&root),
        "engine-status" if !write => {
            let family = target
                .ok_or_else(|| Error::new(format!("engine-status needs a family\n{USAGE}")))?;
            let cli = cli.unwrap_or_else(|| util::join(&root, engine::CLI_DEFAULT));
            print!("{}", engine::run(&root, family, &cli)?);
            Ok(())
        }
        name if !write && family(name).is_some() => match family(name) {
            Some(family) => run_family(&root, family, &options),
            None => Err(Error::new(USAGE)),
        },
        _ => Err(Error::new(USAGE)),
    }
}

fn run_bench(args: &[OsString]) -> Result<()> {
    let args = bench::parse_args(args)?;
    let root = util::repo_root();
    let options = bench::Options {
        cli: args
            .cli
            .clone()
            .unwrap_or_else(|| util::join(&root, engine::CLI_DEFAULT)),
        java: args
            .java
            .clone()
            .or_else(|| std::env::var_os("JAVA"))
            .unwrap_or_else(|| OsString::from("java")),
        saxon: args
            .saxon
            .clone()
            .or_else(|| std::env::var_os("SAXON_JAR").map(PathBuf::from))
            .unwrap_or_else(|| util::join(&root, oracle::SAXON_DEFAULT)),
        args,
    };
    bench::run(&root, &options)
}

fn run_manifest(root: &Path, write: bool) -> Result<()> {
    for rule_set in ruleset::VENDORED {
        let count = if write {
            manifest::write(root, rule_set)?
        } else {
            manifest::verify(root, rule_set)?
        };
        let verb = if write { "written" } else { "verified" };
        println!("manifest {verb}: {}: {count} vendored files", rule_set.name);
    }
    Ok(())
}

fn run_corpus(root: &Path, write: bool) -> Result<()> {
    for rule_set in ruleset::VENDORED {
        let counts = if write {
            corpus::write(root, rule_set)?
        } else {
            corpus::verify(root, rule_set)?
        };
        let verb = if write { "written" } else { "verified" };
        println!(
            "corpus {verb}: {}: {} cases, {} mutation cases",
            rule_set.name, counts.cases, counts.mutations
        );
    }
    Ok(())
}

fn run_oracle(root: &Path, options: &oracle::Options) -> Result<()> {
    let mut problems = Vec::new();
    for rule_set in ruleset::VENDORED {
        for outcome in oracle::run(root, rule_set, options)? {
            println!(
                "oracle {}: {} {}: {} cases, {} with findings, {} failed asserts, {} successful reports, {} stopped by an error, {} with pattern warnings",
                if options.write { "written" } else { "checked" },
                rule_set.name,
                outcome.stylesheet,
                outcome.cases,
                outcome.cases_with_findings,
                outcome.failed_asserts,
                outcome.successful_reports,
                outcome.errors,
                outcome.warned
            );
            problems.extend(outcome.problems);
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(Error::new(format!(
            "{} goldens do not match Saxon (run `cargo xtask oracle --write` and review the diff):\n  {}",
            problems.len(),
            problems.join("\n  ")
        )))
    }
}

fn run_crosscheck(root: &Path) -> Result<()> {
    let mut consistent = true;
    for rule_set in ruleset::VENDORED {
        let (text, ok) = crosscheck::report(root, rule_set)?;
        print!("{text}");
        consistent &= ok;
    }
    if consistent {
        Ok(())
    } else {
        Err(Error::new(format!(
            "observed disagreements differ from {}",
            ruleset::VEFA_RECORD
        )))
    }
}

fn run_family(root: &Path, family: &ruleset::Family, options: &oracle::Options) -> Result<()> {
    let (text, consistent) = fetched::run(root, family, options)?;
    print!("{text}");
    if consistent {
        Ok(())
    } else {
        Err(Error::new(format!(
            "{}: observed disagreements differ from the committed records",
            family.name
        )))
    }
}
