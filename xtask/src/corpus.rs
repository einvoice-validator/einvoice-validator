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
//! - `mutations.tsv`: the mutation cases, see [`render_mutation_index`].
//! - `mutations/<mutation id>.xml`: the documents of the mutation catalogue
//!   of the rule set (see [`crate::mutation`]) applied to its base cases.
//! - `<golden dir>/<case id>.svrl.txt`, one golden directory per stylesheet:
//!   the canonical SVRL Saxon-HE produced; for a mutation case on which
//!   Saxon-HE stops with a dynamic error, `<golden dir>/<case id>.error.txt`
//!   instead.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use crate::mutation;
use crate::ruleset::{RuleSet, Stylesheet};
use crate::util::{self, Error, Result};
use crate::vefa::{self, Expectation, ExpectationKind};
use crate::xmute;

/// The index file of a corpus.
pub(crate) const INDEX: &str = "cases.tsv";
/// The index of the mutation cases.
pub(crate) const MUTATION_INDEX: &str = "mutations.tsv";
/// Directory of the documents extracted from VEFA test sets.
pub(crate) const VEFA_DIR: &str = "vefa";
/// Suffix of a golden file, appended to the case id.
pub(crate) const GOLDEN_SUFFIX: &str = ".svrl.txt";
/// Suffix of the golden of the `svrl:metadata` of a SchXslt report.
pub(crate) const METADATA_SUFFIX: &str = ".metadata.txt";
/// Suffix of the golden of a case on which Saxon-HE stops with an error.
pub(crate) const ERROR_SUFFIX: &str = ".error.txt";

const INDEX_HEADER: &str = "case\tinput\tkind\tscope\tsuccess\terror\twarning\tdescription";
const MUTATION_INDEX_HEADER: &str = "case\tinput\tbase\treason";

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
    /// Whether the case is a mutation case (`mutations.tsv`): Saxon-HE may
    /// stop on it with an error, which its golden then records.
    pub(crate) mutation: bool,
}

impl Case {
    /// The golden file of this case for a stylesheet, relative to the base
    /// directory of the rule set.
    pub(crate) fn golden(&self, rule_set: &RuleSet, stylesheet: &Stylesheet) -> String {
        self.golden_with(rule_set, stylesheet, GOLDEN_SUFFIX)
    }

    /// Whether Saxon-HE may stop on the case with a dynamic error, its golden
    /// then being the error: the mutation cases and the XML Mutate cases,
    /// which the oracle runs on one thread with `-t`.
    pub(crate) fn may_stop(&self) -> bool {
        self.mutation
            || self
                .expectation
                .as_ref()
                .is_some_and(|e| e.kind == ExpectationKind::Xmute)
    }

    /// The golden of the `svrl:metadata` of the report of a SchXslt
    /// stylesheet (`crate::svrl::split`).
    pub(crate) fn metadata_golden(&self, rule_set: &RuleSet, stylesheet: &Stylesheet) -> String {
        self.golden_with(rule_set, stylesheet, METADATA_SUFFIX)
    }

    /// The error golden of this case for a stylesheet (mutation cases only).
    pub(crate) fn error_golden(&self, rule_set: &RuleSet, stylesheet: &Stylesheet) -> String {
        self.golden_with(rule_set, stylesheet, ERROR_SUFFIX)
    }

    fn golden_with(&self, rule_set: &RuleSet, stylesheet: &Stylesheet, suffix: &str) -> String {
        format!(
            "{}/{}/{}{suffix}",
            rule_set.corpus_dir(),
            stylesheet.golden_dir,
            self.id
        )
    }
}

/// One case of `mutations.tsv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MutationCase {
    /// `mutations/<mutation id>.xml`.
    pub(crate) id: String,
    /// The document, relative to the base directory of the rule set.
    pub(crate) input: String,
    /// The id of the base case.
    pub(crate) base: String,
    pub(crate) reason: String,
}

impl MutationCase {
    pub(crate) fn case(&self) -> Case {
        Case {
            id: self.id.clone(),
            input: self.input.clone(),
            expectation: None,
            mutation: true,
        }
    }
}

/// The corpus as derived from the vendored files: the cases, and the derived
/// files (path relative to the corpus directory, contents).
#[derive(Debug)]
pub(crate) struct Derived {
    pub(crate) cases: Vec<Case>,
    pub(crate) mutations: Vec<MutationCase>,
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
                mutation: false,
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
                    mutation: false,
                });
            }
        }
    }

    xmute_cases(&base, rule_set, &mut list, &mut cases)?;

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
    let mutations = derive_mutations(root, rule_set, &cases, &mut files)?;
    files.insert(
        MUTATION_INDEX.to_owned(),
        render_mutation_index(&mutations)?.into_bytes(),
    );
    Ok(Derived {
        cases,
        mutations,
        files,
    })
}

/// The documents of the XML Mutate directories, each with the expectation of
/// its active instruction.
fn xmute_cases(
    base: &Path,
    rule_set: &RuleSet,
    list: &mut impl FnMut(&str) -> Result<Vec<String>>,
    cases: &mut Vec<Case>,
) -> Result<()> {
    let source_dir = rule_set.source;
    for (prefix, dir) in rule_set.xmute_dirs {
        let names = list(dir)?;
        if names.is_empty() {
            return Err(Error::new(format!("{source_dir}/{dir}: no documents")));
        }
        for name in names {
            let input = format!("{source_dir}/{dir}/{name}");
            let text = util::read_string(&util::join(base, &input))?;
            let expectation = xmute::expectation(&text, rule_set.xmute_schematrons)
                .map_err(|e| Error::new(format!("{input}: {e}")))?;
            cases.push(Case {
                id: format!("{prefix}/{name}"),
                input,
                expectation: Some(expectation),
                mutation: false,
            });
        }
    }

    Ok(())
}

/// Applies the mutation catalogue to the base cases of the rule set; adds the
/// documents to `files`.
fn derive_mutations(
    root: &Path,
    rule_set: &RuleSet,
    cases: &[Case],
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<Vec<MutationCase>> {
    let base_dir = rule_set.base(root);
    let corpus = rule_set.corpus_dir();
    let mut bases = BTreeMap::new();
    for (name, id) in rule_set.mutation_bases {
        let case = cases
            .iter()
            .find(|c| c.id == *id && c.expectation.is_none())
            .ok_or_else(|| {
                Error::new(format!(
                    "{}: mutation base {name}: no whole-document case {id}",
                    rule_set.name
                ))
            })?;
        let text = util::read_string(&util::join(&base_dir, &case.input))?;
        bases.insert(*name, (*id, text));
    }
    let mut mutations = Vec::new();
    // Every mutation that cannot be applied is reported, not only the first.
    let mut failures = Vec::new();
    let Some(catalogue) = rule_set.mutation_catalogue else {
        return Ok(mutations);
    };
    for m in mutation::catalogue(root, catalogue)? {
        if let Some(only) = &m.only
            && let Some(unknown) = only
                .iter()
                .find(|o| !crate::ruleset::ALL.iter().any(|r| r.name == o.as_str()))
        {
            return Err(Error::new(format!(
                "{catalogue}: {}: unknown rule set {unknown} in `only`",
                m.id
            )));
        }
        let (base_id, base_text) = bases
            .get(m.base.as_str())
            .ok_or_else(|| Error::new(format!("{catalogue}: {}: unknown base {}", m.id, m.base)))?;
        if !m.applies_to(rule_set.name) {
            continue;
        }
        let id = m.case_id();
        let text = match mutation::apply(base_text, &m.edits, &format!("{}: {id}", rule_set.name)) {
            Ok(text) => text,
            Err(e) => {
                failures.push(e.to_string());
                continue;
            }
        };
        files.insert(id.clone(), text.into_bytes());
        mutations.push(MutationCase {
            input: format!("{corpus}/{id}"),
            id,
            base: (*base_id).to_owned(),
            reason: m.reason.clone(),
        });
    }
    if failures.is_empty() {
        Ok(mutations)
    } else {
        Err(Error::new(format!(
            "{} mutations of {catalogue} cannot be applied:\n  {}",
            failures.len(),
            failures.join("\n  ")
        )))
    }
}

/// `mutations.tsv`: a header line, then one line per mutation case with
/// tab-separated columns `case`, `input`, `base` (the id of the base case)
/// and `reason`.
fn render_mutation_index(mutations: &[MutationCase]) -> Result<String> {
    let mut out = String::new();
    out.push_str(MUTATION_INDEX_HEADER);
    out.push('\n');
    for m in mutations {
        let fields = [&m.id, &m.input, &m.base, &m.reason];
        if let Some(bad) = fields.iter().find(|f| f.contains(['\t', '\n', '\r'])) {
            return Err(Error::new(format!(
                "{}: tab or line break in {bad:?}",
                m.id
            )));
        }
        let _ = writeln!(out, "{}\t{}\t{}\t{}", m.id, m.input, m.base, m.reason);
    }
    Ok(out)
}

/// Reads `mutations.tsv` back.
pub(crate) fn read_mutation_index(root: &Path, rule_set: &RuleSet) -> Result<Vec<MutationCase>> {
    let path = format!("{}/{MUTATION_INDEX}", rule_set.corpus_dir());
    let text = util::read_string(&util::join(&rule_set.base(root), &path))?;
    let mut lines = text.lines();
    if lines.next() != Some(MUTATION_INDEX_HEADER) {
        return Err(Error::new(format!("{path}: unexpected header")));
    }
    lines
        .enumerate()
        .map(
            |(index, line)| match line.split('\t').collect::<Vec<_>>()[..] {
                [id, input, base, reason] => Ok(MutationCase {
                    id: id.to_owned(),
                    input: input.to_owned(),
                    base: base.to_owned(),
                    reason: reason.to_owned(),
                }),
                _ => Err(Error::new(format!("{path}: line {}: malformed", index + 2))),
            },
        )
        .collect()
}

/// Every case the oracle runs: those of `cases.tsv`, then the mutation cases.
pub(crate) fn read_all(root: &Path, rule_set: &RuleSet) -> Result<Vec<Case>> {
    let mut cases = read_index(root, rule_set)?;
    cases.extend(
        read_mutation_index(root, rule_set)?
            .iter()
            .map(MutationCase::case),
    );
    Ok(cases)
}

/// `cases.tsv`: a header line, then one line per case with tab-separated
/// columns `case`, `input`, `kind` (`document`, `vefa` or `xmute`), and for
/// VEFA and XML Mutate cases `scope`, `success`, `error`, `warning`
/// (space-separated rule ids; for XML Mutate, `scope` is the stylesheet the
/// ids are about) and `description` (whitespace normalised). Those five are
/// empty for whole documents.
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
                e.kind.as_str().to_owned(),
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
                [
                    _,
                    _,
                    kind @ ("vefa" | "xmute"),
                    scope,
                    success,
                    error,
                    warning,
                    description,
                ] => Some(Expectation {
                    kind: if kind == "vefa" {
                        ExpectationKind::Vefa
                    } else {
                        ExpectationKind::Xmute
                    },
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
                mutation: false,
            })
        })
        .collect()
}

/// Files under the corpus directory that [`derive()`] owns: the indexes, the
/// licence copy, the VEFA and the mutated documents (the goldens belong to the
/// oracle).
fn derived_on_disk(root: &Path, rule_set: &RuleSet) -> Result<Vec<String>> {
    let corpus = util::join(&rule_set.base(root), &rule_set.corpus_dir());
    let mut files = Vec::new();
    for dir in [VEFA_DIR, mutation::DIR] {
        files.extend(
            util::list_files(&corpus.join(dir))?
                .into_iter()
                .map(|f| format!("{dir}/{f}")),
        );
    }
    for own in [INDEX, MUTATION_INDEX]
        .into_iter()
        .chain(rule_set.licence())
    {
        if corpus.join(own).exists() {
            files.push(own.to_owned());
        }
    }
    files.sort();
    Ok(files)
}

/// The number of cases of `cases.tsv` and of `mutations.tsv`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Counts {
    pub(crate) cases: usize,
    pub(crate) mutations: usize,
}

/// Compares the derived files with the committed ones. Returns the number of
/// cases.
pub(crate) fn verify(root: &Path, rule_set: &RuleSet) -> Result<Counts> {
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
        Ok(derived.counts())
    } else {
        Err(Error::new(format!(
            "{}: derived files are not up to date (run `cargo xtask corpus --write`):\n  {}",
            rule_set.corpus_dir(),
            problems.join("\n  ")
        )))
    }
}

/// Rewrites the derived files. Returns the number of cases.
pub(crate) fn write(root: &Path, rule_set: &RuleSet) -> Result<Counts> {
    let derived = derive(root, rule_set)?;
    let corpus = util::join(&rule_set.base(root), &rule_set.corpus_dir());
    util::remove_dir(&corpus.join(VEFA_DIR))?;
    util::remove_dir(&corpus.join(mutation::DIR))?;
    for (path, contents) in &derived.files {
        util::write(&util::join(&corpus, path), contents)?;
    }
    Ok(derived.counts())
}

impl Derived {
    fn counts(&self) -> Counts {
        Counts {
            cases: self.cases.len(),
            mutations: self.mutations.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{derive, read_index, read_mutation_index, verify};
    use crate::ruleset::VENDORED;
    use crate::util::repo_root;

    /// The committed VEFA documents, mutated documents and indexes are exactly
    /// what extraction and the mutation catalogue produce from the vendored
    /// files.
    #[test]
    fn derived_files_are_reproducible() {
        for rule_set in VENDORED {
            let counts = verify(&repo_root(), rule_set).unwrap_or_else(|e| panic!("{e}"));
            assert!(counts.cases > 0, "{}", rule_set.name);
            assert!(counts.mutations > 0, "{}", rule_set.name);
        }
    }

    #[test]
    fn index_round_trips() {
        for rule_set in VENDORED {
            let derived = derive(&repo_root(), rule_set).unwrap();
            let read = read_index(&repo_root(), rule_set).unwrap();
            assert_eq!(derived.cases, read);
            let read = read_mutation_index(&repo_root(), rule_set).unwrap();
            assert_eq!(derived.mutations, read);
        }
    }
}
