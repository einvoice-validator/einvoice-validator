//! Time to load the built-in `cen-ubl` rule set, and time per document on the whole
//! CEN EN 16931 UBL 1.3.17 corpus, single-threaded, with the share of each stage; then
//! the whole corpus again with each document given as a buffer
//! ([`Validator::validate_owned_with`], copies made before the clock starts).
//!
//! ```sh
//! cargo run --release -p einvoice-validator --example bench
//! ```

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use einvoice_validator::{ValidateOptions, Validator, file_uri};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .ok_or("no workspace root")?;

    let start = Instant::now();
    let validator = Validator::cen_ubl()?;
    let load = start.elapsed();
    let start = Instant::now();
    let schema = einvoice_xsd::Schema::ubl_2_1()?;
    let schema_load = start.elapsed();
    let xslt_path =
        root.join("artefacts/cen-en16931-ubl-1.3.17/ubl/xslt/EN16931-UBL-validation.xslt");
    let xslt = std::fs::read_to_string(&xslt_path)?;
    let start = Instant::now();
    let executable = einvoice_xslt_exec::Executable::new(&einvoice_xslt::load(&xslt)?)?;
    let xslt_load = start.elapsed();

    let cases = std::fs::read_to_string(root.join("corpus/cen-en16931-ubl-1.3.17/cases.tsv"))?;
    let mut documents = Vec::new();
    for line in cases.lines().skip(1) {
        let input = line.split('\t').nth(1).ok_or("bad cases.tsv line")?;
        let mut path = root.clone();
        path.extend(input.split('/'));
        let path = std::path::absolute(path)?;
        let uri = file_uri(&path).ok_or("no URI")?;
        documents.push((std::fs::read(&path)?, uri));
    }

    let mut total = Duration::ZERO;
    let (mut parse, mut xsd, mut transform) = (Duration::ZERO, Duration::ZERO, Duration::ZERO);
    let mut max = (Duration::ZERO, 0);
    for (bytes, uri) in &documents {
        let start = Instant::now();
        let report = validator.validate_with(bytes, &ValidateOptions::new().document_uri(uri))?;
        let elapsed = start.elapsed();
        total += elapsed;
        if elapsed > max.0 {
            max = (elapsed, bytes.len());
        }
        std::hint::black_box(report);

        // The stages on their own.
        let text = std::str::from_utf8(bytes)?;
        let start = Instant::now();
        std::hint::black_box(einvoice_xml::check(text)?);
        parse += start.elapsed();
        let start = Instant::now();
        std::hint::black_box(schema.validate(text)?);
        xsd += start.elapsed();
        let start = Instant::now();
        std::hint::black_box(executable.transform(text, Some(uri))?);
        transform += start.elapsed();
    }
    let buffers: Vec<Vec<u8>> = documents.iter().map(|(bytes, _)| bytes.clone()).collect();
    let start = Instant::now();
    for ((_, uri), bytes) in documents.iter().zip(buffers) {
        std::hint::black_box(
            validator.validate_owned_with(bytes, &ValidateOptions::new().document_uri(uri))?,
        );
    }
    let owned_total = start.elapsed();
    let n = u32::try_from(documents.len())?;
    let share = |d: Duration| 100.0 * d.as_secs_f64() / total.as_secs_f64();
    println!(
        "load: {load:.1?} (UBL 2.1 schemas alone {schema_load:.1?}, CEN stylesheet load + prepare alone {xslt_load:.1?})"
    );
    println!(
        "{n} documents: {total:.1?} in total, {:.1?} per document on average, slowest {:.1?} ({} bytes)",
        total / n,
        max.0,
        max.1
    );
    println!(
        "given as buffers: {owned_total:.1?} in total, {:.1?} per document on average",
        owned_total / n
    );
    println!(
        "stages on their own: well-formedness parse {:.1}%, schema {:.1}%, stylesheet {:.1}% of the validation time",
        share(parse),
        share(xsd),
        share(transform)
    );
    Ok(())
}
