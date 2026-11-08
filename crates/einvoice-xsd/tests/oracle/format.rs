//! The cases of the xmllint oracle and the format of its files, shared by the
//! `xmllint-oracle` example (which writes `expected.tsv`) and `tests/oracle.rs`
//! (which checks this crate against it).
//!
//! Cases, in this order:
//! * `corpus/<case>`: every case of `corpus/cen-en16931-ubl-1.3.17/cases.tsv`;
//! * `document/<stem>`: every file of `tests/oracle/documents/`;
//! * `mutation/<id>`: every line of `tests/oracle/mutations.tsv`, applied to a document.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The corpus whose cases are part of the oracle, relative to the workspace root.
pub(crate) const CORPUS: &str = "corpus/cen-en16931-ubl-1.3.17";

/// A document to validate.
#[derive(Debug, Clone)]
pub(crate) struct Case {
    pub(crate) id: String,
    pub(crate) text: String,
}

/// What xmllint said about a case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Expected {
    /// `valid`, `invalid` or `malformed` (not well-formed XML).
    pub(crate) verdict: String,
    /// The errors, `line:category`, sorted.
    pub(crate) errors: Vec<String>,
}

/// A known, explained difference between this crate and xmllint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Disagreement {
    pub(crate) case: String,
    /// `verdict` (the validity verdicts differ), `errors` (same verdict, different
    /// errors) or `unsupported` (this crate refuses the document as unsupported).
    pub(crate) kind: String,
    pub(crate) explanation: String,
}

/// How the explanation of a disagreement caused by the `XAdES` schemas not being
/// shipped begins. Those disagreements occur with the shipped schemas only: with
/// the complete OASIS set (in the `xmllint-oracle` example) they must not.
pub(crate) const XADES_NOT_SHIPPED: &str = "XAdES schemas not shipped: content validated laxly.";

/// Whether a disagreement is one caused by the `XAdES` schemas not being shipped.
pub(crate) fn caused_by_unshipped_xades(disagreement: &Disagreement) -> bool {
    disagreement.explanation.starts_with(XADES_NOT_SHIPPED)
}

pub(crate) fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub(crate) fn workspace_root() -> PathBuf {
    crate_dir().join("../..")
}

pub(crate) fn oracle_dir() -> PathBuf {
    crate_dir().join("tests/oracle")
}

pub(crate) fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Lines of a TSV file without comments (`#`) and without its header line (first
/// non-comment line), split on tabs.
fn rows(text: &str) -> Vec<Vec<&str>> {
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .skip(1)
        .map(|line| line.split('\t').collect())
        .collect()
}

fn unescape(text: &str) -> Result<String, String> {
    if text == "\\e" {
        return Ok(String::new());
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            other => return Err(format!("bad escape \\{other:?} in {text:?}")),
        }
    }
    Ok(out)
}

/// Applies the edits of a mutation: each `find` must occur exactly once in the
/// text as edited so far.
pub(crate) fn apply(base: &str, edits: &[(String, String)]) -> Result<String, String> {
    let mut text = base.to_owned();
    for (find, replace) in edits {
        let count = text.matches(find.as_str()).count();
        if count != 1 {
            return Err(format!("{find:?} occurs {count} times, expected once"));
        }
        text = text.replacen(find.as_str(), replace, 1);
    }
    Ok(text)
}

/// Every case, in a fixed order.
pub(crate) fn cases() -> Result<Vec<Case>, String> {
    let root = workspace_root();
    let mut cases = Vec::new();

    let corpus = read(&root.join(CORPUS).join("cases.tsv"))?;
    for row in corpus.lines().skip(1).filter(|l| !l.is_empty()) {
        let fields: Vec<&str> = row.split('\t').collect();
        let (Some(id), Some(input)) = (fields.first(), fields.get(1)) else {
            return Err(format!("bad corpus line {row:?}"));
        };
        cases.push(Case {
            id: format!("corpus/{id}"),
            text: read(&root.join(input))?,
        });
    }

    let mut documents = BTreeMap::new();
    let dir = oracle_dir().join("documents");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .map(|entry| entry.map(|e| e.path()).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    entries.sort();
    for path in entries {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("bad file name {}", path.display()))?
            .to_owned();
        let text = read(&path)?;
        cases.push(Case {
            id: format!("document/{stem}"),
            text: text.clone(),
        });
        documents.insert(stem, text);
    }

    let mutations = read(&oracle_dir().join("mutations.tsv"))?;
    for fields in rows(&mutations) {
        let [id, base, edits @ ..] = fields.as_slice() else {
            return Err(format!("bad mutation line {fields:?}"));
        };
        if edits.is_empty() || edits.len() % 2 != 0 {
            return Err(format!(
                "mutation {id}: find and replace columns must come in pairs"
            ));
        }
        let edits: Vec<(String, String)> = edits
            .chunks(2)
            .map(|pair| Ok((unescape(pair[0])?, unescape(pair[1])?)))
            .collect::<Result<_, String>>()?;
        let base_text = documents
            .get(*base)
            .ok_or_else(|| format!("mutation {id}: unknown base document {base}"))?;
        let text = apply(base_text, &edits).map_err(|e| format!("mutation {id}: {e}"))?;
        cases.push(Case {
            id: format!("mutation/{id}"),
            text,
        });
    }

    let mut seen = std::collections::BTreeSet::new();
    for case in &cases {
        if !seen.insert(case.id.as_str()) {
            return Err(format!("duplicate case id {}", case.id));
        }
    }
    Ok(cases)
}

pub(crate) const EXPECTED_HEADER: &str = "case\tverdict\terrors";

/// Parses `expected.tsv`.
pub(crate) fn parse_expected(text: &str) -> Result<BTreeMap<String, Expected>, String> {
    let mut map = BTreeMap::new();
    for fields in rows(text) {
        let [case, verdict, errors] = fields.as_slice() else {
            return Err(format!("bad expected line {fields:?}"));
        };
        let errors = errors
            .split(' ')
            .filter(|e| !e.is_empty())
            .map(str::to_owned)
            .collect();
        let expected = Expected {
            verdict: (*verdict).to_owned(),
            errors,
        };
        if map.insert((*case).to_owned(), expected).is_some() {
            return Err(format!("duplicate expected case {case}"));
        }
    }
    Ok(map)
}

/// Writes `expected.tsv`.
pub(crate) fn format_expected(results: &[(String, Expected)]) -> String {
    let mut out = String::from(
        "# Written by `cargo run -p einvoice-xsd --example xmllint-oracle -- --write`: xmllint's\n\
         # verdict and errors (line:category, sorted) for every case. Do not edit by hand.\n",
    );
    out.push_str(EXPECTED_HEADER);
    out.push('\n');
    for (case, expected) in results {
        out.push_str(case);
        out.push('\t');
        out.push_str(&expected.verdict);
        out.push('\t');
        out.push_str(&expected.errors.join(" "));
        out.push('\n');
    }
    out
}

/// Parses `disagreements.tsv`.
pub(crate) fn parse_disagreements(text: &str) -> Result<Vec<Disagreement>, String> {
    rows(text)
        .into_iter()
        .map(|fields| match fields.as_slice() {
            [case, kind, explanation] if matches!(*kind, "verdict" | "errors" | "unsupported") => {
                Ok(Disagreement {
                    case: (*case).to_owned(),
                    kind: (*kind).to_owned(),
                    explanation: (*explanation).to_owned(),
                })
            }
            _ => Err(format!("bad disagreement line {fields:?}")),
        })
        .collect()
}

/// The xmllint category of an error of this crate (see `classify` in the example).
pub(crate) fn category(error: &einvoice_xsd::ValidationError) -> &'static str {
    use einvoice_xsd::ErrorKind;
    match error.kind {
        ErrorKind::UnknownRoot => "root",
        ErrorKind::UnexpectedElement => "unexpected-element",
        ErrorKind::MissingElement => "missing-element",
        ErrorKind::TextNotAllowed => "text-not-allowed",
        ErrorKind::ElementInSimpleContent => "element-in-simple-content",
        ErrorKind::ContentNotEmpty => "content-not-empty",
        ErrorKind::UndeclaredElement => "strict-wildcard-element",
        ErrorKind::UndeclaredAttribute => "strict-wildcard-attribute",
        ErrorKind::AttributeNotAllowed => "attribute-not-allowed",
        ErrorKind::MissingAttribute => "attribute-missing",
        ErrorKind::InvalidValue => "element-value",
        ErrorKind::InvalidAttributeValue | ErrorKind::InvalidXsiNil => "attribute-value",
        ErrorKind::DuplicateId => {
            if error.attribute.is_some() {
                "attribute-value"
            } else {
                "element-value"
            }
        }
        ErrorKind::InvalidXsiType => "xsi-type",
        ErrorKind::AbstractType => "abstract-type",
        ErrorKind::NotNillable => "not-nillable",
    }
}

/// The line on which the start tag beginning at `line`:`column` (1-based, column in
/// characters) ends. This crate reports where an element's start tag begins;
/// libxml2 reports the line where it ends (its parser creates the element there).
pub(crate) fn start_tag_end_line(text: &str, line: u32, column: u32) -> u32 {
    let line_start = if line <= 1 {
        0
    } else {
        text.match_indices('\n')
            .nth(usize::try_from(line - 2).unwrap_or(usize::MAX))
            .map_or(text.len(), |(i, _)| i + 1)
    };
    let mut quote = None;
    let mut current = line;
    for c in text[line_start..]
        .chars()
        .skip(usize::try_from(column - 1).unwrap_or(0))
    {
        match (c, quote) {
            ('\n', _) => current += 1,
            ('"' | '\'', None) => quote = Some(c),
            (q, Some(open)) if q == open => quote = None,
            ('>', None) => break,
            _ => {}
        }
    }
    current
}

/// What this crate says about a case, in the form of `expected.tsv`: verdict
/// `valid`, `invalid`, `malformed` or `unsupported`.
pub(crate) fn observe(schema: &einvoice_xsd::Schema, case: &Case) -> Expected {
    match schema.validate(&case.text) {
        Ok(report) => {
            let mut errors: Vec<(u32, &str)> = report
                .errors
                .iter()
                .map(|e| {
                    (
                        start_tag_end_line(&case.text, e.line, e.column),
                        category(e),
                    )
                })
                .collect();
            errors.sort_unstable();
            Expected {
                verdict: if report.is_valid() {
                    "valid"
                } else {
                    "invalid"
                }
                .to_owned(),
                errors: errors
                    .into_iter()
                    .map(|(l, c)| format!("{l}:{c}"))
                    .collect(),
            }
        }
        Err(einvoice_xsd::DocumentError::Malformed { .. }) => Expected {
            verdict: "malformed".to_owned(),
            errors: Vec::new(),
        },
        Err(einvoice_xsd::DocumentError::Unsupported { .. }) => Expected {
            verdict: "unsupported".to_owned(),
            errors: Vec::new(),
        },
    }
}

/// One case where this crate and xmllint differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Difference {
    pub(crate) case: String,
    /// As in [`Disagreement::kind`].
    pub(crate) kind: String,
    pub(crate) ours: Expected,
    pub(crate) xmllint: Expected,
}

/// Runs this crate on every case and lists where it differs from `expected`.
/// Fails if a case has no expected result or an expected result has no case.
pub(crate) fn differences(
    schema: &einvoice_xsd::Schema,
    cases: &[Case],
    expected: &BTreeMap<String, Expected>,
) -> Result<Vec<Difference>, String> {
    let mut out = Vec::new();
    for case in cases {
        let xmllint = expected
            .get(&case.id)
            .ok_or_else(|| format!("{}: no expected result", case.id))?;
        let ours = observe(schema, case);
        let kind = if ours.verdict == "unsupported" {
            "unsupported"
        } else if ours.verdict != xmllint.verdict {
            "verdict"
        } else if ours.errors != xmllint.errors {
            "errors"
        } else {
            continue;
        };
        out.push(Difference {
            case: case.id.clone(),
            kind: kind.to_owned(),
            ours,
            xmllint: xmllint.clone(),
        });
    }
    let ids: std::collections::BTreeSet<&str> = cases.iter().map(|c| c.id.as_str()).collect();
    if let Some(stale) = expected.keys().find(|k| !ids.contains(k.as_str())) {
        return Err(format!(
            "{stale}: expected result for a case that does not exist"
        ));
    }
    Ok(out)
}

/// Compares the observed differences with the documented disagreements: one line
/// per difference that is not documented and per documented disagreement that no
/// longer occurs. Empty when they match exactly.
pub(crate) fn unexplained(differences: &[Difference], documented: &[Disagreement]) -> Vec<String> {
    let mut problems = Vec::new();
    for d in differences {
        if !documented
            .iter()
            .any(|x| x.case == d.case && x.kind == d.kind)
        {
            problems.push(format!(
                "undocumented {} disagreement: {}: this crate {} {:?}, xmllint {} {:?}",
                d.kind, d.case, d.ours.verdict, d.ours.errors, d.xmllint.verdict, d.xmllint.errors
            ));
        }
    }
    for x in documented {
        if x.explanation.trim().is_empty() {
            problems.push(format!(
                "documented {} disagreement without explanation: {}",
                x.kind, x.case
            ));
        }
        if !differences
            .iter()
            .any(|d| x.case == d.case && x.kind == d.kind)
        {
            problems.push(format!(
                "documented {} disagreement no longer occurs: {}",
                x.kind, x.case
            ));
        }
    }
    problems
}
