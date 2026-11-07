//! The CEN EN 16931 UBL 1.3.17 corpus (`corpus/README.md`): its cases, its stylesheet and
//! the canonical comparison with the Saxon-HE goldens. Shared by the equivalence test
//! and the examples.

#![allow(
    dead_code,
    reason = "shared by the equivalence test and the examples, each using a part"
)]

pub(crate) mod peppol;

use std::path::{Path, PathBuf};

use einvoice_xslt_exec::Executable;

pub(crate) const RULE_SET: &str = "cen-en16931-ubl-1.3.17";
pub(crate) const XSLT: &str =
    "artefacts/cen-en16931-ubl-1.3.17/ubl/xslt/EN16931-UBL-validation.xslt";

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

/// Every case of `corpus/<rule set>/cases.tsv`.
pub(crate) fn cases(root: &Path) -> Result<Vec<Case>, String> {
    let corpus = join(root, &format!("corpus/{RULE_SET}"));
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
