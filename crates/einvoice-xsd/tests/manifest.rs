//! The vendored OASIS UBL 2.1 schemas match `schemas/oasis-ubl-2.1/SHA256SUMS`, the
//! list has no file too many or too few, and the embedded copies are these files.
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
    assert_eq!(manifest.len(), 15);
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
fn checksum_of_known_input() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
