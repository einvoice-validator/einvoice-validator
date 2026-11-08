//! Heap used by the XML Schema validation of the CEN EN 16931 UBL 1.3.17 corpus,
//! measured by a counting global allocator, to run in release mode:
//!
//! ```text
//! cargo run --release -p einvoice-xsd --example heap
//! ```
//!
//! Prints what loading the embedded UBL 2.1 schemas retains (the compiled schema,
//! shared by every validation) and the peak while it loads; then, for each document
//! of the corpus validated in turn (the text already read), the peak above what was
//! in use before the validation (the text and the schema excluded): for the largest
//! document and for the worst one, and the peak of the process over the whole
//! corpus.

use std::path::PathBuf;

use peak_alloc::PeakAlloc;

#[global_allocator]
static ALLOCATOR: PeakAlloc = PeakAlloc;

fn kib(bytes: usize) -> String {
    format!(
        "{:.1} KiB",
        f64::from(u32::try_from(bytes).unwrap_or(u32::MAX)) / 1024.0
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cases = std::fs::read_to_string(root.join("corpus/cen-en16931-ubl-1.3.17/cases.tsv"))?;
    let mut paths = Vec::new();
    for line in cases.lines().skip(1) {
        let input = line.split('\t').nth(1).ok_or("bad cases.tsv line")?;
        paths.push((input.to_owned(), root.join(input)));
    }

    let before = ALLOCATOR.current_usage();
    ALLOCATOR.reset_peak_usage();
    let schema = einvoice_xsd::Schema::ubl_2_1()?;
    println!(
        "UBL 2.1 schemas loaded: {} retained (loading peaked at {})",
        kib(ALLOCATOR.current_usage() - before),
        kib(ALLOCATOR.peak_usage() - before)
    );

    let mut worst = (0, String::new());
    let mut largest = (0, 0, String::new());
    let mut process_peak = 0;
    let mut invalid = 0_usize;
    for (name, path) in &paths {
        let text = std::fs::read_to_string(path)?;
        let base = ALLOCATOR.current_usage();
        ALLOCATOR.reset_peak_usage();
        let report = schema.validate(&text)?;
        if !report.is_valid() {
            invalid += 1;
        }
        drop(report);
        let peak = ALLOCATOR.peak_usage();
        process_peak = process_peak.max(peak);
        let peak = peak - base;
        if peak > worst.0 {
            worst = (peak, name.clone());
        }
        if text.len() > largest.1 {
            largest = (peak, text.len(), name.clone());
        }
    }
    println!(
        "largest document: {} ({} bytes): peak {} above the text",
        largest.2,
        largest.1,
        kib(largest.0)
    );
    println!(
        "worst document: {}: peak {} above the text",
        worst.1,
        kib(worst.0)
    );
    println!(
        "whole corpus ({} documents, {invalid} invalid, one at a time): process peak {} \
         (schema and text included)",
        paths.len(),
        kib(process_peak)
    );
    Ok(())
}
