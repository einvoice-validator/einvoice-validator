//! Timings, to run in release mode:
//!
//! ```text
//! cargo run --release -p einvoice-xsd --example timing
//! ```
//!
//! For each embedded schema set, prints the time to load it (the UBL 2.1 schemas with
//! both entry points, the CII D16B schemas), to validate the largest document of its
//! CEN corpus, and to validate every document of that corpus once (the schemas loaded
//! once): the CEN UBL corpus, and the CEN CII corpus with its mutation cases. Each
//! figure is the median and the minimum of several runs.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

const RUNS: usize = 21;

/// An embedded schema set and the corpus validated against it.
struct Suite {
    name: &'static str,
    files: usize,
    load: fn() -> Result<einvoice_xsd::Schema, einvoice_xsd::LoadError>,
    /// The index files of the corpus, relative to the workspace root: each line after
    /// the header names a document (second column, relative to the workspace root).
    indexes: &'static [&'static str],
}

const SUITES: [Suite; 2] = [
    Suite {
        name: "UBL 2.1 schemas (both entry points)",
        files: einvoice_xsd::UBL_2_1_FILES.len(),
        load: einvoice_xsd::Schema::ubl_2_1,
        indexes: &["corpus/cen-en16931-ubl-1.3.17/cases.tsv"],
    },
    Suite {
        name: "CII D16B schemas",
        files: einvoice_xsd::CII_D16B_FILES.len(),
        load: einvoice_xsd::Schema::cii_d16b,
        indexes: &[
            "corpus/cen-en16931-cii-1.3.17/cases.tsv",
            "corpus/cen-en16931-cii-1.3.17/mutations.tsv",
        ],
    },
];

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

/// The documents of the index files: (path relative to the root, text).
fn documents(root: &Path, indexes: &[&str]) -> Result<Vec<(String, String)>, String> {
    let mut documents = Vec::new();
    for index in indexes {
        let index_path = root.join(index);
        let lines = std::fs::read_to_string(&index_path)
            .map_err(|e| format!("{}: {e}", index_path.display()))?;
        for line in lines.lines().skip(1) {
            let input = line
                .split('\t')
                .nth(1)
                .ok_or_else(|| format!("bad line {line:?}"))?;
            let path = root.join(input);
            let text =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            documents.push((input.to_owned(), text));
        }
    }
    Ok(documents)
}

fn run() -> Result<(), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for suite in &SUITES {
        let documents = documents(&root, suite.indexes)?;
        let (median, min) = measure(|| (suite.load)().map_err(|e| e.to_string()))?;
        println!(
            "load {} ({} documents): median {median:.2?}, min {min:.2?}",
            suite.name, suite.files
        );

        let schema = (suite.load)().map_err(|e| e.to_string())?;
        let (name, largest) = documents
            .iter()
            .max_by_key(|(_, text)| text.len())
            .ok_or("empty corpus")?;
        let (median, min) = measure(|| schema.validate(largest).map_err(|e| e.to_string()))?;
        let megabytes = f64::from(u32::try_from(largest.len()).map_err(|e| e.to_string())?) / 1e6;
        println!(
            "  validate the largest corpus document ({name}, {megabytes:.2} MB): median {median:.2?}, \
             min {min:.2?} ({:.0} MB/s at the median)",
            megabytes / median.as_secs_f64()
        );

        let (median, min) = measure(|| {
            let mut invalid = 0_usize;
            for (_, text) in &documents {
                // A document the reader refuses (not well-formed) counts as invalid.
                if !schema.validate(text).is_ok_and(|report| report.is_valid()) {
                    invalid += 1;
                }
            }
            Ok(invalid)
        })?;
        println!(
            "  validate the whole corpus ({} documents, schemas loaded once): median {median:.2?}, \
             min {min:.2?}",
            documents.len()
        );
    }
    Ok(())
}
