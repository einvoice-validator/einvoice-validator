//! Local only: the two stylesheets of Peppol BIS Billing 3.0 (`PEPPOL-EN16931-UBL` and
//! the CEN rules it embeds), written from the result trees of ph-schematron's last step,
//! are byte for byte the stylesheets ph-schematron-maven-plugin 10.3.1 compiles: their
//! SHA-256 is the pin of `scripts/fetch-peppol.sh`, and the DOM documents are
//! ph-schematron's, node by node.
//!
//! The Peppol files may not be redistributed, so the inputs live in the cache outside the
//! tracked tree: `target/peppol/peppol-bis-billing-ubl-<tag>/schematron/NAME.step3.xml`
//! and `NAME.dom`, which the `ph-schematron-oracle` example writes with `--peppol` (after
//! `scripts/fetch-peppol.sh`, which needs Java and network access). Without them this test
//! says SKIPPED and passes, unless `EINVOICE_SCHEMATRON_PEPPOL_REQUIRED` is set to a
//! non-empty value: then it fails.

#![cfg(test)]

mod common;

use std::fmt::Write as _;
use std::path::PathBuf;

use einvoice_schematron::{Document, write_stylesheet};
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
