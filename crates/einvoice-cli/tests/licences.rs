//! The licence expression of the crate names exactly the texts in `LICENSES/` and the
//! terms of every workspace crate the binary is built from: the repository's MIT
//! licence, the EUPL-1.2 of the CEN stylesheets that the binary embeds through
//! einvoice-validator's default features, as published with the artefacts, and the
//! terms of the UBL 2.1 schemas of einvoice-xsd. `einvoice-validate --licences` prints
//! the full text of each of them, and its MIT section names exactly the workspace crates
//! the binary is built from (`cargo tree`, default features). Then it prints the
//! notices of the third-party crates it is compiled from, `THIRD-PARTY-NOTICES`, which
//! must be what `einvoice_testkit::notices` generates from `Cargo.lock` and the
//! packages of those crates today, with an entry and a licence text for every crate.

#![cfg(test)]

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use einvoice_testkit::notices;
use einvoice_testkit::workspace::{self, Crate, Features};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The licence terms, in the order `--licences` prints them.
const TERMS: [&str; 5] = [
    "MIT",
    "EUPL-1.2",
    "LicenseRef-OASIS-Copyright-Notice",
    "LicenseRef-UN-CEFACT-Copyright-Notice",
    "W3C-19980720",
];

/// The workspace crates the binary is built from, itself included.
fn built_from() -> Vec<Crate> {
    workspace::dependencies(env!("CARGO_PKG_NAME"), Features::Default).unwrap()
}

#[test]
fn licence_expression_names_exactly_the_shipped_texts() {
    let declared: BTreeSet<&str> = env!("CARGO_PKG_LICENSE").split(" AND ").collect();
    assert_eq!(declared, BTreeSet::from(TERMS));
    let shipped: BTreeSet<String> = std::fs::read_dir(crate_dir().join("LICENSES"))
        .unwrap()
        .map(|entry| {
            let name = entry.unwrap().file_name().into_string().unwrap();
            name.strip_suffix(".txt").unwrap().to_owned()
        })
        .collect();
    assert_eq!(
        declared,
        shipped.iter().map(String::as_str).collect::<BTreeSet<_>>()
    );
}

#[test]
fn licence_expression_is_the_union_of_the_crates_built_from() {
    let crates = built_from();
    assert!(crates.iter().any(|c| c.name == "einvoice-validator"));
    let mut terms = BTreeSet::new();
    for c in &crates {
        terms.extend(c.license_terms().unwrap());
    }
    let declared: BTreeSet<&str> = env!("CARGO_PKG_LICENSE").split(" AND ").collect();
    assert_eq!(terms, declared);
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
    // The terms of the UBL 2.1 schemas: einvoice-xsd's texts.
    for id in &TERMS[2..] {
        assert_eq!(
            read(&licences.join(format!("{id}.txt"))),
            read(&crate_dir().join(format!("../einvoice-xsd/LICENSES/{id}.txt"))),
            "{id}"
        );
    }
}

#[test]
fn licences_option_prints_every_embedded_licence_in_full() {
    let licences = crate_dir().join("LICENSES");
    let text = |path: PathBuf| String::from_utf8(read(&path)).unwrap();
    // The licence, in the order printed, and its full text.
    let expected: Vec<(&str, String)> = TERMS
        .iter()
        .map(|id| (*id, text(licences.join(format!("{id}.txt")))))
        .collect();
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
        // Then the third-party notices, as committed.
        assert_eq!(
            rest,
            format!("\n== Third-party crates\n\n{}", third_party_notices()),
            "{flag}: the output after the licences"
        );
        outputs.push(r.stdout);
    }
    assert_eq!(outputs[0], outputs[1]);
    // The W3C licence asks for its full notice in a location viewable to users.
    assert!(outputs[0].contains("W3C® SOFTWARE NOTICE AND LICENSE"));
}

#[test]
fn mit_section_names_exactly_the_crates_the_binary_is_built_from() {
    let r = common::run(&["--licences"], None);
    assert_eq!(r.code, 0);
    let mit = r
        .stdout
        .split_once("\n== 1/5 MIT\nCovers: ")
        .unwrap()
        .1
        .split_once('\n')
        .unwrap()
        .0;
    assert!(
        mit.starts_with("the code of einvoice-validate (crate einvoice-cli) and of the crates "),
        "{mit}"
    );
    let crates = built_from();
    let others: Vec<&str> = crates
        .iter()
        .map(|c| c.name.as_str())
        .filter(|name| *name != env!("CARGO_PKG_NAME"))
        .collect();
    assert_eq!(workspace::named_crates(mit).unwrap(), others, "{mit}");
    // Each ships the same MIT text.
    let mit_text = read(&crate_dir().join("LICENSES/MIT.txt"));
    for c in &crates {
        assert_eq!(
            read(&c.dir.join("LICENSES/MIT.txt")),
            mit_text,
            "{}",
            c.name
        );
    }
}

/// The committed notices file.
fn third_party_notices() -> String {
    String::from_utf8(read(&crate_dir().join("THIRD-PARTY-NOTICES"))).unwrap()
}

#[test]
fn third_party_notices_are_up_to_date_with_cargo_lock() {
    assert_eq!(notices::CLI.package, env!("CARGO_PKG_NAME"));
    assert_eq!(
        notices::path(&notices::CLI),
        crate_dir().join("THIRD-PARTY-NOTICES")
    );
    if let Err(e) = notices::check(&notices::CLI) {
        panic!("{e}");
    }
}

#[test]
fn every_third_party_crate_compiled_in_has_its_notice_and_licence_text() {
    let crates = notices::third_party(&notices::CLI).unwrap();
    let names: BTreeSet<&str> = crates.iter().map(|c| c.name.as_str()).collect();
    // The crates of the engine, of the schemas, of the PDF reader and their own.
    for name in [
        "regex",
        "roxmltree",
        "sha2",
        "miniz_oxide",
        "memchr",
        // The HTTPS client of `rules fetch` (default feature `fetch`): rustls, the
        // RustCrypto crates of its cryptography provider, the Mozilla roots.
        "rustls",
        "aes-gcm",
        "chacha20poly1305",
        "p256",
        "rsa",
        "webpki-roots",
    ] {
        assert!(names.contains(name), "{name}: {names:?}");
    }
    // Nothing that builds C or assembly (deny.toml bans them, tests/supply_chain.rs of
    // einvoice-testkit checks the packages).
    for name in ["ring", "cc", "aws-lc-sys", "openssl-sys"] {
        assert!(!names.contains(name), "{name}: {names:?}");
    }
    // No crate of this project, no build dependency, no procedural macro.
    let members = workspace::members().unwrap();
    assert!(names.iter().all(|n| !members.iter().any(|m| m == n)));
    assert!(!names.contains("autocfg"), "{names:?}");
    let r = common::run(&["--licences"], None);
    assert_eq!(r.code, 0);
    let (_, printed) = r.stdout.split_once("\n== Third-party crates\n\n").unwrap();
    assert!(printed.contains(&format!("\nCrates ({}):\n", crates.len())));
    let sections: Vec<&str> = printed.split("\n== ").skip(1).collect();
    assert_eq!(sections.len(), crates.len());
    for (c, section) in crates.iter().zip(&sections) {
        let heading = format!(
            "{} {}\nLicence: {}",
            c.name,
            c.version,
            c.chosen.join(" AND ")
        );
        assert!(section.starts_with(&heading), "{}: {section}", c.name);
        // MIT where there is a choice, and the text of each licence chosen. Without an
        // MIT alternative (the HTTPS client of the feature `fetch`: rustls-webpki,
        // untrusted, subtle, webpki-roots), the licences the crate offers.
        let mit_offered = notices::alternatives(&c.license)
            .unwrap()
            .iter()
            .any(|terms| terms.len() == 1 && terms.contains("MIT"));
        if mit_offered {
            assert_eq!(c.chosen, ["MIT"], "{}", c.name);
        } else {
            assert!(
                ["rustls-webpki", "untrusted", "subtle", "webpki-roots"].contains(&c.name.as_str()),
                "{}: {} offers no MIT alternative",
                c.name,
                c.license
            );
            assert!(!c.chosen.is_empty(), "{}", c.name);
        }
        assert_eq!(c.texts.len(), c.chosen.len(), "{}", c.name);
        for t in &c.texts {
            assert!(
                t.text.contains("Permission is hereby granted") || t.licence != "MIT",
                "{}: {}",
                c.name,
                t.file
            );
            let block = format!("\n-- {}, from {}\n\n{}", t.licence, t.file, t.text);
            assert!(
                section.contains(&block),
                "{}: no {} text",
                c.name,
                t.licence
            );
        }
    }
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
    // The other licences are the library's, in its order.
    let ids: Vec<&str> = library.iter().map(|l| l.id).collect();
    assert_eq!(ids, TERMS);
}
