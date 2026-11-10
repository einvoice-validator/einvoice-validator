//! The whole pipeline in Rust on the synthetic Schematron files of `tests/data/sch/`: the
//! three steps of the ISO Schematron skeleton run by this project's engine give the
//! results ph-schematron 10.3.1 gives on Saxon-HE 12.10, byte for byte (`NAME.step1.xml`,
//! `NAME.step2.xml`, `NAME.step3.xml` of `tests/data/expected/`, written by the
//! `ph-schematron-oracle` example), and the compiled stylesheet is the plugin's
//! (`NAME.xslt`).

#![cfg(test)]

mod common;

use std::path::Path;

use einvoice_schematron::{Skeleton, Sources};

const FIXTURES: [&str; 7] = [
    "abstract",
    "basic",
    "characters",
    "foreign",
    "include",
    "markup",
    "minimal",
];

/// The `file:` URI of a path, as Saxon gives it to a file it reads.
fn uri(path: &Path) -> String {
    einvoice_xpath_eval::file_uri(&std::path::absolute(path).unwrap()).unwrap()
}

/// Every file under `tests/data/sch/`, by URI.
fn sources() -> Sources {
    let mut sources = Sources::new();
    let mut pending = vec![common::data().join("sch")];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                sources = sources.with(uri(&path), common::read(&path));
            }
        }
    }
    sources
}

#[test]
fn the_steps_and_the_stylesheet_are_ph_schematron_s() {
    let skeleton = Skeleton::new(&common::skeleton_files()).unwrap();
    let sources = sources();
    let expected = common::data().join("expected");
    let mut failures = Vec::new();
    for name in FIXTURES {
        let path = common::data().join("sch").join(format!("{name}.sch"));
        let compilation = match skeleton.compile(&common::read(&path), &uri(&path), &sources) {
            Ok(c) => c,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        for (step, ours) in compilation.steps.iter().enumerate() {
            let file = format!("{name}.step{}.xml", step + 1);
            let golden = common::read(&expected.join(&file));
            if *ours != golden {
                failures.push(format!(
                    "{file}: {}",
                    common::first_difference(&golden, ours)
                ));
            }
        }
        let xslt = common::read(&expected.join(format!("{name}.xslt")));
        if compilation.stylesheet != xslt {
            failures.push(format!(
                "{name}.xslt: {}",
                common::first_difference(&xslt, &compilation.stylesheet)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
