//! The fetched rule sets used as a plain install uses them: `einvoice-validate --rules
//! NAME` with the bundles `rules fetch` put in the cache. On every case and mutation case
//! of the corpus of each family (Factur-X, XRechnung, the French rules, Peppol BIS
//! Billing), the findings of each stylesheet of each bundle (kind, id, flag, location, in
//! order), or the error that stopped it, agree with the Saxon-HE goldens: the results of
//! the equivalence tests of einvoice-xslt-exec, through the cache. And the verdict is the
//! reference validator's: invalid with schema errors, or when a stylesheet of the bundle
//! has a finding that the reference validator of the rule set reads as an error, as the
//! acceptance table of the corpus says (`acceptance.tsv`, which `cargo xtask <family>`
//! writes from the goldens: an unflagged failed assert is an error, and for XRechnung the
//! scenario of the KoSIT configuration the document matches overrides the levels of the
//! rule ids it lists).
//!
//! The XRechnung bundles select by scenario, as the KoSIT validator runs its
//! configuration: the scenario each document matches (by the `match` expressions, which
//! the bundles evaluate with the engine and xtask on its own) must be the table's, the
//! stylesheets that ran those of the scenario, compared with their goldens, and the
//! verdict the configuration's (the table's `verdict`): a document that matches no
//! scenario, or several, is invalid and nothing runs on it.
//!
//! The corpora and their goldens live in `target/` (built by `scripts/check.sh facturx
//! xrechnung fr-ctc peppol`), the cache in `target/rules-cache/`. A family missing from
//! the cache is fetched into it from the downloads of the fetch scripts (`rules fetch
//! --from target/downloads`; Peppol compiles its Schematron, which needs Java and Maven
//! Central). Without a corpus, or when a family cannot be fetched, its bundles say
//! SKIPPED and pass, unless `EINVOICE_RULES_REQUIRED` is set, as `scripts/check.sh rules`
//! and CI set it.

#![cfg(test)]

mod common;

use std::collections::HashMap;
use std::path::PathBuf;

use common::{Golden, MutationCase, check_document, golden_of, join, run_env, workspace_root};
use einvoice_validator::{FetchedRuleSet, RuleSet, fetched_rule_set, fetched_rule_sets};

const REQUIRED_VAR: &str = "EINVOICE_RULES_REQUIRED";
/// The maximum depth of the run again of a document deeper than the default (as the
/// equivalence tests of einvoice-xslt-exec).
const RAISED_MAX_DEPTH: &str = "1024";

/// Each bundle: its name, its corpus directory (in its family's release directory), and
/// its stylesheets with their golden directories (in the corpus directory).
type Bundle = (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);

const BUNDLES: &[Bundle] = &[
    (
        "factur-x-minimum",
        "corpus",
        &[("FACTUR-X_MINIMUM", "golden-minimum")],
    ),
    (
        "factur-x-basic-wl",
        "corpus",
        &[("FACTUR-X_BASIC-WL", "golden-basic-wl")],
    ),
    (
        "factur-x-basic",
        "corpus",
        &[("FACTUR-X_BASIC", "golden-basic")],
    ),
    (
        "factur-x-en16931",
        "corpus",
        &[("FACTUR-X_EN16931", "golden-en16931")],
    ),
    (
        "factur-x-extended",
        "corpus",
        &[("FACTUR-X_EXTENDED", "golden-extended")],
    ),
    (
        "xrechnung-ubl",
        "corpus-ubl",
        &[
            ("EN16931-UBL", "golden-cen"),
            ("XRechnung-UBL", "golden-xrechnung"),
        ],
    ),
    (
        "xrechnung-cii",
        "corpus-cii",
        &[
            ("EN16931-CII", "golden-cen"),
            ("XRechnung-CII", "golden-xrechnung"),
        ],
    ),
    (
        "fr-ctc-cii-en16931",
        "corpus-cii",
        &[
            ("EN16931-CII", "golden-en16931"),
            ("BR-FR-Flux2-CII", "golden-br-fr"),
        ],
    ),
    (
        "fr-ctc-cii-en16931-warning",
        "corpus-cii",
        &[
            ("EN16931-CII", "golden-en16931"),
            ("BR-FR-Flux2-CII_WARNING", "golden-br-fr-warning"),
        ],
    ),
    (
        "fr-ctc-cii-extended-ctc-fr",
        "corpus-cii",
        &[
            ("EXTENDED-CTC-FR-CII", "golden-extended-ctc-fr"),
            ("BR-FR-Flux2-CII", "golden-br-fr"),
        ],
    ),
    (
        "fr-ctc-cii-extended-ctc-fr-warning",
        "corpus-cii",
        &[
            ("EXTENDED-CTC-FR-CII", "golden-extended-ctc-fr"),
            ("BR-FR-Flux2-CII_WARNING", "golden-br-fr-warning"),
        ],
    ),
    (
        "fr-ctc-ubl-en16931",
        "corpus-ubl",
        &[
            ("EN16931-UBL", "golden-en16931"),
            ("BR-FR-Flux2-UBL", "golden-br-fr"),
        ],
    ),
    (
        "fr-ctc-ubl-en16931-warning",
        "corpus-ubl",
        &[
            ("EN16931-UBL", "golden-en16931"),
            ("BR-FR-Flux2-UBL_WARNING", "golden-br-fr-warning"),
        ],
    ),
    (
        "fr-ctc-ubl-extended-ctc-fr",
        "corpus-ubl",
        &[
            ("EXTENDED-CTC-FR-UBL", "golden-extended-ctc-fr"),
            ("BR-FR-Flux2-UBL", "golden-br-fr"),
        ],
    ),
    (
        "fr-ctc-ubl-extended-ctc-fr-warning",
        "corpus-ubl",
        &[
            ("EXTENDED-CTC-FR-UBL", "golden-extended-ctc-fr"),
            ("BR-FR-Flux2-UBL_WARNING", "golden-br-fr-warning"),
        ],
    ),
    (
        "fr-ctc-cdar",
        "corpus-cdar",
        &[("BR-FR-CDV-CDAR", "golden")],
    ),
    (
        "fr-ctc-cdar-warning",
        "corpus-cdar",
        &[("BR-FR-CDV-CDAR_WARNING", "golden-warning")],
    ),
    (
        "fr-ctc-factur-x-basic-wl",
        "corpus-cii",
        &[
            ("FACTUR-X_BASIC-WL", "golden-factur-x-basic-wl"),
            ("BR-FR-Flux2-CII", "golden-br-fr"),
        ],
    ),
    (
        "fr-ctc-factur-x-en16931",
        "corpus-cii",
        &[
            ("FACTUR-X_EN16931", "golden-factur-x-en16931"),
            ("BR-FR-Flux2-CII", "golden-br-fr"),
        ],
    ),
    (
        "fr-ctc-factur-x-extended",
        "corpus-cii",
        &[
            ("FACTUR-X_EXTENDED", "golden-factur-x-extended"),
            ("BR-FR-Flux2-CII", "golden-br-fr"),
        ],
    ),
    (
        "peppol-bis-billing-ubl",
        "corpus",
        &[
            ("CEN-EN16931-UBL", "golden-cen"),
            ("PEPPOL-EN16931-UBL", "golden-peppol"),
        ],
    ),
];

/// The rule set without a corpus: the e-reporting rules (Flux 10), whose stylesheet no
/// corpus of the repository runs.
const WITHOUT_CORPUS: &[&str] = &["fr-ctc-flux10"];

/// The directory a family's fetch script writes its releases to.
fn output_of(family: &str) -> &'static str {
    match family {
        "factur-x" => "target/facturx",
        "xrechnung" => "target/xrechnung",
        "fr-ctc" => "target/fr-ctc",
        _ => "target/peppol",
    }
}

/// The release directory of the rule set's family as its script wrote it, relative to
/// the workspace root.
fn release_of(rule_set: &FetchedRuleSet) -> String {
    let path = rule_set.bundle_path();
    format!(
        "{}/{}",
        output_of(rule_set.family()),
        path.split('/').next().unwrap()
    )
}

fn cache() -> PathBuf {
    workspace_root().join("target/rules-cache")
}

fn env(cache: &str) -> [(&str, &str); 2] {
    // No mirror is reachable: a family missing from the downloads is not fetched.
    [
        ("EINVOICE_VALIDATOR_CACHE", cache),
        ("EINVOICE_VALIDATOR_MIRROR", "http://127.0.0.1:9"),
    ]
}

fn skip(message: &str) {
    assert!(
        std::env::var_os(REQUIRED_VAR).is_none_or(|v| v.is_empty()),
        "{message}; {REQUIRED_VAR} is set"
    );
    eprintln!("SKIPPED: {message}");
}

/// Makes sure the family of `rule_set` is in the cache: fetched from the downloads of
/// the fetch scripts when it is not. Whether it is.
fn ensure_cached(rule_set: &FetchedRuleSet) -> bool {
    let cache = cache();
    let cache_text = cache.to_str().unwrap().to_owned();
    let r = run_env(&["rules", "path", rule_set.name()], None, &env(&cache_text));
    if r.code == 0 {
        return true;
    }
    let root = workspace_root();
    let downloads = root.join("target/downloads");
    let peppol = root.join("target/peppol/downloads");
    // Maven downloads the Schematron compiler from Maven Central: the real one.
    let r = run_env(
        &[
            "rules",
            "fetch",
            rule_set.family(),
            "--from",
            downloads.to_str().unwrap(),
            "--from",
            peppol.to_str().unwrap(),
        ],
        None,
        &[("EINVOICE_VALIDATOR_CACHE", cache_text.as_str())],
    );
    if r.code != 0 {
        skip(&format!(
            "{}: cannot be fetched into {}: {}",
            rule_set.family(),
            cache.display(),
            r.stderr.trim()
        ));
        return false;
    }
    true
}

/// The cases and mutation cases of a corpus, inputs relative to the workspace root;
/// `None` without a complete corpus (its stamp).
fn corpus_cases(release: &str, corpus: &str) -> Option<Vec<MutationCase>> {
    let root = workspace_root();
    let dir = join(&root, &format!("{release}/{corpus}"));
    if !dir.join("complete.txt").is_file() {
        return None;
    }
    let tsv = std::fs::read_to_string(dir.join("cases.tsv")).unwrap();
    let mut cases: Vec<MutationCase> = tsv
        .lines()
        .skip(1)
        .map(|line| {
            let mut columns = line.split('\t');
            let id = columns.next().unwrap().to_owned();
            let input = format!("{release}/{}", columns.next().unwrap());
            MutationCase { id, input }
        })
        .collect();
    if dir.join("mutations.tsv").is_file() {
        cases.extend(common::mutation_cases(
            &format!("{release}/{corpus}"),
            release,
        ));
    }
    Some(cases)
}

/// The acceptance table of a corpus (`cargo xtask <family>`): for each case, its
/// `scenario`, the verdict of each stylesheet, by golden directory (`accept`, `reject` or
/// `error`), and the `verdict` of the configuration for the XRechnung corpora.
type Acceptance = HashMap<String, HashMap<String, String>>;

/// The acceptance table of a corpus, `None` when there is none.
fn acceptance_of(release: &str, corpus: &str) -> Option<Acceptance> {
    let path = join(
        &workspace_root(),
        &format!("{release}/{corpus}/acceptance.tsv"),
    );
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next()?.split('\t').collect();
    assert_eq!(header[..2], ["case", "scenario"]);
    Some(
        lines
            .map(|line| {
                let columns: Vec<&str> = line.split('\t').collect();
                assert_eq!(columns.len(), header.len(), "{line}");
                let verdicts = header[1..]
                    .iter()
                    .zip(&columns[1..])
                    .map(|(dir, verdict)| ((*dir).to_owned(), (*verdict).to_owned()))
                    .collect();
                (columns[0].to_owned(), verdicts)
            })
            .collect(),
    )
}

#[test]
fn every_fetched_rule_set_but_flux10_has_a_corpus_here() {
    let all = fetched_rule_sets().unwrap();
    for r in all {
        let listed = BUNDLES.iter().any(|(name, ..)| *name == r.name());
        assert_eq!(listed, !WITHOUT_CORPUS.contains(&r.name()), "{}", r.name());
    }
    assert_eq!(BUNDLES.len() + WITHOUT_CORPUS.len(), all.len());
}

#[test]
fn fetched_rule_sets_from_the_cache_agree_with_the_saxon_goldens() {
    let cache = cache();
    let cache_text = cache.to_str().unwrap().to_owned();
    let jobs = std::thread::available_parallelism()
        .map_or(4, std::num::NonZero::get)
        .to_string();
    let mut problems = Vec::new();
    let mut checked = 0;
    for (name, corpus, stylesheets) in BUNDLES {
        let rule_set = fetched_rule_set(name, None).unwrap().unwrap();
        let release = release_of(rule_set);
        let Some(cases) = corpus_cases(&release, corpus) else {
            skip(&format!(
                "{name}: no corpus in {release}/{corpus}: build it with scripts/check.sh {}",
                rule_set.family().replace("factur-x", "facturx")
            ));
            continue;
        };
        let Some(acceptance) = acceptance_of(&release, corpus) else {
            skip(&format!(
                "{name}: no acceptance table in {release}/{corpus}: build it with scripts/check.sh {}",
                rule_set.family().replace("factur-x", "facturx")
            ));
            continue;
        };
        if !ensure_cached(rule_set) {
            continue;
        }
        let (documents, deep) = run_documents(name, &cases, &cache_text, &jobs);
        let goldens: Vec<(&str, String)> = stylesheets
            .iter()
            .map(|(s, dir)| (*s, format!("{release}/{corpus}/{dir}")))
            .collect();
        let goldens: Vec<(&str, &str)> = goldens.iter().map(|(s, d)| (*s, d.as_str())).collect();
        let manifest = RuleSet::parse(rule_set.manifest()).unwrap();
        let (found, summary) = if manifest.selects_by_scenario() {
            check_selected(
                name,
                &manifest,
                stylesheets,
                &goldens,
                &cases,
                &documents,
                &acceptance,
            )
        } else {
            check_documents(name, stylesheets, &goldens, &cases, &documents, &acceptance)
        };
        eprintln!(
            "{name}: {} documents ({summary}; {deep} run again with --max-depth {RAISED_MAX_DEPTH}), {} disagreements",
            cases.len(),
            found.len()
        );
        for p in found.iter().take(10) {
            eprintln!("DISAGREE {p}");
        }
        problems.extend(found);
        checked += 1;
    }
    eprintln!("{checked} of {} bundles checked", BUNDLES.len());
    assert_eq!(problems, Vec::<String>::new());
}

/// The JSON documents `einvoice-validate --rules NAME` gives the cases, in order, and the
/// number run again with the maximum depth raised.
fn run_documents(
    name: &str,
    cases: &[MutationCase],
    cache: &str,
    jobs: &str,
) -> (Vec<common::json::Value>, usize) {
    let inputs: Vec<&str> = cases.iter().map(|c| c.input.as_str()).collect();
    let runs = common::run_json_batches_env(
        &["--rules", name, "--format", "json", "--jobs", jobs],
        &inputs,
        &env(cache),
    );
    assert_eq!(runs.stderr(), "", "{name}");
    assert_eq!(runs.first().get("rule_set").get("name").str(), Some(name));
    let mut documents: Vec<common::json::Value> = runs.documents().into_iter().cloned().collect();
    assert_eq!(documents.len(), cases.len(), "{name}");
    // A document deeper than the default maximum depth is refused, with the limit,
    // rather than risking the stack: run again with the maximum raised, as the
    // equivalence tests of the engine do.
    let deep: Vec<usize> = documents
        .iter()
        .enumerate()
        .filter(|(_, d)| {
            d.get("status").str() == Some("error")
                && d.get("error")
                    .get("message")
                    .str()
                    .is_some_and(|m| m.starts_with("limit exceeded"))
        })
        .map(|(i, _)| i)
        .collect();
    if !deep.is_empty() {
        let inputs: Vec<&str> = deep.iter().map(|&i| cases[i].input.as_str()).collect();
        let raised = common::run_json_batches_env(
            &[
                "--rules",
                name,
                "--format",
                "json",
                "--max-depth",
                RAISED_MAX_DEPTH,
            ],
            &inputs,
            &env(cache),
        );
        assert_eq!(raised.stderr(), "", "{name}");
        for (&i, document) in deep.iter().zip(raised.documents()) {
            documents[i] = document.clone();
        }
    }
    (documents, deep.len())
}

/// Compares the documents of a bundle with the goldens of its stylesheets and with the
/// acceptance table of the corpus. The problems, and a summary: the statuses, and the
/// verdicts that the reference levels change (documents without schema errors that no
/// stylesheet stopped on, invalid by the levels and valid by the fatal flags alone, or
/// the reverse).
fn check_documents(
    name: &str,
    stylesheets: &[(&str, &str)],
    goldens: &[(&str, &str)],
    cases: &[MutationCase],
    documents: &[common::json::Value],
    acceptance: &Acceptance,
) -> (Vec<String>, String) {
    let mut found = Vec::new();
    let (mut valid, mut invalid, mut errors) = (0, 0, 0);
    let (mut raised, mut lowered) = (0, 0);
    for (case, document) in cases.iter().zip(documents) {
        assert_eq!(document.get("path").str(), Some(case.input.as_str()));
        assert_eq!(document.get("rule_set").str(), Some(name));
        let Some(verdicts) = acceptance.get(&case.id) else {
            found.push(format!("{}: not in the acceptance table", case.id));
            continue;
        };
        let verdict = |dir: &str| verdicts.get(dir).map_or("", String::as_str);
        let rejecting: Vec<bool> = stylesheets
            .iter()
            .map(|(_, dir)| verdict(dir) == "reject")
            .collect();
        found.extend(check_document(
            &case.id,
            document,
            goldens,
            Some(&rejecting),
        ));
        // The fatal flags alone, as before the levels of the manifests.
        let mut stopped = false;
        let mut flagged = false;
        for ((_, dir), (_, golden_dir)) in stylesheets.iter().zip(goldens) {
            match golden_of(golden_dir, &case.id) {
                Golden::Error(_) => {
                    stopped = true;
                    if verdict(dir) != "error" {
                        found.push(format!("{}: {dir}: stopped, not in the table", case.id));
                    }
                }
                Golden::Findings(f) => {
                    flagged |= f.iter().any(|f| f.2.as_deref() == Some("fatal"));
                    if verdict(dir) == "error" {
                        found.push(format!(
                            "{}: {dir}: not stopped, error in the table",
                            case.id
                        ));
                    }
                }
            }
        }
        if !stopped && document.get("counts").get("schema_errors").number() == 0 {
            let reference = rejecting.contains(&true);
            raised += usize::from(reference && !flagged);
            lowered += usize::from(!reference && flagged);
        }
        match document.get("status").str() {
            Some("valid") => valid += 1,
            Some("invalid") => invalid += 1,
            _ => errors += 1,
        }
    }
    let summary = format!(
        "{valid} valid, {invalid} invalid, {errors} stopped by an error; the reference levels make {raised} invalid and {lowered} valid that the fatal flags alone would not"
    );
    (found, summary)
}

/// [`check_documents`] for a bundle that selects by scenario (XRechnung): for each case,
/// the scenario of the document must be the table's (`-` and `several: ...`: none), the
/// stylesheets that ran those of that scenario, in its order, their findings those of
/// their goldens, the status the one their levels give, and the verdict that of the
/// configuration (the table's `verdict`, with the schema errors: invalid with any). A
/// document without a scenario is invalid, without findings, schema errors or stylesheet
/// errors. The problems, and a summary.
fn check_selected(
    name: &str,
    manifest: &RuleSet,
    stylesheets: &[(&str, &str)],
    goldens: &[(&str, &str)],
    cases: &[MutationCase],
    documents: &[common::json::Value],
    acceptance: &Acceptance,
) -> (Vec<String>, String) {
    let mut found = Vec::new();
    let (mut valid, mut invalid, mut errors, mut unmatched) = (0, 0, 0, 0);
    for (case, document) in cases.iter().zip(documents) {
        assert_eq!(document.get("path").str(), Some(case.input.as_str()));
        assert_eq!(document.get("rule_set").str(), Some(name));
        let Some(table) = acceptance.get(&case.id) else {
            found.push(format!("{}: not in the acceptance table", case.id));
            continue;
        };
        match document.get("status").str() {
            Some("valid") => valid += 1,
            Some("invalid") => invalid += 1,
            _ => errors += 1,
        }
        // Stopped by a stylesheet error, there is no report: the scenario is the table's.
        let stopped = document.get("status").str() == Some("error");
        if document.get("scenario").str().is_none() && !stopped {
            unmatched += 1;
            found.extend(check_unmatched(&case.id, document, table));
        } else {
            found.extend(check_matched(
                &case.id,
                document,
                table,
                manifest,
                stylesheets,
                goldens,
            ));
        }
    }
    let summary = format!(
        "{valid} valid, {invalid} invalid ({unmatched} without a scenario), {errors} stopped by an error"
    );
    (found, summary)
}

/// The column `key` of a case in the acceptance table.
fn column<'t>(table: &'t HashMap<String, String>, key: &str) -> &'t str {
    table.get(key).map_or("", String::as_str)
}

/// The stylesheets that ran on a document.
fn stylesheets_run(document: &common::json::Value) -> Vec<&str> {
    document
        .get("stylesheets_run")
        .array()
        .iter()
        .map(|s| s.str().unwrap())
        .collect()
}

/// A document without a scenario: the table's must be none (`-`), or the same several;
/// the document invalid, without findings, schema errors, stylesheet errors or
/// stylesheets that ran; the configuration's verdict `reject`.
fn check_unmatched(
    id: &str,
    document: &common::json::Value,
    table: &HashMap<String, String>,
) -> Vec<String> {
    let mut found = Vec::new();
    let expected = column(table, "scenario");
    let no_scenario = document.get("no_scenario");
    if *no_scenario == common::json::Value::Null {
        return vec![format!("{id}: no scenario and no no_scenario")];
    }
    let matching: Vec<&str> = no_scenario
        .get("matching")
        .array()
        .iter()
        .map(|m| m.str().unwrap())
        .collect();
    let table_matching: Vec<&str> = match expected.strip_prefix("several: ") {
        Some(names) => names.split(" | ").collect(),
        None if expected == "-" => Vec::new(),
        None => return vec![format!("{id}: no scenario, the table's is {expected:?}")],
    };
    if matching != table_matching {
        found.push(format!(
            "{id}: scenarios {matching:?}, the table's {table_matching:?}"
        ));
    }
    let status = document.get("status").str().unwrap_or("");
    let empty = document.get("findings").array().is_empty()
        && document.get("stylesheet_errors").array().is_empty()
        && document.get("counts").get("schema_errors").number() == 0
        && stylesheets_run(document).is_empty();
    if status != "invalid" || !empty || column(table, "verdict") != "reject" {
        found.push(format!(
            "{id}: without a scenario: {status}, empty {empty}, the table's verdict {:?}",
            column(table, "verdict")
        ));
    }
    found
}

/// A document with a scenario: the table's; the stylesheets that ran those of the
/// scenario, in its order, their findings those of their goldens and the status the one
/// their levels give ([`check_document`]); the status the configuration's verdict (the
/// table's `verdict`), invalid with schema errors. A document a stylesheet stopped on
/// (status `error`) has no report: its scenario is the table's, and [`check_document`]
/// checks that the first of its stylesheets with an error golden is the one that stopped,
/// with that error.
fn check_matched(
    id: &str,
    document: &common::json::Value,
    table: &HashMap<String, String>,
    manifest: &RuleSet,
    stylesheets: &[(&str, &str)],
    goldens: &[(&str, &str)],
) -> Vec<String> {
    let expected = column(table, "scenario");
    // Stopped by a stylesheet error, there is no report: the scenario is the table's,
    // which check_document confirms with the stylesheet that stopped.
    let stopped = document.get("status").str() == Some("error");
    let scenario = if stopped {
        expected
    } else {
        document.get("scenario").str().unwrap_or("")
    };
    if scenario != expected || *document.get("no_scenario") != common::json::Value::Null {
        return vec![format!(
            "{id}: scenario {scenario:?}, the table's {expected:?}"
        )];
    }
    let Some(selected) = manifest.scenarios().iter().find(|s| s.name() == scenario) else {
        return vec![format!("{id}: no scenario {scenario:?} in the bundle")];
    };
    let names: Vec<&str> = selected.stylesheets().map_or_else(
        || stylesheets.iter().map(|(s, _)| *s).collect(),
        |names| names.iter().map(String::as_str).collect(),
    );
    let ran = stylesheets_run(document);
    if ran != names && !stopped {
        return vec![format!("{id}: ran {ran:?}, the scenario runs {names:?}")];
    }
    // The findings and the status, from the goldens of the stylesheets that ran.
    let run_goldens: Vec<(&str, &str)> = names
        .iter()
        .filter_map(|n| goldens.iter().find(|(s, _)| s == n).copied())
        .collect();
    let rejecting: Vec<bool> = names
        .iter()
        .map(|n| {
            stylesheets
                .iter()
                .find(|(s, _)| s == n)
                .is_some_and(|(_, dir)| column(table, dir) == "reject")
        })
        .collect();
    let mut found = check_document(id, document, &run_goldens, Some(&rejecting));
    // The verdict of the configuration.
    let status = document.get("status").str().unwrap_or("");
    let schema_errors = document.get("counts").get("schema_errors").number();
    let verdict = column(table, "verdict");
    let expected_status = match (verdict, schema_errors) {
        (_, 1..) | ("reject", 0) => "invalid",
        ("accept", 0) => "valid",
        ("error", 0) => "error",
        (other, _) => return vec![format!("{id}: table verdict {other:?}")],
    };
    if status != expected_status {
        found.push(format!(
            "{id}: {status}, the configuration's verdict {verdict:?} ({schema_errors} schema errors)"
        ));
    }
    found
}
