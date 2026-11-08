//! The files vendored in the crate: the CEN stylesheet of each built-in rule set
//! (`bundles/cen-ubl/`, `bundles/cen-cii/`) matches its `SHA256SUMS`, which lists exactly
//! the vendored files, is byte-identical to the copy in `artefacts/`, and is what the
//! built-in rule set embeds;
//! the licence expression of the crate names exactly the texts in `LICENSES/`, which are
//! the repository's MIT licence, the EUPL-1.2 text published with the artefacts and the
//! terms of the UBL 2.1 schemas of einvoice-xsd, and `licences()` lists every term of
//! the workspace crates this build is built from (`cargo tree`, with the features of the
//! build), its MIT entry naming exactly those crates.

#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use einvoice_testkit::workspace::{self, Crate, Features};
use sha2::{Digest, Sha256};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Each vendored bundle: its name, the vendored release it comes from (in `artefacts/`),
/// and the files it vendors.
const BUNDLES: &[(&str, &str, &str)] = &[
    (
        "cen-ubl",
        "cen-en16931-ubl-1.3.17",
        "ubl/xslt/EN16931-UBL-validation.xslt",
    ),
    (
        "cen-cii",
        "cen-en16931-cii-1.3.17",
        "cii/xslt/EN16931-CII-validation.xslt",
    ),
];

fn bundle_dir(name: &str) -> PathBuf {
    crate_dir().join("bundles").join(name)
}

/// The vendored release, in the repository.
fn artefacts_dir(release: &str) -> PathBuf {
    crate_dir().join("../../artefacts").join(release)
}

/// The UBL release, whose licence text the crate ships.
fn ubl_artefacts_dir() -> PathBuf {
    artefacts_dir("cen-en16931-ubl-1.3.17")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut out = String::new();
    for byte in Sha256::digest(bytes) {
        write!(out, "{byte:02x}").unwrap();
    }
    out
}

/// A `SHA256SUMS` file as path -> digest.
fn sums(path: &Path) -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(path).unwrap();
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let (digest, file) = line
            .split_once("  ")
            .unwrap_or_else(|| panic!("bad line {line:?}"));
        assert_eq!(digest.len(), 64, "{line}");
        assert!(
            map.insert(file.to_owned(), digest.to_owned()).is_none(),
            "duplicate {file}"
        );
    }
    map
}

fn files_under(dir: &Path, root: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files_under(&path, root, out);
        } else {
            out.push(
                path.strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('\\', "/"),
            );
        }
    }
}

#[test]
fn vendored_files_match_their_checksums() {
    for (name, _, file) in BUNDLES {
        let sums = sums(&bundle_dir(name).join("SHA256SUMS"));
        assert_eq!(sums.keys().collect::<Vec<_>>(), [file], "{name}");
        for (path, digest) in &sums {
            let bytes = std::fs::read(bundle_dir(name).join(path)).unwrap();
            assert_eq!(&sha256_hex(&bytes), digest, "{name}: {path}");
        }
    }
}

#[test]
fn checksums_list_exactly_the_vendored_files() {
    let mut bundles = Vec::new();
    for entry in std::fs::read_dir(crate_dir().join("bundles")).unwrap() {
        bundles.push(entry.unwrap().file_name().into_string().unwrap());
    }
    bundles.sort();
    let mut known: Vec<&str> = BUNDLES.iter().map(|(name, _, _)| *name).collect();
    known.sort_unstable();
    assert_eq!(bundles, known);
    for (name, _, _) in BUNDLES {
        let root = bundle_dir(name);
        let mut found = Vec::new();
        files_under(&root, &root, &mut found);
        found.retain(|p| !["README.md", "SHA256SUMS", "bundle.toml"].contains(&p.as_str()));
        found.sort();
        let listed: Vec<String> = sums(&root.join("SHA256SUMS")).into_keys().collect();
        assert_eq!(found, listed, "{name}");
    }
}

#[test]
fn vendored_files_are_the_artefacts() {
    for (name, release, _) in BUNDLES {
        let artefacts = sums(&artefacts_dir(release).join("SHA256SUMS"));
        for (path, digest) in sums(&bundle_dir(name).join("SHA256SUMS")) {
            assert_eq!(artefacts.get(&path), Some(&digest), "{name}: {path}");
            assert_eq!(
                std::fs::read(bundle_dir(name).join(&path)).unwrap(),
                std::fs::read(artefacts_dir(release).join(&path)).unwrap(),
                "{name}: {path}"
            );
        }
    }
}

/// Loading checks each embedded file against the manifest's SHA-256, which must be the
/// vendored file's; and the manifest is the vendored one.
#[cfg(any(feature = "cen-ubl", feature = "cen-cii"))]
fn check_embedded(name: &str) {
    let validator = einvoice_validator::Validator::builtin(name).unwrap();
    let manifest = std::fs::read_to_string(bundle_dir(name).join("bundle.toml")).unwrap();
    assert_eq!(
        validator.rule_set(),
        &einvoice_validator::RuleSet::parse(&manifest).unwrap()
    );
    let sums = sums(&bundle_dir(name).join("SHA256SUMS"));
    for entry in validator.rule_set().stylesheets() {
        assert_eq!(sums.get(entry.path()), Some(&entry.sha256().to_owned()));
    }
}

#[cfg(feature = "cen-ubl")]
#[test]
fn the_builtin_rule_set_embeds_the_vendored_files() {
    check_embedded("cen-ubl");
}

#[cfg(feature = "cen-cii")]
#[test]
fn the_builtin_cii_rule_set_embeds_the_vendored_files() {
    check_embedded("cen-cii");
    // The UN/CEFACT CII D16B schemas, embedded in einvoice-xsd.
    assert_eq!(
        einvoice_validator::RuleSet::builtin("cen-cii")
            .unwrap()
            .schema(),
        Some(einvoice_validator::SchemaId::CiiD16b)
    );
}

fn licences_dir() -> PathBuf {
    crate_dir().join("LICENSES")
}

/// The terms of the licence expression, in the order of `licences()` with the default
/// features.
const TERMS: [&str; 5] = [
    "MIT",
    "EUPL-1.2",
    "LicenseRef-OASIS-Copyright-Notice",
    "LicenseRef-UN-CEFACT-Copyright-Notice",
    "W3C-19980720",
];

/// The features of this build of the crate.
fn features() -> Vec<&'static str> {
    let mut features = Vec::new();
    if cfg!(feature = "cen-ubl") {
        features.push("cen-ubl");
    }
    if cfg!(feature = "cen-cii") {
        features.push("cen-cii");
    }
    if cfg!(feature = "pdf") {
        features.push("pdf");
    }
    features
}

/// The workspace crates this build of the crate is built from, itself included.
fn built_from() -> Vec<Crate> {
    let features = features();
    workspace::dependencies(env!("CARGO_PKG_NAME"), Features::Only(&features)).unwrap()
}

#[test]
fn licence_expression_names_exactly_the_shipped_texts() {
    let declared: BTreeSet<&str> = env!("CARGO_PKG_LICENSE").split(" AND ").collect();
    assert_eq!(declared, BTreeSet::from(TERMS));
    let mut shipped = Vec::new();
    files_under(&licences_dir(), &licences_dir(), &mut shipped);
    let shipped: BTreeSet<String> = shipped
        .into_iter()
        .map(|file| file.strip_suffix(".txt").unwrap().to_owned())
        .collect();
    assert_eq!(
        declared,
        shipped.iter().map(String::as_str).collect::<BTreeSet<_>>()
    );
    // With the default features: the EUPL-1.2 of its own stylesheets, and every term of
    // the crates it is built from.
    let defaults = workspace::dependencies(env!("CARGO_PKG_NAME"), Features::Default).unwrap();
    let mut terms = BTreeSet::from(["EUPL-1.2"]);
    for c in &defaults {
        terms.extend(c.license_terms().unwrap());
    }
    assert_eq!(terms, declared);
}

#[test]
fn licence_texts_are_the_published_ones() {
    let read = |path: PathBuf| std::fs::read(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    assert_eq!(
        read(licences_dir().join("MIT.txt")),
        read(crate_dir().join("../../LICENSE"))
    );
    assert_eq!(
        read(licences_dir().join("EUPL-1.2.txt")),
        read(ubl_artefacts_dir().join("LICENSE.txt"))
    );
    // The CII release publishes the same text.
    assert_eq!(
        read(licences_dir().join("EUPL-1.2.txt")),
        read(artefacts_dir("cen-en16931-cii-1.3.17").join("LICENSE.txt"))
    );
    // The terms of the UBL 2.1 schemas: einvoice-xsd's texts.
    for id in &TERMS[2..] {
        assert_eq!(
            read(licences_dir().join(format!("{id}.txt"))),
            read(crate_dir().join(format!("../einvoice-xsd/LICENSES/{id}.txt"))),
            "{id}"
        );
    }
}

#[test]
fn embedded_licences_are_the_shipped_texts() {
    let licences = einvoice_validator::licences();
    let ids: Vec<&str> = licences.iter().map(|l| l.id).collect();
    let expected: Vec<&str> = TERMS
        .into_iter()
        .filter(|id| *id != "EUPL-1.2" || cfg!(any(feature = "cen-ubl", feature = "cen-cii")))
        .collect();
    assert_eq!(ids, expected);
    let text =
        |path: PathBuf| std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    for licence in &licences {
        assert!(!licence.covers.is_empty(), "{}", licence.id);
        assert_eq!(
            licence.text,
            text(licences_dir().join(format!("{}.txt", licence.id))),
            "{}",
            licence.id
        );
    }
    // The EUPL-1.2 entry names every built-in rule set whose stylesheet it covers.
    if let Some(eupl) = licences.iter().find(|l| l.id == "EUPL-1.2") {
        for name in einvoice_validator::BUILTIN_RULE_SETS {
            assert!(eupl.covers.contains(name), "{name}: {}", eupl.covers);
        }
    }
}

#[test]
fn embedded_licences_cover_exactly_the_crates_built_from() {
    let licences = einvoice_validator::licences();
    let ids: BTreeSet<&str> = licences.iter().map(|l| l.id).collect();
    let crates = built_from();
    // Every term of every crate this build is built from is listed (this crate's own
    // expression is that of its default features, checked above).
    for c in crates.iter().filter(|c| c.name != env!("CARGO_PKG_NAME")) {
        for term in c.license_terms().unwrap() {
            assert!(
                ids.contains(term),
                "{}: {term} is not in licences()",
                c.name
            );
        }
    }
    // The MIT entry stands for the code of this crate and of exactly the crates of the
    // project it is built from: each is named and ships that same text.
    let mit = &licences[0];
    assert!(
        mit.covers
            .starts_with("the code of einvoice-validator and of the crates of the project "),
        "{}",
        mit.covers
    );
    let others: Vec<&str> = crates
        .iter()
        .map(|c| c.name.as_str())
        .filter(|name| *name != env!("CARGO_PKG_NAME"))
        .collect();
    assert_eq!(
        workspace::named_crates(mit.covers).unwrap(),
        others,
        "{}",
        mit.covers
    );
    for c in &crates {
        assert_eq!(
            mit.text,
            std::fs::read_to_string(c.dir.join("LICENSES/MIT.txt")).unwrap(),
            "{}",
            c.name
        );
    }
}
