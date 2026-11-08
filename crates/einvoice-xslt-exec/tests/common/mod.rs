//! The vendored corpora (`corpus/README.md`), CEN EN 16931 UBL 1.3.17 by default and CII
//! 1.3.17: their cases, their stylesheets and the canonical comparison with the Saxon-HE
//! goldens. Shared by the equivalence tests and the examples.

#![allow(
    dead_code,
    reason = "shared by the equivalence test and the examples, each using a part"
)]

pub(crate) mod peppol;

use std::path::{Path, PathBuf};

use einvoice_xpath_eval::{DEFAULT_MAX_DEPTH, Document, LimitExceeded, NodeAxis, NodeKind};
use einvoice_xslt_exec::{
    Executable, TransformError, TransformOptions, Transformation, max_nesting_for_depth,
};

pub(crate) const RULE_SET: &str = "cen-en16931-ubl-1.3.17";
pub(crate) const XSLT: &str =
    "artefacts/cen-en16931-ubl-1.3.17/ubl/xslt/EN16931-UBL-validation.xslt";
pub(crate) const CII_RULE_SET: &str = "cen-en16931-cii-1.3.17";
pub(crate) const CII_XSLT: &str =
    "artefacts/cen-en16931-cii-1.3.17/cii/xslt/EN16931-CII-validation.xslt";

pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// Joins a `/`-separated relative path onto `base`.
pub(crate) fn join(base: &Path, relative: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    for part in relative.split('/').filter(|p| !p.is_empty()) {
        path.push(part);
    }
    path
}

pub(crate) fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// One case: its id, its input document (absolute path) and its golden.
#[derive(Debug, Clone)]
pub(crate) struct Case {
    pub(crate) id: String,
    pub(crate) input: String,
    pub(crate) golden: PathBuf,
}

/// Every case of `corpus/<RULE_SET>/cases.tsv`.
pub(crate) fn cases(root: &Path) -> Result<Vec<Case>, String> {
    cases_of(root, RULE_SET)
}

/// Every case of `corpus/<rule_set>/cases.tsv`.
pub(crate) fn cases_of(root: &Path, rule_set: &str) -> Result<Vec<Case>, String> {
    let corpus = join(root, &format!("corpus/{rule_set}"));
    read(&corpus.join("cases.tsv"))?
        .lines()
        .skip(1)
        .map(|line| {
            let mut columns = line.split('\t');
            let (Some(id), Some(input)) = (columns.next(), columns.next()) else {
                return Err(format!("cases.tsv: bad line {line:?}"));
            };
            let input = std::path::absolute(join(root, input)).map_err(|e| e.to_string())?;
            let input = input
                .to_str()
                .ok_or_else(|| format!("{}: not UTF-8", input.display()))?
                .to_owned();
            let golden = join(&corpus, &format!("golden/{id}.svrl.txt"));
            Ok(Case {
                id: id.to_owned(),
                input,
                golden,
            })
        })
        .collect()
}

/// The prepared CEN stylesheet.
pub(crate) fn executable(root: &Path) -> Result<Executable, String> {
    prepare(&join(root, XSLT))
}

/// The stylesheet at `path`, loaded and prepared.
pub(crate) fn prepare(path: &Path) -> Result<Executable, String> {
    let text = read(path)?;
    let sheet = einvoice_xslt::load(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Executable::new(&sheet).map_err(|e| format!("{}: {e}", path.display()))
}

/// Runs the stylesheet on the document `text` read from `path` (with the `file:` URI
/// Saxon gives it, as its base URI) and returns the SVRL. The text is borrowed (`&str`)
/// or given (`String`), as [`Executable::transform`] takes it.
pub(crate) fn svrl<'s>(
    executable: &Executable,
    text: impl Into<std::borrow::Cow<'s, str>>,
    path: &str,
) -> Result<String, String> {
    let uri = einvoice_xpath_eval::file_uri(Path::new(path));
    executable
        .transform(text, uri.as_deref())
        .map(|t| t.output)
        .map_err(|e| format!("{path}: {e}"))
}

/// The maximum depth the equivalence tests raise the limit to for the cases deeper than
/// [`DEFAULT_MAX_DEPTH`], to compare them with Saxon-HE, which has no such limit.
pub(crate) const RAISED_MAX_DEPTH: u32 = 1024;

/// The element depth of a document (the document element is at depth 1); 0 for text
/// that is not XML.
pub(crate) fn depth(text: &str) -> u32 {
    let Ok(doc) = Document::parse_with_max_depth(text, None, u32::MAX) else {
        return 0;
    };
    doc.root()
        .axis(NodeAxis::Descendant)
        .filter(|n| n.kind() == NodeKind::Element)
        .map(|n| {
            n.axis(NodeAxis::Ancestor)
                .filter(|a| a.kind() == NodeKind::Element)
                .count()
                + 1
        })
        .max()
        .map_or(0, |d| u32::try_from(d).unwrap_or(u32::MAX))
}

/// Runs the stylesheet on a case as a caller with the default options does. A document
/// deeper than [`DEFAULT_MAX_DEPTH`] must be refused, with a `LIMIT` error that names
/// the depth; it is then run again with the maximum raised to [`RAISED_MAX_DEPTH`], on
/// the same thread (the default 2 MiB stack of a test thread), for the comparison with
/// Saxon-HE. Every other document must give its result under the default options.
pub(crate) fn transform_case(
    executable: &Executable,
    text: &str,
    uri: Option<&str>,
) -> Result<Transformation, TransformError> {
    let result = executable.transform(text, uri);
    let deeper = depth(text) > DEFAULT_MAX_DEPTH;
    match &result {
        Err(e) if deeper => {
            assert!(
                matches!(
                    e.limit(),
                    Some(LimitExceeded::Depth {
                        max: DEFAULT_MAX_DEPTH,
                        ..
                    })
                ),
                "{e}"
            );
            assert_eq!(e.code(), "LIMIT");
            let options = TransformOptions {
                max_depth: RAISED_MAX_DEPTH,
                max_nesting: max_nesting_for_depth(RAISED_MAX_DEPTH),
                ..TransformOptions::default()
            };
            executable.transform_with(text, uri, &options)
        }
        _ => {
            assert!(
                !deeper,
                "a document deeper than the default maximum depth was accepted"
            );
            if let Err(e) = &result {
                assert_eq!(e.limit(), None, "{e}");
            }
            result
        }
    }
}

/// The SVRL of a case read from `path` ([`transform_case`]).
pub(crate) fn case_svrl(executable: &Executable, text: &str, path: &str) -> Result<String, String> {
    let uri = einvoice_xpath_eval::file_uri(Path::new(path));
    transform_case(executable, text, uri.as_deref())
        .map(|t| t.output)
        .map_err(|e| format!("{path}: {e}"))
}

/// The canonical form of the SVRL of a document read from `path`.
pub(crate) fn canonical(svrl: &str, path: &str) -> Result<String, String> {
    einvoice_svrl::canonicalize(svrl, &|u| einvoice_svrl::is_document_uri(u, path))
        .map_err(|e| format!("{path}: the SVRL cannot be canonicalised: {e}"))
}

/// The first line where two canonical files differ, with both versions.
pub(crate) fn first_difference(golden: &str, ours: &str) -> String {
    let (g, o): (Vec<&str>, Vec<&str>) = (golden.lines().collect(), ours.lines().collect());
    let index = g
        .iter()
        .zip(&o)
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| g.len().min(o.len()));
    format!(
        "  line {}\n  saxon: {}\n  ours:  {}\n  (saxon {} lines, ours {} lines)",
        index + 1,
        g.get(index).unwrap_or(&"<end>"),
        o.get(index).unwrap_or(&"<end>"),
        g.len(),
        o.len()
    )
}

/// One mutation case (`corpus/<rule set>/mutations.tsv`): its id and its input document
/// (absolute path).
#[derive(Debug, Clone)]
pub(crate) struct MutationCase {
    pub(crate) id: String,
    pub(crate) input: String,
}

/// The cases of `<corpus>/mutations.tsv`, whose inputs are relative to `base`.
pub(crate) fn mutation_cases(corpus: &Path, base: &Path) -> Result<Vec<MutationCase>, String> {
    let text = read(&corpus.join("mutations.tsv"))?;
    let mut lines = text.lines();
    if lines.next() != Some("case\tinput\tbase\treason") {
        return Err("mutations.tsv: unexpected header".to_owned());
    }
    lines
        .map(|line| {
            let mut columns = line.split('\t');
            let (Some(id), Some(input)) = (columns.next(), columns.next()) else {
                return Err(format!("mutations.tsv: bad line {line:?}"));
            };
            let input = std::path::absolute(join(base, input)).map_err(|e| e.to_string())?;
            let input = input
                .to_str()
                .ok_or_else(|| format!("{}: not UTF-8", input.display()))?
                .to_owned();
            Ok(MutationCase {
                id: id.to_owned(),
                input,
            })
        })
        .collect()
}

/// What Saxon-HE gave on a case: its canonical SVRL, or the dynamic error that stopped
/// it (`<id>.error.txt`: `error <CODE>`, then `message <message>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Expected {
    Svrl(String),
    Error { code: String, message: String },
}

/// The golden of case `id` in `golden_dir`.
pub(crate) fn expected(golden_dir: &Path, id: &str) -> Result<Expected, String> {
    let error = join(golden_dir, &format!("{id}.error.txt"));
    if error.exists() {
        let text = read(&error)?;
        let mut lines = text.lines();
        let code = lines.next().and_then(|l| l.strip_prefix("error "));
        let message = lines.next().and_then(|l| l.strip_prefix("message "));
        return match (code, message, lines.next()) {
            (Some(code), Some(message), None) => Ok(Expected::Error {
                code: code.to_owned(),
                message: message.to_owned(),
            }),
            _ => Err(format!("{}: malformed", error.display())),
        };
    }
    read(&join(golden_dir, &format!("{id}.svrl.txt"))).map(Expected::Svrl)
}

/// Runs the stylesheet on every mutation case and compares with the goldens in
/// `golden_dir`. Returns the differences (an error with the same code but another message
/// is not one: it is returned apart, for information).
pub(crate) fn check_mutations(
    executable: &Executable,
    cases: &[MutationCase],
    golden_dir: &Path,
) -> (Vec<String>, Vec<String>) {
    let mut problems = Vec::new();
    let mut messages = Vec::new();
    for case in cases {
        let outcome = (|| -> Result<(), String> {
            let text = read(Path::new(&case.input))?;
            let uri = einvoice_xpath_eval::file_uri(Path::new(&case.input));
            let ours = transform_case(executable, &text, uri.as_deref());
            match (expected(golden_dir, &case.id)?, ours) {
                (Expected::Svrl(golden), Ok(t)) => {
                    let canonical = canonical(&t.output, &case.input)?;
                    if canonical == golden {
                        Ok(())
                    } else {
                        Err(first_difference(&golden, &canonical))
                    }
                }
                (Expected::Svrl(_), Err(e)) => {
                    Err(format!("  saxon: a report\n  ours:  error {e}"))
                }
                (Expected::Error { code, .. }, Ok(_)) => {
                    Err(format!("  saxon: error {code}\n  ours:  a report"))
                }
                (Expected::Error { code, message }, Err(e)) => {
                    if e.code() != code {
                        return Err(format!(
                            "  saxon: error {code} {message}\n  ours:  error {e}"
                        ));
                    }
                    if e.message() != message {
                        messages.push(format!(
                            "{}: {code}\n  saxon: {message}\n  ours:  {}",
                            case.id,
                            e.message()
                        ));
                    }
                    Ok(())
                }
            }
        })();
        if let Err(e) = outcome {
            problems.push(format!("{}\n{e}", case.id));
        }
    }
    (problems, messages)
}
