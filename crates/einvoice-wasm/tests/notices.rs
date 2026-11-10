//! `THIRD-PARTY-NOTICES`, which the npm package ships (npm.sh copies it): the notices
//! of the third-party crates compiled into the WebAssembly module, as
//! `einvoice_testkit::notices` generates them from `Cargo.lock` and the packages of those
//! crates today, with an entry and a licence text for every crate.

#![cfg(test)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use einvoice_testkit::notices;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn third_party_notices_are_up_to_date_with_cargo_lock() {
    assert_eq!(notices::NPM.package, env!("CARGO_PKG_NAME"));
    assert_eq!(
        notices::path(&notices::NPM),
        crate_dir().join("THIRD-PARTY-NOTICES")
    );
    if let Err(e) = notices::check(&notices::NPM) {
        panic!("{e}");
    }
}

#[test]
fn every_third_party_crate_compiled_in_has_its_notice_and_licence_text() {
    let crates = notices::third_party(&notices::NPM).unwrap();
    let names: BTreeSet<&str> = crates.iter().map(|c| c.name.as_str()).collect();
    // The bindings and the engine; not the PDF reader (no feature pdf), not the macros
    // that run on the build machine.
    for name in ["wasm-bindgen", "js-sys", "regex", "roxmltree"] {
        assert!(names.contains(name), "{name}: {names:?}");
    }
    for name in [
        "miniz_oxide",
        "wasm-bindgen-macro",
        "syn",
        "proc-macro2",
        "bumpalo",
    ] {
        assert!(!names.contains(name), "{name}: {names:?}");
    }
    let file = std::fs::read_to_string(crate_dir().join("THIRD-PARTY-NOTICES")).unwrap();
    assert!(file.contains(&format!("\nCrates ({}):\n", crates.len())));
    let sections: Vec<&str> = file.split("\n== ").skip(1).collect();
    assert_eq!(sections.len(), crates.len());
    for (c, section) in crates.iter().zip(&sections) {
        let heading = format!(
            "{} {}\nLicence: {}",
            c.name,
            c.version,
            c.chosen.join(" AND ")
        );
        assert!(section.starts_with(&heading), "{}: {section}", c.name);
        assert!(c.chosen.contains(&"MIT".to_owned()), "{}", c.name);
        assert_eq!(c.texts.len(), c.chosen.len(), "{}", c.name);
        for t in &c.texts {
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
