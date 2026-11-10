//! Running xmllint (libxml2) and reading its verdicts and errors in the form of
//! `expected.tsv`, shared by the `xmllint-oracle` example and the local check of
//! `tests/oracle.rs` on schema sets outside the repository.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::format::{Case, Expected};

/// The xmllint to run: `$XMLLINT`, else `xmllint` on the `PATH`.
pub(crate) fn program() -> String {
    std::env::var("XMLLINT").unwrap_or_else(|_| "xmllint".to_owned())
}

/// The first line of `xmllint --version`, or why it cannot be run.
pub(crate) fn version(xmllint: &str) -> Result<String, String> {
    let version = Command::new(xmllint)
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot run {xmllint}: {e}"))?;
    let text = String::from_utf8_lossy(&version.stderr);
    Ok(text
        .lines()
        .next()
        .unwrap_or("xmllint: unknown version")
        .to_owned())
}

/// Whether xmllint compiles the schema whose entry point is `schema` (it validates
/// a throw-away document in `dir` with it).
pub(crate) fn compiles(xmllint: &str, schema: &Path, dir: &Path) -> Result<bool, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let document = dir.join("probe.xml");
    std::fs::write(&document, "<probe/>\n").map_err(|e| format!("{}: {e}", document.display()))?;
    let output = Command::new(xmllint)
        .arg("--noout")
        .arg("--schema")
        .arg(schema)
        .arg(&document)
        .output()
        .map_err(|e| format!("cannot run {xmllint}: {e}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(!stderr.contains("failed to compile"))
}

/// Runs xmllint once on all cases, each written to its own file in `dir/cases`,
/// against the schema whose entry point is `schema`; the raw output is kept in
/// `dir/xmllint.log`. Results in case order.
pub(crate) fn run(
    xmllint: &str,
    cases: &[Case],
    schema: &Path,
    dir: &Path,
) -> Result<Vec<(String, Expected)>, String> {
    let docs = dir.join("cases");
    if docs.exists() {
        std::fs::remove_dir_all(&docs).map_err(|e| format!("{}: {e}", docs.display()))?;
    }
    std::fs::create_dir_all(&docs).map_err(|e| format!("{}: {e}", docs.display()))?;
    let mut paths: Vec<PathBuf> = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let path = docs.join(format!("case-{index:05}.xml"));
        std::fs::write(&path, &case.text).map_err(|e| format!("{}: {e}", path.display()))?;
        paths.push(path);
    }
    let output = Command::new(xmllint)
        .arg("--noout")
        .arg("--schema")
        .arg(schema)
        .args(&paths)
        .output()
        .map_err(|e| format!("cannot run {xmllint}: {e}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let log = dir.join("xmllint.log");
    std::fs::write(&log, stderr.as_bytes()).map_err(|e| format!("{}: {e}", log.display()))?;
    if stderr.contains("failed to compile") {
        return Err(format!(
            "xmllint cannot compile {} (see {})",
            schema.display(),
            log.display()
        ));
    }
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
/// `format::category`.
pub(crate) fn classify(message: &str) -> Result<String, String> {
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
    } else if message.contains("is not a valid value of")
        || message.contains("[facet '")
        || message.contains("does not match the fixed value constraint")
        || message.contains("has no corresponding namespace declaration in scope")
    {
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
