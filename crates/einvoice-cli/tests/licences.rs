//! The licence expression of the crate names exactly the texts in `LICENSES/`: the
//! repository's MIT licence, and the EUPL-1.2 of the CEN stylesheet that the binary
//! embeds through einvoice-validator's default feature, as published with the artefacts.

#![cfg(test)]

use std::path::{Path, PathBuf};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn licence_expression_names_exactly_the_shipped_texts() {
    let mut declared: Vec<&str> = env!("CARGO_PKG_LICENSE").split(" AND ").collect();
    declared.sort_unstable();
    assert_eq!(declared, ["EUPL-1.2", "MIT"]);
    let mut shipped: Vec<String> = std::fs::read_dir(crate_dir().join("LICENSES"))
        .unwrap()
        .map(|entry| {
            let name = entry.unwrap().file_name().into_string().unwrap();
            name.strip_suffix(".txt").unwrap().to_owned()
        })
        .collect();
    shipped.sort();
    assert_eq!(declared, shipped);
}

#[test]
fn licence_texts_are_the_published_ones() {
    let licences = crate_dir().join("LICENSES");
    assert_eq!(
        read(&licences.join("MIT.txt")),
        read(&crate_dir().join("../../LICENSE"))
    );
    assert_eq!(
        read(&licences.join("EUPL-1.2.txt")),
        read(&crate_dir().join("../../artefacts/cen-en16931-ubl-1.3.17/LICENSE.txt"))
    );
}
