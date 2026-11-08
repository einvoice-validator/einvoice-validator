//! Extracts the embedded invoice of each PDF given on the command line and reports what
//! was found (or the error), the time per extraction (median of repeated runs, the file
//! already in memory) and the heap used by one extraction, measured by a counting global
//! allocator (peak above what was in use before: the file's own bytes are not counted).
//!
//! ```sh
//! cargo run --release -p einvoice-pdf --example measure -- invoice.pdf [more.pdf ...]
//! ```

use std::time::{Duration, Instant};

use peak_alloc::PeakAlloc;

#[global_allocator]
static ALLOCATOR: PeakAlloc = PeakAlloc;

/// Runs per file for the timing.
const RUNS: usize = 200;

fn describe(result: &Result<einvoice_pdf::Embedded, einvoice_pdf::Error>) -> String {
    match result {
        Err(error) => format!("error: {error}"),
        Ok(embedded) => {
            let invoice = embedded.metadata.as_ref().and_then(|m| m.invoice.as_ref());
            format!(
                "version {}: {} ({} bytes, {}, {}), XMP {}",
                embedded.pdf_version,
                embedded.xml.file_name,
                embedded.xml.data.len(),
                embedded
                    .xml
                    .relationship
                    .as_ref()
                    .map_or("no /AFRelationship", |r| r.as_str()),
                embedded.xml.mime_type.as_deref().unwrap_or("no /Subtype"),
                invoice.map_or_else(
                    || "without invoice properties".to_owned(),
                    |i| format!(
                        "{:?} {:?} {:?} {:?}",
                        i.document_type, i.document_file_name, i.version, i.conformance_level
                    )
                ),
            )
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        return Err("usage: measure FILE.pdf...".into());
    }
    for path in paths {
        let pdf = std::fs::read(&path)?;
        ALLOCATOR.reset_peak_usage();
        let before = ALLOCATOR.current_usage();
        let result = einvoice_pdf::extract(&pdf);
        let peak = ALLOCATOR.peak_usage().saturating_sub(before);
        let mut times: Vec<Duration> = (0..RUNS)
            .map(|_| {
                let start = Instant::now();
                let again = einvoice_pdf::extract(&pdf);
                let elapsed = start.elapsed();
                assert_eq!(again.is_ok(), result.is_ok());
                elapsed
            })
            .collect();
        times.sort();
        println!(
            "{path}: PDF {} bytes, {}; median {:?} over {RUNS} runs (min {:?}, max {:?}); \
             heap peak {peak} bytes",
            pdf.len(),
            describe(&result),
            times[RUNS / 2],
            times[0],
            times[RUNS - 1],
        );
    }
    Ok(())
}
