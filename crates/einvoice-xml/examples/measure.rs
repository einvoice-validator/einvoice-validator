//! Throughput and peak heap of a well-formedness pass, the reader against
//! `roxmltree::Document::parse` (what the workspace calls today), on the largest
//! document of the corpus, on a synthetic invoice of more than 4 MB (the lines of
//! `ubl-tc434-example1.xml` repeated), and on a hostile document (one element with
//! many attributes). The synthetic invoice is also written to
//! `target/einvoice-xml/synthetic-invoice.xml`, to measure a whole validation on it.
//!
//! ```sh
//! cargo run --release -p einvoice-xml --example measure
//! ```

use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use einvoice_xml::{Event, Reader};
use peak_alloc::PeakAlloc;

#[global_allocator]
static ALLOCATOR: PeakAlloc = PeakAlloc;

const RUNS: usize = 10;

/// The shortest time of [`RUNS`] runs and the peak heap above what was allocated
/// before (the text) during one run, in bytes.
fn measure(f: &dyn Fn() -> bool) -> (Duration, usize, bool) {
    let before = ALLOCATOR.current_usage();
    ALLOCATOR.reset_peak_usage();
    let ok = f();
    let peak = ALLOCATOR.peak_usage().saturating_sub(before);
    let best = (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            std::hint::black_box(f());
            start.elapsed()
        })
        .min()
        .unwrap_or_default();
    (best, peak, ok)
}

fn roxmltree_parse(text: &str) -> bool {
    let options = roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: u32::MAX,
        ..roxmltree::ParsingOptions::default()
    };
    std::hint::black_box(roxmltree::Document::parse_with_options(text, options)).is_ok()
}

fn reader_check(text: &str) -> bool {
    einvoice_xml::check(text).is_ok()
}

/// Every event read and every name and value looked at, as a validator would.
fn reader_consume(text: &str) -> bool {
    let mut reader = Reader::new(text);
    let mut sum = 0usize;
    loop {
        match reader.next_event() {
            Ok(Some(Event::Start(start))) => {
                let name = start.name();
                sum += name.local.len() + name.namespace.map_or(0, str::len);
                for attribute in start.attributes() {
                    sum += attribute.name.local.len() + attribute.value.len();
                }
            }
            Ok(Some(Event::Text(text))) => sum += text.value.len(),
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => return false,
        }
    }
    std::hint::black_box(sum) > 0
}

#[expect(clippy::cast_precision_loss, reason = "a size shown in KiB")]
fn kib(bytes: usize) -> String {
    format!("{:.0} KiB", bytes as f64 / 1024.0)
}

#[expect(clippy::cast_precision_loss, reason = "sizes and rates shown rounded")]
fn report(name: &str, text: &str) {
    let mib = text.len() as f64 / (1024.0 * 1024.0);
    println!("\n{name}: {} bytes ({mib:.2} MiB)", text.len());
    let mut times = Vec::new();
    for (label, f) in [
        (
            "roxmltree::Document::parse",
            &(|| roxmltree_parse(text)) as &dyn Fn() -> bool,
        ),
        ("einvoice_xml::check", &|| reader_check(text)),
        ("einvoice_xml::Reader, every value read", &|| {
            reader_consume(text)
        }),
    ] {
        let (time, peak, ok) = measure(f);
        times.push(time);
        println!(
            "  {label:<40} {:>9.3} ms  {:>7.0} MiB/s  peak {:>9} above the text ({:.2}x the text){}",
            time.as_secs_f64() * 1e3,
            mib / time.as_secs_f64(),
            kib(peak),
            peak as f64 / text.len() as f64,
            if ok { "" } else { "  (refused)" }
        );
    }
    println!(
        "  speed-up of the check over roxmltree: {:.2}x",
        times[0].as_secs_f64() / times[1].as_secs_f64()
    );
}

/// The invoice of `base` with its lines repeated until the text has at least
/// `size` bytes.
fn synthetic_invoice(base: &str, size: usize) -> String {
    let start = base.find("<cac:InvoiceLine>").unwrap_or(0);
    let end = base
        .rfind("</cac:InvoiceLine>")
        .map_or(base.len(), |i| i + "</cac:InvoiceLine>".len());
    let (head, lines, tail) = (&base[..start], &base[start..end], &base[end..]);
    let mut out = String::with_capacity(size + lines.len() + base.len());
    out.push_str(head);
    while out.len() + tail.len() < size {
        out.push_str(lines);
    }
    out.push_str(tail);
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let largest = root.join("artefacts/cen-en16931-ubl-1.3.17/ubl/examples/FT G2G_TD01 con Allegato, Bonifico e Split Payment.xml");
    let text = std::fs::read_to_string(&largest)?;
    report(
        "largest corpus document (FT G2G_TD01 con Allegato, Bonifico e Split Payment.xml)",
        &text,
    );
    drop(text);

    let base = std::fs::read_to_string(
        root.join("artefacts/cen-en16931-ubl-1.3.17/ubl/examples/ubl-tc434-example1.xml"),
    )?;
    let synthetic = synthetic_invoice(&base, 4_100_000);
    let out = root.join("target/einvoice-xml");
    std::fs::create_dir_all(&out)?;
    std::fs::write(out.join("synthetic-invoice.xml"), &synthetic)?;
    let lines = synthetic.matches("<cac:InvoiceLine>").count();
    report(&format!("synthetic invoice ({lines} lines)"), &synthetic);
    drop(synthetic);

    let attributes = (0..20_000).fold(String::new(), |mut s, i| {
        let _ = write!(s, " a{i}='{i}'");
        s
    });
    report(
        "one element with 20000 attributes",
        &format!("<e{attributes}/>"),
    );
    Ok(())
}
