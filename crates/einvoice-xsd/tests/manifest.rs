//! The vendored OASIS UBL 2.1 and UN/CEFACT CII D16B schemas match the `SHA256SUMS` of
//! their directories (`schemas/oasis-ubl-2.1/`, `schemas/uncefact-cii-d16b/`), each list
//! has no file too many or too few, and the embedded copies are these files. The
//! licence expression of the crate names exactly the texts in `LICENSES/`, and each
//! vendored file carries the terms its licence term stands for.
#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The embedded files of a set: path and text.
type Files = &'static [(&'static str, &'static str)];

/// The vendored directories: name under `schemas/`, embedded files, number of files.
const SETS: [(&str, Files, usize); 2] = [
    ("oasis-ubl-2.1", einvoice_xsd::UBL_2_1_FILES, 13),
    ("uncefact-cii-d16b", einvoice_xsd::CII_D16B_FILES, 4),
];

fn schemas_dir(set: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("schemas")
        .join(set)
}

/// Every embedded schema document of every set: set, path, text.
fn embedded() -> impl Iterator<Item = (&'static str, &'static str, &'static str)> {
    SETS.into_iter()
        .flat_map(|(set, files, _)| files.iter().map(move |(path, text)| (set, *path, *text)))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut out = String::new();
    for byte in Sha256::digest(bytes) {
        write!(out, "{byte:02x}").unwrap();
    }
    out
}

/// `SHA256SUMS` of a set as path -> digest.
fn manifest(set: &str) -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(schemas_dir(set).join("SHA256SUMS")).unwrap();
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let (digest, path) = line
            .split_once("  ")
            .unwrap_or_else(|| panic!("bad line {line:?}"));
        assert_eq!(digest.len(), 64, "{line}");
        assert!(
            map.insert(path.to_owned(), digest.to_owned()).is_none(),
            "duplicate {path}"
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
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            out.push(relative);
        }
    }
}

#[test]
fn vendored_files_match_their_checksums() {
    for (set, _, count) in SETS {
        let manifest = manifest(set);
        assert_eq!(manifest.len(), count, "{set}");
        for (path, digest) in &manifest {
            let bytes = std::fs::read(schemas_dir(set).join(path)).unwrap();
            assert_eq!(&sha256_hex(&bytes), digest, "{set}/{path}");
        }
    }
}

#[test]
fn manifest_lists_exactly_the_vendored_files() {
    let mut directories = Vec::new();
    for entry in
        std::fs::read_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas")).unwrap()
    {
        directories.push(entry.unwrap().file_name().into_string().unwrap());
    }
    directories.sort();
    assert_eq!(directories, SETS.map(|(set, _, _)| set));
    for (set, _, _) in SETS {
        let root = schemas_dir(set);
        let mut found = Vec::new();
        files_under(&root, &root, &mut found);
        found.retain(|p| p != "README.md" && p != "SHA256SUMS");
        found.sort();
        let listed: Vec<String> = manifest(set).into_keys().collect();
        assert_eq!(found, listed, "{set}");
    }
}

#[test]
fn embedded_files_are_the_vendored_files() {
    for (set, files, _) in SETS {
        let listed: Vec<String> = manifest(set).into_keys().collect();
        let mut embedded: Vec<String> = files.iter().map(|(p, _)| (*p).to_owned()).collect();
        embedded.sort();
        assert_eq!(embedded, listed, "{set}");
        for (path, text) in files {
            let bytes = std::fs::read(schemas_dir(set).join(path)).unwrap();
            assert_eq!(text.as_bytes(), bytes.as_slice(), "{set}/{path}");
        }
    }
}

#[test]
fn cii_schemas_are_the_unece_originals() {
    // UNECE publishes these files with CRLF line ends: a checkout that converted them
    // to LF (without the `-text` rule of .gitattributes) would not hold the published
    // bytes. The checksums catch that; this says why.
    for (path, text) in einvoice_xsd::CII_D16B_FILES {
        let lines = text.matches('\n').count();
        assert!(lines > 20, "{path}");
        assert_eq!(
            text.matches("\r\n").count(),
            lines,
            "{path}: not CRLF throughout"
        );
        // The closure of the entry point is the vendored set: no code-list module is
        // imported.
        for location in text.split("schemaLocation=\"").skip(1) {
            let target = location.split('"').next().unwrap();
            let target = format!("uncefact/data/standard/{target}");
            assert!(
                einvoice_xsd::CII_D16B_FILES
                    .iter()
                    .any(|(p, _)| *p == target),
                "{path}: imports {target}"
            );
        }
    }
    assert_eq!(
        einvoice_xsd::CII_D16B_FILES[0].0,
        einvoice_xsd::CII_D16B_ENTRY_POINT
    );
}

#[test]
fn xades_schemas_are_not_vendored() {
    // No licence allows the ETSI XAdES schemas to be redistributed: they must not be
    // in the vendored directory, and the only vendored document that names their
    // namespaces is the one importing them.
    let root = schemas_dir("oasis-ubl-2.1");
    let mut found = Vec::new();
    files_under(&root, &root, &mut found);
    assert!(
        found
            .iter()
            .all(|p| !p.to_ascii_lowercase().contains("xades")),
        "{found:?}"
    );
    for (path, text) in einvoice_xsd::UBL_2_1_FILES {
        let mentions = text.matches("http://uri.etsi.org/01903/").count();
        if *path == "xsd/common/UBL-SignatureAggregateComponents-2.1.xsd" {
            assert_eq!(mentions, 2, "{path}");
            for namespace in einvoice_xsd::XADES_NAMESPACES {
                assert!(
                    text.contains(&format!("<xsd:import namespace=\"{namespace}\"")),
                    "{path}: {namespace}"
                );
            }
        } else {
            assert_eq!(mentions, 0, "{path}");
        }
    }
}

#[test]
fn checksum_of_known_input() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

fn licences_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("LICENSES")
}

fn licence_text(id: &str) -> String {
    std::fs::read_to_string(licences_dir().join(format!("{id}.txt"))).unwrap()
}

/// The terms of the licence expression of `Cargo.toml`, sorted.
fn declared_licences() -> Vec<&'static str> {
    let mut declared: Vec<&str> = env!("CARGO_PKG_LICENSE").split(" AND ").collect();
    declared.sort_unstable();
    declared
}

/// Runs of whitespace collapsed to one space, ends trimmed.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The term of the licence expression that covers each vendored file.
fn licence_of(set: &str, path: &str) -> &'static str {
    match (set, path) {
        ("uncefact-cii-d16b", _)
        | ("oasis-ubl-2.1", "xsd/common/CCTS_CCT_SchemaModule-2.1.xsd") => {
            "LicenseRef-UN-CEFACT-Copyright-Notice"
        }
        ("oasis-ubl-2.1", "xsd/common/UBL-xmldsig-core-schema-2.1.xsd") => "W3C-19980720",
        ("oasis-ubl-2.1", _) => "LicenseRef-OASIS-Copyright-Notice",
        _ => panic!("{set}/{path}: unknown set"),
    }
}

/// The year of the UN/CEFACT copyright of the files of a set under that notice.
fn un_cefact_year(set: &str) -> &'static str {
    if set == "uncefact-cii-d16b" {
        "2016"
    } else {
        "2006"
    }
}

#[test]
fn licence_expression_names_exactly_the_shipped_texts() {
    let mut shipped = Vec::new();
    files_under(&licences_dir(), &licences_dir(), &mut shipped);
    let mut shipped: Vec<String> = shipped
        .into_iter()
        .map(|file| file.strip_suffix(".txt").unwrap().to_owned())
        .collect();
    shipped.sort();
    assert_eq!(declared_licences(), shipped);
}

#[test]
fn mit_text_is_the_repository_licence() {
    let repository =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../LICENSE"))
            .unwrap();
    assert_eq!(licence_text("MIT"), repository);
}

#[test]
fn every_vendored_file_carries_the_terms_of_its_licence() {
    let mut used = std::collections::BTreeSet::new();
    for (set, path, text) in embedded() {
        let licence = licence_of(set, path);
        used.insert(licence);
        let text = collapse(text);
        match licence {
            "LicenseRef-OASIS-Copyright-Notice" => {
                // Each paragraph of the licence text is in the file, word for word: the
                // copyright line at the top and the notice.
                for paragraph in licence_text(licence).split("\n\n") {
                    assert!(text.contains(&collapse(paragraph)), "{path}: {paragraph}");
                }
            }
            "LicenseRef-UN-CEFACT-Copyright-Notice" => {
                // The licence text: the copyright line of each year, then the notice.
                // The file has the copyright line of its year, then the notice, word
                // for word.
                let licence_text = licence_text(licence);
                let (copyrights, notice) = licence_text.split_once("\n\n").unwrap();
                let year = un_cefact_year(set);
                let copyright = format!("Copyright (C) UN/CEFACT ({year}). All Rights Reserved.");
                assert!(
                    copyrights.lines().any(|line| line == copyright),
                    "{copyright}"
                );
                assert!(
                    text.contains(&format!("{copyright} {}", collapse(notice))),
                    "{set}/{path}: no UN/CEFACT {year} notice"
                );
            }
            "W3C-19980720" => assert!(
                text.contains("This document is governed by the W3C Software License")
                    && text
                        .contains("http://www.w3.org/Consortium/Legal/copyright-software-19980720"),
                "{path}"
            ),
            other => panic!("{path}: no check for {other}"),
        }
    }
    let mut declared = declared_licences();
    declared.retain(|id| *id != "MIT");
    assert_eq!(used.into_iter().collect::<Vec<_>>(), declared);
}

#[test]
fn embedded_licences_are_the_shipped_texts() {
    // One entry per term of the licence expression, in the order of the expression,
    // each with the text of LICENSES/<term>.txt.
    let ids: Vec<&str> = einvoice_xsd::LICENCES.iter().map(|l| l.id).collect();
    assert_eq!(
        ids,
        env!("CARGO_PKG_LICENSE").split(" AND ").collect::<Vec<_>>()
    );
    for licence in einvoice_xsd::LICENCES {
        assert_eq!(licence.text, licence_text(licence.id), "{}", licence.id);
        assert!(!licence.covers.is_empty(), "{}", licence.id);
    }
    // Each schema term names the files it covers, as licence_of() assigns them.
    let covers = |id: &str| {
        einvoice_xsd::LICENCES
            .iter()
            .find(|l| l.id == id)
            .unwrap()
            .covers
    };
    for (set, path, _) in embedded() {
        let licence = licence_of(set, path);
        let file = path.rsplit('/').next().unwrap();
        if licence == "LicenseRef-OASIS-Copyright-Notice" {
            assert!(file.starts_with("UBL-"), "{path}");
        } else {
            assert!(
                covers(licence).contains(path),
                "{path}: {}",
                covers(licence)
            );
        }
    }
    assert!(
        covers("LicenseRef-OASIS-Copyright-Notice")
            .contains("except UBL-xmldsig-core-schema-2.1.xsd")
    );
}
