//! `SHA256SUMS`: the checksum of every vendored upstream file.
//!
//! The format is the one `sha256sum` writes and checks (`<hex>  <path>`, one
//! file per line, sorted by path), so `sha256sum -c SHA256SUMS` run inside the
//! vendored directory verifies it without this tool.

use std::fmt::Write as _;
use std::path::Path;

use crate::ruleset::RuleSet;
use crate::util::{self, Error, Result};

/// The manifest file, inside the vendored directory.
pub(crate) const FILE_NAME: &str = "SHA256SUMS";

/// Files of the vendored directory that are this project's own and therefore
/// not listed: the manifest itself and the provenance README.
const OWN_FILES: &[&str] = &[FILE_NAME, "README.md"];

/// The manifest that the vendored files on disk produce.
fn compute(dir: &Path) -> Result<String> {
    let mut manifest = String::new();
    for relative in util::list_files(dir)? {
        if OWN_FILES.contains(&relative.as_str()) {
            continue;
        }
        if relative.contains(['\\', '\n', '\r']) {
            // sha256sum escapes these; refuse rather than write an ambiguous line.
            return Err(Error::new(format!(
                "{relative}: backslash or line break in a vendored file name"
            )));
        }
        let hash = util::sha256_hex(&util::read(&util::join(dir, &relative))?);
        let _ = writeln!(manifest, "{hash}  {relative}");
    }
    if manifest.is_empty() {
        return Err(Error::new(format!("{}: no vendored files", dir.display())));
    }
    Ok(manifest)
}

/// Checks the vendored files against the committed manifest: same set of
/// files, same checksums. Returns the number of files verified.
pub(crate) fn verify(root: &Path, rule_set: &RuleSet) -> Result<usize> {
    let dir = util::join(root, &vendored_dir(rule_set)?);
    let manifest_path = dir.join(FILE_NAME);
    let committed = util::read_string(&manifest_path)?;
    let expected =
        parse(&committed).map_err(|e| Error::new(format!("{}: {e}", manifest_path.display())))?;
    let computed = compute(&dir)?;
    let actual = parse(&computed)?;

    let mut problems = Vec::new();
    for (path, hash) in &expected {
        match actual.iter().find(|(p, _)| p == path) {
            None => problems.push(format!("missing: {path}")),
            Some((_, h)) if h != hash => problems.push(format!("checksum mismatch: {path}")),
            Some(_) => {}
        }
    }
    for (path, _) in &actual {
        if !expected.iter().any(|(p, _)| p == path) {
            problems.push(format!("not in {FILE_NAME}: {path}"));
        }
    }
    if committed != computed && problems.is_empty() {
        problems.push(format!(
            "{FILE_NAME} is not in canonical form (sorted, two spaces, LF)"
        ));
    }
    if problems.is_empty() {
        Ok(expected.len())
    } else {
        Err(Error::new(format!(
            "{}: vendored files do not match {FILE_NAME}:\n  {}",
            dir.display(),
            problems.join("\n  ")
        )))
    }
}

/// Rewrites the manifest from the files on disk. Returns the number of files.
pub(crate) fn write(root: &Path, rule_set: &RuleSet) -> Result<usize> {
    let dir = util::join(root, &vendored_dir(rule_set)?);
    let manifest = compute(&dir)?;
    util::write(&dir.join(FILE_NAME), manifest.as_bytes())?;
    Ok(manifest.lines().count())
}

fn vendored_dir(rule_set: &RuleSet) -> Result<String> {
    rule_set
        .vendored_dir()
        .ok_or_else(|| Error::new(format!("{}: not a vendored rule set", rule_set.name)))
}

fn parse(manifest: &str) -> Result<Vec<(String, String)>> {
    manifest
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let (hash, path) = line.split_once("  ").ok_or_else(|| {
                Error::new(format!("line {}: expected `<sha256>  <path>`", index + 1))
            })?;
            if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Error::new(format!("line {}: not a SHA-256", index + 1)));
            }
            Ok((path.to_owned(), hash.to_ascii_lowercase()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::verify;
    use crate::ruleset::VENDORED;
    use crate::util::repo_root;

    /// The vendored files are exactly the ones listed, unmodified.
    #[test]
    fn vendored_files_match_their_manifest() {
        for rule_set in VENDORED {
            let count = verify(&repo_root(), rule_set).unwrap_or_else(|e| panic!("{e}"));
            assert!(count > 0, "{}", rule_set.name);
        }
    }
}
