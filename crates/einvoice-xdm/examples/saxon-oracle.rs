//! The Saxon-HE oracle of this crate: runs `tests/oracle/probe.xsl` on every case
//! of `tests/oracle/cases.tsv` and `tests/oracle/generated.tsv` and compares the
//! results with `tests/oracle/expected.tsv` (or writes them).
//!
//! ```text
//! cargo run -p einvoice-xdm --example saxon-oracle -- generate   rewrite generated.tsv (deterministic)
//! cargo run -p einvoice-xdm --example saxon-oracle               run Saxon, compare with expected.tsv
//! cargo run -p einvoice-xdm --example saxon-oracle -- --write    run Saxon, rewrite expected.tsv
//! ```
//!
//! Saxon-HE 9.9.1-8 is taken from `$SAXON_JAR`, else where `scripts/fetch-saxon.sh`
//! puts it (`target/oracle/saxon/`); its SHA-256 is checked. Java from `$JAVA`, else
//! `java`. Saxon runs with the UTC timezone, the implicit timezone the test uses.
//! Double-to-string conversion of subnormal numbers and the decimal digits accepted
//! in long integers come from the JVM: the goldens are produced with Java 21.

#[path = "../tests/oracle/format.rs"]
mod format;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use sha2::{Digest, Sha256};

const SAXON_SHA256: &str = "504ad5c71b55a3fd04c5609419b5e51d51b419f26628b4b6254c38766487abdf";
const SAXON_DEFAULT: &str = "target/oracle/saxon/Saxon-HE-9.9.1-8.jar";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workspace_root() -> PathBuf {
    crate_dir().join("../..")
}

fn oracle_dir() -> PathBuf {
    crate_dir().join("tests/oracle")
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["generate"] => {
            let text = generate();
            let count = text.lines().filter(|l| !l.starts_with('#')).count();
            write(&oracle_dir().join("generated.tsv"), &text)?;
            println!("generated.tsv written: {count} cases");
            Ok(())
        }
        [] => oracle(false),
        ["--write"] => oracle(true),
        _ => Err("usage: saxon-oracle [generate | --write]".to_owned()),
    }
}

fn oracle(write_goldens: bool) -> Result<(), String> {
    let mut cases = format::read_cases(&read(&oracle_dir().join("cases.tsv"))?)?;
    cases.extend(format::read_cases(&read(
        &oracle_dir().join("generated.tsv"),
    )?)?);
    let mut ids = std::collections::BTreeSet::new();
    for case in &cases {
        if !ids.insert(case.id.as_str()) {
            return Err(format!("duplicate case id {}", case.id));
        }
    }

    let jar = std::env::var_os("SAXON_JAR")
        .map_or_else(|| workspace_root().join(SAXON_DEFAULT), PathBuf::from);
    let bytes = std::fs::read(&jar).map_err(|e| {
        format!(
            "{}: {e} (run scripts/fetch-saxon.sh or set SAXON_JAR)",
            jar.display()
        )
    })?;
    let hash = Sha256::digest(&bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        });
    if hash != SAXON_SHA256 {
        return Err(format!(
            "{}: SHA-256 {hash}, expected {SAXON_SHA256}",
            jar.display()
        ));
    }

    let work = workspace_root().join("target/oracle/xdm");
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let input = work.join("cases.xml");
    let output = work.join("results.xml");
    write(&input, &cases_xml(&cases))?;
    let java = std::env::var_os("JAVA").unwrap_or_else(|| "java".into());
    let status = Command::new(&java)
        .env("TZ", "UTC")
        .arg("-Duser.timezone=UTC")
        .arg("-jar")
        .arg(&jar)
        .arg(format!("-xsl:{}", oracle_dir().join("probe.xsl").display()))
        .arg(format!("-s:{}", input.display()))
        .arg(format!("-o:{}", output.display()))
        .output()
        .map_err(|e| format!("cannot run {}: {e}", java.display()))?;
    if !status.status.success() || !status.stderr.is_empty() {
        return Err(format!(
            "Saxon failed ({}):\n{}",
            status.status,
            String::from_utf8_lossy(&status.stderr)
        ));
    }
    let results = parse_results(&read(&output)?)?;
    if results.len() != cases.len() {
        return Err(format!(
            "Saxon returned {} results for {} cases",
            results.len(),
            cases.len()
        ));
    }
    let mut golden = String::from(
        "# Saxon-HE 9.9.1-8 results of probe.xsl on cases.tsv and generated.tsv.\n\
         # Written by `cargo run -p einvoice-xdm --example saxon-oracle -- --write`; do not edit.\n",
    );
    for ((id, result), case) in results.iter().zip(&cases) {
        if *id != case.id {
            return Err(format!("result {id} out of order (expected {})", case.id));
        }
        let _ = writeln!(golden, "{id}\t{result}");
    }
    let path = oracle_dir().join("expected.tsv");
    if write_goldens {
        write(&path, &golden)?;
        println!("expected.tsv written: {} cases", cases.len());
        return Ok(());
    }
    let committed = format::read_expected(&read(&path)?);
    let fresh = format::read_expected(&golden);
    if committed == fresh {
        println!("expected.tsv matches Saxon: {} cases", cases.len());
        return Ok(());
    }
    let mut problems = Vec::new();
    let committed_map: std::collections::BTreeMap<_, _> = committed.into_iter().collect();
    for (id, result) in &fresh {
        match committed_map.get(id) {
            Some(old) if old == result => {}
            Some(old) => problems.push(format!("{id}: golden {old:?}, Saxon {result:?}")),
            None => problems.push(format!("{id}: missing from expected.tsv")),
        }
    }
    if problems.is_empty() {
        problems.push("expected.tsv has stale or reordered cases".to_owned());
    }
    Err(format!(
        "{} differences with Saxon (review, then rerun with --write):\n  {}",
        problems.len(),
        problems.join("\n  ")
    ))
}

/// Escapes text for XML character data, every non-ASCII or control character as a
/// character reference (so that CR and tabs survive parsing).
fn xml_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if c.is_ascii_graphic() || c == ' ' => out.push(c),
            c => {
                let _ = write!(out, "&#{};", u32::from(c));
            }
        }
    }
    out
}

fn cases_xml(cases: &[format::Case]) -> String {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<cases>\n");
    for case in cases {
        let _ = write!(
            xml,
            "<c id=\"{}\" op=\"{}\">",
            xml_text(&case.id),
            xml_text(&case.op)
        );
        for arg in &case.args {
            let _ = write!(
                xml,
                "<a t=\"{}\">{}</a>",
                xml_text(&arg.kind),
                xml_text(&arg.lexical)
            );
        }
        xml.push_str("</c>\n");
    }
    xml.push_str("</cases>\n");
    xml
}

fn parse_results(xml: &str) -> Result<Vec<(String, String)>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("Saxon output: {e}"))?;
    let mut results = Vec::new();
    for r in doc
        .root_element()
        .children()
        .filter(|n| n.has_tag_name("r"))
    {
        let id = r.attribute("id").unwrap_or_default().to_owned();
        let mut columns = Vec::new();
        for item in r.children().filter(roxmltree::Node::is_element) {
            if item.has_tag_name("e") {
                columns.push(format!(
                    "error:{}",
                    item.attribute("code").unwrap_or_default()
                ));
            } else {
                let text: String = item.children().filter_map(|n| n.text()).collect();
                columns.push(format!(
                    "{}:{}",
                    item.attribute("t").unwrap_or_default(),
                    format::escape(&text)
                ));
            }
        }
        if columns.is_empty() {
            columns.push("empty".to_owned());
        }
        results.push((id, columns.join("\t")));
    }
    Ok(results)
}

/// xorshift64*: a small deterministic generator, so that `generate` always writes
/// the same file.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn digits(&mut self, count: u64) -> String {
        (0..count)
            .map(|_| char::from(b'0' + u8::try_from(self.below(10)).unwrap_or(0)))
            .collect()
    }

    /// A decimal lexical form: sign, up to `int_max` integer digits, up to
    /// `frac_max` fractional digits (trailing zeros included at times).
    fn decimal(&mut self, int_max: u64, frac_max: u64) -> String {
        let sign = if self.below(3) == 0 { "-" } else { "" };
        let int_len = self.below(int_max + 1);
        let frac_len = self.below(frac_max + 1);
        let mut s = String::from(sign);
        if int_len == 0 {
            s.push('0');
        } else {
            let first = char::from(b'1' + u8::try_from(self.below(9)).unwrap_or(0));
            s.push(first);
            s.push_str(&self.digits(int_len - 1));
        }
        if frac_len > 0 {
            s.push('.');
            s.push_str(&self.digits(frac_len));
            if self.below(4) == 0 {
                s.push_str("00");
            }
        }
        s
    }

    fn double(&mut self) -> f64 {
        loop {
            let v = f64::from_bits(self.next());
            if v.is_finite() {
                return v;
            }
        }
    }
}

fn double_lexical(v: f64) -> String {
    format!("{v:e}")
}

/// The generated cases, one line each.
struct Lines(String);

impl Lines {
    fn line(&mut self, id: &str, op: &str, args: &[String]) {
        let _ = writeln!(self.0, "{id}\t{op}\t{}", args.join("\t"));
    }
}

fn generate() -> String {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut out = Lines(String::from(
        "# Generated by `cargo run -p einvoice-xdm --example saxon-oracle -- generate`; do not edit.\n",
    ));
    double_to_string(&mut rng, &mut out);
    double_conversions(&mut rng, &mut out);
    decimal_arithmetic(&mut rng, &mut out);
    integer_division(&mut rng, &mut out);
    rounding(&mut rng, &mut out);
    untyped_comparisons(&mut rng, &mut out);
    lexical_forms(&mut rng, &mut out);
    out.0
}

fn double_to_string(rng: &mut Rng, out: &mut Lines) {
    // Double to string: random bit patterns (normal and subnormal), random
    // magnitudes around the notation thresholds, powers of two.
    for i in 0..400 {
        let v = rng.double();
        out.line(
            &format!("gen-dstr-bits-{i:03}"),
            "string",
            &[format!("double:{}", double_lexical(v))],
        );
    }
    for i in 0..300 {
        let exponent = i32::try_from(rng.below(17)).unwrap_or(0) - 8;
        let mantissa = 1.0
            + f64::from(u32::try_from(rng.below(1 << 30)).unwrap_or(0)) / f64::from(1_u32 << 30)
                * 9.0;
        let v = mantissa * 10_f64.powi(exponent);
        out.line(
            &format!("gen-dstr-mag-{i:03}"),
            "string",
            &[format!("double:{}", double_lexical(v))],
        );
    }
    for (i, k) in (-1074_i32..=1023).enumerate() {
        let v = 2_f64.powi(k);
        out.line(
            &format!("gen-dstr-pow2-{i:04}"),
            "string",
            &[format!("double:{}", double_lexical(v))],
        );
    }
    for i in 0..60 {
        let v = f64::from_bits(rng.below(1 << 52) + 1);
        out.line(
            &format!("gen-dstr-sub-{i:03}"),
            "string",
            &[format!("double:{}", double_lexical(v))],
        );
    }
    for (i, base) in [1e6_f64, 1e-6, 0.01, 1e-3, 1e7, 9_007_199_254_740_992.0]
        .iter()
        .enumerate()
    {
        for (j, v) in [base.next_down(), *base, base.next_up()].iter().enumerate() {
            out.line(
                &format!("gen-dstr-edge-{i}{j}"),
                "string",
                &[format!("double:{}", double_lexical(*v))],
            );
        }
    }
}

fn double_conversions(rng: &mut Rng, out: &mut Lines) {
    // Double to decimal (exact) and to integer (truncation, big values).
    for i in 0..100 {
        let v = rng.double();
        out.line(
            &format!("gen-d2dec-{i:03}"),
            "cast-decimal",
            &[format!("double:{}", double_lexical(v))],
        );
    }
    for i in 0..100 {
        let v = f64::from_bits(0x43E0_0000_0000_0000 + rng.below(0x0300_0000_0000_0000));
        let v = if i % 2 == 0 { v } else { -v };
        out.line(
            &format!("gen-d2int-{i:03}"),
            "cast-integer",
            &[format!("double:{}", double_lexical(v))],
        );
    }
}

fn decimal_arithmetic(rng: &mut Rng, out: &mut Lines) {
    // Decimal arithmetic, with the operand kinds that change Saxon's scales.
    let ops = ["div", "div", "div", "mul", "add", "sub", "mod", "idiv"];
    let kinds = ["decimal", "decimal", "integer", "idecimal", "untyped"];
    for i in 0..400 {
        let op = ops[usize::try_from(rng.below(8)).unwrap_or(0)];
        let operand = |rng: &mut Rng| {
            let kind = kinds[usize::try_from(rng.below(5)).unwrap_or(0)];
            let lexical = if matches!(kind, "integer" | "idecimal") {
                let mut d = rng.decimal(12, 0);
                if rng.below(4) == 0 {
                    d.push_str("000");
                }
                d
            } else {
                rng.decimal(12, 8)
            };
            format!("{kind}:{lexical}")
        };
        let a = operand(rng);
        let b = operand(rng);
        out.line(&format!("gen-arith-{i:03}"), op, &[a, b]);
    }
}

fn integer_division(rng: &mut Rng, out: &mut Lines) {
    // Integer division: 64-bit and big values, negatives, exact and inexact.
    for i in 0..150 {
        let a = rng.decimal(if i % 3 == 0 { 20 } else { 10 }, 0);
        let b = rng.decimal(if i % 5 == 0 { 20 } else { 4 }, 0);
        let b = if b == "0" || b == "-0" {
            "7".to_owned()
        } else {
            b
        };
        out.line(
            &format!("gen-idiv-{i:03}"),
            "div",
            &[format!("integer:{a}"), format!("integer:{b}")],
        );
    }
}

fn rounding(rng: &mut Rng, out: &mut Lines) {
    // Rounding.
    for i in 0..120 {
        let mut d = rng.decimal(6, 3);
        if i % 2 == 0 {
            if !d.contains('.') {
                d.push('.');
            }
            d.push('5');
        }
        let op = ["round", "floor", "ceiling", "abs"][i % 4];
        out.line(
            &format!("gen-round-dec-{i:03}"),
            op,
            &[format!("decimal:{d}")],
        );
        out.line(
            &format!("gen-round-dbl-{i:03}"),
            op,
            &[format!("double:{d}")],
        );
    }
}

fn untyped_comparisons(rng: &mut Rng, out: &mut Lines) {
    // Untyped values compared with numbers.
    let cmps = ["g-eq", "g-ne", "g-lt", "g-le", "g-gt", "g-ge"];
    for i in 0..150 {
        let u = rng.decimal(4, 2);
        let n = rng.decimal(4, 2);
        let kind = ["integer", "decimal", "double"][i % 3];
        let n = if kind == "integer" {
            rng.decimal(4, 0)
        } else {
            n
        };
        let op = cmps[i % 6];
        out.line(
            &format!("gen-ucmp-{i:03}"),
            op,
            &[format!("untyped:{u}"), format!("{kind}:{n}")],
        );
    }
}

fn lexical_forms(rng: &mut Rng, out: &mut Lines) {
    // Lexical forms of doubles, short and long, with odd whitespace.
    let pieces = [" ", "1", "2", ".", "5", "0", "e", "-", "+", "9"];
    for i in 0..200 {
        let len = 1 + rng.below(10);
        let s: String = (0..len)
            .map(|_| pieces[usize::try_from(rng.below(10)).unwrap_or(0)])
            .collect();
        out.line(
            &format!("gen-dparse-{i:03}"),
            "number",
            &[format!("string:{}", format::escape(&s))],
        );
        out.line(
            &format!("gen-decparse-{i:03}"),
            "castable-decimal",
            &[format!("string:{}", format::escape(&s))],
        );
    }
}
