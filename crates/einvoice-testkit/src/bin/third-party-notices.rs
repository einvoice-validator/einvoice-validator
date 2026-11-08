//! The third-party notices files of the `einvoice-validate` binary and of the npm package
//! ([`einvoice_testkit::notices`]):
//!
//! ```text
//! cargo run --locked -p einvoice-testkit --bin third-party-notices            check them
//! cargo run --locked -p einvoice-testkit --bin third-party-notices -- --write  regenerate them
//! ```
//!
//! Exit status 1 when a file is not up to date (without `--write`) or cannot be generated.

use std::process::ExitCode;

use einvoice_testkit::notices::{self, ARTEFACTS};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let write = match args.as_slice() {
        [] => false,
        [flag] if flag == "--write" => true,
        _ => {
            eprintln!("usage: third-party-notices [--write]");
            return ExitCode::from(2);
        }
    };
    let mut status = ExitCode::SUCCESS;
    for artefact in &ARTEFACTS {
        let result = if write {
            notices::generate(artefact).and_then(|text| {
                std::fs::write(notices::path(artefact), &text)
                    .map(|()| text)
                    .map_err(|e| format!("{}: {e}", artefact.file))
            })
        } else {
            notices::check(artefact).and_then(|()| {
                std::fs::read_to_string(notices::path(artefact))
                    .map_err(|e| format!("{}: {e}", artefact.file))
            })
        };
        match result {
            Ok(text) => println!(
                "{}: {} crates, {} bytes{}",
                artefact.file,
                text.matches("\n== ").count(),
                text.len(),
                if write { ", written" } else { ", up to date" }
            ),
            Err(e) => {
                eprintln!("{e}");
                status = ExitCode::FAILURE;
            }
        }
    }
    status
}
