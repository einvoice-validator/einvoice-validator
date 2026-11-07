//! Timings, to run in release mode:
//!
//! ```text
//! cargo run --release -p einvoice-xsd --example timing
//! ```
//!
//! Prints the time to load the embedded UBL 2.1 schemas (both entry points), to
//! validate the largest document of the CEN UBL corpus, and to validate every
//! document of the corpus once (the schemas loaded once). Each figure is the median
//! and the minimum of several runs.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

const RUNS: usize = 21;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn measure<T>(mut f: impl FnMut() -> Result<T, String>) -> Result<(Duration, Duration), String> {
    let mut times = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let start = Instant::now();
        std::hint::black_box(f()?);
        times.push(start.elapsed());
    }
    times.sort_unstable();
    Ok((times[RUNS / 2], times[0]))
}

fn run() -> Result<(), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cases_path = root.join("corpus/cen-en16931-ubl-1.3.17/cases.tsv");
    let cases = std::fs::read_to_string(&cases_path)
        .map_err(|e| format!("{}: {e}", cases_path.display()))?;
    let mut documents = Vec::new();
    for line in cases.lines().skip(1) {
        let input = line
            .split('\t')
            .nth(1)
            .ok_or_else(|| format!("bad line {line:?}"))?;
        let path = root.join(input);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        documents.push((input.to_owned(), text));
    }

    let (median, min) = measure(|| einvoice_xsd::Schema::ubl_2_1().map_err(|e| e.to_string()))?;
    println!(
        "load UBL 2.1 schemas (15 documents, both entry points): median {median:.2?}, min {min:.2?}"
    );

    let schema = einvoice_xsd::Schema::ubl_2_1().map_err(|e| e.to_string())?;
    let (name, largest) = documents
        .iter()
        .max_by_key(|(_, text)| text.len())
        .ok_or("empty corpus")?;
    let (median, min) = measure(|| schema.validate(largest).map_err(|e| e.to_string()))?;
    let megabytes = f64::from(u32::try_from(largest.len()).map_err(|e| e.to_string())?) / 1e6;
    println!(
        "validate the largest corpus document ({name}, {megabytes:.2} MB): median {median:.2?}, min {min:.2?} \
         ({:.0} MB/s at the median)",
        megabytes / median.as_secs_f64()
    );

    let (median, min) = measure(|| {
        let mut invalid = 0_usize;
        for (_, text) in &documents {
            if !schema.validate(text).map_err(|e| e.to_string())?.is_valid() {
                invalid += 1;
            }
        }
        Ok(invalid)
    })?;
    println!(
        "validate the whole corpus ({} documents, schemas loaded once): median {median:.2?}, min {min:.2?}",
        documents.len()
    );
    Ok(())
}
