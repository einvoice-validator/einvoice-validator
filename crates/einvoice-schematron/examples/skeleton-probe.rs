//! What the XSLT loader of this project (`einvoice_xslt::load`) says of the stylesheets of
//! the ISO Schematron skeleton that ph-schematron 10.3.1 runs: each error, grouped by
//! kind, with the number of occurrences and the first lines. This is the work left before
//! the three XSLT steps can run in Rust.
//!
//! ```text
//! cargo run -p einvoice-schematron --example ph-schematron-oracle   # copies the skeleton
//! cargo run -p einvoice-schematron --example skeleton-probe [DIR]
//! ```
//!
//! The stylesheets are read from `DIR`, by default `target/schematron-oracle/skeleton/`,
//! where the `ph-schematron-oracle` example copies them from ph-schematron's class path.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

const STYLESHEETS: [&str; 4] = [
    "iso_dsdl_include.xsl",
    "iso_abstract_expand.xsl",
    "iso_svrl_for_xslt2.xsl",
    "iso_schematron_skeleton_for_saxon.xsl",
];

fn main() -> ExitCode {
    let dir = std::env::args().nth(1).map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/schematron-oracle/skeleton")
        },
        PathBuf::from,
    );
    let mut status = ExitCode::SUCCESS;
    for name in STYLESHEETS {
        let path = dir.join(name);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                eprintln!(
                    "{}: {e} (run the ph-schematron-oracle example first)",
                    path.display()
                );
                return ExitCode::FAILURE;
            }
        };
        match einvoice_xslt::load(&text) {
            Ok(_) => println!("{name}: loads"),
            Err(errors) => {
                status = ExitCode::FAILURE;
                // Grouped by what is refused, the line numbers dropped.
                let mut groups: BTreeMap<String, Vec<u32>> = BTreeMap::new();
                for error in &errors.0 {
                    let message = error.to_string();
                    let what = message
                        .split_once(": ")
                        .map_or(message.as_str(), |(_, rest)| rest)
                        .to_owned();
                    groups.entry(what).or_default().push(error.line);
                }
                println!(
                    "{name}: refused, {} errors, {} distinct",
                    errors.0.len(),
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
        }
    }
    status
}
