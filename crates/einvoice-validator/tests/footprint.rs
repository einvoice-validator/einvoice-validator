//! Memory budget of a validation (`cen-ubl`): the peak heap of one whole validation,
//! above what was in use before it (the document's bytes excluded), measured by a
//! counting global allocator, must stay within the budgets recorded below, on the
//! largest document of the CEN EN 16931 UBL 1.3.17 corpus and on a synthetic invoice
//! of 1000 lines. A change that makes a validation use more memory fails here; if the
//! increase is intended, measure (`examples/footprint.rs`) and raise the budget in the
//! same change.
//!
//! The allocator counts every allocation of the process: the test is the only one of
//! its binary, so that no other test allocates while it measures.
#![cfg(test)]
#![cfg(feature = "cen-ubl")]

use std::path::{Path, PathBuf};

use einvoice_validator::{ValidateOptions, Validator};
use peak_alloc::PeakAlloc;

#[global_allocator]
static ALLOCATOR: PeakAlloc = PeakAlloc;

/// The peak above the document given as a buffer, for the largest corpus document
/// (3291892 bytes, mostly a base64 attachment): measured 51 KiB.
const LARGEST_BUDGET: usize = 64 * 1024;

/// The peak above the document given as a buffer, for the invoice of 1000 lines
/// (806 KiB): measured 1657 KiB, mostly the stylesheet's input tree; the SVRL report
/// (1.4 MB of text, 8051 fired rules) is kept compactly and read as it is written.
/// Before: 5020 KiB, when the report was written to a string and parsed again into a
/// tree; 7628 KiB in release 0.2.0, which also built a tree of the document to check it
/// and another to validate it against the schema.
const SYNTHETIC_BUDGET: usize = 1850 * 1024;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The peak heap of a validation of `bytes` given as a buffer, above what was in use
/// before it, the buffer included.
fn peak(validator: &Validator, bytes: Vec<u8>) -> usize {
    let before = ALLOCATOR.current_usage();
    ALLOCATOR.reset_peak_usage();
    let report = validator
        .validate_owned_with(bytes, &ValidateOptions::new())
        .unwrap();
    drop(report);
    ALLOCATOR.peak_usage().saturating_sub(before)
}

#[test]
fn the_peak_heap_of_a_validation_stays_within_its_budget() {
    let validator = Validator::cen_ubl().unwrap();
    let examples = root().join("artefacts/cen-en16931-ubl-1.3.17/ubl/examples");
    let largest =
        std::fs::read(examples.join("FT G2G_TD01 con Allegato, Bonifico e Split Payment.xml"))
            .unwrap();
    assert_eq!(largest.len(), 3_291_892);
    let base = std::fs::read_to_string(examples.join("ubl-tc434-example1.xml")).unwrap();
    let synthetic = einvoice_testkit::invoice::with_lines(&base, 1000)
        .unwrap()
        .into_bytes();

    let largest_peak = peak(&validator, largest);
    let synthetic_size = synthetic.len();
    let synthetic_peak = peak(&validator, synthetic);
    println!(
        "largest corpus document: peak {} KiB (budget {} KiB); \
         synthetic invoice, 1000 lines ({} KiB): peak {} KiB (budget {} KiB)",
        largest_peak / 1024,
        LARGEST_BUDGET / 1024,
        synthetic_size / 1024,
        synthetic_peak / 1024,
        SYNTHETIC_BUDGET / 1024
    );
    assert!(
        largest_peak <= LARGEST_BUDGET,
        "largest corpus document: peak {largest_peak} bytes, budget {LARGEST_BUDGET}"
    );
    assert!(
        synthetic_peak <= SYNTHETIC_BUDGET,
        "synthetic invoice: peak {synthetic_peak} bytes, budget {SYNTHETIC_BUDGET}"
    );
}
