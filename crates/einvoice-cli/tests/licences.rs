//! The licence expression of the crate names exactly the texts in `LICENSES/`: the
//! repository's MIT licence, and the EUPL-1.2 of the CEN stylesheet that the binary
//! embeds through einvoice-validator's default feature, as published with the artefacts.
//! `einvoice-validate --licences` prints the full text of every licence of what the
//! binary embeds: these two and those of the UBL 2.1 schemas of einvoice-xsd.

#![cfg(test)]

mod common;

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

#[test]
fn licences_option_prints_every_embedded_licence_in_full() {
    let licences = crate_dir().join("LICENSES");
    let xsd = crate_dir().join("../einvoice-xsd/LICENSES");
    let text = |path: PathBuf| String::from_utf8(read(&path)).unwrap();
    // The licence, in the order printed, and its full text.
    let expected = [
        ("MIT", text(licences.join("MIT.txt"))),
        ("EUPL-1.2", text(licences.join("EUPL-1.2.txt"))),
        (
            "LicenseRef-OASIS-Copyright-Notice",
            text(xsd.join("LicenseRef-OASIS-Copyright-Notice.txt")),
        ),
        (
            "LicenseRef-UN-CEFACT-Copyright-Notice",
            text(xsd.join("LicenseRef-UN-CEFACT-Copyright-Notice.txt")),
        ),
        ("W3C-19980720", text(xsd.join("W3C-19980720.txt"))),
    ];
    let mut outputs = Vec::new();
    for flag in ["--licences", "--licenses"] {
        let r = common::run(&[flag], None);
        assert_eq!(r.code, 0, "{flag}");
        assert_eq!(r.stderr, "", "{flag}");
        assert!(
            r.stdout.starts_with(&format!(
                "einvoice-validate {}: the licences of the works embedded in this program.\n",
                common::VERSION
            )),
            "{flag}: {}",
            r.stdout
        );
        // Each section: its heading, the line saying what it covers, then the text.
        let mut rest = r.stdout.as_str();
        for (i, (id, body)) in expected.iter().enumerate() {
            let heading = format!("\n== {}/{} {id}\nCovers: ", i + 1, expected.len());
            let at = rest
                .find(&heading)
                .unwrap_or_else(|| panic!("{flag}: no section {heading:?}"));
            rest = &rest[at + heading.len()..];
            let (covers, after) = rest.split_once(".\n\n").unwrap();
            assert!(
                !covers.is_empty() && !covers.contains('\n'),
                "{id}: {covers}"
            );
            assert!(
                after.starts_with(body.as_str()),
                "{flag}: {id}: text differs"
            );
            rest = &after[body.len()..];
        }
        assert_eq!(rest, "", "{flag}: trailing output");
        outputs.push(r.stdout);
    }
    assert_eq!(outputs[0], outputs[1]);
    // The MIT section names every crate of the project the binary is built from.
    let mit = outputs[0]
        .split_once("\n== 1/5 MIT\nCovers: ")
        .unwrap()
        .1
        .split_once('\n')
        .unwrap()
        .0;
    let mut crates = 0;
    for entry in std::fs::read_dir(crate_dir().join("..")).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        crates += 1;
        let named = if name == "einvoice-cli" {
            "einvoice-validate (crate einvoice-cli)".to_owned()
        } else {
            name
        };
        assert!(mit.contains(&named), "{named}: {mit}");
    }
    assert_eq!(crates, 9);
    // The W3C licence asks for its full notice in a location viewable to users.
    assert!(outputs[0].contains("W3C® SOFTWARE NOTICE AND LICENSE"));
}

#[test]
fn help_mentions_the_licences_option() {
    let r = common::run(&["--help"], None);
    assert_eq!(r.code, 0);
    assert!(
        r.stdout
            .contains("      --licences       Print the licences of everything embedded")
    );
    assert!(r.stdout.contains("Also --licenses."));
}

#[test]
fn library_mit_text_is_the_binarys() {
    // The binary prints its own MIT text for the code of every crate: it must be the
    // one einvoice-validator lists for the library crates.
    let library = einvoice_validator::licences();
    assert_eq!(library[0].id, "MIT");
    assert_eq!(
        library[0].text.as_bytes(),
        read(&crate_dir().join("LICENSES/MIT.txt")).as_slice()
    );
}
