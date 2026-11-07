//! Times the whole Peppol BIS Billing 3.0.22 job: both stylesheets (the Peppol rules and
//! the CEN EN 16931 rules it embeds) loaded and prepared once, then every corpus document
//! parsed, transformed and serialized to SVRL by each, one after the other on one
//! thread. Reading the files is not timed. Needs the cache built by
//! `scripts/check.sh peppol`. Run in release mode:
//!
//! ```text
//! cargo run --release -p einvoice-xslt-exec --example bench-peppol [-- --repeat N]
//! ```
//!
//! `--repeat N` runs the job N times and reports the best run.

#[path = "../tests/common/mod.rs"]
mod common;

use std::process::ExitCode;
use std::time::{Duration, Instant};

use common::peppol;

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
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--repeat" {
            repeat = args
                .next()
                .and_then(|n| n.parse().ok())
                .ok_or("--repeat needs a number")?;
        } else {
            return Err(format!("unexpected argument {arg}"));
        }
    }
    let corpus = peppol::corpus(&root)?.ok_or_else(|| {
        format!(
            "no Peppol corpus in {}: run scripts/check.sh peppol",
            peppol::BASE
        )
    })?;
    let texts = corpus
        .cases
        .iter()
        .map(|c| common::read(std::path::Path::new(&c.input)).map(|t| (c.input.clone(), t)))
        .collect::<Result<Vec<_>, _>>()?;

    let start = Instant::now();
    let mut executables = Vec::new();
    for stylesheet in &corpus.stylesheets {
        let t = Instant::now();
        executables.push(common::prepare(&stylesheet.xslt)?);
        println!(
            "{}: loaded and prepared in {:.2?}",
            stylesheet.name,
            t.elapsed()
        );
    }
    let prepared = start.elapsed();

    let mut best: Option<(Duration, Vec<Duration>)> = None;
    for _ in 0..repeat {
        let mut times = Vec::with_capacity(texts.len());
        let job = Instant::now();
        for (path, text) in &texts {
            let t = Instant::now();
            for executable in &executables {
                std::hint::black_box(common::svrl(executable, text, path)?);
            }
            times.push(t.elapsed());
        }
        let total = job.elapsed();
        if best.as_ref().is_none_or(|(b, _)| total < *b) {
            best = Some((total, times));
        }
    }
    let Some((total, times)) = best else {
        return Err("nothing run".to_owned());
    };
    let bytes: usize = texts.iter().map(|(_, t)| t.len()).sum();
    let count = u32::try_from(times.len()).map_err(|e| e.to_string())?;
    println!(
        "{} documents ({:.1} MB), both stylesheets each: total {total:.2?}, mean {:.2?} per document; with preparation {:.2?}",
        times.len(),
        f64::from(u32::try_from(bytes).unwrap_or(u32::MAX)) / 1e6,
        total / count.max(1),
        total + prepared
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
    Ok(())
}
