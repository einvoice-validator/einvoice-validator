//! The xmllint oracle of this crate: validates every case of the oracle (the CEN UBL
//! corpus, `tests/oracle/documents/` and the mutations of `tests/oracle/mutations.tsv`)
//! with xmllint (libxml2) against the complete OASIS UBL 2.1 schema set, and compares
//! xmllint's verdicts and errors with `tests/oracle/expected.tsv` (or writes them). It
//! then runs this crate on every case, twice, and reports how it compares with
//! xmllint, listing each disagreement and whether `tests/oracle/disagreements.tsv`
//! explains it:
//!
//! * with the schemas this crate ships ([`einvoice_xsd::Schema::ubl_2_1`]): every
//!   difference must be documented, and every documented one must occur (as
//!   `tests/oracle.rs` checks);
//! * with the complete set, loaded with [`einvoice_xsd::Schema::load`]: the same,
//!   except for the disagreements caused by the `XAdES` schemas not being shipped,
//!   which must then not occur.
//!
//! ```text
//! cargo run -p einvoice-xsd --example xmllint-oracle             run xmllint, compare with expected.tsv
//! cargo run -p einvoice-xsd --example xmllint-oracle -- --write  run xmllint, rewrite expected.tsv
//! ```
//!
//! xmllint is taken from `$XMLLINT`, else `xmllint` on the `PATH`. The goldens were
//! produced with libxml2 2.9.14. Both entry points are loaded together, through a
//! generated schema that imports `UBL-Invoice-2.1.xsd` and `UBL-CreditNote-2.1.xsd`
//! (this crate does the same).
//!
//! The complete set is assembled in `target/xsd-oracle/oasis-ubl-2.1/`: the vendored
//! schema documents, and the two ETSI `XAdES` schemas of the OASIS package, which are
//! not part of the repository. These are read from an extracted copy of the OASIS
//! UBL 2.1 package when `$OASIS_UBL_2_1_DIR` names its root directory (whose copies
//! of the vendored documents must then be identical to them), else downloaded from
//! `docs.oasis-open.org` with `curl`. Their SHA-256 are checked either way, and so
//! is a copy left in the directory by an earlier run.

#[path = "../tests/oracle/format.rs"]
mod format;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use format::{Case, Expected};

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let write = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => false,
        ["--write"] => true,
        _ => return Err("usage: xmllint-oracle [--write]".to_owned()),
    };
    let cases = format::cases()?;
    let xmllint = std::env::var("XMLLINT").unwrap_or_else(|_| "xmllint".to_owned());
    let version = Command::new(&xmllint)
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot run {xmllint}: {e}"))?;
    let version_text = String::from_utf8_lossy(&version.stderr);
    println!(
        "{}",
        version_text
            .lines()
            .next()
            .unwrap_or("xmllint: unknown version")
    );

    let full = full_set(&oracle_work_dir())?;
    let observed = run_xmllint(&xmllint, &cases, &full)?;
    let expected_path = format::oracle_dir().join("expected.tsv");
    let mut ok = true;
    if write {
        std::fs::write(&expected_path, format::format_expected(&observed))
            .map_err(|e| format!("{}: {e}", expected_path.display()))?;
        println!("expected.tsv written: {} cases", observed.len());
    } else {
        let expected = format::parse_expected(&format::read(&expected_path)?)?;
        let mut differences = 0;
        for (case, result) in &observed {
            if expected.get(case) != Some(result) {
                differences += 1;
                println!(
                    "xmllint differs from expected.tsv: {case}: xmllint {} {:?}, expected {:?}",
                    result.verdict,
                    result.errors,
                    expected.get(case)
                );
            }
        }
        let observed_ids: BTreeSet<&str> = observed.iter().map(|(c, _)| c.as_str()).collect();
        for case in expected.keys() {
            if !observed_ids.contains(case.as_str()) {
                differences += 1;
                println!("expected.tsv has a case that no longer exists: {case}");
            }
        }
        if differences == 0 {
            println!(
                "xmllint agrees with expected.tsv on all {} cases",
                observed.len()
            );
        } else {
            println!("{differences} differences between xmllint and expected.tsv");
            ok = false;
        }
    }
    // This crate against what xmllint said just now, with the shipped schemas and
    // with the complete set.
    let documented = format::parse_disagreements(&format::read(
        &format::oracle_dir().join("disagreements.tsv"),
    )?)?;
    let observed_map: BTreeMap<String, Expected> = observed.iter().cloned().collect();
    let shipped = einvoice_xsd::Schema::ubl_2_1().map_err(|e| e.to_string())?;
    let shipped_ok = compare(
        "einvoice-xsd (shipped schemas) vs xmllint",
        &shipped,
        &cases,
        &observed_map,
        &documented,
    )?;
    let complete = load_complete(&full)?;
    let with_xades: Vec<format::Disagreement> = documented
        .iter()
        .filter(|x| !format::caused_by_unshipped_xades(x))
        .cloned()
        .collect();
    let complete_ok = compare(
        "einvoice-xsd (complete OASIS set) vs xmllint",
        &complete,
        &cases,
        &observed_map,
        &with_xades,
    )?;
    Ok(ok && shipped_ok && complete_ok)
}

/// Loads the complete set with this crate; every document must be loaded and no
/// import omitted.
fn load_complete(full: &FullSet) -> Result<einvoice_xsd::Schema, String> {
    let texts: Vec<(String, String)> = full
        .files
        .iter()
        .map(|(path, bytes)| {
            String::from_utf8(bytes.clone())
                .map(|text| (path.clone(), text))
                .map_err(|e| format!("{path}: {e}"))
        })
        .collect::<Result<_, _>>()?;
    let files: Vec<(&str, &str)> = texts
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let complete = einvoice_xsd::Schema::load(
        &files,
        &[
            einvoice_xsd::INVOICE_ENTRY_POINT,
            einvoice_xsd::CREDIT_NOTE_ENTRY_POINT,
        ],
    )
    .map_err(|e| format!("complete OASIS UBL 2.1 schema set: {e}"))?;
    if !complete.omitted_imports().is_empty() || complete.documents().len() != files.len() {
        return Err(format!(
            "complete OASIS UBL 2.1 schema set: {} of {} documents loaded, imports omitted: {:?}",
            complete.documents().len(),
            files.len(),
            complete.omitted_imports()
        ));
    }
    Ok(complete)
}

/// Runs this crate with `schema` on every case, prints how it compares with xmllint
/// and whether `documented` explains each difference. Whether they match exactly.
fn compare(
    title: &str,
    schema: &einvoice_xsd::Schema,
    cases: &[Case],
    observed: &BTreeMap<String, Expected>,
    documented: &[format::Disagreement],
) -> Result<bool, String> {
    let differences = format::differences(schema, cases, observed)?;
    let count = |kind: &str| differences.iter().filter(|d| d.kind == kind).count();
    let verdicts = count("verdict");
    let corpus = cases.iter().filter(|c| c.id.starts_with("corpus/")).count();
    let corpus_differences = differences
        .iter()
        .filter(|d| d.case.starts_with("corpus/"))
        .count();
    println!(
        "{title}: {} cases, {} same verdict ({} identical including errors), \
         {verdicts} different verdicts, {} same verdict with different errors, {} refused as unsupported; \
         corpus: {corpus} cases, {corpus_differences} differences",
        cases.len(),
        cases.len() - verdicts - count("unsupported"),
        cases.len() - differences.len(),
        count("errors"),
        count("unsupported"),
    );
    for d in &differences {
        let explanation = documented
            .iter()
            .find(|x| x.case == d.case && x.kind == d.kind)
            .map_or("UNDOCUMENTED", |x| x.explanation.as_str());
        println!(
            "  {} [{}] ours {} {:?} / xmllint {} {:?}: {explanation}",
            d.case, d.kind, d.ours.verdict, d.ours.errors, d.xmllint.verdict, d.xmllint.errors
        );
    }
    let problems = format::unexplained(&differences, documented);
    for problem in &problems {
        println!("{problem}");
    }
    Ok(problems.is_empty() && corpus_differences == 0)
}

fn oracle_work_dir() -> PathBuf {
    format::workspace_root().join("target/xsd-oracle")
}

/// The ETSI `XAdES` schemas of the OASIS UBL 2.1 package, which the repository does
/// not ship: path in the package, SHA-256 (the published files).
const XADES: [(&str, &str); 2] = [
    (
        "xsd/common/UBL-XAdESv132-2.1.xsd",
        "a4f726bcf8cc3f7d9ffa4dab99e005535a8e8b60dced1e5d94578d2e05afa96e",
    ),
    (
        "xsd/common/UBL-XAdESv141-2.1.xsd",
        "1fa4625e9cefcb7a9abb5ac1b64315547450031eece8a55bd584e4ba4b79dbc1",
    ),
];

/// Where the OASIS UBL 2.1 package is published, file by file.
const OASIS_UBL_2_1_URL: &str = "https://docs.oasis-open.org/ubl/os-UBL-2.1";

/// The complete OASIS UBL 2.1 schema set, written to a directory.
struct FullSet {
    /// The root of the set (the layout of the OASIS package).
    root: PathBuf,
    /// Every schema document: path relative to the root, bytes.
    files: Vec<(String, Vec<u8>)>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    use std::fmt::Write as _;
    let mut out = String::new();
    for byte in sha2::Sha256::digest(bytes) {
        // Writing to a String cannot fail.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Assembles the complete set: the vendored documents and the `XAdES` schemas, from
/// `$OASIS_UBL_2_1_DIR` or downloaded, all checked.
fn full_set(dir: &Path) -> Result<FullSet, String> {
    let root = dir.join("oasis-ubl-2.1");
    let package = std::env::var_os("OASIS_UBL_2_1_DIR").map(PathBuf::from);
    let mut files = Vec::new();
    for (path, text) in einvoice_xsd::UBL_2_1_FILES {
        if let Some(package) = &package {
            let theirs = package.join(path);
            let bytes = std::fs::read(&theirs).map_err(|e| format!("{}: {e}", theirs.display()))?;
            if bytes != text.as_bytes() {
                return Err(format!(
                    "{} differs from the vendored {path}: OASIS_UBL_2_1_DIR is not the OASIS UBL 2.1 package",
                    theirs.display()
                ));
            }
        }
        files.push(((*path).to_owned(), text.as_bytes().to_vec()));
    }
    for (path, sha256) in XADES {
        let target = root.join(path);
        let bytes = if let Some(package) = &package {
            let source = package.join(path);
            std::fs::read(&source).map_err(|e| format!("{}: {e}", source.display()))?
        } else if target.exists() {
            std::fs::read(&target).map_err(|e| format!("{}: {e}", target.display()))?
        } else {
            download(&format!("{OASIS_UBL_2_1_URL}/{path}"), dir)?
        };
        let actual = sha256_hex(&bytes);
        if actual != sha256 {
            return Err(format!(
                "{path}: SHA-256 {actual}, expected {sha256} (delete {} and run again)",
                target.display()
            ));
        }
        files.push((path.to_owned(), bytes));
    }
    for (path, bytes) in &files {
        let target = root.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&target, bytes).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    println!(
        "complete OASIS UBL 2.1 schema set: {} documents in {} (XAdES schemas from {}, checksums verified)",
        files.len(),
        root.display(),
        package
            .as_ref()
            .map_or_else(|| OASIS_UBL_2_1_URL.to_owned(), |p| p.display().to_string()),
    );
    Ok(FullSet { root, files })
}

/// Downloads `url` with curl (HTTPS only) and returns the bytes.
fn download(url: &str, dir: &Path) -> Result<Vec<u8>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = dir.join(".download");
    let status = Command::new("curl")
        .args([
            "--proto",
            "=https",
            "--tlsv1.2",
            "-fsSL",
            "--retry",
            "3",
            "-o",
        ])
        .arg(&tmp)
        .arg(url)
        .status()
        .map_err(|e| format!("cannot run curl: {e}"))?;
    if !status.success() {
        return Err(format!("cannot download {url} (curl: {status})"));
    }
    let bytes = std::fs::read(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::remove_file(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    Ok(bytes)
}

/// The schema xmllint loads: imports both entry points of the complete set.
fn wrapper_schema(dir: &Path, full: &FullSet) -> Result<PathBuf, String> {
    let schemas = full
        .root
        .join("xsd/maindoc")
        .canonicalize()
        .map_err(|e| format!("complete schema set: {e}"))?;
    let location = |file: &str| format!("file://{}", schemas.join(file).display());
    let text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <xsd:schema xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\">\n  \
         <xsd:import namespace=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\" schemaLocation=\"{}\"/>\n  \
         <xsd:import namespace=\"urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2\" schemaLocation=\"{}\"/>\n\
         </xsd:schema>\n",
        location("UBL-Invoice-2.1.xsd"),
        location("UBL-CreditNote-2.1.xsd"),
    );
    let path = dir.join("ubl-2.1-entry-points.xsd");
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Runs xmllint once on all cases, each written to its own file.
fn run_xmllint(
    xmllint: &str,
    cases: &[Case],
    full: &FullSet,
) -> Result<Vec<(String, Expected)>, String> {
    let dir = oracle_work_dir();
    let docs = dir.join("cases");
    if docs.exists() {
        std::fs::remove_dir_all(&docs).map_err(|e| format!("{}: {e}", docs.display()))?;
    }
    std::fs::create_dir_all(&docs).map_err(|e| format!("{}: {e}", docs.display()))?;
    let schema = wrapper_schema(&dir, full)?;
    let mut paths = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let path = docs.join(format!("case-{index:05}.xml"));
        std::fs::write(&path, &case.text).map_err(|e| format!("{}: {e}", path.display()))?;
        paths.push(path);
    }
    let output = Command::new(xmllint)
        .arg("--noout")
        .arg("--schema")
        .arg(&schema)
        .args(&paths)
        .output()
        .map_err(|e| format!("cannot run {xmllint}: {e}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let log = dir.join("xmllint.log");
    std::fs::write(&log, stderr.as_bytes()).map_err(|e| format!("{}: {e}", log.display()))?;
    let by_path: BTreeMap<String, usize> = paths
        .iter()
        .enumerate()
        .map(|(i, p)| (p.display().to_string(), i))
        .collect();
    let mut verdicts: Vec<Option<&str>> = vec![None; cases.len()];
    // Raw messages: (case index, line, message). A message that quotes a value with
    // line breaks continues on the following lines of the output.
    let mut messages: Vec<(usize, u32, String)> = Vec::new();
    let mut in_message = false;
    let mut malformed = vec![false; cases.len()];
    for line in stderr.lines() {
        let Some((index, rest)) = split_path(line, &by_path) else {
            if in_message && let Some(last) = messages.last_mut() {
                last.2.push('\n');
                last.2.push_str(line);
            }
            // Otherwise a context line after a parser error (source line, caret).
            continue;
        };
        in_message = false;
        if rest == " validates" {
            verdicts[index] = Some("valid");
        } else if rest == " fails to validate" {
            verdicts[index] = Some("invalid");
        } else if let Some(rest) = rest.strip_prefix(':') {
            let (line_number, message) = rest
                .split_once(": ")
                .ok_or_else(|| format!("unrecognised xmllint output: {line}"))?;
            let line_number: u32 = line_number
                .parse()
                .map_err(|_| format!("unrecognised xmllint output: {line}"))?;
            if message.contains("parser error") || message.contains("namespace error") {
                malformed[index] = true;
            } else if let Some((_, message)) = message.split_once("Schemas validity error : ") {
                messages.push((index, line_number, message.to_owned()));
                in_message = true;
            } else {
                return Err(format!("unrecognised xmllint output: {line}"));
            }
        } else {
            return Err(format!("unrecognised xmllint output: {line}"));
        }
    }
    let mut errors: Vec<Vec<(u32, String)>> = vec![Vec::new(); cases.len()];
    for (index, line_number, message) in &messages {
        errors[*index].push((*line_number, classify(message)?));
    }
    let mut results = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let verdict = if malformed[index] {
            "malformed"
        } else {
            verdicts[index].ok_or_else(|| format!("{}: no verdict from xmllint", case.id))?
        };
        if (verdict == "valid") != errors[index].is_empty() && verdict != "malformed" {
            return Err(format!(
                "{}: verdict {verdict} with errors {:?}",
                case.id, errors[index]
            ));
        }
        let mut list = std::mem::take(&mut errors[index]);
        if verdict == "malformed" {
            list.clear();
        }
        list.sort();
        results.push((
            case.id.clone(),
            Expected {
                verdict: verdict.to_owned(),
                errors: list.into_iter().map(|(l, c)| format!("{l}:{c}")).collect(),
            },
        ));
    }
    Ok(results)
}

/// The case index of a line of xmllint output and the text after the file path.
fn split_path<'a>(line: &'a str, by_path: &BTreeMap<String, usize>) -> Option<(usize, &'a str)> {
    let end = line.find(".xml")? + ".xml".len();
    let (path, rest) = line.split_at(end);
    by_path.get(path).map(|&index| (index, rest))
}

/// The category of an xmllint schema validity error. Must stay in step with
/// `einvoice_xsd::ErrorKind::category`.
fn classify(message: &str) -> Result<String, String> {
    let category = if message
        .contains("No matching global declaration available for the validation root")
    {
        "root"
    } else if message.contains("This element is not expected") {
        "unexpected-element"
    } else if message.contains("Missing child element(s)") {
        "missing-element"
    } else if message.contains("Character content other than whitespace is not allowed") {
        "text-not-allowed"
    } else if message.contains(
        "Element content is not allowed, because the content type is a simple type definition",
    ) || message
        .contains("Element content is not allowed, because the type definition is simple")
    {
        "element-in-simple-content"
    } else if message.contains("content is not allowed, because the content type is empty") {
        "content-not-empty"
    } else if message.contains(
        "No matching global element declaration available, but demanded by the strict wildcard",
    ) {
        "strict-wildcard-element"
    } else if message.contains(
        "No matching global attribute declaration available, but demanded by the strict wildcard",
    ) {
        "strict-wildcard-attribute"
    } else if message.contains("is not allowed.") && message.contains(", attribute '") {
        "attribute-not-allowed"
    } else if message.contains("is required but missing") {
        "attribute-missing"
    } else if message.contains("xsi:type")
        || message.contains("attribute '{http://www.w3.org/2001/XMLSchema-instance}type'")
    {
        "xsi-type"
    } else if message.contains("type definition is abstract") {
        "abstract-type"
    } else if message.contains("not 'nillable'") || message.contains("nillable") {
        "not-nillable"
    } else if message.contains("is not a valid value of") || message.contains("[facet '") {
        if message.starts_with("Element '") && message.contains("', attribute '") {
            "attribute-value"
        } else {
            "element-value"
        }
    } else {
        return Err(format!("unclassified xmllint error: {message}"));
    };
    Ok(category.to_owned())
}
