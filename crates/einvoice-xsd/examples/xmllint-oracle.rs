//! The xmllint oracle of this crate, two suites:
//!
//! * UBL: validates every case (the CEN UBL corpus, `tests/oracle/documents/` and the
//!   mutations of `tests/oracle/mutations.tsv`) with xmllint (libxml2) against the
//!   complete OASIS UBL 2.1 schema set, and compares xmllint's verdicts and errors
//!   with `tests/oracle/expected.tsv` (or writes them). It then runs this crate on
//!   every case, twice, and reports how it compares with xmllint, listing each
//!   disagreement and whether `tests/oracle/disagreements.tsv` explains it:
//!   * with the schemas this crate ships ([`einvoice_xsd::Schema::ubl_2_1`]): every
//!     difference must be documented, and every documented one must occur (as
//!     `tests/oracle.rs` checks);
//!   * with the complete set, loaded with [`einvoice_xsd::Schema::load`]: the same,
//!     except for the disagreements caused by the `XAdES` schemas not being shipped,
//!     which must then not occur.
//! * synthetic: the same with the synthetic schema set of
//!   `tests/oracle/synthetic/schemas/` (value constraints, length facets, anonymous
//!   complex types) and its documents, mutations and schema mutations, against
//!   `tests/oracle/synthetic/expected.tsv` and `disagreements.tsv`; a schema
//!   mutation's verdict is whether xmllint compiles the edited set.
//! * CII: every case and mutation case of the CEN CII corpus validated with xmllint
//!   against the vendored CII D16B schemas (the same files this crate embeds),
//!   compared with `tests/oracle/cii/expected.tsv` (or written there), then this crate
//!   ([`einvoice_xsd::Schema::cii_d16b`]) against xmllint, every difference documented
//!   in `tests/oracle/cii/disagreements.tsv` and none on a corpus case.
//!
//! ```text
//! cargo run -p einvoice-xsd --example xmllint-oracle             run xmllint, compare with expected.tsv
//! cargo run -p einvoice-xsd --example xmllint-oracle -- --write  run xmllint, rewrite expected.tsv
//! ```
//!
//! xmllint is taken from `$XMLLINT`, else `xmllint` on the `PATH`. The goldens were
//! produced with libxml2 2.9.14. Both UBL entry points are loaded together, through a
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
#[path = "../tests/oracle/xmllint.rs"]
mod xmllint;

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
    let program = xmllint::program();
    println!("{}", xmllint::version(&program)?);
    let ubl_ok = ubl(&program, write)?;
    let synthetic_ok = synthetic(&program, write)?;
    let cii_ok = cii(&program, write)?;
    Ok(ubl_ok && synthetic_ok && cii_ok)
}

/// The CII suite.
fn cii(program: &str, write: bool) -> Result<bool, String> {
    let cases = format::cii_cases()?;
    let entry = format::cii_entry_point()
        .canonicalize()
        .map_err(|e| format!("CII D16B schemas: {e}"))?;
    let observed = xmllint::run(program, &cases, &entry, &oracle_work_dir().join("cii"))?;
    let ok = check_expected(&format::cii_dir().join("expected.tsv"), &observed, write)?;
    let documented =
        format::parse_disagreements(&format::read(&format::cii_dir().join("disagreements.tsv"))?)?;
    let observed_map: BTreeMap<String, Expected> = observed.iter().cloned().collect();
    let schema = einvoice_xsd::Schema::cii_d16b().map_err(|e| e.to_string())?;
    let cii_ok = report(
        "einvoice-xsd (CII D16B schemas) vs xmllint",
        &cases,
        &format::differences(&schema, &cases, &observed_map)?,
        &documented,
    );
    Ok(ok && cii_ok)
}

/// The UBL suite.
fn ubl(program: &str, write: bool) -> Result<bool, String> {
    let cases = format::cases()?;
    let full = full_set(&oracle_work_dir())?;
    let schema = wrapper_schema(&oracle_work_dir(), &full)?;
    let observed = xmllint::run(program, &cases, &schema, &oracle_work_dir())?;
    let ok = check_expected(&format::oracle_dir().join("expected.tsv"), &observed, write)?;
    // This crate against what xmllint said just now, with the shipped schemas and
    // with the complete set.
    let documented = format::parse_disagreements(&format::read(
        &format::oracle_dir().join("disagreements.tsv"),
    )?)?;
    let observed_map: BTreeMap<String, Expected> = observed.iter().cloned().collect();
    let shipped = einvoice_xsd::Schema::ubl_2_1().map_err(|e| e.to_string())?;
    let shipped_ok = report(
        "einvoice-xsd (shipped schemas) vs xmllint",
        &cases,
        &format::differences(&shipped, &cases, &observed_map)?,
        &documented,
    );
    let complete = load_complete(&full)?;
    let with_xades: Vec<format::Disagreement> = documented
        .iter()
        .filter(|x| !format::caused_by_unshipped_xades(x))
        .cloned()
        .collect();
    let complete_ok = report(
        "einvoice-xsd (complete OASIS set) vs xmllint",
        &cases,
        &format::differences(&complete, &cases, &observed_map)?,
        &with_xades,
    );
    Ok(ok && shipped_ok && complete_ok)
}

/// The synthetic suite.
fn synthetic(program: &str, write: bool) -> Result<bool, String> {
    let (cases, schema_cases) = format::synthetic_cases()?;
    let dir = oracle_work_dir().join("synthetic");
    let entry = format::synthetic_dir()
        .join("schemas")
        .join(format::SYNTHETIC_ENTRY_POINT)
        .canonicalize()
        .map_err(|e| format!("synthetic schema set: {e}"))?;
    let mut observed = xmllint::run(program, &cases, &entry, &dir)?;
    for case in &schema_cases {
        let set = dir.join(&case.id);
        if set.exists() {
            std::fs::remove_dir_all(&set).map_err(|e| format!("{}: {e}", set.display()))?;
        }
        std::fs::create_dir_all(&set).map_err(|e| format!("{}: {e}", set.display()))?;
        for (path, text) in &case.files {
            let target = set.join(path);
            std::fs::write(&target, text).map_err(|e| format!("{}: {e}", target.display()))?;
        }
        let compiles = xmllint::compiles(program, &set.join(format::SYNTHETIC_ENTRY_POINT), &set)?;
        observed.push((
            case.id.clone(),
            Expected {
                verdict: if compiles { "loads" } else { "fails" }.to_owned(),
                errors: Vec::new(),
            },
        ));
    }
    let ok = check_expected(
        &format::synthetic_dir().join("expected.tsv"),
        &observed,
        write,
    )?;
    let documented = format::parse_disagreements(&format::read(
        &format::synthetic_dir().join("disagreements.tsv"),
    )?)?;
    let observed_map: BTreeMap<String, Expected> = observed.iter().cloned().collect();
    let files = format::synthetic_schema_files()?;
    let schema = format::load_set(&files, format::SYNTHETIC_ENTRY_POINT)
        .map_err(|e| format!("synthetic schema set: {e}"))?;
    let all: Vec<Case> = cases
        .iter()
        .cloned()
        .chain(schema_cases.iter().map(|c| Case {
            id: c.id.clone(),
            text: String::new(),
        }))
        .collect();
    let synthetic_ok = report(
        "einvoice-xsd (synthetic schemas) vs xmllint",
        &all,
        &format::synthetic_differences(&schema, &cases, &schema_cases, &observed_map)?,
        &documented,
    );
    Ok(ok && synthetic_ok)
}

/// Writes xmllint's results to `path` (`--write`), or compares them with it and
/// prints every difference. Whether they agree (always, when writing).
fn check_expected(
    path: &Path,
    observed: &[(String, Expected)],
    write: bool,
) -> Result<bool, String> {
    let name = path
        .strip_prefix(format::crate_dir())
        .unwrap_or(path)
        .display()
        .to_string();
    if write {
        std::fs::write(path, format::format_expected(observed))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        println!("{name} written: {} cases", observed.len());
        return Ok(true);
    }
    let expected = format::parse_expected(&format::read(path)?)?;
    let mut differences = 0;
    for (case, result) in observed {
        if expected.get(case) != Some(result) {
            differences += 1;
            println!(
                "xmllint differs from {name}: {case}: xmllint {} {:?}, expected {:?}",
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
            println!("{name} has a case that no longer exists: {case}");
        }
    }
    if differences == 0 {
        println!("xmllint agrees with {name} on all {} cases", observed.len());
        Ok(true)
    } else {
        println!("{differences} differences between xmllint and {name}");
        Ok(false)
    }
}

/// Prints how this crate compares with xmllint on `cases` and whether
/// `documented` explains each difference. Whether they match exactly (and no
/// corpus case differs).
fn report(
    title: &str,
    cases: &[Case],
    differences: &[format::Difference],
    documented: &[format::Disagreement],
) -> bool {
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
    for d in differences {
        let explanation = documented
            .iter()
            .find(|x| x.case == d.case && x.kind == d.kind)
            .map_or("UNDOCUMENTED", |x| x.explanation.as_str());
        println!(
            "  {} [{}] ours {} {:?} / xmllint {} {:?}: {explanation}",
            d.case, d.kind, d.ours.verdict, d.ours.errors, d.xmllint.verdict, d.xmllint.errors
        );
    }
    let problems = format::unexplained(differences, documented);
    for problem in &problems {
        println!("{problem}");
    }
    problems.is_empty() && corpus_differences == 0
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
