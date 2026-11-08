//! Equivalence with xmllint (libxml2 2.9.14): every case of the oracle (the CEN
//! EN 16931 UBL corpus, `tests/oracle/documents/` and the mutations of
//! `tests/oracle/mutations.tsv`; the synthetic suite of `tests/oracle/synthetic/`) is
//! validated with this crate and compared with what xmllint said (`expected.tsv`,
//! written by the `xmllint-oracle` example). Every difference must be documented,
//! with its cause, in `disagreements.tsv`, and every documented disagreement must
//! still occur. Needs neither xmllint nor libxml2.
//!
//! One more check runs only when `EINVOICE_XSD_EXTRA_SETS` names a manifest of
//! schema sets kept outside the repository (see [`extra_schema_sets_agree_with_xmllint`]):
//! it runs xmllint itself, and is skipped with a notice otherwise.
#![cfg(test)]

#[path = "oracle/format.rs"]
mod format;
#[path = "oracle/xmllint.rs"]
mod xmllint;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn load() -> (
    Vec<format::Case>,
    BTreeMap<String, format::Expected>,
    Vec<format::Disagreement>,
) {
    let cases = format::cases().unwrap();
    let expected_text = format::read(&format::oracle_dir().join("expected.tsv")).unwrap();
    let expected = format::parse_expected(&expected_text).unwrap();
    let documented = format::parse_disagreements(
        &format::read(&format::oracle_dir().join("disagreements.tsv")).unwrap(),
    )
    .unwrap();
    (cases, expected, documented)
}

#[test]
fn agrees_with_xmllint_except_documented_disagreements() {
    let (cases, expected, documented) = load();
    let schema = einvoice_xsd::Schema::ubl_2_1().unwrap();
    let differences = format::differences(&schema, &cases, &expected).unwrap();
    let problems = format::unexplained(&differences, &documented);
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
    let verdicts = differences.iter().filter(|d| d.kind == "verdict").count();
    assert!(
        verdicts * 20 < cases.len(),
        "{verdicts} verdict disagreements out of {} cases",
        cases.len()
    );
}

#[test]
fn every_corpus_verdict_agrees() {
    // The official examples and test documents: no disagreement is tolerated there.
    let (cases, expected, _) = load();
    let schema = einvoice_xsd::Schema::ubl_2_1().unwrap();
    let corpus: Vec<format::Case> = cases
        .into_iter()
        .filter(|c| c.id.starts_with("corpus/"))
        .collect();
    assert!(corpus.len() > 1000, "{} corpus cases", corpus.len());
    for case in &corpus {
        let ours = format::observe(&schema, case);
        assert_eq!(Some(&ours), expected.get(&case.id), "{}", case.id);
    }
}

#[test]
fn xades_disagreements_concern_xades_content() {
    // A disagreement put down to the XAdES schemas not being shipped is on a mutation
    // whose document has XAdES content; none is on a corpus case.
    let (cases, _, documented) = load();
    let xades: Vec<&format::Disagreement> = documented
        .iter()
        .filter(|x| format::caused_by_unshipped_xades(x))
        .collect();
    assert!(xades.len() > 40, "{} XAdES disagreements", xades.len());
    for x in xades {
        assert!(x.case.starts_with("mutation/"), "{}", x.case);
        let case = cases.iter().find(|c| c.id == x.case).unwrap();
        assert!(
            einvoice_xsd::XADES_NAMESPACES
                .iter()
                .any(|ns| case.text.contains(ns)),
            "{}",
            x.case
        );
        assert!(
            x.explanation.len() > format::XADES_NOT_SHIPPED.len() + 20,
            "{}: the explanation must say what XAdES would check",
            x.case
        );
    }
}

/// The synthetic suite: its schema set loaded, its cases, what xmllint said and the
/// documented disagreements.
struct Synthetic {
    schema: einvoice_xsd::Schema,
    cases: Vec<format::Case>,
    schema_cases: Vec<format::SchemaCase>,
    expected: BTreeMap<String, format::Expected>,
    documented: Vec<format::Disagreement>,
}

fn load_synthetic() -> Synthetic {
    let dir = format::synthetic_dir();
    let (cases, schema_cases) = format::synthetic_cases().unwrap();
    let files = format::synthetic_schema_files().unwrap();
    let schema = format::load_set(&files, format::SYNTHETIC_ENTRY_POINT).unwrap();
    let expected =
        format::parse_expected(&format::read(&dir.join("expected.tsv")).unwrap()).unwrap();
    let documented =
        format::parse_disagreements(&format::read(&dir.join("disagreements.tsv")).unwrap())
            .unwrap();
    Synthetic {
        schema,
        cases,
        schema_cases,
        expected,
        documented,
    }
}

#[test]
fn synthetic_suite_agrees_with_xmllint_except_documented_disagreements() {
    let suite = load_synthetic();
    assert!(
        suite.cases.len() > 150,
        "{} document cases",
        suite.cases.len()
    );
    assert!(
        suite.schema_cases.len() > 50,
        "{} schema cases",
        suite.schema_cases.len()
    );
    let differences = format::synthetic_differences(
        &suite.schema,
        &suite.cases,
        &suite.schema_cases,
        &suite.expected,
    )
    .unwrap();
    let problems = format::unexplained(&differences, &suite.documented);
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn synthetic_documents_are_valid() {
    // The unmutated documents are valid for both: every mutation starts from a
    // valid document.
    let suite = load_synthetic();
    for case in suite.cases.iter().filter(|c| c.id.starts_with("document/")) {
        let ours = format::observe(&suite.schema, case);
        assert_eq!(ours.verdict, "valid", "{}: {:?}", case.id, ours.errors);
        assert_eq!(suite.expected.get(&case.id), Some(&ours), "{}", case.id);
    }
}

#[test]
fn synthetic_expected_file_is_canonical() {
    let expected = load_synthetic().expected;
    let text = format::read(&format::synthetic_dir().join("expected.tsv")).unwrap();
    let rewritten = format::format_expected(
        &expected
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Vec<_>>(),
    );
    let mut original: Vec<&str> = text.lines().collect();
    let mut again: Vec<&str> = rewritten.lines().collect();
    original.sort_unstable();
    again.sort_unstable();
    assert_eq!(original, again);
}

#[test]
fn expected_file_is_canonical() {
    let (_, expected, _) = load();
    let text = format::read(&format::oracle_dir().join("expected.tsv")).unwrap();
    let rewritten = format::format_expected(
        &expected
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Vec<_>>(),
    );
    // expected.tsv lists cases in case order, the map in key order: compare sets.
    let mut original: Vec<&str> = text.lines().collect();
    let mut again: Vec<&str> = rewritten.lines().collect();
    original.sort_unstable();
    again.sort_unstable();
    assert_eq!(original, again);
    assert_eq!(format::EXPECTED_HEADER, "case\tverdict\terrors");
}

#[test]
fn mutations_apply_exactly_once() {
    // `format::cases` fails on a find text that occurs zero or several times.
    let cases = format::cases().unwrap();
    let mutations = cases
        .iter()
        .filter(|c| c.id.starts_with("mutation/"))
        .count();
    assert!(mutations > 300, "{mutations} mutations");
    assert_eq!(
        format::apply("a b a", &[("b".to_owned(), "c".to_owned())]).unwrap(),
        "a c a"
    );
    assert!(format::apply("a b a", &[("a".to_owned(), "c".to_owned())]).is_err());
    assert!(format::apply("a b a", &[("x".to_owned(), "c".to_owned())]).is_err());
    let documents = format::workspace_root().join(format::CORPUS);
    assert!(documents.is_dir());
    assert!(
        format::crate_dir()
            .join("examples/xmllint-oracle.rs")
            .is_file()
    );
}

/// The environment variable naming the manifest of the extra schema sets.
const EXTRA_SETS: &str = "EINVOICE_XSD_EXTRA_SETS";

/// One schema set of the manifest.
struct ExtraSet {
    name: String,
    /// The root of the set: every `.xsd` below it is given to this crate.
    schemas: PathBuf,
    /// The entry point, relative to `schemas`.
    entry_point: String,
    /// The directories whose `.xml` files (recursively) are validated, those whose
    /// document element is in the target namespace of the entry point.
    documents: Vec<PathBuf>,
}

/// Parses the manifest: one set per line, tab-separated: name, schema directory,
/// entry point (relative to the schema directory), then one or more document
/// directories; paths relative to the manifest's directory; `#` starts a comment.
fn extra_sets(manifest: &Path) -> Vec<ExtraSet> {
    let base = manifest.parent().unwrap_or(Path::new("."));
    let text = format::read(manifest).unwrap();
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let [name, schemas, entry_point, documents @ ..] = fields.as_slice() else {
                panic!("{}: bad line {line:?}", manifest.display());
            };
            assert!(!documents.is_empty(), "{name}: no document directory");
            ExtraSet {
                name: (*name).to_owned(),
                schemas: base.join(schemas),
                entry_point: (*entry_point).to_owned(),
                documents: documents.iter().map(|d| base.join(d)).collect(),
            }
        })
        .collect()
}

/// The files below `dir` with the given extension, recursively, sorted.
fn files_below(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            files_below(&path, extension, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some(extension) {
            out.push(path);
        }
    }
}

/// The target namespace of a schema document.
fn target_namespace(text: &str) -> Option<String> {
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..roxmltree::ParsingOptions::default()
    };
    let doc = roxmltree::Document::parse_with_options(text, options).ok()?;
    doc.root_element()
        .attribute("targetNamespace")
        .map(str::to_owned)
}

/// The namespace of the document element of an XML document, if it parses.
fn root_namespace(text: &str) -> Option<String> {
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..roxmltree::ParsingOptions::default()
    };
    let doc = roxmltree::Document::parse_with_options(text, options).ok()?;
    doc.root_element().tag_name().namespace().map(str::to_owned)
}

/// The schema documents of a set: every `.xsd` below its directory, as a path
/// relative to it (`/`-separated) and its text.
fn set_files(set: &ExtraSet) -> Vec<(String, String)> {
    let mut paths = Vec::new();
    files_below(&set.schemas, "xsd", &mut paths);
    paths
        .iter()
        .map(|path| {
            let relative: Vec<&str> = path
                .strip_prefix(&set.schemas)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_str().unwrap())
                .collect();
            (relative.join("/"), format::read(path).unwrap())
        })
        .collect()
}

/// The documents of a set: every `.xml` below its document directories whose
/// document element is in `namespace`, without a byte order mark.
fn set_documents(set: &ExtraSet, namespace: Option<&str>) -> Vec<format::Case> {
    let mut cases = Vec::new();
    for (index, dir) in set.documents.iter().enumerate() {
        let mut documents = Vec::new();
        files_below(dir, "xml", &mut documents);
        for path in documents {
            let text = format::read(&path).unwrap();
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned();
            if root_namespace(&text).as_deref() != namespace {
                continue;
            }
            let relative = path.strip_prefix(dir).unwrap().display();
            cases.push(format::Case {
                id: format!("{index}/{relative}"),
                text,
            });
        }
    }
    cases
}

/// Checks one set (see [`extra_schema_sets_agree_with_xmllint`]): prints its
/// counts and returns its problems.
fn check_extra_set(program: &str, set: &ExtraSet) -> Vec<String> {
    let files = set_files(set);
    let entry = set.schemas.join(&set.entry_point);
    let started = std::time::Instant::now();
    let schema = match format::load_set(&files, &set.entry_point) {
        Ok(schema) => schema,
        Err(e) => return vec![format!("{}: does not load: {e}", set.name)],
    };
    let load_time = started.elapsed();
    let work = format::workspace_root()
        .join("target/xsd-oracle/extra")
        .join(&set.name);
    if !xmllint::compiles(program, &entry, &work).unwrap() {
        return vec![format!("{}: xmllint does not compile it", set.name)];
    }
    let namespace = files
        .iter()
        .find(|(path, _)| *path == set.entry_point)
        .and_then(|(_, text)| target_namespace(text));
    let cases = set_documents(set, namespace.as_deref());
    if cases.is_empty() {
        return vec![format!("{}: no document in its namespace", set.name)];
    }
    let observed = xmllint::run(program, &cases, &entry, &work).unwrap();
    let started = std::time::Instant::now();
    let ours: Vec<format::Expected> = cases
        .iter()
        .map(|case| format::observe(&schema, case))
        .collect();
    let validate_time = started.elapsed();
    let mut problems = Vec::new();
    let (mut same_verdict, mut identical, mut valid, mut errors) = (0, 0, 0, 0);
    for ((case, ours), (_, xmllint)) in cases.iter().zip(&ours).zip(&observed) {
        errors += xmllint.errors.len();
        valid += usize::from(ours.verdict == "valid");
        if ours.verdict == xmllint.verdict {
            same_verdict += 1;
            if ours.errors == xmllint.errors {
                identical += 1;
                continue;
            }
        }
        problems.push(format!(
            "{}: {}: this crate {} {:?}, xmllint {} {:?}",
            set.name, case.id, ours.verdict, ours.errors, xmllint.verdict, xmllint.errors
        ));
    }
    eprintln!(
        "{}: {} schema documents ({} loaded, {} types, loaded in {load_time:.2?}); {} documents ({valid} valid), \
         {same_verdict} same verdict as xmllint, {identical} identical including the {errors} errors \
         of xmllint (validated in {validate_time:.2?})",
        set.name,
        files.len(),
        schema.documents().len(),
        schema.type_count(),
        cases.len(),
    );
    problems
}

/// Schema sets kept outside the repository (such as the CII-family schemas:
/// UN/CEFACT CII D16B and D22B, the Factur-X profiles, which are not part of it),
/// each with documents to validate: when `EINVOICE_XSD_EXTRA_SETS` names a
/// manifest of them ([`extra_sets`]), every set must load with this crate and
/// compile with xmllint, and every document whose document element is in the
/// target namespace of the set's entry point is validated by both, which must
/// agree on the verdict and the errors (line and category). Skipped, with a notice,
/// without the variable or without xmllint.
#[test]
fn extra_schema_sets_agree_with_xmllint() {
    let Some(manifest) = std::env::var_os(EXTRA_SETS) else {
        eprintln!(
            "SKIPPED: {EXTRA_SETS} is not set (a manifest of schema sets outside the repository)"
        );
        return;
    };
    let program = xmllint::program();
    let version = match xmllint::version(&program) {
        Ok(version) => version,
        Err(e) => {
            eprintln!("SKIPPED: extra schema sets: {e}");
            return;
        }
    };
    eprintln!("{version}");
    let sets = extra_sets(Path::new(&manifest));
    assert!(!sets.is_empty(), "no schema set in the manifest");
    let problems: Vec<String> = sets
        .iter()
        .flat_map(|set| check_extra_set(&program, set))
        .collect();
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}
