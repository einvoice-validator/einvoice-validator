//! What the XSLT loader and executor of this project say of the three stylesheets of the
//! ISO Schematron skeleton that ph-schematron 10.3.1 runs (the third importing the
//! skeleton itself): each is loaded (`einvoice_xslt::load_with_imports`) and prepared
//! (`einvoice_xslt_exec::Executable::new`), and every error is listed, grouped by kind,
//! with the number of occurrences and the first lines.
//!
//! ```text
//! cargo run -p einvoice-schematron --example skeleton-probe [DIR]
//! ```
//!
//! The stylesheets are read from `DIR`, by default the copy of the skeleton in the test
//! data of this crate (`tests/data/skeleton/`).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

const STYLESHEETS: [&str; 3] = [
    "iso_dsdl_include.xsl",
    "iso_abstract_expand.xsl",
    "iso_svrl_for_xslt2.xsl",
];

struct Dir(PathBuf);

impl einvoice_xslt::ModuleResolver for Dir {
    fn resolve(&self, uri: &str) -> Option<String> {
        let name = uri.rsplit('/').next()?;
        std::fs::read_to_string(self.0.join(name)).ok()
    }
}

fn report(name: &str, what: &str, errors: &[(String, u32)]) {
    // Grouped by what is refused, the line numbers dropped.
    let mut groups: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for (message, line) in errors {
        let what = message
            .split_once(": ")
            .map_or(message.as_str(), |(_, rest)| rest)
            .to_owned();
        groups.entry(what).or_default().push(*line);
    }
    println!(
        "{name}: {what}, {} errors, {} distinct",
        errors.len(),
        groups.len()
    );
    for (what, lines) in &groups {
        let first: Vec<String> = lines.iter().take(5).map(u32::to_string).collect();
        println!(
            "  {:>4} x {what} (lines {}{})",
            lines.len(),
            first.join(", "),
            if lines.len() > 5 { ", ..." } else { "" }
        );
    }
}

fn main() -> ExitCode {
    let dir = std::env::args().nth(1).map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("data")
                .join("skeleton")
        },
        PathBuf::from,
    );
    let resolver = Dir(dir.clone());
    let mut status = ExitCode::SUCCESS;
    for name in STYLESHEETS {
        let path = dir.join(name);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        };
        let uri = format!("skeleton:/{name}");
        let sheet = match einvoice_xslt::load_with_imports(&text, &uri, &resolver) {
            Ok(sheet) => sheet,
            Err(errors) => {
                status = ExitCode::FAILURE;
                report(
                    name,
                    "refused by the loader",
                    &errors
                        .0
                        .iter()
                        .map(|e| (e.to_string(), e.line))
                        .collect::<Vec<_>>(),
                );
                continue;
            }
        };
        match einvoice_xslt_exec::Executable::new(&sheet) {
            Ok(_) => println!("{name}: loads and prepares"),
            Err(errors) => {
                status = ExitCode::FAILURE;
                report(
                    name,
                    "refused by the executor",
                    &errors
                        .0
                        .iter()
                        .map(|e| (e.to_string(), e.line))
                        .collect::<Vec<_>>(),
                );
            }
        }
    }
    status
}
