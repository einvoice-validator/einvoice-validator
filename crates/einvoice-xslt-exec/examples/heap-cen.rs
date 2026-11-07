//! Heap used by the CEN EN 16931 UBL 1.3.17 job, measured by a counting global allocator
//! (every byte the program allocates, including the document text and the SVRL):
//!
//! - the prepared stylesheet, kept for the whole run;
//! - the peak while one document is transformed (its text, its tree, the evaluation and
//!   the SVRL), above what was in use before, for the largest corpus document and for the
//!   worst document of the corpus;
//! - the peak of the whole process over the whole corpus, documents processed one after
//!   the other.
//!
//! Measured twice: with the text read as a `String` and given to the transformation (the
//! tree keeps it), then with the text borrowed (the tree copies the texts of its nodes).
//!
//! ```text
//! cargo run --release -p einvoice-xslt-exec --example heap-cen
//! ```

#[path = "../tests/common/mod.rs"]
mod common;

use std::process::ExitCode;

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
    let cases = common::cases(&root)?;

    let before = ALLOCATOR.current_usage();
    let stylesheet = common::read(&common::join(&root, common::XSLT))?;
    let sheet = einvoice_xslt::load(&stylesheet).map_err(|e| e.to_string())?;
    let executable = einvoice_xslt_exec::Executable::new(&sheet).map_err(|e| e.to_string())?;
    let with_ir = ALLOCATOR.current_usage();
    drop(sheet);
    drop(stylesheet);
    let prepared = ALLOCATOR.current_usage();
    println!(
        "prepared stylesheet: {} retained (loading peaked at {}; text and IR, {} more, are freed)",
        kib(prepared - before),
        kib(ALLOCATOR.peak_usage() - before),
        kib(with_ir - prepared)
    );

    for owned in [true, false] {
        println!(
            "{}:",
            if owned {
                "text given (String)"
            } else {
                "text borrowed (&str)"
            }
        );
        pass(&executable, &cases, owned, prepared)?;
    }
    Ok(())
}

/// Both passes over the corpus, the text given to the transformation or borrowed.
fn pass(
    executable: &einvoice_xslt_exec::Executable,
    cases: &[common::Case],
    owned: bool,
    prepared: usize,
) -> Result<(), String> {
    let mut worst = (0, String::new());
    let mut largest = (0, 0, String::new());
    let mut after_pass = Vec::new();
    let mut process_peak = 0;
    // Two passes: memory still in use after the first one must not grow in the second.
    for case in cases.iter().chain(cases) {
        let base = ALLOCATOR.current_usage();
        ALLOCATOR.reset_peak_usage();
        let text = common::read(std::path::Path::new(&case.input))?;
        let size = text.len();
        let svrl = if owned {
            common::svrl(executable, text, &case.input)?
        } else {
            let svrl = common::svrl(executable, &text, &case.input)?;
            drop(text);
            svrl
        };
        let peak = ALLOCATOR.peak_usage();
        process_peak = process_peak.max(peak);
        let peak = peak - base;
        drop(svrl);
        if peak > worst.0 {
            worst = (peak, case.id.clone());
        }
        if size > largest.1 {
            largest = (peak, size, case.id.clone());
        }
        if std::ptr::eq(case, cases.last().unwrap_or(case)) {
            after_pass.push(ALLOCATOR.current_usage());
        }
    }
    println!(
        "  largest document: {} ({} bytes): peak {} above the prepared stylesheet",
        largest.2,
        largest.1,
        kib(largest.0)
    );
    println!("  worst document: {}: peak {}", worst.1, kib(worst.0));
    println!(
        "  whole corpus ({} documents, one at a time, twice): process peak {} (stylesheet included); in use after each pass: {}",
        cases.len(),
        kib(process_peak.max(worst.0 + prepared)),
        after_pass
            .iter()
            .map(|&b| kib(b))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}
