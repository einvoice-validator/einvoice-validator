//! The corpora of the fetched rule sets (Peppol BIS Billing, Factur-X,
//! XRechnung, the French rules), built in the cache that the fetch script of
//! their family fills (nothing of it is committed: see the README of each
//! `corpus/<rule set>/`): the cases, the Saxon-HE goldens of every stylesheet,
//! what the reference validator makes of them (`crate::acceptance`: the table
//! of the reference acceptance of every case, and for XRechnung the levels of
//! the bundles checked against the configuration), then the cross-check with
//! the expectations (VEFA, XML Mutate, accepted documents) against the
//! committed record of disagreements.
//!
//! The stylesheets, and the files they read with `document()`, are checked
//! against the SHA-256 pinned in the fetch script before Saxon runs. When
//! everything is written, a stamp file, [`STAMP`], records the number of cases
//! and the checksums of the stylesheets: the tests of the engine use a cache
//! only when its stamp is there and agrees.

use std::fmt::Write as _;
use std::path::Path;

use crate::acceptance;
use crate::corpus;
use crate::crosscheck;
use crate::oracle;
use crate::ruleset::{Family, Origin, RuleSet};
use crate::util::{self, Error, Result};

/// The stamp of a complete cache, relative to the corpus directory.
pub(crate) const STAMP: &str = "complete.txt";

const PINS_START: &str = "# --- pins ";
const PINS_END: &str = "# --- end of pins ";

/// The `name=value` lines between the pin markers of `script` (relative to
/// the repository root).
pub(crate) fn pins(root: &Path, script: &str) -> Result<Vec<(String, String)>> {
    let text = util::read_string(&util::join(root, script))?;
    let mut lines = text.lines().skip_while(|l| !l.starts_with(PINS_START));
    if lines.next().is_none() {
        return Err(Error::new(format!("{script}: no `{PINS_START}` line")));
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
            .ok_or_else(|| Error::new(format!("{script}: bad pin line {line:?}")))?;
        pins.push((name.to_owned(), value.to_owned()));
    }
    Err(Error::new(format!("{script}: no `{PINS_END}` line")))
}

pub(crate) fn pin<'a>(pins: &'a [(String, String)], script: &str, name: &str) -> Result<&'a str> {
    pins.iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
        .ok_or_else(|| Error::new(format!("{script}: no pin {name}")))
}

/// Checks that the cache holds the release the rule set names and the
/// stylesheets and their side files with their pinned SHA-256. Returns
/// `(stylesheet, sha256)`.
pub(crate) fn check_cache(
    root: &Path,
    family: &Family,
    rule_set: &RuleSet,
) -> Result<Vec<(String, String)>> {
    let Origin::Fetched { base, .. } = rule_set.origin else {
        return Err(Error::new(format!(
            "{}: not a fetched rule set",
            rule_set.name
        )));
    };
    let script = family.script;
    let pins = pins(root, script)?;
    let version = pin(&pins, script, family.version_pin)?;
    if !rule_set.name.ends_with(&format!("-{version}")) {
        return Err(Error::new(format!(
            "{script} pins {} {version}, the rule set is {}",
            family.version_pin, rule_set.name
        )));
    }
    let fetch = format!("run {script} to build {base}");
    let source = util::join(&rule_set.base(root), rule_set.source);
    if !source.is_dir() {
        return Err(Error::new(format!(
            "{} not found: {fetch}",
            source.display()
        )));
    }
    let check = |path: &str, name: &str| -> Result<String> {
        let path = util::join(&rule_set.base(root), path);
        if !path.is_file() {
            return Err(Error::new(format!("{} not found: {fetch}", path.display())));
        }
        let expected = pin(&pins, script, name)?;
        let actual = util::sha256_hex(&util::read(&path)?);
        if actual == expected {
            Ok(actual)
        } else {
            Err(Error::new(format!(
                "{}: SHA-256 {actual}, expected {expected} ({name} in {script}): {fetch}",
                path.display()
            )))
        }
    };
    let mut checked = Vec::new();
    for stylesheet in rule_set.stylesheets {
        let name = stylesheet.sha256_pin.ok_or_else(|| {
            Error::new(format!(
                "{}: no SHA-256 pin for {}",
                rule_set.name, stylesheet.name
            ))
        })?;
        let sha256 = check(stylesheet.xslt, name)?;
        let dir = stylesheet.xslt.rsplit_once('/').map_or("", |(d, _)| d);
        for (file, file_pin) in stylesheet.side_files {
            check(&format!("{dir}/{file}"), file_pin)?;
        }
        checked.push((stylesheet.name.to_owned(), sha256));
    }
    Ok(checked)
}

/// The stamp of a rule set's corpus in the cache.
pub(crate) fn stamp(root: &Path, rule_set: &RuleSet) -> std::path::PathBuf {
    util::join(
        &rule_set.base(root),
        &format!("{}/{STAMP}", rule_set.corpus_dir()),
    )
}

/// Builds the corpus of each rule set of the family in the cache, runs
/// Saxon-HE with every stylesheet and cross-checks the goldens with the
/// expectations. Returns the report and whether every cross-check agrees with
/// its record.
pub(crate) fn run(
    root: &Path,
    family: &Family,
    options: &oracle::Options,
) -> Result<(String, bool)> {
    let mut out = String::new();
    let mut consistent = true;
    for rule_set in family.rule_sets {
        let stylesheets = check_cache(root, family, rule_set)?;
        let stamp = stamp(root, rule_set);
        if stamp.exists() {
            std::fs::remove_file(&stamp)
                .map_err(|e| Error::new(format!("cannot remove {}: {e}", stamp.display())))?;
        }

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

        // What the reference validator makes of the goldens, case by case, and
        // the levels of the bundle the fetch script wrote.
        let _ = writeln!(out, "{}", acceptance::write_table(root, rule_set)?);
        if let Some(checked) = acceptance::check_bundle(root, rule_set)? {
            let _ = writeln!(out, "{checked}");
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

        let (text, agrees) = crosscheck::report(root, rule_set)?;
        out.push_str(&text);
        if !agrees {
            consistent = false;
            let _ = writeln!(
                out,
                "{}: observed disagreements differ from {}/{}",
                rule_set.name,
                rule_set.record_dir(),
                rule_set.record
            );
        }
    }
    Ok((out, consistent))
}

#[cfg(test)]
mod tests {
    use super::{check_cache, pin, pins, stamp};
    use crate::ruleset::FAMILIES;
    use crate::util::repo_root;

    /// Every pin the tooling reads is in its script, well formed, and the
    /// rule sets are named after the version their script pins.
    #[test]
    fn pins_are_readable() {
        let root = repo_root();
        for family in FAMILIES {
            let pins = pins(&root, family.script).unwrap();
            let version = pin(&pins, family.script, family.version_pin).unwrap();
            for rule_set in family.rule_sets {
                assert!(
                    rule_set.name.ends_with(&format!("-{version}")),
                    "{}",
                    rule_set.name
                );
                for stylesheet in rule_set.stylesheets {
                    let sha256 = pin(&pins, family.script, stylesheet.sha256_pin.unwrap()).unwrap();
                    assert_eq!(sha256.len(), 64, "{}", stylesheet.name);
                    for (_, file_pin) in stylesheet.side_files {
                        assert_eq!(pin(&pins, family.script, file_pin).unwrap().len(), 64);
                    }
                }
            }
        }
        let peppol = pins(&root, "scripts/fetch-peppol.sh").unwrap();
        for name in [
            "tag",
            "commit",
            "archive_sha256",
            "maven",
            "maven_sha512",
            "plugin",
        ] {
            assert!(
                pin(&peppol, "scripts/fetch-peppol.sh", name).is_ok(),
                "{name}"
            );
        }
    }

    /// When a cache is complete (its fetch script, then `cargo xtask
    /// <family>`), it holds the pinned stylesheets.
    #[test]
    fn complete_caches_match_the_pins() {
        let root = repo_root();
        for family in FAMILIES {
            for rule_set in family.rule_sets {
                if !stamp(&root, rule_set).exists() {
                    println!(
                        "SKIPPED: no complete cache of {} (scripts/check.sh {} builds it)",
                        rule_set.name, family.name
                    );
                    continue;
                }
                check_cache(&root, family, rule_set).unwrap_or_else(|e| panic!("{e}"));
            }
        }
    }
}
