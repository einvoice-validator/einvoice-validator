//! Times the whole CEN EN 16931 UBL 1.3.17 job: the stylesheet loaded and prepared once,
//! then every corpus document (or the documents given as arguments) parsed, transformed
//! and serialized to SVRL, one after the other on one thread. Reading the files is not
//! timed. Run in release mode:
//!
//! ```text
//! cargo run --release -p einvoice-xslt-exec --example bench-cen [-- --repeat N] [FILE...]
//! ```
//!
//! `--repeat N` runs the job N times and reports the best run. Each run is made twice:
//! the texts given to the transformations as `String`s (copies made before the clock
//! starts), whose trees keep them, then borrowed, the trees copying the texts of their
//! nodes; the details are those of the first.

#[path = "../tests/common/mod.rs"]
mod common;

use std::process::ExitCode;
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let root = common::workspace_root();
    let mut repeat = 1_usize;
    let mut files = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--repeat" {
            repeat = args
                .next()
                .and_then(|n| n.parse().ok())
                .ok_or("--repeat needs a number")?;
        } else {
            let path = std::path::absolute(&arg).map_err(|e| e.to_string())?;
            files.push(path.to_str().ok_or("non-UTF-8 path")?.to_owned());
        }
    }
    if files.is_empty() {
        files = common::cases(&root)?.into_iter().map(|c| c.input).collect();
    }
    let texts = files
        .iter()
        .map(|f| common::read(std::path::Path::new(f)).map(|t| (f.clone(), t)))
        .collect::<Result<Vec<_>, _>>()?;

    let start = Instant::now();
    let stylesheet = common::read(&common::join(&root, common::XSLT))?;
    let sheet = einvoice_xslt::load(&stylesheet).map_err(|e| e.to_string())?;
    let loaded = start.elapsed();
    let start = Instant::now();
    let executable = einvoice_xslt_exec::Executable::new(&sheet).map_err(|e| e.to_string())?;
    let prepared = start.elapsed();
    println!(
        "stylesheet: load {loaded:.2?}, prepare {prepared:.2?} ({} bytes)",
        stylesheet.len()
    );

    let mut best: Option<(Duration, Vec<Duration>)> = None;
    let mut best_borrowed: Option<Duration> = None;
    for _ in 0..repeat {
        let given: Vec<String> = texts.iter().map(|(_, t)| t.clone()).collect();
        let mut times = Vec::with_capacity(texts.len());
        let job = Instant::now();
        for ((path, _), text) in texts.iter().zip(given) {
            let t = Instant::now();
            let svrl = common::svrl(&executable, text, path)?;
            times.push(t.elapsed());
            std::hint::black_box(svrl);
        }
        let total = job.elapsed();
        if best.as_ref().is_none_or(|(b, _)| total < *b) {
            best = Some((total, times));
        }
        let job = Instant::now();
        for (path, text) in &texts {
            std::hint::black_box(common::svrl(&executable, text, path)?);
        }
        let total = job.elapsed();
        if best_borrowed.is_none_or(|b| total < b) {
            best_borrowed = Some(total);
        }
    }
    let (Some((total, times)), Some(borrowed)) = (best, best_borrowed) else {
        return Err("nothing run".to_owned());
    };
    let bytes: usize = texts.iter().map(|(_, t)| t.len()).sum();
    let count = u32::try_from(times.len()).map_err(|e| e.to_string())?;
    println!(
        "{} documents ({:.1} MB): total {total:.2?}, mean {:.2?} per document (texts given); total {borrowed:.2?}, mean {:.2?} (texts borrowed)",
        times.len(),
        f64::from(u32::try_from(bytes).unwrap_or(u32::MAX)) / 1e6,
        total / count.max(1),
        borrowed / count.max(1)
    );
    let mut order: Vec<usize> = (0..times.len()).collect();
    order.sort_by(|&a, &b| times[b].cmp(&times[a]));
    let mut sorted = times.clone();
    sorted.sort();
    println!(
        "median {:.2?}, slowest:",
        sorted.get(sorted.len() / 2).copied().unwrap_or_default()
    );
    for &i in order.iter().take(5) {
        let (path, text) = &texts[i];
        let name = path.rsplit('/').next().unwrap_or(path);
        println!("  {:>10.2?}  {:>8} bytes  {name}", times[i], text.len());
    }
    let largest = (0..texts.len()).max_by_key(|&i| texts[i].1.len());
    if let Some(i) = largest {
        let (path, text) = &texts[i];
        let name = path.rsplit('/').next().unwrap_or(path);
        println!("largest: {name} ({} bytes): {:.2?}", text.len(), times[i]);
    }
    Ok(())
}
