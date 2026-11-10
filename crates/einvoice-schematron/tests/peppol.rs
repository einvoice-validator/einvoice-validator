//! Local only: the two stylesheets of Peppol BIS Billing 3.0 (`PEPPOL-EN16931-UBL` and
//! the CEN rules it embeds) are byte for byte the stylesheets ph-schematron-maven-plugin
//! 10.3.1 compiles, their SHA-256 the pin of `scripts/fetch-peppol.sh`: written from the
//! result trees of ph-schematron's last step (the DOM documents are ph-schematron's, node
//! by node), and compiled from the Schematron files by the whole pipeline in Rust (each
//! step's result is ph-schematron's, byte for byte).
//!
//! The Peppol files may not be redistributed, so the inputs live in the cache outside the
//! tracked tree: the Schematron files in
//! `target/peppol/peppol-bis-billing-ubl-<tag>/source/rules/sch/`, which
//! `scripts/fetch-peppol.sh` extracts, and ph-schematron's results in
//! `target/peppol/peppol-bis-billing-ubl-<tag>/schematron/` (`NAME.step1.xml` to
//! `NAME.step3.xml` and `NAME.dom`), which the `ph-schematron-oracle` example writes with
//! `--peppol` (which needs Java). Without them these tests say SKIPPED and pass, unless
//! `EINVOICE_SCHEMATRON_PEPPOL_REQUIRED` is set to a non-empty value: then they fail.

#![cfg(test)]

mod common;

use std::fmt::Write as _;
use std::path::PathBuf;

use einvoice_schematron::{Document, Skeleton, Sources, write_stylesheet};
use sha2::{Digest, Sha256};

const REQUIRED_VAR: &str = "EINVOICE_SCHEMATRON_PEPPOL_REQUIRED";

/// The value of a `name=value` pin of `scripts/fetch-peppol.sh`.
fn pin(script: &str, name: &str) -> String {
    script
        .lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
        .unwrap_or_else(|| panic!("no pin {name} in scripts/fetch-peppol.sh"))
        .trim()
        .to_owned()
}

#[test]
fn the_peppol_stylesheets_are_the_pinned_ones() {
    let root = common::workspace_root();
    let script = common::read(&root.join("scripts").join("fetch-peppol.sh"));
    let tag = pin(&script, "tag");
    let release: PathBuf = root
        .join("target")
        .join("peppol")
        .join(format!("peppol-bis-billing-ubl-{tag}"));
    let dir = release.join("schematron");
    let stylesheets = [
        ("PEPPOL-EN16931-UBL", pin(&script, "peppol_xslt_sha256")),
        ("CEN-EN16931-UBL", pin(&script, "cen_xslt_sha256")),
    ];
    let missing: Vec<String> = stylesheets
        .iter()
        .flat_map(|(name, _)| [format!("{name}.step3.xml"), format!("{name}.dom")])
        .filter(|file| !dir.join(file).is_file())
        .collect();
    if !missing.is_empty() {
        let message = format!(
            "no {} in {}: write them with cargo run -p einvoice-schematron --example \
             ph-schematron-oracle -- --peppol (after scripts/fetch-peppol.sh)",
            missing.join(", "),
            dir.display()
        );
        assert!(
            std::env::var_os(REQUIRED_VAR).is_none_or(|v| v.is_empty()),
            "{message}; {REQUIRED_VAR} is set"
        );
        eprintln!("SKIPPED: {message}");
        return;
    }
    for (name, sha256) in &stylesheets {
        let result = common::read(&dir.join(format!("{name}.step3.xml")));
        let document = Document::from_result(&result).unwrap_or_else(|e| panic!("{name}: {e}"));
        let dom = common::read(&dir.join(format!("{name}.dom")));
        let ours = common::dom_text(&document);
        assert!(
            ours == dom,
            "{name}.dom: {}",
            common::first_difference(&dom, &ours)
        );
        let written = write_stylesheet(&document).unwrap_or_else(|e| panic!("{name}: {e}"));
        let hash = Sha256::digest(written.as_bytes())
            .iter()
            .fold(String::new(), |mut out, b| {
                let _ = write!(out, "{b:02x}");
                out
            });
        let compiled = release.join("xslt").join(format!("{name}.xslt"));
        if hash != *sha256 && compiled.is_file() {
            let expected = common::read(&compiled);
            panic!(
                "{name}: SHA-256 {hash}, pinned {sha256}; {}",
                common::first_difference(&expected, &written)
            );
        }
        assert_eq!(hash, *sha256, "{name}: SHA-256 of the stylesheet written");
        eprintln!(
            "{name}: {} bytes written, SHA-256 {hash} as pinned",
            written.len()
        );
    }
}

fn sha256_of(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

#[test]
fn the_pipeline_in_rust_compiles_the_pinned_stylesheets() {
    let root = common::workspace_root();
    let script = common::read(&root.join("scripts").join("fetch-peppol.sh"));
    let tag = pin(&script, "tag");
    let release: PathBuf = root
        .join("target")
        .join("peppol")
        .join(format!("peppol-bis-billing-ubl-{tag}"));
    let sch = release.join("source").join("rules").join("sch");
    let stylesheets = [
        ("PEPPOL-EN16931-UBL", pin(&script, "peppol_xslt_sha256")),
        ("CEN-EN16931-UBL", pin(&script, "cen_xslt_sha256")),
    ];
    let missing: Vec<String> = stylesheets
        .iter()
        .map(|(name, _)| format!("{name}.sch"))
        .filter(|file| !sch.join(file).is_file())
        .collect();
    if !missing.is_empty() {
        let message = format!(
            "no {} in {}: run scripts/fetch-peppol.sh",
            missing.join(", "),
            sch.display()
        );
        assert!(
            std::env::var_os(REQUIRED_VAR).is_none_or(|v| v.is_empty()),
            "{message}; {REQUIRED_VAR} is set"
        );
        eprintln!("SKIPPED: {message}");
        return;
    }
    let skeleton = Skeleton::new(&common::skeleton_files()).unwrap();
    let oracle = release.join("schematron");
    for (name, sha256) in &stylesheets {
        let path = sch.join(format!("{name}.sch"));
        let text = common::read(&path);
        let uri = einvoice_xpath_eval::file_uri(&path).unwrap();
        let started = std::time::Instant::now();
        let compilation = skeleton
            .compile(&text, &uri, &Sources::new().with(uri.clone(), text.clone()))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let elapsed = started.elapsed();
        // The steps, as ph-schematron gives them, when the oracle wrote them.
        for (step, ours) in compilation.steps.iter().enumerate() {
            let golden = oracle.join(format!("{name}.step{}.xml", step + 1));
            if golden.is_file() {
                let expected = common::read(&golden);
                assert!(
                    *ours == expected,
                    "{name} step {}: {}",
                    step + 1,
                    common::first_difference(&expected, ours)
                );
            }
        }
        let hash = sha256_of(&compilation.stylesheet);
        let compiled = release.join("xslt").join(format!("{name}.xslt"));
        if hash != *sha256 && compiled.is_file() {
            let expected = common::read(&compiled);
            panic!(
                "{name}: SHA-256 {hash}, pinned {sha256}; {}",
                common::first_difference(&expected, &compilation.stylesheet)
            );
        }
        assert_eq!(hash, *sha256, "{name}: SHA-256 of the stylesheet compiled");
        eprintln!(
            "{name}: compiled in Rust in {elapsed:.2?}, {} bytes, SHA-256 {hash} as pinned, {} messages",
            compilation.stylesheet.len(),
            compilation.messages.len()
        );
    }
}
