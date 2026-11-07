//! Heap used by validations with the built-in `cen-ubl` rule set on the whole CEN
//! EN 16931 UBL 1.3.17 corpus, measured by a counting global allocator (every byte the
//! program allocates, the document read from its file included): the peak while one
//! document is read and validated, above what was in use before, for the largest
//! document and the worst one, and the peak of the process over the corpus, documents
//! validated one after the other. Measured with each document given as a buffer
//! ([`Validator::validate_owned_with`]), then borrowed ([`Validator::validate_with`]).
//!
//! ```sh
//! cargo run --release -p einvoice-validator --example heap
//! ```

use std::path::{Path, PathBuf};

use einvoice_validator::{ValidateOptions, Validator, file_uri};
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
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .ok_or("no workspace root")?;
    let cases = std::fs::read_to_string(root.join("corpus/cen-en16931-ubl-1.3.17/cases.tsv"))?;
    let mut documents = Vec::new();
    for line in cases.lines().skip(1) {
        let input = line.split('\t').nth(1).ok_or("bad cases.tsv line")?;
        let mut path = root.clone();
        path.extend(input.split('/'));
        let path = std::path::absolute(path)?;
        let uri = file_uri(&path).ok_or("no URI")?;
        documents.push((input.to_owned(), path, uri));
    }

    let before = ALLOCATOR.current_usage();
    let validator = Validator::cen_ubl()?;
    println!(
        "cen-ubl loaded: {} retained (loading peaked at {})",
        kib(ALLOCATOR.current_usage() - before),
        kib(ALLOCATOR.peak_usage() - before)
    );

    for owned in [true, false] {
        println!(
            "{}:",
            if owned {
                "documents given as buffers (validate_owned_with)"
            } else {
                "documents borrowed (validate_with)"
            }
        );
        let mut worst = (0, String::new());
        let mut largest = (0, 0, String::new());
        let mut process_peak = 0;
        for (name, path, uri) in &documents {
            let base = ALLOCATOR.current_usage();
            ALLOCATOR.reset_peak_usage();
            let bytes = std::fs::read(path)?;
            let size = bytes.len();
            let options = ValidateOptions::new().document_uri(uri);
            let report = if owned {
                validator.validate_owned_with(bytes, &options)?
            } else {
                let report = validator.validate_with(&bytes, &options)?;
                drop(bytes);
                report
            };
            drop(report);
            let peak = ALLOCATOR.peak_usage();
            process_peak = process_peak.max(peak);
            let peak = peak - base;
            if peak > worst.0 {
                worst = (peak, name.clone());
            }
            if size > largest.1 {
                largest = (peak, size, name.clone());
            }
        }
        println!(
            "  largest document: {} ({} bytes): peak {}",
            largest.2,
            largest.1,
            kib(largest.0)
        );
        println!("  worst document: {}: peak {}", worst.1, kib(worst.0));
        println!(
            "  whole corpus ({} documents, one at a time): process peak {} (rule set included)",
            documents.len(),
            kib(process_peak)
        );
    }
    Ok(())
}
