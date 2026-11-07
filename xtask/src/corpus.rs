//! The test corpus of a rule set: every case (an XML document to validate),
//! the documents extracted from the VEFA test sets, and `cases.tsv`, the
//! index of all cases with the VEFA expectations.
//!
//! Layout, relative to the corpus directory of the rule set
//! ([`RuleSet::corpus_dir`]):
//!
//! - `cases.tsv`: one line per case, see [`render_index`].
//! - for a vendored rule set, the upstream licence file, copied unchanged:
//!   everything in the directory is derived from the vendored files and shares
//!   their licence.
//! - `vefa/<test set directory>/<test set name>.<NNN>.xml`: the document of
//!   the NNN-th `<test>` (1-based) of that test set.
//! - `<golden dir>/<case id>.svrl.txt`, one golden directory per stylesheet:
//!   the canonical SVRL Saxon-HE produced.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use crate::ruleset::{RuleSet, Stylesheet};
use crate::util::{self, Error, Result};
use crate::vefa::{self, Expectation};

/// The index file of a corpus.
pub(crate) const INDEX: &str = "cases.tsv";
/// Directory of the documents extracted from VEFA test sets.
pub(crate) const VEFA_DIR: &str = "vefa";
/// Suffix of a golden file, appended to the case id.
pub(crate) const GOLDEN_SUFFIX: &str = ".svrl.txt";

const INDEX_HEADER: &str = "case\tinput\tkind\tscope\tsuccess\terror\twarning\tdescription";

/// One document to validate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Case {
    /// Unique id: `examples/<file>`, `testfiles/<file>` or
    /// `vefa/<set dir>/<set>.<NNN>.xml`. Also names the golden file.
    pub(crate) id: String,
    /// The document, relative to the base directory of the rule set (the
    /// repository root for a vendored rule set).
    pub(crate) input: String,
    /// The VEFA expectation, for cases extracted from a test set.
    pub(crate) expectation: Option<Expectation>,
}

impl Case {
    /// The golden file of this case for a stylesheet, relative to the base
    /// directory of the rule set.
    pub(crate) fn golden(&self, rule_set: &RuleSet, stylesheet: &Stylesheet) -> String {
        format!(
            "{}/{}/{}{GOLDEN_SUFFIX}",
            rule_set.corpus_dir(),
            stylesheet.golden_dir,
            self.id
        )
    }
}

/// The corpus as derived from the vendored files: the cases, and the derived
/// files (path relative to the corpus directory, contents).
#[derive(Debug)]
pub(crate) struct Derived {
    pub(crate) cases: Vec<Case>,
    pub(crate) files: BTreeMap<String, Vec<u8>>,
}

/// Derives the corpus from the upstream files.
pub(crate) fn derive(root: &Path, rule_set: &RuleSet) -> Result<Derived> {
    let base = rule_set.base(root);
    let source_dir = rule_set.source;
    let corpus = rule_set.corpus_dir();
    let mut cases = Vec::new();
    let mut files = BTreeMap::new();
    let mut ignored_seen = Vec::new();
    // The files of `dir` (relative to the source directory), less the ignored ones.
    let mut list = |dir: &str| -> Result<Vec<String>> {
        let mut names = Vec::new();
        for name in util::list_files(&util::join(&base, &format!("{source_dir}/{dir}")))? {
            let relative = format!("{dir}/{name}");
            if rule_set.ignored.contains(&relative.as_str()) {
                ignored_seen.push(relative);
            } else {
                names.push(name);
            }
        }
        Ok(names)
    };

    for (prefix, dir) in rule_set.document_dirs {
        let names = list(dir)?;
        if names.is_empty() {
            return Err(Error::new(format!("{source_dir}/{dir}: no documents")));
        }
        for name in names {
            cases.push(Case {
                id: format!("{prefix}/{name}"),
                input: format!("{source_dir}/{dir}/{name}"),
                expectation: None,
            });
        }
    }

    for dir in rule_set.vefa_dirs {
        let set_dir = dir.rsplit('/').next().unwrap_or(dir);
        let names = list(dir)?;
        if names.is_empty() {
            return Err(Error::new(format!("{source_dir}/{dir}: no test sets")));
        }
        for name in names {
            let stem = name
                .strip_suffix(".xml")
                .filter(|s| !s.contains('/'))
                .ok_or_else(|| {
                    Error::new(format!("{source_dir}/{dir}/{name}: not a test set file"))
                })?;
            let source_path = format!("{source_dir}/{dir}/{name}");
            let source = util::read_string(&util::join(&base, &source_path))?;
            let tests =
                vefa::extract(&source).map_err(|e| Error::new(format!("{source_path}: {e}")))?;
            if tests.is_empty() {
                return Err(Error::new(format!("{source_path}: no <test>")));
            }
            for test in tests {
                if test.ordinal > 999 {
                    return Err(Error::new(format!("{source_path}: more than 999 tests")));
                }
                let id = format!("{VEFA_DIR}/{set_dir}/{stem}.{:03}.xml", test.ordinal);
                files.insert(id.clone(), test.document.into_bytes());
                cases.push(Case {
                    input: format!("{corpus}/{id}"),
                    id,
                    expectation: Some(test.expectation),
                });
            }
        }
    }

    for ignored in rule_set.ignored {
        if !ignored_seen.iter().any(|seen| seen == ignored) {
            return Err(Error::new(format!(
                "{source_dir}/{ignored}: listed as ignored, but not found"
            )));
        }
    }

    let mut ids: Vec<&str> = cases.iter().map(|c| c.id.as_str()).collect();
    ids.sort_unstable();
    if let Some([id, _]) = ids.windows(2).find(|w| w[0] == w[1]) {
        return Err(Error::new(format!("duplicate case id {id}")));
    }

    files.insert(INDEX.to_owned(), render_index(&cases)?.into_bytes());
    if let Some(licence) = rule_set.licence() {
        let text = util::read(&util::join(&base, &format!("{source_dir}/{licence}")))?;
        files.insert(licence.to_owned(), text);
    }
    Ok(Derived { cases, files })
}

/// `cases.tsv`: a header line, then one line per case with tab-separated
/// columns `case`, `input`, `kind` (`document` or `vefa`), and for VEFA cases
/// `scope`, `success`, `error`, `warning` (space-separated rule ids) and
/// `description` (whitespace normalised). Those five are empty for whole
/// documents.
fn render_index(cases: &[Case]) -> Result<String> {
    let mut out = String::new();
    out.push_str(INDEX_HEADER);
    out.push('\n');
    for case in cases {
        let fields = match &case.expectation {
            None => vec![case.id.clone(), case.input.clone(), "document".to_owned()],
            Some(e) => vec![
                case.id.clone(),
                case.input.clone(),
                "vefa".to_owned(),
                e.scope.join(" "),
                e.success.join(" "),
                e.error.join(" "),
                e.warning.join(" "),
                e.description.clone(),
            ],
        };
        if let Some(bad) = fields.iter().find(|f| f.contains(['\t', '\n', '\r'])) {
            return Err(Error::new(format!(
                "{}: tab or line break in {bad:?}",
                case.id
            )));
        }
        let _ = writeln!(out, "{}", fields.join("\t"));
    }
    Ok(out)
}

/// Reads `cases.tsv` back.
pub(crate) fn read_index(root: &Path, rule_set: &RuleSet) -> Result<Vec<Case>> {
    let path = format!("{}/{INDEX}", rule_set.corpus_dir());
    let text = util::read_string(&util::join(&rule_set.base(root), &path))?;
    let mut lines = text.lines();
    if lines.next() != Some(INDEX_HEADER) {
        return Err(Error::new(format!("{path}: unexpected header")));
    }
    let ids = |s: &str| -> Vec<String> {
        s.split(' ')
            .filter(|x| !x.is_empty())
            .map(str::to_owned)
            .collect()
    };
    lines
        .enumerate()
        .map(|(index, line)| {
            let fields: Vec<&str> = line.split('\t').collect();
            let expectation = match fields[..] {
                [_, _, "document"] => None,
                [_, _, "vefa", scope, success, error, warning, description] => Some(Expectation {
                    scope: ids(scope),
                    success: ids(success),
                    error: ids(error),
                    warning: ids(warning),
                    description: description.to_owned(),
                }),
                _ => return Err(Error::new(format!("{path}: line {}: malformed", index + 2))),
            };
            Ok(Case {
                id: fields[0].to_owned(),
                input: fields[1].to_owned(),
                expectation,
            })
        })
        .collect()
}

/// Files under the corpus directory that [`derive()`] owns: the index, the
/// licence copy and the VEFA documents (the goldens belong to the oracle).
fn derived_on_disk(root: &Path, rule_set: &RuleSet) -> Result<Vec<String>> {
    let corpus = util::join(&rule_set.base(root), &rule_set.corpus_dir());
    let mut files: Vec<String> = util::list_files(&corpus.join(VEFA_DIR))?
        .into_iter()
        .map(|f| format!("{VEFA_DIR}/{f}"))
        .collect();
    for own in std::iter::once(INDEX).chain(rule_set.licence()) {
        if corpus.join(own).exists() {
            files.push(own.to_owned());
        }
    }
    files.sort();
    Ok(files)
}

/// Compares the derived files with the committed ones. Returns the number of
/// cases.
pub(crate) fn verify(root: &Path, rule_set: &RuleSet) -> Result<usize> {
    let derived = derive(root, rule_set)?;
    let corpus = util::join(&rule_set.base(root), &rule_set.corpus_dir());
    let mut problems = Vec::new();
    for (path, contents) in &derived.files {
        let on_disk = util::join(&corpus, path);
        if !on_disk.exists() {
            problems.push(format!("missing: {path}"));
        } else if util::read(&on_disk)? != *contents {
            problems.push(format!("differs: {path}"));
        }
    }
    for path in derived_on_disk(root, rule_set)? {
        if !derived.files.contains_key(&path) {
            problems.push(format!("stale: {path}"));
        }
    }
    if problems.is_empty() {
        Ok(derived.cases.len())
    } else {
        Err(Error::new(format!(
            "{}: derived files are not up to date (run `cargo xtask corpus --write`):\n  {}",
            rule_set.corpus_dir(),
            problems.join("\n  ")
        )))
    }
}

/// Rewrites the derived files. Returns the number of cases.
pub(crate) fn write(root: &Path, rule_set: &RuleSet) -> Result<usize> {
    let derived = derive(root, rule_set)?;
    let corpus = util::join(&rule_set.base(root), &rule_set.corpus_dir());
    util::remove_dir(&corpus.join(VEFA_DIR))?;
    for (path, contents) in &derived.files {
        util::write(&util::join(&corpus, path), contents)?;
    }
    Ok(derived.cases.len())
}

#[cfg(test)]
mod tests {
    use super::{derive, read_index, verify};
    use crate::ruleset::VENDORED;
    use crate::util::repo_root;

    /// The committed VEFA documents and index are exactly what extraction
    /// produces from the vendored test sets.
    #[test]
    fn derived_files_are_reproducible() {
        for rule_set in VENDORED {
            let count = verify(&repo_root(), rule_set).unwrap_or_else(|e| panic!("{e}"));
            assert!(count > 0, "{}", rule_set.name);
        }
    }

    #[test]
    fn index_round_trips() {
        for rule_set in VENDORED {
            let derived = derive(&repo_root(), rule_set).unwrap();
            let read = read_index(&repo_root(), rule_set).unwrap();
            assert_eq!(derived.cases, read);
        }
    }
}
