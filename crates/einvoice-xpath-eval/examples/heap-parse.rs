//! Heap and time of parsing the documents of the CEN EN 16931 UBL 1.3.17 corpus into
//! trees, measured by a counting global allocator: every byte allocated above the
//! document text itself, which is read first and is not counted.
//!
//! For each document: the peak while [`Document::parse`] runs and what the tree retains
//! once parsed, from the borrowed text (`&str`, the text is copied into the tree) and from
//! the owned text (`String`, moved into the tree, which then refers to it); and the time
//! to parse, the fastest of several runs. Reported for the largest corpus document and
//! for the whole corpus, documents parsed one after the other.
//!
//! ```text
//! cargo run --release -p einvoice-xpath-eval --example heap-parse
//! cargo run --release -p einvoice-xpath-eval --example heap-parse -- FILE...   the given documents
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use einvoice_xpath_eval::Document;
use peak_alloc::PeakAlloc;

#[global_allocator]
static ALLOCATOR: PeakAlloc = PeakAlloc;

const CASES: &str = "corpus/cen-en16931-ubl-1.3.17/cases.tsv";
/// Runs per document; the fastest is kept.
const RUNS: usize = 5;

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

/// What parsing one document costs.
#[derive(Debug, Clone, Copy, Default)]
struct Cost {
    /// Peak above the text while parsing.
    peak: usize,
    /// Retained by the tree, above the text.
    retained: usize,
    /// Fastest parse.
    time: Duration,
}

fn measure(text: &str, owned: bool) -> Result<(Cost, usize), String> {
    let mut cost = Cost {
        time: Duration::MAX,
        ..Cost::default()
    };
    let mut nodes = 0;
    for _ in 0..RUNS {
        // The copy given to the owned parse is made before the measurement starts: it
        // stands for the text the caller read.
        let input = owned.then(|| text.to_owned());
        let base = ALLOCATOR.current_usage();
        ALLOCATOR.reset_peak_usage();
        let started = Instant::now();
        let doc = match input {
            Some(input) => Document::parse(input, None),
            None => Document::parse(text, None),
        }
        .map_err(|e| e.to_string())?;
        let time = started.elapsed();
        let peak = ALLOCATOR.peak_usage() - base;
        // The owned text now belongs to the tree: what it holds beyond the text.
        let retained = ALLOCATOR.current_usage() - base;
        nodes = doc.node_count();
        drop(doc);
        cost.peak = cost.peak.max(peak);
        cost.retained = cost.retained.max(retained);
        cost.time = cost.time.min(time);
    }
    Ok((cost, nodes))
}

fn run() -> Result<(), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let files: Vec<PathBuf> = if args.is_empty() {
        let cases = std::fs::read_to_string(root.join(CASES)).map_err(|e| e.to_string())?;
        let mut files: Vec<PathBuf> = cases
            .lines()
            .skip(1)
            .filter_map(|l| l.split('\t').nth(1))
            .map(|p| root.join(p))
            .collect();
        files.sort();
        files.dedup();
        files
    } else {
        args.iter().map(PathBuf::from).collect()
    };
    let mut largest: Option<(usize, String, usize, Cost, Cost)> = None;
    let mut totals = [Cost::default(); 2];
    let mut bytes = 0;
    for file in &files {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let (borrowed, nodes) =
            measure(&text, false).map_err(|e| format!("{}: {e}", file.display()))?;
        let (owned, _) = measure(&text, true).map_err(|e| format!("{}: {e}", file.display()))?;
        for (total, cost) in totals.iter_mut().zip([borrowed, owned]) {
            total.peak = total.peak.max(cost.peak);
            total.retained += cost.retained;
            total.time += cost.time;
        }
        bytes += text.len();
        if largest.as_ref().is_none_or(|l| text.len() > l.0) {
            largest = Some((text.len(), name(file), nodes, borrowed, owned));
        }
    }
    if let Some((size, file, nodes, borrowed, owned)) = largest {
        println!("largest document: {file} ({size} bytes, {nodes} nodes)");
        report("  from &str  ", &borrowed);
        report("  from String", &owned);
    }
    println!(
        "whole corpus: {} documents, {bytes} bytes (peak: the highest document; retained and time: the sum)",
        files.len()
    );
    report("  from &str  ", &totals[0]);
    report("  from String", &totals[1]);
    Ok(())
}

fn name(file: &Path) -> String {
    file.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

fn report(label: &str, cost: &Cost) {
    println!(
        "{label}: peak {}, retained {}, parse {:.2?}",
        kib(cost.peak),
        kib(cost.retained),
        cost.time
    );
}
