//! The vendored OASIS UBL 2.1 schemas match `schemas/oasis-ubl-2.1/SHA256SUMS`, the
//! list has no file too many or too few, and the embedded copies are these files. The
//! licence expression of the crate names exactly the texts in `LICENSES/`, and each
//! vendored file carries the terms its licence term stands for.
#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

fn schemas_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas/oasis-ubl-2.1")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut out = String::new();
    for byte in Sha256::digest(bytes) {
        write!(out, "{byte:02x}").unwrap();
    }
    out
}

/// `SHA256SUMS` as path -> digest.
fn manifest() -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(schemas_dir().join("SHA256SUMS")).unwrap();
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
    let manifest = manifest();
    assert_eq!(manifest.len(), 13);
    for (path, digest) in &manifest {
        let bytes = std::fs::read(schemas_dir().join(path)).unwrap();
        assert_eq!(&sha256_hex(&bytes), digest, "{path}");
    }
}

#[test]
fn manifest_lists_exactly_the_vendored_files() {
    let root = schemas_dir();
    let mut found = Vec::new();
    files_under(&root, &root, &mut found);
    found.retain(|p| p != "README.md" && p != "SHA256SUMS");
    found.sort();
    let listed: Vec<String> = manifest().into_keys().collect();
    assert_eq!(found, listed);
}

#[test]
fn embedded_files_are_the_vendored_files() {
    let listed: Vec<String> = manifest().into_keys().collect();
    let mut embedded: Vec<String> = einvoice_xsd::UBL_2_1_FILES
        .iter()
        .map(|(p, _)| (*p).to_owned())
        .collect();
    embedded.sort();
    assert_eq!(embedded, listed);
    for (path, text) in einvoice_xsd::UBL_2_1_FILES {
        let bytes = std::fs::read(schemas_dir().join(path)).unwrap();
        assert_eq!(text.as_bytes(), bytes.as_slice(), "{path}");
    }
}

#[test]
fn xades_schemas_are_not_vendored() {
    // No licence allows the ETSI XAdES schemas to be redistributed: they must not be
    // in the vendored directory, and the only vendored document that names their
    // namespaces is the one importing them.
    let root = schemas_dir();
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
fn licence_of(path: &str) -> &'static str {
    match path {
        "xsd/common/CCTS_CCT_SchemaModule-2.1.xsd" => "LicenseRef-UN-CEFACT-Copyright-Notice",
        "xsd/common/UBL-xmldsig-core-schema-2.1.xsd" => "W3C-19980720",
        _ => "LicenseRef-OASIS-Copyright-Notice",
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
    for (path, text) in einvoice_xsd::UBL_2_1_FILES {
        let licence = licence_of(path);
        used.insert(licence);
        let text = collapse(text);
        match licence {
            "LicenseRef-OASIS-Copyright-Notice" | "LicenseRef-UN-CEFACT-Copyright-Notice" => {
                // Each paragraph of the licence text is in the file, word for word: the
                // copyright line at the top and the notice.
                for paragraph in licence_text(licence).split("\n\n") {
                    assert!(text.contains(&collapse(paragraph)), "{path}: {paragraph}");
                }
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
    for (path, _) in einvoice_xsd::UBL_2_1_FILES {
        let licence = licence_of(path);
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
