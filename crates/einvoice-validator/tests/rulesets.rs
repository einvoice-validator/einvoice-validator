//! The catalogue of the fetched rule sets against the fetch scripts of the repository,
//! their single source: the pins embedded in the crate are the scripts' pin blocks,
//! byte for byte, and the URLs and file names the catalogue builds from them are the
//! ones the scripts build. Then the bundles `RuleSetCache::fetch` assembles are, byte
//! for byte, the ones the scripts wrote in `target/` (offline, from the scripts'
//! downloads), and, in an ignored test, the ones it assembles from the publishers.
//!
//! The scripts' outputs are not committed (`corpus/README.md`): without them the bundle
//! test says SKIPPED and passes, unless `EINVOICE_RULES_REQUIRED` is set, as
//! `scripts/check.sh rules` and CI set it.
//!
//! To update the embedded pins after a script's pins changed:
//! `EINVOICE_UPDATE_PINS=1 cargo test -p einvoice-validator --features fetch --test rulesets`.

#![cfg(test)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use einvoice_validator::{
    CacheStatus, FetchOptions, FetchedRuleSet, MANIFEST_FILE, RuleSet, RuleSetCache,
    fetched_rule_sets,
};

const REQUIRED_VAR: &str = "EINVOICE_RULES_REQUIRED";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

/// The fetch script of each family, and the directory it writes its releases to.
const FAMILIES: [(&str, &str, &str); 4] = [
    ("factur-x", "scripts/fetch-facturx.sh", "target/facturx"),
    (
        "xrechnung",
        "scripts/fetch-xrechnung.sh",
        "target/xrechnung",
    ),
    ("fr-ctc", "scripts/fetch-fr-ctc.sh", "target/fr-ctc"),
    ("peppol", "scripts/fetch-peppol.sh", "target/peppol"),
];

fn script_of(family: &str) -> &'static str {
    FAMILIES.iter().find(|f| f.0 == family).unwrap().1
}

fn output_of(family: &str) -> PathBuf {
    root().join(FAMILIES.iter().find(|f| f.0 == family).unwrap().2)
}

/// The pin block of a script: the lines between its markers.
fn pin_block(script: &str) -> String {
    let text = std::fs::read_to_string(root().join(script)).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.starts_with("# --- pins "))
        .unwrap();
    let end = lines
        .iter()
        .position(|l| l.starts_with("# --- end of pins "))
        .unwrap();
    let mut block = lines[start + 1..end].join("\n");
    block.push('\n');
    block
}

#[test]
fn the_embedded_pins_are_the_scripts_pin_blocks() {
    let update = std::env::var_os("EINVOICE_UPDATE_PINS").is_some_and(|v| !v.is_empty());
    let mut stale = Vec::new();
    for (_, script, _) in FAMILIES {
        let name = script.rsplit('/').next().unwrap().replace(".sh", ".pins");
        let copy = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("pins")
            .join(&name);
        let block = pin_block(script);
        if std::fs::read_to_string(&copy).ok().as_deref() != Some(block.as_str()) {
            if update {
                std::fs::write(&copy, &block).unwrap();
            } else {
                stale.push(name);
            }
        }
    }
    assert!(
        stale.is_empty(),
        "pins/{stale:?} differ from the scripts' pin blocks: run EINVOICE_UPDATE_PINS=1 cargo test -p einvoice-validator --features fetch --test rulesets, then rebuild"
    );
}

/// The variables a script assigns at the start of a line (`name=value`, a value without
/// spaces or quotes), with `$name` and `${name}` expanded from the earlier ones (one
/// the script does not assign, such as `$PWD`, is left as it is).
fn variables(script: &str) -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(root().join(script)).unwrap();
    let mut vars: BTreeMap<String, String> = BTreeMap::new();
    for line in text.lines() {
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            || value.contains([' ', '"', '\'', '(', '`'])
        {
            continue;
        }
        let mut out = String::new();
        let mut rest = value;
        while let Some(at) = rest.find('$') {
            out.push_str(&rest[..at]);
            let after = &rest[at + 1..];
            let (var, tail) = if let Some(braced) = after.strip_prefix('{') {
                let end = braced.find('}').unwrap();
                (&braced[..end], &braced[end + 1..])
            } else {
                let end = after
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(after.len());
                (&after[..end], &after[end..])
            };
            if let Some(v) = vars.get(var) {
                out.push_str(v);
            } else {
                out.push('$');
                out.push_str(var);
            }
            rest = tail;
        }
        out.push_str(rest);
        vars.insert(name.to_owned(), out);
    }
    vars
}

#[test]
fn the_catalogue_downloads_what_the_scripts_download() {
    // For each family, the variables of its script holding the URL and the download
    // path of each source, in the order of the catalogue.
    let expected: [(&str, &[(&str, &str)]); 4] = [
        ("factur-x", &[("mustang_url", "jar")]),
        ("xrechnung", &[("configuration_url", "configuration_zip")]),
        ("fr-ctc", &[("pack_url", "pack_zip")]),
        (
            "peppol",
            &[("archive_url", "archive"), ("isosch_url", "isosch_jar")],
        ),
    ];
    let all = fetched_rule_sets().unwrap();
    for (family, sources) in expected {
        let vars = variables(script_of(family));
        let members: Vec<&FetchedRuleSet> = all.iter().filter(|r| r.family() == family).collect();
        assert!(!members.is_empty(), "{family}");
        for r in &members {
            assert_eq!(r.sources().len(), sources.len(), "{}", r.name());
            for (source, (url, file)) in r.sources().iter().zip(sources) {
                assert_eq!(source.url(), vars[*url], "{}: {url}", r.name());
                let path = &vars[*file];
                assert_eq!(
                    source.file(),
                    path.rsplit('/').next().unwrap(),
                    "{}: {file}",
                    r.name()
                );
                // The digest is the script's pin of that file.
                assert!(
                    vars.values().any(|v| v == source.digest()),
                    "{}: {} not pinned in the script",
                    r.name(),
                    source.digest()
                );
            }
        }
    }
}

#[test]
fn the_catalogue_names_the_bundles_of_the_scripts() {
    // The release directory each script writes (`release=$dir/<name>`), and each
    // bundle's directory under it.
    for r in fetched_rule_sets().unwrap() {
        let vars = variables(script_of(r.family()));
        let release = r.bundle_path();
        let release = release.split('/').next().unwrap();
        let script = std::fs::read_to_string(root().join(script_of(r.family()))).unwrap();
        let release_line = script
            .lines()
            .find(|l| l.starts_with("release=$dir/"))
            .unwrap();
        let template = release_line.strip_prefix("release=$dir/").unwrap();
        let expanded = template
            .split('$')
            .enumerate()
            .fold(String::new(), |mut out, (i, part)| {
                if i == 0 {
                    out.push_str(part);
                } else {
                    let end = part
                        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .unwrap_or(part.len());
                    out.push_str(&vars[&part[..end]]);
                    out.push_str(&part[end..]);
                }
                out
            });
        assert_eq!(release, expanded, "{}", r.name());
        let parsed = RuleSet::parse(r.manifest()).unwrap();
        assert_eq!(parsed.name(), r.name());
    }
}

fn required() -> bool {
    std::env::var_os(REQUIRED_VAR).is_some_and(|v| !v.is_empty())
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "einvoice-rulesets-test-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Compares the bundle of `rule_set` in `cache` with the one the script of its family
/// wrote: the manifest, and every file it names, byte for byte; and the cached bundle
/// holds nothing else. Returns the differences.
fn compare_with_script(cache: &RuleSetCache, rule_set: &FetchedRuleSet) -> Vec<String> {
    let mut problems = Vec::new();
    let ours = cache.bundle_dir(rule_set);
    let mut theirs = output_of(rule_set.family());
    theirs.extend(rule_set.bundle_path().split('/'));
    let parsed = RuleSet::parse(rule_set.manifest()).unwrap();
    let mut files: Vec<String> = vec![MANIFEST_FILE.to_owned()];
    files.extend(parsed.stylesheets().iter().map(|s| s.path().to_owned()));
    files.extend(parsed.resources().iter().map(|r| r.path().to_owned()));
    files.extend(
        parsed
            .schema_documents()
            .iter()
            .map(|d| d.path().to_owned()),
    );
    for file in &files {
        let (mut a, mut b) = (ours.clone(), theirs.clone());
        a.extend(file.split('/'));
        b.extend(file.split('/'));
        match (std::fs::read(&a), std::fs::read(&b)) {
            (Ok(x), Ok(y)) if x == y => {}
            (Ok(_), Ok(_)) => problems.push(format!("{}: {file} differs", rule_set.name())),
            (x, y) => problems.push(format!(
                "{}: {file}: ours {:?}, the script's {:?}",
                rule_set.name(),
                x.err(),
                y.err()
            )),
        }
    }
    // Nothing else in the cached bundle (a release directory holds its bundles).
    let mut found = Vec::new();
    let mut stack = vec![ours.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                // The other bundles of the family, in their own directories.
                let is_bundle = fetched_rule_sets()
                    .unwrap()
                    .iter()
                    .any(|r| cache.bundle_dir(r) == path);
                if !is_bundle {
                    stack.push(path);
                }
            } else {
                let relative = path.strip_prefix(&ours).unwrap();
                found.push(
                    relative
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/"),
                );
            }
        }
    }
    found.sort();
    let mut named = files.clone();
    named.sort();
    if found != named {
        problems.push(format!(
            "{}: the cached bundle holds {found:?}, the manifest names {named:?}",
            rule_set.name()
        ));
    }
    problems
}

/// Whether the script of `family` has written its bundles in `target/`.
fn script_output(family: &str) -> bool {
    fetched_rule_sets()
        .unwrap()
        .iter()
        .filter(|r| r.family() == family)
        .all(|r| {
            let mut dir = output_of(family);
            dir.extend(r.bundle_path().split('/'));
            dir.join(MANIFEST_FILE).is_file()
        })
}

#[test]
fn fetched_bundles_are_the_scripts_bundles_byte_for_byte() {
    // Offline: the downloads of the scripts, and no network (a mirror nobody listens
    // on). Peppol's Schematron is compiled in Rust; its script compiled it with
    // ph-schematron.
    let mut checked = 0;
    for family in ["factur-x", "xrechnung", "fr-ctc", "peppol"] {
        let downloads = if family == "peppol" {
            output_of(family).join("downloads")
        } else {
            root().join("target/downloads")
        };
        let present = script_output(family)
            && fetched_rule_sets()
                .unwrap()
                .iter()
                .filter(|r| r.family() == family)
                .flat_map(FetchedRuleSet::sources)
                .all(|s| downloads.join(s.file()).is_file());
        if !present {
            let message = format!(
                "{family}: no bundles and downloads of {} in target/: run it (scripts/check.sh {})",
                script_of(family),
                family.replace("factur-x", "facturx")
            );
            assert!(!required(), "{message}; {REQUIRED_VAR} is set");
            eprintln!("SKIPPED: {message}");
            continue;
        }
        let cache = RuleSetCache::at(temp(family));
        let options = FetchOptions::new()
            .from_dir(&downloads)
            .mirror("http://127.0.0.1:9")
            .attempts(1);
        let fetched = cache.fetch(family, &options).unwrap();
        assert_eq!(fetched.len(), 1);
        assert!(!fetched[0].already_cached);
        let mut problems = Vec::new();
        for r in fetched_rule_sets()
            .unwrap()
            .iter()
            .filter(|r| r.family() == family)
        {
            assert_eq!(cache.status(r), CacheStatus::Cached, "{}", r.name());
            problems.extend(compare_with_script(&cache, r));
            // Loads, every file checked.
            cache.load_rule_set(r).unwrap();
            checked += 1;
        }
        assert_eq!(problems, Vec::<String>::new());
        // Again: from the cache, nothing fetched.
        let again = cache.fetch(family, &options).unwrap();
        assert!(again[0].already_cached);
        // The downloads were not copied, and nothing is left in progress.
        assert!(!cache.root().join("downloads").exists());
        assert!(!cache.root().join("work").exists());
        cache.clear().unwrap();
        assert!(!cache.root().exists());
    }
    eprintln!("{checked} bundles identical to the scripts' bundles");
}

/// Fetches every rule set from its publisher, for real, and compares each bundle with the
/// one the script of its family wrote in `target/` (which must be there). The cache is
/// `target/rules-cache/` (kept, for the equivalence tests of einvoice-cli), emptied
/// first. `EINVOICE_FETCH_FROM` may name directories (`:`-separated) to take published
/// files from instead of downloading them.
#[test]
#[ignore = "downloads every rule set from its publisher (network)"]
fn fetches_every_rule_set_from_its_publisher() {
    let cache = RuleSetCache::at(root().join("target/rules-cache"));
    cache.clear().unwrap();
    let mut options = FetchOptions::new().progress(|line| eprintln!("fetch: {line}"));
    if let Some(dirs) = std::env::var_os("EINVOICE_FETCH_FROM") {
        for dir in std::env::split_paths(&dirs) {
            options = options.from_dir(dir);
        }
    }
    let mut problems = Vec::new();
    for (family, script, _) in FAMILIES {
        let started = std::time::Instant::now();
        let fetched = cache.fetch(family, &options).unwrap();
        eprintln!(
            "{family}: {} rule sets in {:.1} s",
            fetched[0].rule_sets.len(),
            started.elapsed().as_secs_f64()
        );
        assert!(script_output(family), "{family}: run {script} first");
        for r in fetched_rule_sets()
            .unwrap()
            .iter()
            .filter(|r| r.family() == family)
        {
            assert_eq!(cache.status(r), CacheStatus::Cached, "{}", r.name());
            problems.extend(compare_with_script(&cache, r));
        }
    }
    assert_eq!(problems, Vec::<String>::new());
    eprintln!(
        "{} bundles fetched, identical to the scripts' bundles",
        fetched_rule_sets().unwrap().len()
    );
}
