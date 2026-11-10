//! The reference engines on documents in the encodings other than UTF-8 that the
//! validator reads: every document of the CEN EN 16931 UBL and CII 1.3.17 corpora (cases
//! and mutation cases), re-encoded as the test `tests/encodings.rs` re-encodes them (UTF-16
//! in either byte order, with and without a byte order mark, ISO-8859-1, ISO-8859-15,
//! windows-1252 and US-ASCII, where the characters of the document allow), must give the
//! results of the document in UTF-8:
//!
//! * Saxon-HE 12.10 with the CEN stylesheet of the corpus: the same SVRL report, byte for
//!   byte, or no report where it stops on the document in UTF-8;
//! * xmllint (libxml2) with the XML schema of the corpus (the complete OASIS UBL 2.1 set,
//!   or the UN/CEFACT CII D16B schemas): the same messages, with the same lines, and the
//!   same verdict.
//!
//! The test shows that the validator gives the result of the UTF-8 document on each; with
//! this example, the reference engines do too.
//!
//! ```text
//! cargo run --release -p einvoice-xsd --example xmllint-oracle   # assembles the OASIS set
//! cargo run --release -p einvoice-validator --example encoding-oracle
//! ```
//!
//! Saxon-HE 12.10 is taken from `$SAXON_JAR`, else where `scripts/fetch-saxon.sh` puts it
//! (`target/oracle/saxon/`), with the two XML Resolver jars it needs from the same
//! directory, each checked against its SHA-256; Java from `$JAVA`, else `java`; xmllint
//! from `$XMLLINT`, else `xmllint`. The complete OASIS UBL 2.1 schema set is the one the
//! `xmllint-oracle` example of einvoice-xsd assembles in `target/xsd-oracle/oasis-ubl-2.1/`
//! (with the two `XAdES` schemas the repository does not ship).

#[path = "../tests/common/reencode.rs"]
mod reencode;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use reencode::{TARGETS, reencode};
use sha2::{Digest, Sha256};

const SAXON_JARS: [(&str, &str); 3] = [
    (
        "Saxon-HE-12.10.jar",
        "b571af282f25d7301059f788b9a149aab8b5cdc14ef3d212dc5425d3dcbb9a97",
    ),
    (
        "xmlresolver-5.3.3.jar",
        "1fe4d5b92f708dcdb82dbce12919e0171e6b5ca62c6dca6220483625098feb5f",
    ),
    (
        "xmlresolver-5.3.3-data.jar",
        "b0c487ad2f3e558be8d829c916d2458d10aca6a5bafa7a4d0524b70845e48a5c",
    ),
];
const SAXON_DEFAULT: &str = "target/oracle/saxon/Saxon-HE-12.10.jar";

/// A corpus: its directory, its stylesheet and the entry point of its XML schema
/// (relative to the workspace root), and a name for the work directory.
struct Corpus {
    name: &'static str,
    dir: &'static str,
    stylesheet: &'static str,
    schema: fn(&Path) -> Result<PathBuf, String>,
}

const CORPORA: [Corpus; 2] = [
    Corpus {
        name: "ubl",
        dir: "corpus/cen-en16931-ubl-1.3.17",
        stylesheet: "artefacts/cen-en16931-ubl-1.3.17/ubl/xslt/EN16931-UBL-validation.xslt",
        schema: ubl_schema,
    },
    Corpus {
        name: "cii",
        dir: "corpus/cen-en16931-cii-1.3.17",
        stylesheet: "artefacts/cen-en16931-cii-1.3.17/cii/xslt/EN16931-CII-validation.xslt",
        schema: cii_schema,
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

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> String + '_ {
    move |e| format!("{}: {e}", path.display())
}

/// The input documents of `cases.tsv` and `mutations.tsv` of a corpus, relative to the
/// workspace root.
fn inputs(root: &Path, corpus: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for file in ["cases.tsv", "mutations.tsv"] {
        let path = root.join(corpus).join(file);
        let text = std::fs::read_to_string(&path).map_err(io(&path))?;
        for line in text.lines().skip(1) {
            let input = line
                .split('\t')
                .nth(1)
                .ok_or_else(|| format!("{}: {line}", path.display()))?;
            out.push(input.to_owned());
        }
    }
    Ok(out)
}

fn run() -> Result<(), String> {
    let root = std::path::absolute(workspace_root()).map_err(|e| e.to_string())?;
    let class_path = saxon_class_path(&root)?;
    let xmllint = std::env::var_os("XMLLINT").unwrap_or_else(|| "xmllint".into());
    let work = root.join("target/oracle/encodings");
    if work.exists() {
        std::fs::remove_dir_all(&work).map_err(io(&work))?;
    }
    let mut failures = Vec::new();
    for corpus in &CORPORA {
        let base = work.join(corpus.name);
        let inputs = inputs(&root, corpus.dir)?;
        // The documents in UTF-8, then in each target encoding, by the same file names.
        let utf8 = base.join("utf-8");
        std::fs::create_dir_all(&utf8).map_err(io(&utf8))?;
        let mut names: Vec<Vec<String>> = vec![Vec::new(); TARGETS.len()];
        for target in &TARGETS {
            let dir = base.join(target.label);
            std::fs::create_dir_all(&dir).map_err(io(&dir))?;
        }
        for (i, input) in inputs.iter().enumerate() {
            let path = root.join(input);
            let bytes = std::fs::read(&path).map_err(io(&path))?;
            let text = einvoice_xml::decode(&bytes)
                .map_err(|e| format!("{input}: {e}"))?
                .into_owned();
            let name = format!("case-{i:05}.xml");
            let file = utf8.join(&name);
            std::fs::write(&file, &bytes).map_err(io(&file))?;
            for (target, names) in TARGETS.iter().zip(&mut names) {
                if let Some(encoded) = reencode(&text, target) {
                    let file = base.join(target.label).join(&name);
                    std::fs::write(&file, encoded).map_err(io(&file))?;
                    names.push(name.clone());
                }
            }
        }
        let stylesheet = root.join(corpus.stylesheet);
        let schema = (corpus.schema)(&base)?;
        let utf8_svrl = saxon(&class_path, &utf8, &stylesheet)?;
        for (target, names) in TARGETS.iter().zip(&names) {
            if target.form_is_utf16() && names.len() != inputs.len() {
                failures.push(format!(
                    "{} {}: {} of {} documents re-encoded",
                    corpus.name,
                    target.label,
                    names.len(),
                    inputs.len()
                ));
            }
            let dir = base.join(target.label);
            // Saxon-HE: the SVRL of each document, as in UTF-8.
            let svrl = saxon(&class_path, &dir, &stylesheet)?;
            let mut same = 0;
            for name in names {
                let (expected, actual) = (utf8_svrl.join(name), svrl.join(name));
                let read = |p: &Path| std::fs::read(p).ok();
                if read(&expected) == read(&actual) {
                    same += 1;
                } else {
                    failures.push(format!(
                        "{} {}: Saxon-HE: the SVRL of {name} differs from UTF-8 ({})",
                        corpus.name,
                        target.label,
                        inputs[index_of(name)?]
                    ));
                }
            }
            // xmllint: the same output, on the same file names.
            let expected = xmllint_output(&xmllint, &schema, &utf8, names)?;
            let actual = xmllint_output(&xmllint, &schema, &dir, names)?;
            let xmllint_same = expected == actual;
            if !xmllint_same {
                let first = expected
                    .lines()
                    .zip(actual.lines())
                    .find(|(e, a)| e != a)
                    .map_or_else(String::new, |(e, a)| {
                        format!("\n  UTF-8: {e}\n  {}: {a}", target.label)
                    });
                failures.push(format!(
                    "{} {}: xmllint's output differs from UTF-8{first}",
                    corpus.name, target.label
                ));
            }
            println!(
                "{} {}: {} of {} documents, Saxon-HE: {same} SVRL reports as in UTF-8, xmllint: {}",
                corpus.name,
                target.label,
                names.len(),
                inputs.len(),
                if xmllint_same {
                    format!("the output in UTF-8 ({} lines)", expected.lines().count())
                } else {
                    "DIFFERS".to_owned()
                }
            );
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

/// The index of a file `case-NNNNN.xml`.
fn index_of(name: &str) -> Result<usize, String> {
    name.strip_prefix("case-")
        .and_then(|n| n.strip_suffix(".xml"))
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("not a case file: {name}"))
}

/// Runs Saxon-HE with `stylesheet` on every document of `dir`; the directory of the SVRL
/// reports (a document on which it stops has none).
fn saxon(class_path: &std::ffi::OsStr, dir: &Path, stylesheet: &Path) -> Result<PathBuf, String> {
    let out = dir.with_extension("svrl");
    std::fs::create_dir_all(&out).map_err(io(&out))?;
    let java = std::env::var_os("JAVA").unwrap_or_else(|| "java".into());
    let output = Command::new(&java)
        .arg("-cp")
        .arg(class_path)
        .arg("net.sf.saxon.Transform")
        .arg(format!("-s:{}", dir.display()))
        .arg(format!("-xsl:{}", stylesheet.display()))
        .arg(format!("-o:{}", out.display()))
        .output()
        .map_err(|e| format!("cannot run {}: {e}", java.display()))?;
    // A document on which the stylesheet stops makes the status a failure; the other
    // reports are written all the same.
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() && !stderr.contains("transformation") {
        return Err(format!("Saxon-HE failed on {}:\n{stderr}", dir.display()));
    }
    Ok(out)
}

/// xmllint's messages on the documents `names` of `dir`, validated against `schema`, run
/// from `dir` so that they name the documents the same way in every directory.
fn xmllint_output(
    xmllint: &std::ffi::OsStr,
    schema: &Path,
    dir: &Path,
    names: &[String],
) -> Result<String, String> {
    let output = Command::new(xmllint)
        .current_dir(dir)
        .arg("--noout")
        .arg("--schema")
        .arg(schema)
        .args(names)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", xmllint.display()))?;
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if stderr.contains("failed to compile") {
        return Err(format!("xmllint cannot compile {}", schema.display()));
    }
    Ok(stderr)
}

/// The complete OASIS UBL 2.1 set (as the `xmllint-oracle` example of einvoice-xsd
/// assembles it), through a schema importing both entry points, written to `base`.
fn ubl_schema(base: &Path) -> Result<PathBuf, String> {
    let set = workspace_root().join("target/xsd-oracle/oasis-ubl-2.1/xsd/maindoc");
    let set = std::path::absolute(&set).map_err(|e| e.to_string())?;
    if !set.join("UBL-Invoice-2.1.xsd").is_file() {
        return Err(format!(
            "{}: missing (run cargo run --release -p einvoice-xsd --example xmllint-oracle)",
            set.display()
        ));
    }
    let mut schema = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xsd:schema xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\">\n",
    );
    for (namespace, file) in [
        ("Invoice-2", "UBL-Invoice-2.1.xsd"),
        ("CreditNote-2", "UBL-CreditNote-2.1.xsd"),
    ] {
        let _ = writeln!(
            schema,
            "  <xsd:import namespace=\"urn:oasis:names:specification:ubl:schema:xsd:{namespace}\" schemaLocation=\"file://{}\"/>",
            set.join(file).display()
        );
    }
    schema.push_str("</xsd:schema>\n");
    let path = base.join("ubl-2.1-entry-points.xsd");
    std::fs::write(&path, schema).map_err(io(&path))?;
    Ok(path)
}

/// The UN/CEFACT CII D16B schemas of einvoice-xsd.
fn cii_schema(_: &Path) -> Result<PathBuf, String> {
    let path = workspace_root()
        .join("crates/einvoice-xsd/schemas/uncefact-cii-d16b")
        .join(einvoice_xsd::CII_D16B_ENTRY_POINT);
    std::path::absolute(&path).map_err(|e| e.to_string())
}

fn saxon_class_path(root: &Path) -> Result<std::ffi::OsString, String> {
    let saxon =
        std::env::var_os("SAXON_JAR").map_or_else(|| root.join(SAXON_DEFAULT), PathBuf::from);
    let dir = saxon.parent().unwrap_or_else(|| Path::new(""));
    let mut jars = Vec::with_capacity(SAXON_JARS.len());
    for (index, (name, sha256)) in SAXON_JARS.iter().enumerate() {
        let jar = if index == 0 {
            saxon.clone()
        } else {
            dir.join(name)
        };
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
        if hash != *sha256 {
            return Err(format!(
                "{}: SHA-256 {hash}, expected {sha256}",
                jar.display()
            ));
        }
        jars.push(jar);
    }
    std::env::join_paths(&jars).map_err(|e| format!("class path: {e}"))
}
