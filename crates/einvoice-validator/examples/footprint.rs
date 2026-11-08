//! Peak heap and time of a whole validation with the built-in `cen-ubl` rule set, on
//! the largest document of the CEN EN 16931 UBL 1.3.17 corpus and on a synthetic
//! invoice of 5000 lines (the lines of `ubl-tc434-example1.xml` repeated), measured
//! by a counting global allocator: the peak above what was in use before the
//! validation (the document's bytes, already in memory, excluded), with the document
//! given as a buffer ([`Validator::validate_owned_with`]) and borrowed
//! ([`Validator::validate_with`]), and the shortest time of a few runs. Then the
//! same for the stages on their own: the XML Schema validation
//! ([`einvoice_xsd::Schema::validate`]) and the tree the stylesheets run on
//! ([`einvoice_xpath_eval::Document::parse`], the text given).
//!
//! ```sh
//! cargo run --release -p einvoice-validator --example footprint
//! cargo run --release -p einvoice-validator --example footprint -- --write target/synthetic.xml
//! ```
//!
//! With `--write PATH`, the synthetic invoice is also written to `PATH`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use einvoice_validator::{ValidateOptions, Validator};
use peak_alloc::PeakAlloc;

#[global_allocator]
static ALLOCATOR: PeakAlloc = PeakAlloc;

/// Runs timed per document and measurement; the shortest is kept.
const RUNS: usize = 3;

/// The number of invoice lines of the synthetic invoice.
const LINES: usize = 5000;

#[expect(clippy::cast_precision_loss, reason = "a size shown in KiB")]
fn kib(bytes: usize) -> String {
    format!("{:.0} KiB", bytes as f64 / 1024.0)
}

#[expect(clippy::cast_precision_loss, reason = "a ratio shown rounded")]
fn ratio(peak: usize, size: usize) -> f64 {
    peak as f64 / size as f64
}

/// The peak heap during `f` above what was in use before, and its shortest time
/// over [`RUNS`] runs (the peak is that of the first).
fn measure<T>(
    mut f: impl FnMut() -> Result<T, Box<dyn std::error::Error>>,
) -> Result<(usize, Duration), Box<dyn std::error::Error>> {
    let mut peak = 0;
    let mut best = Duration::MAX;
    for run in 0..RUNS {
        let before = ALLOCATOR.current_usage();
        ALLOCATOR.reset_peak_usage();
        let start = Instant::now();
        let result = f()?;
        let elapsed = start.elapsed();
        drop(result);
        if run == 0 {
            peak = ALLOCATOR.peak_usage().saturating_sub(before);
        }
        best = best.min(elapsed);
    }
    Ok((peak, best))
}

fn report(
    validator: &Validator,
    schema: &einvoice_xsd::Schema,
    name: &str,
    bytes: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let size = bytes.len();
    let text = std::str::from_utf8(bytes)?;
    println!("\n{name}: {size} bytes");
    let options = ValidateOptions::new();
    let summary = validator.validate_with(bytes, &options)?;
    println!(
        "  verdict {:?}, {} schema errors, {} findings",
        summary.verdict(),
        summary.schema_errors.len(),
        summary.findings().count()
    );
    let line = |label: &str, (peak, time): (usize, Duration)| {
        println!(
            "  {label:<44} peak {:>10} ({:>5.2}x the input)  {:>9.1} ms",
            kib(peak),
            ratio(peak, size),
            time.as_secs_f64() * 1e3
        );
    };
    // The copy is made before the measure starts: the buffer is then moved into
    // the validation.
    let mut copies: Vec<Vec<u8>> = (0..RUNS).map(|_| bytes.to_vec()).collect();
    line(
        "whole validation, buffer given",
        measure(|| {
            let bytes = copies.pop().ok_or("no copy left")?;
            Ok(validator.validate_owned_with(bytes, &options)?)
        })?,
    );
    line(
        "whole validation, borrowed",
        measure(|| Ok(validator.validate_with(bytes, &options)?))?,
    );
    line(
        "stage: XML Schema validation",
        measure(|| Ok(schema.validate(text)?))?,
    );
    let mut texts: Vec<String> = (0..RUNS).map(|_| text.to_owned()).collect();
    line(
        "stage: stylesheet input tree, text given",
        measure(|| {
            let text = texts.pop().ok_or("no copy left")?;
            Ok(einvoice_xpath_eval::Document::parse(text, None)?)
        })?,
    );
    line(
        "stage: stylesheet input tree, borrowed",
        measure(|| Ok(einvoice_xpath_eval::Document::parse(text, None)?))?,
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .ok_or("no workspace root")?;
    let mut args = std::env::args().skip(1);
    let write = match (args.next().as_deref(), args.next()) {
        (None, _) => None,
        (Some("--write"), Some(path)) => Some(PathBuf::from(path)),
        _ => return Err("usage: footprint [--write PATH]".into()),
    };

    let validator = Validator::cen_ubl()?;
    let schema = einvoice_xsd::Schema::ubl_2_1()?;

    let cases = std::fs::read_to_string(root.join("corpus/cen-en16931-ubl-1.3.17/cases.tsv"))?;
    let mut largest: Option<(u64, String)> = None;
    for line in cases.lines().skip(1) {
        let input = line.split('\t').nth(1).ok_or("bad cases.tsv line")?;
        let size = std::fs::metadata(root.join(input))?.len();
        if largest.as_ref().is_none_or(|(s, _)| size > *s) {
            largest = Some((size, input.to_owned()));
        }
    }
    let (_, input) = largest.ok_or("empty corpus")?;
    let bytes = std::fs::read(root.join(&input))?;
    report(
        &validator,
        &schema,
        &format!("largest corpus document ({input})"),
        &bytes,
    )?;
    drop(bytes);

    let base = std::fs::read_to_string(
        root.join("artefacts/cen-en16931-ubl-1.3.17/ubl/examples/ubl-tc434-example1.xml"),
    )?;
    let synthetic = einvoice_testkit::invoice::with_lines(&base, LINES).ok_or("no invoice line")?;
    if let Some(path) = write {
        std::fs::write(&path, &synthetic)?;
        println!("synthetic invoice written to {}", path.display());
    }
    report(
        &validator,
        &schema,
        &format!("synthetic invoice ({LINES} lines of ubl-tc434-example1.xml)"),
        synthetic.as_bytes(),
    )?;
    Ok(())
}
