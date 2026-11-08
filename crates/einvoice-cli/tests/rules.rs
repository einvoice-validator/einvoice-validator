//! The rule sets fetched on demand, through the binary: `rules list`, `rules fetch`,
//! `rules path`, `rules clear`, `--rules NAME` from the cache and `--fetch`. Every run
//! has its own cache (`EINVOICE_VALIDATOR_CACHE`) and a mirror on a local server
//! (`EINVOICE_VALIDATOR_MIRROR`): nothing goes to the network.
//!
//! The end-to-end tests serve the files the fetch scripts downloaded into
//! `target/downloads/` (not committed): without them they say SKIPPED and pass, unless
//! `EINVOICE_RULES_REQUIRED` is set, as `scripts/check.sh rules` and CI set it.

#![cfg(test)]

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use common::server::{Server, mirror_path};
use common::{Run, run_env, temp_dir, workspace_root};
use einvoice_validator::fetched_rule_sets;

const UBL: &str = "artefacts/cen-en16931-ubl-1.3.17/ubl/examples/ubl-tc434-example1.xml";
const CII: &str = "artefacts/cen-en16931-cii-1.3.17/cii/examples/CII_example1.xml";
const REQUIRED_VAR: &str = "EINVOICE_RULES_REQUIRED";

/// Runs the binary with the cache `cache` and the mirror `mirror`.
fn run_with(cache: &Path, mirror: &str, args: &[&str]) -> Run {
    run_env(
        args,
        None,
        &[
            ("EINVOICE_VALIDATOR_CACHE", cache.to_str().unwrap()),
            ("EINVOICE_VALIDATOR_MIRROR", mirror),
        ],
    )
}

/// A mirror that answers nothing but 404, and the paths it was asked for.
fn empty_mirror() -> Server {
    Server::start(BTreeMap::new())
}

#[test]
fn rules_list_names_every_rule_set_with_its_source_and_cache_state() {
    let cache = temp_dir("rules-list");
    let mirror = empty_mirror();
    let r = run_with(&cache, &mirror.base, &["rules", "list"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stderr, "");
    assert!(
        r.stdout.starts_with("Built-in rule sets:\n  cen-ubl "),
        "{}",
        r.stdout
    );
    assert!(r.stdout.contains(&cache.display().to_string()));
    for rule_set in fetched_rule_sets().unwrap() {
        let line = r
            .stdout
            .lines()
            .find(|l| l.split_whitespace().next() == Some(rule_set.name()))
            .unwrap_or_else(|| panic!("{} not listed", rule_set.name()));
        let columns: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(columns[1], rule_set.version(), "{line}");
        assert_eq!(columns[2], "no", "{line}");
        assert!(line.ends_with(rule_set.title()), "{line}");
        for source in rule_set.sources() {
            assert!(r.stdout.contains(source.url()), "{}", source.url());
            assert!(r.stdout.contains(source.digest()), "{}", source.url());
        }
    }
    assert!(r.stdout.contains("needs Java"));
    assert!(r.stdout.contains(
        "Aliases: peppol (peppol-bis-billing-ubl), xrechnung (xrechnung-ubl or xrechnung-cii)"
    ));
    // Listing creates nothing, and asks nothing of the network.
    assert!(!cache.join("rules").exists());
    assert_eq!(mirror.requests(), Vec::<String>::new());
}

#[test]
fn rules_path_and_clear() {
    let cache = temp_dir("rules-path");
    let mirror = empty_mirror();
    let r = run_with(&cache, &mirror.base, &["rules", "path"]);
    assert_eq!(
        (r.code, r.stdout.as_str()),
        (0, format!("{}\n", cache.display()).as_str())
    );
    // A rule set not fetched yet: where it would be, and how to fetch it.
    let r = run_with(&cache, &mirror.base, &["rules", "path", "factur-x-en16931"]);
    assert_eq!(r.code, 2);
    assert_eq!(r.stdout, "");
    assert!(
        r.stderr.contains("is not in the cache")
            && r.stderr.contains("rules fetch factur-x-en16931"),
        "{}",
        r.stderr
    );
    let r = run_with(&cache, &mirror.base, &["rules", "path", "cen-ubl"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("cen-ubl is built in"), "{}", r.stderr);
    let r = run_with(&cache, &mirror.base, &["rules", "path", "nope"]);
    assert_eq!(r.code, 2);
    assert!(
        r.stderr.contains("no rule set named \"nope\""),
        "{}",
        r.stderr
    );
    // Clearing a cache that does not exist is fine; one with foreign files keeps them.
    let r = run_with(&cache.join("missing"), &mirror.base, &["rules", "clear"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    std::fs::create_dir_all(cache.join("rules/x")).unwrap();
    std::fs::write(cache.join("mine.txt"), "keep").unwrap();
    let r = run_with(&cache, &mirror.base, &["rules", "clear"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("is empty"), "{}", r.stdout);
    assert!(!cache.join("rules").exists());
    assert!(cache.join("mine.txt").exists());
    std::fs::remove_dir_all(&cache).unwrap();
}

#[test]
fn a_rule_set_not_in_the_cache_is_never_downloaded_without_consent() {
    let cache = temp_dir("rules-consent");
    let mirror = empty_mirror();
    for (name, doc) in [
        ("factur-x-en16931", CII),
        ("xrechnung", UBL),
        ("peppol", UBL),
        ("fr-ctc-ubl-en16931", UBL),
    ] {
        let r = run_with(&cache, &mirror.base, &["--rules", name, doc]);
        assert_eq!(r.code, 2, "{name}: {}", r.stderr);
        assert_eq!(r.stdout, "", "{name}");
        assert!(
            r.stderr.contains("is not in the cache"),
            "{name}: {}",
            r.stderr
        );
        assert!(
            r.stderr.contains("is not redistributed with this program"),
            "{}",
            r.stderr
        );
        assert!(
            r.stderr.contains("einvoice-validate rules fetch "),
            "{}",
            r.stderr
        );
        assert!(r.stderr.contains("--fetch"), "{}", r.stderr);
    }
    assert_eq!(mirror.requests(), Vec::<String>::new());
    assert!(!cache.join("rules").exists());
}

#[test]
fn an_unknown_rule_set_is_a_usage_error_naming_them_all() {
    let cache = temp_dir("rules-unknown");
    let mirror = empty_mirror();
    let r = run_with(&cache, &mirror.base, &["--rules", "bogus", UBL]);
    assert_eq!(r.code, 2);
    assert_eq!(r.stdout, "");
    for name in [
        "no rule set named \"bogus\"",
        "cen-ubl",
        "factur-x-en16931",
        "peppol",
        "rules list",
    ] {
        assert!(r.stderr.contains(name), "{name}: {}", r.stderr);
    }
    let r = run_with(&cache, &mirror.base, &["rules", "fetch", "bogus"]);
    assert_eq!(r.code, 2);
    assert!(
        r.stderr.contains("no fetched rule set named \"bogus\""),
        "{}",
        r.stderr
    );
    // A built-in rule set needs no fetching.
    let r = run_with(&cache, &mirror.base, &["rules", "fetch", "cen-ubl"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "cen-ubl: built in, nothing to fetch\n");
    assert_eq!(mirror.requests(), Vec::<String>::new());
}

#[test]
fn usage_errors_of_the_rules_command() {
    let cache = temp_dir("rules-usage");
    let mirror = empty_mirror();
    for (args, message) in [
        (&["rules"][..], "rules: expected list, fetch, path or clear"),
        (&["rules", "bogus"], "rules: unknown command \"bogus\""),
        (&["rules", "list", "x"], "rules list: unexpected argument x"),
        (
            &["rules", "clear", "x"],
            "rules clear: unexpected argument x",
        ),
        (
            &["rules", "path", "a", "b"],
            "rules path: unexpected argument b",
        ),
        (
            &["rules", "fetch"],
            "rules fetch: name the rule sets to fetch, or --all",
        ),
        (
            &["rules", "fetch", "--all", "peppol"],
            "--all and rule-set names exclude each other",
        ),
        (
            &["rules", "fetch", "--from"],
            "rules fetch: --from needs a value",
        ),
        (
            &["rules", "fetch", "--bogus"],
            "rules fetch: unknown option --bogus",
        ),
        (&["--fetch=yes", UBL], "--fetch takes no value"),
        (&["--fetch", "--fetch", UBL], "--fetch given twice"),
    ] {
        let r = run_with(&cache, &mirror.base, args);
        assert_eq!(r.code, 2, "{args:?}");
        assert_eq!(r.stdout, "", "{args:?}");
        assert!(r.stderr.contains(message), "{args:?}: {}", r.stderr);
    }
    // `rules --help` is the help.
    let r = run_with(&cache, &mirror.base, &["rules", "fetch", "--help"]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("rules fetch NAME..."), "{}", r.stdout);
    // A file named rules is validated as ./rules or after --.
    let r = run_with(&cache, &mirror.base, &["--", "rules"]);
    assert_eq!(r.code, 2);
    assert!(r.stdout.contains("rules: error (io)"), "{}", r.stdout);
}

#[test]
fn a_download_with_another_digest_is_refused_and_nothing_is_cached() {
    let cache = temp_dir("rules-digest");
    // Every source of the catalogue, served with the wrong content.
    let files: BTreeMap<String, Vec<u8>> = fetched_rule_sets()
        .unwrap()
        .iter()
        .flat_map(einvoice_validator::FetchedRuleSet::sources)
        .map(|s| (mirror_path(s.url()), b"not the published file".to_vec()))
        .collect();
    let mirror = Server::start(files);
    let r = run_with(
        &cache,
        &mirror.base,
        &["rules", "fetch", "xrechnung", "factur-x-basic"],
    );
    assert_eq!(r.code, 2, "{}", r.stderr);
    assert!(r.stderr.contains("refused"), "{}", r.stderr);
    assert!(
        r.stderr.contains("not fetched: xrechnung, factur-x-basic"),
        "{}",
        r.stderr
    );
    assert_eq!(mirror.requests().len(), 2, "{:?}", mirror.requests());
    let list = run_with(&cache, &mirror.base, &["rules", "list"]);
    assert!(
        list.stdout
            .lines()
            .filter(|l| l.starts_with("  xrechnung-") || l.starts_with("  factur-x-"))
            .all(|l| l.split_whitespace().nth(2) == Some("no")),
        "{}",
        list.stdout
    );
    // And with --fetch, the validation fails the same way, without a report.
    let r = run_with(
        &cache,
        &mirror.base,
        &["--rules", "xrechnung", "--fetch", UBL],
    );
    assert_eq!(r.code, 2);
    assert_eq!(r.stdout, "");
    assert!(r.stderr.contains("refused"), "{}", r.stderr);
    std::fs::remove_dir_all(&cache).unwrap();
}

/// The files of the families published as compiled XSLT, as the fetch scripts
/// downloaded them into `target/downloads/`, by the path a mirror serves them under;
/// `None` when one is missing.
fn published_files(families: &[&str]) -> Option<BTreeMap<String, Vec<u8>>> {
    let downloads = workspace_root().join("target/downloads");
    let mut files = BTreeMap::new();
    for r in fetched_rule_sets().unwrap() {
        if !families.contains(&r.family()) {
            continue;
        }
        for s in r.sources() {
            let bytes = std::fs::read(downloads.join(s.file())).ok()?;
            files.insert(mirror_path(s.url()), bytes);
        }
    }
    Some(files)
}

fn skip(what: &str) {
    let message = format!(
        "{what}: no downloads of the fetch scripts in target/downloads/ (scripts/check.sh facturx xrechnung)"
    );
    assert!(
        std::env::var_os(REQUIRED_VAR).is_none_or(|v| v.is_empty()),
        "{message}; {REQUIRED_VAR} is set"
    );
    eprintln!("SKIPPED: {message}");
}

#[test]
fn fetch_then_validate_offline_from_the_cache() {
    let Some(files) = published_files(&["xrechnung"]) else {
        skip("fetch_then_validate_offline_from_the_cache");
        return;
    };
    let cache = temp_dir("rules-fetch");
    let mirror = Server::start(files);
    let r = run_with(&cache, &mirror.base, &["rules", "fetch", "xrechnung"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stderr.contains("downloading"), "{}", r.stderr);
    assert!(r.stderr.contains("SHA-256 verified"), "{}", r.stderr);
    assert!(
        r.stdout
            .starts_with("xrechnung: xrechnung-ubl, xrechnung-cii in "),
        "{}",
        r.stdout
    );
    assert_eq!(mirror.requests().len(), 1);
    // Again: already there, nothing downloaded.
    let r = run_with(&cache, &mirror.base, &["rules", "fetch", "xrechnung-cii"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("(already in the cache)"), "{}", r.stdout);
    assert_eq!(mirror.requests().len(), 1);
    drop(mirror);

    // Offline (the mirror is gone): the alias picks the rule set of each document.
    let offline = "http://127.0.0.1:9";
    let r = run_with(
        &cache,
        offline,
        &["--rules", "xrechnung", "--format", "json", UBL, CII],
    );
    assert!(r.code <= 1, "{}", r.stderr);
    assert_eq!(r.stderr, "");
    let report = common::json::parse(&r.stdout).unwrap();
    let names: Vec<&str> = report
        .get("documents")
        .array()
        .iter()
        .map(|d| d.get("rule_set").str().unwrap())
        .collect();
    assert_eq!(names, ["xrechnung-ubl", "xrechnung-cii"]);
    let r = run_with(&cache, offline, &["rules", "list"]);
    let cached: Vec<&str> = r
        .stdout
        .lines()
        .filter(|l| l.split_whitespace().nth(2) == Some("yes"))
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(cached, ["xrechnung-ubl", "xrechnung-cii"]);
    let r = run_with(&cache, offline, &["rules", "path", "xrechnung-ubl"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let dir = PathBuf::from(r.stdout.trim_end());
    assert!(dir.join("bundle.toml").is_file());
    // The bundle is usable with --bundle as it is.
    let r = run_with(&cache, offline, &["--bundle", dir.to_str().unwrap(), UBL]);
    assert!(r.code <= 1, "{}", r.stderr);

    // A cached file changed: refused, every load checks the files.
    let xsl = dir.join("XRechnung-UBL-validation.xsl");
    assert!(xsl.is_file(), "{}", xsl.display());
    let original = std::fs::read(&xsl).unwrap();
    let mut changed = original.clone();
    changed.extend(b"\n<!-- changed -->\n");
    std::fs::write(&xsl, &changed).unwrap();
    let r = run_with(&cache, offline, &["--rules", "xrechnung-ubl", UBL]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("SHA-256"), "{}", r.stderr);
    let r = run_with(&cache, offline, &["rules", "list"]);
    assert!(
        r.stdout
            .contains("xrechnung-ubl: XRechnung-UBL-validation.xsl: not the pinned SHA-256"),
        "{}",
        r.stdout
    );
    std::fs::write(&xsl, &original).unwrap();

    // Cleared: gone.
    let r = run_with(&cache, offline, &["rules", "clear"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let r = run_with(&cache, offline, &["--rules", "xrechnung-ubl", UBL]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("is not in the cache"), "{}", r.stderr);
}

#[test]
fn fetch_option_downloads_in_the_validation_run() {
    let Some(files) = published_files(&["factur-x"]) else {
        skip("fetch_option_downloads_in_the_validation_run");
        return;
    };
    let cache = temp_dir("rules-fetch-option");
    let mirror = Server::start(files);
    let r = run_with(
        &cache,
        &mirror.base,
        &[
            "--rules",
            "factur-x-en16931",
            "--fetch",
            "--format",
            "json",
            CII,
        ],
    );
    assert!(r.code <= 1, "{}", r.stderr);
    assert!(r.stderr.contains("Factur-X"), "{}", r.stderr);
    let report = common::json::parse(&r.stdout).unwrap();
    assert_eq!(
        report.get("rule_set").get("name").str(),
        Some("factur-x-en16931")
    );
    assert_eq!(mirror.requests().len(), 1);
    // Next run: from the cache, nothing downloaded, nothing said.
    let r = run_with(
        &cache,
        &mirror.base,
        &["--rules", "factur-x-basic", "--fetch", CII],
    );
    assert!(r.code <= 1, "{}", r.stderr);
    assert_eq!(r.stderr, "");
    assert_eq!(mirror.requests().len(), 1);
    std::fs::remove_dir_all(&cache).unwrap();
}

#[test]
fn fetch_from_a_local_directory() {
    let downloads = workspace_root().join("target/downloads");
    if published_files(&["fr-ctc"]).is_none() {
        skip("fetch_from_a_local_directory");
        return;
    }
    let cache = temp_dir("rules-from");
    let mirror = empty_mirror();
    let r = run_with(
        &cache,
        &mirror.base,
        &[
            "rules",
            "fetch",
            "fr-ctc-flux10",
            "--from",
            downloads.to_str().unwrap(),
        ],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stderr.contains("taken from"), "{}", r.stderr);
    assert_eq!(mirror.requests(), Vec::<String>::new());
    let count = r.stdout.split(", ").count();
    assert_eq!(count, 14, "{}", r.stdout);
    std::fs::remove_dir_all(&cache).unwrap();
}
