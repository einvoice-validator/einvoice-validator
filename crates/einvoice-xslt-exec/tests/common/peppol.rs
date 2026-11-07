//! The Peppol BIS Billing 3.0.22 corpus (`corpus/README.md`). None of it is committed:
//! `scripts/fetch-peppol.sh` downloads the release and compiles its two stylesheets into
//! `target/peppol/`, then `cargo xtask peppol` extracts the cases and writes the Saxon-HE
//! goldens of both stylesheets there, and a stamp, `corpus/complete.txt`, last.
//!
//! The cache is used only when the stamp is there and agrees: the same number of cases as
//! `cases.tsv`, and each stylesheet with the SHA-256 pinned in `scripts/fetch-peppol.sh`.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{join, read};

/// The cache directory of the rule set, relative to the workspace root.
pub(crate) const BASE: &str = "target/peppol/peppol-bis-billing-ubl-3.0.22";
/// The script that builds the cache and pins the stylesheets.
pub(crate) const PINS_SCRIPT: &str = "scripts/fetch-peppol.sh";
/// When this variable is set (to anything but an empty string), a missing cache is an
/// error instead of a skipped test: `scripts/check.sh peppol` sets it, after building
/// the cache.
pub(crate) const REQUIRED_VAR: &str = "EINVOICE_PEPPOL_REQUIRED";

/// (name, compiled XSLT relative to [`BASE`], golden directory relative to the corpus
/// directory, name of the SHA-256 pin in [`PINS_SCRIPT`]): Peppol validation runs both.
const STYLESHEETS: [(&str, &str, &str, &str); 2] = [
    (
        "PEPPOL-EN16931-UBL",
        "xslt/PEPPOL-EN16931-UBL.xslt",
        "golden-peppol",
        "peppol_xslt_sha256",
    ),
    (
        "CEN-EN16931-UBL",
        "xslt/CEN-EN16931-UBL.xslt",
        "golden-cen",
        "cen_xslt_sha256",
    ),
];

/// One stylesheet of the rule set.
#[derive(Debug, Clone)]
pub(crate) struct Stylesheet {
    pub(crate) name: &'static str,
    pub(crate) xslt: PathBuf,
    golden_dir: PathBuf,
}

impl Stylesheet {
    /// The golden of a case.
    pub(crate) fn golden(&self, case: &Case) -> PathBuf {
        join(&self.golden_dir, &format!("{}.svrl.txt", case.id))
    }
}

/// One case: its id and its input document (absolute path).
#[derive(Debug, Clone)]
pub(crate) struct Case {
    pub(crate) id: String,
    pub(crate) input: String,
}

/// The cached corpus.
#[derive(Debug)]
pub(crate) struct Corpus {
    pub(crate) cases: Vec<Case>,
    pub(crate) stylesheets: Vec<Stylesheet>,
}

/// Whether a missing cache must fail (see [`REQUIRED_VAR`]).
pub(crate) fn required() -> bool {
    std::env::var_os(REQUIRED_VAR).is_some_and(|v| !v.is_empty())
}

/// The cached corpus, `None` when the cache is absent or incomplete (no stamp). An error
/// when it is there but does not agree with the stamp or the pins.
pub(crate) fn corpus(root: &Path) -> Result<Option<Corpus>, String> {
    let base = join(root, BASE);
    let corpus = base.join("corpus");
    let stamp_path = corpus.join("complete.txt");
    if !stamp_path.exists() {
        return Ok(None);
    }
    let stamp = read(&stamp_path)?;
    let pins = pins(root)?;
    let mut stylesheets = Vec::new();
    for (name, xslt, golden_dir, pin) in STYLESHEETS {
        let expected = pins
            .iter()
            .find(|(n, _)| n == pin)
            .map(|(_, v)| v.as_str())
            .ok_or_else(|| format!("{PINS_SCRIPT}: no pin {pin}"))?;
        let xslt = join(&base, xslt);
        let bytes = std::fs::read(&xslt).map_err(|e| format!("{}: {e}", xslt.display()))?;
        let actual = sha256_hex(&bytes);
        if actual != expected {
            return Err(format!(
                "{}: SHA-256 {actual}, expected {expected} ({pin} in {PINS_SCRIPT}); rebuild the cache with scripts/check.sh peppol",
                xslt.display()
            ));
        }
        if !stamp
            .lines()
            .any(|l| l == format!("stylesheet {name} {expected}"))
        {
            return Err(format!(
                "{}: the goldens were not made with the pinned {name}; rebuild the cache with scripts/check.sh peppol",
                stamp_path.display()
            ));
        }
        stylesheets.push(Stylesheet {
            name,
            xslt,
            golden_dir: corpus.join(golden_dir),
        });
    }

    let cases = read(&corpus.join("cases.tsv"))?
        .lines()
        .skip(1)
        .map(|line| {
            let mut columns = line.split('\t');
            let (Some(id), Some(input)) = (columns.next(), columns.next()) else {
                return Err(format!("cases.tsv: bad line {line:?}"));
            };
            let input = std::path::absolute(join(&base, input)).map_err(|e| e.to_string())?;
            let input = input
                .to_str()
                .ok_or_else(|| format!("{}: not UTF-8", input.display()))?
                .to_owned();
            Ok(Case {
                id: id.to_owned(),
                input,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if !stamp.lines().any(|l| l == format!("cases {}", cases.len())) {
        return Err(format!(
            "{}: does not record {} cases; rebuild the cache with scripts/check.sh peppol",
            stamp_path.display(),
            cases.len()
        ));
    }
    Ok(Some(Corpus { cases, stylesheets }))
}

/// The `name=value` lines between the pin markers of [`PINS_SCRIPT`].
fn pins(root: &Path) -> Result<Vec<(String, String)>, String> {
    let text = read(&join(root, PINS_SCRIPT))?;
    let mut lines = text.lines().skip_while(|l| !l.starts_with("# --- pins "));
    lines
        .next()
        .ok_or_else(|| format!("{PINS_SCRIPT}: no pins"))?;
    Ok(lines
        .take_while(|l| !l.starts_with("# --- end of pins "))
        .filter_map(|l| l.split_once('='))
        .map(|(n, v)| (n.to_owned(), v.to_owned()))
        .collect())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        })
}
