//! The bundles of the families published as compiled XSLT, assembled from their
//! downloaded archive as the fetch script of each family assembles them: the same
//! checks, the same files under the same names, the manifest of the catalogue (the one
//! the script writes, byte for byte).

use super::archive::Zip;
use super::check_sha256;
use crate::Error;
use crate::rulesets::catalogue::{
    FACTURX_PROFILES, FR_CTC_BUNDLES, FR_CTC_PACK_DIR, FR_CTC_STYLESHEETS, FamilySpec,
    FetchedRuleSet, Kind, XRECHNUNG_STYLESHEETS, fr_ctc_side_file,
};

/// The files of a release directory: path relative to it, content.
pub(crate) type Files = Vec<(String, Vec<u8>)>;

fn fetch_error(message: impl Into<String>) -> Error {
    Error::Fetch {
        message: message.into(),
    }
}

/// A downloaded ZIP archive of a family, and what its bundles are checked against.
struct Archive<'a> {
    zip: Zip<'a>,
    family: &'a FamilySpec,
    members: &'a [&'a FetchedRuleSet],
    limit: usize,
}

impl Archive<'_> {
    /// The content of the file `name` of the archive.
    fn read(&self, name: &str) -> Result<Vec<u8>, Error> {
        let title = self.family.title;
        let entry = self
            .zip
            .entry(name)
            .filter(|e| !e.is_dir())
            .ok_or_else(|| fetch_error(format!("{title}: the archive has no {name}")))?;
        self.zip
            .read(entry, self.limit)
            .map_err(|e| fetch_error(format!("{title}: {e}")))
    }

    fn pin(&self, name: &str) -> Result<&'static str, Error> {
        self.family.pins.get(name).map_err(Error::internal)
    }

    /// The manifest of the bundle in `dir`.
    fn manifest(&self, dir: &str) -> Result<Vec<u8>, Error> {
        self.members
            .iter()
            .find(|r| r.spec.dir == dir)
            .map(|r| r.manifest().as_bytes().to_vec())
            .ok_or_else(|| Error::internal(format!("{}: no rule set in {dir}", self.family.name)))
    }
}

/// The bundles of `family` (whose rule sets are `members`) from its downloaded archive
/// `bytes`.
pub(crate) fn assemble(
    family: &FamilySpec,
    members: &[&FetchedRuleSet],
    bytes: &[u8],
    limit: usize,
) -> Result<Files, Error> {
    let zip = Zip::open(bytes).map_err(|e| fetch_error(format!("{}: {e}", family.title)))?;
    let archive = Archive {
        zip,
        family,
        members,
        limit,
    };
    match family.kind {
        Kind::FacturX => facturx(&archive),
        Kind::XRechnung => xrechnung(&archive),
        Kind::FrCtc => fr_ctc(&archive),
        Kind::Peppol => Err(Error::internal(
            "Peppol is not assembled from a ZIP archive",
        )),
    }
}

/// As `scripts/fetch-facturx.sh`: each profile's stylesheet and code list from the
/// Mustang validator, the Schematron stating the pinned Factur-X version.
fn facturx(archive: &Archive<'_>) -> Result<Files, Error> {
    let facturx = archive.pin("facturx")?;
    let mut files = Files::new();
    for (dir, file, xslt_pin, codedb_pin) in FACTURX_PROFILES {
        let xslt_name = format!("FACTUR-X_{file}.xslt");
        let codedb_name = format!("FACTUR-X_{file}_codedb.xml");
        let xslt = archive.read(&format!("xslt/ZF_250/{xslt_name}"))?;
        check_sha256(
            &format!("xslt/ZF_250/{xslt_name}"),
            &xslt,
            archive.pin(xslt_pin)?,
        )?;
        let codedb = archive.read(&format!("xslt/ZF_250/{codedb_name}"))?;
        check_sha256(
            &format!("xslt/ZF_250/{codedb_name}"),
            &codedb,
            archive.pin(codedb_pin)?,
        )?;
        // "Factur-X release metadata ... Version: 1.09.2 ..." (CRLF line ends, the
        // carriage returns removed as `tr -d '\r'` does).
        let sch_name = format!("schematron/ZF_250/FACTUR-X_{file}.sch");
        let sch = String::from_utf8_lossy(&archive.read(&sch_name)?).replace('\r', "");
        let wanted = format!("Version: {facturx}");
        if !sch.split('\n').any(|l| l.trim_start_matches(' ') == wanted) {
            return Err(fetch_error(format!(
                "{sch_name} does not state Factur-X version {facturx}"
            )));
        }
        files.push((format!("{dir}/{xslt_name}"), xslt));
        files.push((format!("{dir}/{codedb_name}"), codedb));
        files.push((format!("{dir}/bundle.toml"), archive.manifest(dir)?));
    }
    Ok(files)
}

/// As `scripts/fetch-xrechnung.sh`: the four stylesheets of the KoSIT configuration, the
/// XRechnung ones stating the pinned release, the changelog naming the pinned CEN rules.
fn xrechnung(archive: &Archive<'_>) -> Result<Files, Error> {
    let (xrechnung, schematron, cen, configuration) = (
        archive.pin("xrechnung")?,
        archive.pin("schematron")?,
        archive.pin("cen")?,
        archive.pin("configuration")?,
    );
    let mut stylesheets = Vec::new();
    for (name, path, sha, _) in XRECHNUNG_STYLESHEETS {
        let path = archive.family.pins.expand(path).map_err(Error::internal)?;
        let bytes = archive.read(&path)?;
        check_sha256(&path, &bytes, archive.pin(sha)?)?;
        stylesheets.push((name, path, bytes));
    }
    // "Schematron Version 2.6.0 - XRechnung 3.0.2 compatible - UBL - ..."
    let title =
        format!("title=\"Schematron Version {schematron} - XRechnung {xrechnung} compatible");
    for (name, path, bytes) in &stylesheets {
        if name.starts_with("XRechnung-") && !String::from_utf8_lossy(bytes).contains(&title) {
            return Err(fetch_error(format!(
                "{path} is not XRechnung Schematron {schematron}"
            )));
        }
    }
    // "## 2026-08-31 ... * Using [CEN Schematron Rules 1.3.16](...)": the section of the
    // release, up to the next one (as `sed -n '/^## X$/,/^## /p'`).
    let changelog = String::from_utf8_lossy(&archive.read("CHANGELOG.md")?).into_owned();
    let heading = format!("## {configuration}");
    let mut lines = changelog.split('\n').skip_while(|l| *l != heading);
    let mut section = Vec::new();
    if let Some(first) = lines.next() {
        section.push(first);
        for line in lines {
            section.push(line);
            if line.starts_with("## ") {
                break;
            }
        }
    }
    let rules = format!("CEN Schematron Rules {cen}]");
    if !section.iter().any(|l| l.contains(&rules)) {
        return Err(fetch_error(format!(
            "the changelog of the configuration {configuration} does not name CEN Schematron Rules {cen}"
        )));
    }
    let mut files = Files::new();
    for dir in ["ubl", "cii"] {
        let suffix = format!("-{}", dir.to_ascii_uppercase());
        for (name, path, bytes) in &stylesheets {
            if name.ends_with(&suffix) {
                let file = path.rsplit('/').next().unwrap_or(path);
                files.push((format!("{dir}/{file}"), bytes.clone()));
            }
        }
        files.push((format!("{dir}/bundle.toml"), archive.manifest(dir)?));
    }
    Ok(files)
}

/// Whether a file of the package is a compiled stylesheet or a code list (`find -name
/// '*.xslt' -o -name '*_codedb.xml'`).
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "the names the fetch script matches, case-sensitively as find -name does"
)]
fn compiled(name: &str) -> bool {
    let base = name.rsplit('/').next().unwrap_or(name);
    base.ends_with(".xslt") || base.ends_with("_codedb.xml")
}

/// As `scripts/fetch-fr-ctc.sh`: the stylesheets the bundles name, every other compiled
/// stylesheet or code list of the package one of the pinned files, byte for byte.
fn fr_ctc(archive: &Archive<'_>) -> Result<Files, Error> {
    let top = archive
        .family
        .pins
        .expand(FR_CTC_PACK_DIR)
        .map_err(Error::internal)?;
    let prefix = format!("{top}/");
    if !archive
        .zip
        .entries()
        .iter()
        .any(|e| e.name.starts_with(&prefix))
    {
        return Err(fetch_error(format!("the package has no {top}/")));
    }
    let mut known = Vec::new();
    let mut stylesheets = Vec::new();
    for (name, path, sha, codedb) in FR_CTC_STYLESHEETS {
        let bytes = archive.read(&format!("{prefix}{path}"))?;
        check_sha256(path, &bytes, archive.pin(sha)?)?;
        known.push(archive.pin(sha)?);
        if let Some(codedb) = codedb {
            known.push(archive.pin(codedb)?);
        }
        stylesheets.push((name, path, codedb, bytes));
    }
    let mut count = 0;
    for entry in archive.zip.entries() {
        if entry.is_dir() || !entry.name.starts_with(&prefix) || !compiled(&entry.name) {
            continue;
        }
        let bytes = archive
            .zip
            .read(entry, archive.limit)
            .map_err(|e| fetch_error(format!("the package: {e}")))?;
        let actual = super::digest(super::Algorithm::Sha256, &bytes);
        if !known.contains(&actual.as_str()) {
            return Err(fetch_error(format!(
                "{} has SHA-256 {actual}, none of the pinned ones",
                &entry.name[prefix.len()..]
            )));
        }
        count += 1;
    }
    if count == 0 {
        return Err(fetch_error("the package has no compiled stylesheet"));
    }
    let mut files = Files::new();
    for (dir, _, _, names) in FR_CTC_BUNDLES {
        for name in names {
            let (_, path, codedb, bytes) = stylesheets
                .iter()
                .find(|(n, ..)| n == name)
                .ok_or_else(|| Error::internal(format!("no stylesheet {name}")))?;
            files.push((format!("{dir}/{name}.xslt"), bytes.clone()));
            if let Some(side) = fr_ctc_side_file(path) {
                let pin_name =
                    codedb.ok_or_else(|| Error::internal(format!("{path}: no code list pin")))?;
                let content = archive.read(&format!("{prefix}{side}"))?;
                check_sha256(&side, &content, archive.pin(pin_name)?)?;
                let base = side.rsplit('/').next().unwrap_or(&side);
                files.push((format!("{dir}/{base}"), content));
            }
        }
        files.push((format!("{dir}/bundle.toml"), archive.manifest(dir)?));
    }
    Ok(files)
}
