//! Peppol BIS Billing 3.0, published as Schematron only: its bundle is built as
//! `scripts/fetch-peppol.sh` builds it, byte for byte. The two Schematron files of the
//! release archive (its commit id checked) are compiled as ph-schematron compiles them
//! (`ph-schematron-maven-plugin:convert`, engine `iso-schematron`), what the reference
//! validators run: with the ISO Schematron skeleton of the pinned jar of
//! ph-schematron-isosch, run in Rust by einvoice-schematron (no Java). Each compiled
//! stylesheet must have its pinned SHA-256.

use super::archive::{Tar, Zip, gunzip};
use super::assemble::Files;
use super::{FetchOptions, MAX_TAR, check_sha256};
use crate::Error;
use crate::rulesets::catalogue::{FamilySpec, FetchedRuleSet, PEPPOL_STYLESHEETS};

fn fetch_error(message: impl Into<String>) -> Error {
    Error::Fetch {
        message: message.into(),
    }
}

/// The Schematron files to compile, by stylesheet name, in the order of
/// [`PEPPOL_STYLESHEETS`], and every file of their directory of the archive
/// (`rules/sch/`), which they may include, by path.
#[derive(Debug)]
pub(crate) struct Schematron {
    files: Vec<(&'static str, Vec<u8>)>,
    directory: Vec<(String, Vec<u8>)>,
}

/// The directory of the Schematron files in the release archive.
const SCH_DIR: &str = "rules/sch/";

/// The version the line states after `Schematron version ` (`sed 's/.*Schematron
/// version \([0-9][0-9.]*[0-9]\) .*/\1/'`: the last such statement of the line).
fn stated_version(line: &str) -> Option<&str> {
    const KEY: &str = "Schematron version ";
    let mut positions: Vec<usize> = line.match_indices(KEY).map(|(i, _)| i).collect();
    positions.reverse();
    positions.into_iter().find_map(|i| {
        let rest = &line[i + KEY.len()..];
        let run = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        let version = &rest[..run];
        (version.len() >= 2
            && version.starts_with(|c: char| c.is_ascii_digit())
            && version.ends_with(|c: char| c.is_ascii_digit())
            && rest[run..].starts_with(' '))
        .then_some(version)
    })
}

/// The two Schematron files of the release archive `bytes`, checked: the archive is the
/// pinned commit, and the CEN rules state the pinned version.
pub(crate) fn schematron(family: &FamilySpec, bytes: &[u8]) -> Result<Schematron, Error> {
    let pin = |name: &str| family.pins.get(name).map_err(Error::internal);
    let (tag, commit, cen) = (pin("tag")?, pin("commit")?, pin("cen")?);
    let tar = gunzip(bytes, MAX_TAR).map_err(|e| fetch_error(format!("Peppol {tag}: {e}")))?;
    let archive = Tar::open(&tar).map_err(|e| fetch_error(format!("Peppol {tag}: {e}")))?;
    match archive.commit_comment.as_deref() {
        Some(recorded) if recorded == commit => {}
        Some(recorded) => {
            return Err(fetch_error(format!(
                "the Peppol {tag} archive is commit {recorded}, expected {commit}"
            )));
        }
        None => {
            return Err(fetch_error(format!(
                "the Peppol {tag} archive records no commit id"
            )));
        }
    }
    let mut files = Vec::new();
    for (name, _) in PEPPOL_STYLESHEETS {
        let path = format!("{SCH_DIR}{name}.sch");
        let content = archive
            .stripped(&path)
            .ok_or_else(|| fetch_error(format!("the Peppol {tag} archive has no {path}")))?;
        files.push((name, content.to_vec()));
    }
    let cen_sch = files
        .iter()
        .find(|(n, _)| *n == "CEN-EN16931-UBL")
        .map(|(_, c)| String::from_utf8_lossy(c).into_owned())
        .unwrap_or_default();
    // "<!--Schematron version 1.3.16 - Last update: ...-->"
    let stated = cen_sch.lines().find_map(stated_version);
    match stated {
        Some(v) if v == cen => {}
        Some(v) => {
            return Err(fetch_error(format!(
                "rules/sch/CEN-EN16931-UBL.sch states Schematron version {v}, expected {cen}"
            )));
        }
        None => {
            return Err(fetch_error(
                "rules/sch/CEN-EN16931-UBL.sch states no Schematron version",
            ));
        }
    }
    let directory = archive
        .stripped_under(SCH_DIR)
        .into_iter()
        .map(|(path, content)| (path.to_owned(), content.to_vec()))
        .collect();
    Ok(Schematron { files, directory })
}

/// The stylesheets of the ISO Schematron skeleton in the jar of ph-schematron-isosch
/// (`bytes`), by name: those einvoice-schematron runs ([`einvoice_schematron::SKELETON_FILES`]).
pub(crate) fn skeleton(family: &FamilySpec, bytes: &[u8]) -> Result<Vec<(String, String)>, Error> {
    let plugin = family.pins.get("plugin").map_err(Error::internal)?;
    let jar_error = |e: String| fetch_error(format!("ph-schematron-isosch {plugin}: {e}"));
    let zip = Zip::open(bytes).map_err(jar_error)?;
    let mut files = Vec::new();
    for (name, _) in einvoice_schematron::SKELETON_FILES {
        let path = format!("{}{name}", einvoice_schematron::SKELETON_DIR);
        let entry = zip
            .entry(&path)
            .ok_or_else(|| jar_error(format!("no {path}")))?;
        let content = zip.read(entry, MAX_TAR).map_err(jar_error)?;
        let text =
            String::from_utf8(content).map_err(|_| jar_error(format!("{path} is not UTF-8")))?;
        files.push((name.to_owned(), text));
    }
    Ok(files)
}

/// The URI the Schematron file `path` of the archive is compiled under: what its includes
/// are resolved against (ph-schematron gives Saxon the file's system ID, which reaches
/// the compiled stylesheet only through includes).
fn archive_uri(path: &str) -> String {
    format!("archive:/{path}")
}

/// Builds the Peppol bundle: each Schematron file compiled in Rust with the skeleton
/// (`skeleton`, from [`skeleton`]), its includes read from its directory of the archive,
/// each stylesheet checked against its pinned SHA-256, and the manifest of the
/// catalogue.
pub(crate) fn build(
    family: &FamilySpec,
    members: &[&FetchedRuleSet],
    schematron: &Schematron,
    skeleton: &[(String, String)],
    options: &FetchOptions,
) -> Result<Files, Error> {
    let pin = |name: &str| family.pins.get(name).map_err(Error::internal);
    let plugin = pin("plugin")?;
    let compile_error = |name: &str, e: einvoice_schematron::Error| {
        fetch_error(format!(
            "compiling {name}.sch as ph-schematron {plugin} does: {e}"
        ))
    };
    let skeleton = einvoice_schematron::Skeleton::new(skeleton)
        .map_err(|e| fetch_error(format!("ph-schematron-isosch {plugin}: {e}")))?;
    let mut sources = einvoice_schematron::Sources::new();
    for (path, content) in &schematron.directory {
        if let Ok(text) = std::str::from_utf8(content) {
            sources = sources.with(archive_uri(path), text);
        }
    }
    options.say(&format!(
        "Peppol BIS Billing: compiling the Schematron as ph-schematron {plugin} does (in Rust)"
    ));
    let mut files = Files::new();
    for (name, sha) in PEPPOL_STYLESHEETS {
        let content = schematron
            .files
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, c)| c)
            .ok_or_else(|| Error::internal(format!("peppol: no {name}.sch")))?;
        let text = std::str::from_utf8(content)
            .map_err(|_| fetch_error(format!("{SCH_DIR}{name}.sch is not UTF-8")))?;
        let compilation = skeleton
            .compile(
                text,
                &archive_uri(&format!("{SCH_DIR}{name}.sch")),
                &sources,
            )
            .map_err(|e| compile_error(name, e))?;
        for message in &compilation.messages {
            options.say(&format!("{name}.sch: {message}"));
        }
        let bytes = compilation.stylesheet.into_bytes();
        check_sha256(&format!("the compiled {name}.xslt"), &bytes, pin(sha)?)?;
        files.push((format!("xslt/{name}.xslt"), bytes));
    }
    let manifest = members
        .first()
        .map(|r| r.manifest().as_bytes().to_vec())
        .ok_or_else(|| Error::internal("peppol: no rule set"))?;
    files.push(("bundle.toml".to_owned(), manifest));
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_stated_version_as_sed_does() {
        assert_eq!(
            stated_version("<!--Schematron version 1.3.16 - Last update: 2025-->"),
            Some("1.3.16")
        );
        // The last statement of the line (the greedy `.*`).
        assert_eq!(
            stated_version("Schematron version 1.2 and Schematron version 1.3.4 x"),
            Some("1.3.4")
        );
        // Not followed by a space, or not ending with a digit: none.
        assert_eq!(stated_version("Schematron version 1.3.16"), None);
        assert_eq!(stated_version("Schematron version 1.3. x"), None);
        assert_eq!(stated_version("Schematron version 1 x"), None);
        assert_eq!(
            stated_version("Schematron version 1.3 - Schematron version x "),
            Some("1.3")
        );
    }
}
