//! The Peppol BIS Billing 3.0 corpus, built in the cache that
//! `scripts/fetch-peppol.sh` fills (nothing of it is committed: the upstream
//! files carry no licence): the cases, the Saxon-HE goldens of both
//! stylesheets, then the cross-check with the VEFA expectations against the
//! committed record of disagreements.
//!
//! The stylesheets are checked against the SHA-256 pinned in
//! `scripts/fetch-peppol.sh` before Saxon runs. When everything is written,
//! a stamp file, [`STAMP`], records the number of cases and the checksums of
//! the stylesheets: the equivalence test of `einvoice-xslt-exec` uses the
//! cache only when the stamp is there and agrees.

use std::fmt::Write as _;
use std::path::Path;

use crate::corpus;
use crate::crosscheck;
use crate::oracle;
use crate::ruleset::{Origin, PEPPOL_UBL, RuleSet};
use crate::util::{self, Error, Result};

/// The script that builds the cache and holds the pins, relative to the
/// repository root.
pub(crate) const PINS_SCRIPT: &str = "scripts/fetch-peppol.sh";
/// The stamp of a complete cache, relative to the corpus directory.
pub(crate) const STAMP: &str = "complete.txt";

const PINS_START: &str = "# --- pins ";
const PINS_END: &str = "# --- end of pins ";

/// The `name=value` lines between the pin markers of [`PINS_SCRIPT`].
pub(crate) fn pins(root: &Path) -> Result<Vec<(String, String)>> {
    let text = util::read_string(&util::join(root, PINS_SCRIPT))?;
    let mut lines = text.lines().skip_while(|l| !l.starts_with(PINS_START));
    if lines.next().is_none() {
        return Err(Error::new(format!("{PINS_SCRIPT}: no `{PINS_START}` line")));
    }
    let mut pins = Vec::new();
    for line in lines {
        if line.starts_with(PINS_END) {
            return Ok(pins);
        }
        let (name, value) = line
            .split_once('=')
            .filter(|(n, v)| {
                !n.is_empty()
                    && n.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                    && !v.is_empty()
                    && v.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
            })
            .ok_or_else(|| Error::new(format!("{PINS_SCRIPT}: bad pin line {line:?}")))?;
        pins.push((name.to_owned(), value.to_owned()));
    }
    Err(Error::new(format!("{PINS_SCRIPT}: no `{PINS_END}` line")))
}

fn pin<'a>(pins: &'a [(String, String)], name: &str) -> Result<&'a str> {
    pins.iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
        .ok_or_else(|| Error::new(format!("{PINS_SCRIPT}: no pin {name}")))
}

/// Checks that the cache holds the release the rule set names and the
/// stylesheets with their pinned SHA-256. Returns `(stylesheet, sha256)`.
pub(crate) fn check_cache(root: &Path, rule_set: &RuleSet) -> Result<Vec<(String, String)>> {
    let Origin::Fetched { base } = rule_set.origin else {
        return Err(Error::new(format!(
            "{}: not a fetched rule set",
            rule_set.name
        )));
    };
    let pins = pins(root)?;
    let tag = pin(&pins, "tag")?;
    if !rule_set.name.ends_with(&format!("-{tag}")) {
        return Err(Error::new(format!(
            "{PINS_SCRIPT} pins tag {tag}, the rule set is {}",
            rule_set.name
        )));
    }
    let fetch = format!("run {PINS_SCRIPT} to build {base}");
    let source = util::join(&rule_set.base(root), rule_set.source);
    if !source.is_dir() {
        return Err(Error::new(format!(
            "{} not found: {fetch}",
            source.display()
        )));
    }
    let mut checked = Vec::new();
    for stylesheet in rule_set.stylesheets {
        let path = util::join(&rule_set.base(root), stylesheet.xslt);
        if !path.is_file() {
            return Err(Error::new(format!("{} not found: {fetch}", path.display())));
        }
        let name = stylesheet.sha256_pin.ok_or_else(|| {
            Error::new(format!(
                "{}: no SHA-256 pin for {}",
                rule_set.name, stylesheet.name
            ))
        })?;
        let expected = pin(&pins, name)?;
        let actual = util::sha256_hex(&util::read(&path)?);
        if actual != expected {
            return Err(Error::new(format!(
                "{}: SHA-256 {actual}, expected {expected} ({name} in {PINS_SCRIPT}): {fetch}",
                path.display()
            )));
        }
        checked.push((stylesheet.name.to_owned(), actual));
    }
    Ok(checked)
}

/// Builds the corpus of the Peppol rule set in the cache, runs Saxon-HE with
/// both stylesheets and cross-checks the goldens with the VEFA expectations.
/// Returns the report and whether the cross-check agrees with the record.
pub(crate) fn run(root: &Path, options: &oracle::Options) -> Result<(String, bool)> {
    let rule_set = &PEPPOL_UBL;
    let stylesheets = check_cache(root, rule_set)?;
    let stamp = util::join(
        &rule_set.base(root),
        &format!("{}/{STAMP}", rule_set.corpus_dir()),
    );
    if stamp.exists() {
        std::fs::remove_file(&stamp)
            .map_err(|e| Error::new(format!("cannot remove {}: {e}", stamp.display())))?;
    }

    let mut out = String::new();
    let counts = corpus::write(root, rule_set)?;
    let _ = writeln!(
        out,
        "corpus written: {}: {} cases, {} mutation cases",
        rule_set.name, counts.cases, counts.mutations
    );

    let options = oracle::Options {
        java: options.java.clone(),
        saxon: options.saxon.clone(),
        write: true,
    };
    for outcome in oracle::run(root, rule_set, &options)? {
        let _ = writeln!(
            out,
            "oracle written: {} {}: {} cases, {} with findings, {} failed asserts, {} successful reports, {} stopped by an error, {} with pattern warnings",
            rule_set.name,
            outcome.stylesheet,
            outcome.cases,
            outcome.cases_with_findings,
            outcome.failed_asserts,
            outcome.successful_reports,
            outcome.errors,
            outcome.warned
        );
    }

    let mut record = format!(
        "cases {}\nmutations {}\nsaxon {}\n",
        counts.cases,
        counts.mutations,
        oracle::SAXON_VERSION
    );
    for (name, sha256) in &stylesheets {
        let _ = writeln!(record, "stylesheet {name} {sha256}");
    }
    util::write(&stamp, record.as_bytes())?;

    let (text, consistent) = crosscheck::report(root, rule_set)?;
    out.push_str(&text);
    Ok((out, consistent))
}

#[cfg(test)]
mod tests {
    use super::{STAMP, check_cache, pin, pins};
    use crate::ruleset::PEPPOL_UBL;
    use crate::util::repo_root;

    /// Every pin the tooling reads is in the script, well formed.
    #[test]
    fn pins_are_readable() {
        let pins = pins(&repo_root()).unwrap();
        for name in [
            "tag",
            "commit",
            "archive_sha256",
            "maven",
            "maven_sha512",
            "plugin",
        ] {
            assert!(pin(&pins, name).is_ok(), "{name}");
        }
        for stylesheet in PEPPOL_UBL.stylesheets {
            let sha256 = pin(&pins, stylesheet.sha256_pin.unwrap()).unwrap();
            assert_eq!(sha256.len(), 64, "{}", stylesheet.name);
        }
        assert!(PEPPOL_UBL.name.ends_with(pin(&pins, "tag").unwrap()));
    }

    /// When the cache is complete (`scripts/fetch-peppol.sh`, then `cargo
    /// xtask peppol`), it holds the pinned stylesheets.
    #[test]
    fn complete_cache_matches_the_pins() {
        let root = repo_root();
        let stamp = PEPPOL_UBL
            .base(&root)
            .join(PEPPOL_UBL.corpus_dir())
            .join(STAMP);
        if !stamp.exists() {
            println!("SKIPPED: no complete Peppol cache (scripts/check.sh peppol builds it)");
            return;
        }
        check_cache(&root, &PEPPOL_UBL).unwrap_or_else(|e| panic!("{e}"));
    }
}
