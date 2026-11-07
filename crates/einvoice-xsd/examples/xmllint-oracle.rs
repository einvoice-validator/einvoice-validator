//! The xmllint oracle of this crate: validates every case of the oracle (the CEN UBL
//! corpus, `tests/oracle/documents/` and the mutations of `tests/oracle/mutations.tsv`)
//! with xmllint (libxml2) against the vendored UBL 2.1 schemas, and compares xmllint's
//! verdicts and errors with `tests/oracle/expected.tsv` (or writes them). It then runs
//! this crate on every case and reports how it compares with xmllint, listing each
//! disagreement and whether `tests/oracle/disagreements.tsv` explains it.
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

    let observed = run_xmllint(&xmllint, &cases)?;
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
    // This crate against what xmllint said just now.
    let documented = format::parse_disagreements(&format::read(
        &format::oracle_dir().join("disagreements.tsv"),
    )?)?;
    let schema = einvoice_xsd::Schema::ubl_2_1().map_err(|e| e.to_string())?;
    let observed_map: BTreeMap<String, Expected> = observed.iter().cloned().collect();
    let differences = format::differences(&schema, &cases, &observed_map)?;
    let count = |kind: &str| differences.iter().filter(|d| d.kind == kind).count();
    let verdicts = count("verdict");
    println!(
        "einvoice-xsd vs xmllint: {} cases, {} same verdict ({} identical including errors), \
         {verdicts} different verdicts, {} same verdict with different errors, {} refused as unsupported",
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
    let problems = format::unexplained(&differences, &documented);
    for problem in &problems {
        println!("{problem}");
    }
    Ok(ok && problems.is_empty())
}

fn oracle_work_dir() -> PathBuf {
    format::workspace_root().join("target/xsd-oracle")
}

/// The schema xmllint loads: imports both entry points of the vendored schemas.
fn wrapper_schema(dir: &Path) -> Result<PathBuf, String> {
    let schemas = format::crate_dir()
        .join("schemas/oasis-ubl-2.1/xsd/maindoc")
        .canonicalize()
        .map_err(|e| format!("vendored schemas: {e}"))?;
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
fn run_xmllint(xmllint: &str, cases: &[Case]) -> Result<Vec<(String, Expected)>, String> {
    let dir = oracle_work_dir();
    let docs = dir.join("cases");
    if docs.exists() {
        std::fs::remove_dir_all(&docs).map_err(|e| format!("{}: {e}", docs.display()))?;
    }
    std::fs::create_dir_all(&docs).map_err(|e| format!("{}: {e}", docs.display()))?;
    let schema = wrapper_schema(&dir)?;
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
