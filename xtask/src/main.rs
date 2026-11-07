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
//!                                   build the Peppol corpus and its goldens in the cache
//!                                   of scripts/fetch-peppol.sh, then cross-check them
//! ```
//!
//! `manifest`, `corpus`, `oracle` and `crosscheck` work on the vendored rule
//! sets, whose corpus is committed. Without `--write` they only check, and
//! exit with status 1 on any difference. `peppol` always writes, into the
//! cache under `target/peppol/` (the Peppol files may not be redistributed),
//! and fails when the cross-check differs from the committed record.
//! `oracle` and `peppol` take Java from `--java`, else `$JAVA`, else `java` on
//! the PATH, and the Saxon-HE jar from `--saxon`, else `$SAXON_JAR`, else
//! where `scripts/fetch-saxon.sh` puts it; the two XML Resolver jars that
//! Saxon-HE 12.10 needs are taken from the directory of Saxon's jar.

mod corpus;
mod crosscheck;
mod manifest;
mod mutation;
mod oracle;
mod peppol;
mod ruleset;
mod svrl;
mod toml;
mod util;
mod vefa;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use util::{Error, Result};

const USAGE: &str = "usage: cargo xtask (manifest [--write] | corpus [--write] | oracle [--write] [--java JAVA] [--saxon JAR] | crosscheck | peppol [--java JAVA] [--saxon JAR])";

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
    let mut write = false;
    let mut java = None;
    let mut saxon = None;
    let mut rest = rest.iter();
    while let Some(arg) = rest.next() {
        match arg.to_str() {
            Some("--write") => write = true,
            Some(flag @ ("--java" | "--saxon")) if command == "oracle" || command == "peppol" => {
                let value = rest
                    .next()
                    .ok_or_else(|| Error::new(format!("{flag} needs a value\n{USAGE}")))?;
                if flag == "--java" {
                    java = Some(value.clone());
                } else {
                    saxon = Some(PathBuf::from(value));
                }
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
        "peppol" if !write => run_peppol(&root, &options),
        _ => Err(Error::new(USAGE)),
    }
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
            crosscheck::RECORD
        )))
    }
}

fn run_peppol(root: &Path, options: &oracle::Options) -> Result<()> {
    let (text, consistent) = peppol::run(root, options)?;
    print!("{text}");
    if consistent {
        Ok(())
    } else {
        Err(Error::new(format!(
            "observed disagreements differ from {}/{}",
            ruleset::PEPPOL_UBL.record_dir(),
            crosscheck::RECORD
        )))
    }
}
