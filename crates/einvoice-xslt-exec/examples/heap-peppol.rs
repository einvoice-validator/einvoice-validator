//! Heap used by the Peppol BIS Billing 3.0.22 job (both stylesheets), measured by a
//! counting global allocator (every byte the program allocates, including the document
//! text and the SVRL):
//!
//! - the two prepared stylesheets, kept for the whole run;
//! - the peak while one document is validated by both (its text, its trees, the
//!   evaluation and the SVRL), above what was in use before, for the worst document;
//! - the peak while validating the whole corpus, documents processed one after the
//!   other, and the peak of the whole process, loading the stylesheets included.
//!
//! Measured twice: with the text read as a `String`, borrowed by the first stylesheet and
//! given to the last one (whose tree keeps it), then with the text borrowed by both
//! (each tree copies the texts of its nodes).
//!
//! Needs the cache built by `scripts/check.sh peppol`.
//!
//! ```text
//! cargo run --release -p einvoice-xslt-exec --example heap-peppol
//! ```

#[path = "../tests/common/mod.rs"]
mod common;

use std::process::ExitCode;

use common::peppol;
use peak_alloc::PeakAlloc;

#[global_allocator]
static ALLOCATOR: PeakAlloc = PeakAlloc;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn kib(bytes: usize) -> String {
    format!(
        "{:.1} KiB",
        f64::from(u32::try_from(bytes).unwrap_or(u32::MAX)) / 1024.0
    )
}

fn run() -> Result<(), String> {
    let root = common::workspace_root();
    let corpus = peppol::corpus(&root)?.ok_or_else(|| {
        format!(
            "no Peppol corpus in {}: run scripts/check.sh peppol",
            peppol::BASE
        )
    })?;

    let before = ALLOCATOR.current_usage();
    let mut executables = Vec::new();
    for stylesheet in &corpus.stylesheets {
        let start = ALLOCATOR.current_usage();
        executables.push(common::prepare(&stylesheet.xslt)?);
        println!(
            "{}: prepared stylesheet retains {}",
            stylesheet.name,
            kib(ALLOCATOR.current_usage() - start)
        );
    }
    let prepared = ALLOCATOR.current_usage();
    let loading_peak = ALLOCATOR.peak_usage();
    println!(
        "both prepared: {} retained (loading peaked at {})",
        kib(prepared - before),
        kib(loading_peak - before)
    );

    for owned in [true, false] {
        println!(
            "{}:",
            if owned {
                "text given (String) to the last stylesheet"
            } else {
                "text borrowed (&str)"
            }
        );
        pass(&executables, &corpus.cases, owned, prepared, loading_peak)?;
    }
    Ok(())
}

/// Both passes over the corpus, the text given to the last stylesheet or borrowed.
fn pass(
    executables: &[einvoice_xslt_exec::Executable],
    cases: &[peppol::Case],
    owned: bool,
    prepared: usize,
    loading_peak: usize,
) -> Result<(), String> {
    let mut worst = (0, String::new());
    let mut after_pass = Vec::new();
    let mut running_peak = 0;
    // Two passes: memory still in use after the first one must not grow in the second.
    for case in cases.iter().chain(cases) {
        let base = ALLOCATOR.current_usage();
        ALLOCATOR.reset_peak_usage();
        let text = common::read(std::path::Path::new(&case.input))?;
        let Some((last, others)) = executables.split_last() else {
            return Err("no stylesheet".to_owned());
        };
        for executable in others {
            drop(common::svrl(executable, &text, &case.input)?);
        }
        if owned {
            drop(common::svrl(last, text, &case.input)?);
        } else {
            drop(common::svrl(last, &text, &case.input)?);
            drop(text);
        }
        let peak = ALLOCATOR.peak_usage();
        running_peak = running_peak.max(peak);
        let peak = peak - base;
        if peak > worst.0 {
            worst = (peak, case.id.clone());
        }
        if std::ptr::eq(case, cases.last().unwrap_or(case)) {
            after_pass.push(ALLOCATOR.current_usage());
        }
    }
    println!(
        "  worst document: {}: peak {} above the prepared stylesheets",
        worst.1,
        kib(worst.0)
    );
    let running_peak = running_peak.max(worst.0 + prepared);
    println!(
        "  whole corpus ({} documents, one at a time, twice): peak while validating {} (stylesheets included), process peak {} (loading included); in use after each pass: {}",
        cases.len(),
        kib(running_peak),
        kib(running_peak.max(loading_peak)),
        after_pass
            .iter()
            .map(|&b| kib(b))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}
