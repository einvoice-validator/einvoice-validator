//! The probes of `tests/probes/`: synthetic stylesheets run on `input.xml`, and the
//! format of what Saxon-HE gave for each (`expected/<name>.txt`):
//!
//! ```text
//! result                  or: error <CODE>
//! warnings <N>            the number of XTDE0540 (ambiguous rule match) warnings, each
//!                         distinct warning once, as Saxon's error reporter prints them
//! ---
//! <the serialized result, exactly as Saxon wrote it; empty after an error>
//! ```

#![allow(
    dead_code,
    reason = "shared by the probe test and the oracle example, each using a part"
)]

use std::path::{Path, PathBuf};

/// What Saxon gave for a probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Expected {
    /// The error code, when the transformation failed.
    pub(crate) error: Option<String>,
    pub(crate) warnings: usize,
    pub(crate) output: String,
}

impl Expected {
    pub(crate) fn render(&self) -> String {
        let status = match &self.error {
            Some(code) => format!("error {code}"),
            None => "result".to_owned(),
        };
        format!("{status}\nwarnings {}\n---\n{}", self.warnings, self.output)
    }

    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let mut parts = text.splitn(4, '\n');
        let (Some(status), Some(warnings), Some("---")) =
            (parts.next(), parts.next(), parts.next())
        else {
            return Err("expected file: bad header".to_owned());
        };
        let error = match status {
            "result" => None,
            s => Some(
                s.strip_prefix("error ")
                    .ok_or_else(|| format!("bad status line {s:?}"))?
                    .to_owned(),
            ),
        };
        let warnings = warnings
            .strip_prefix("warnings ")
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| format!("bad warnings line {warnings:?}"))?;
        Ok(Self {
            error,
            warnings,
            output: parts.next().unwrap_or("").to_owned(),
        })
    }
}

pub(crate) fn probes_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("probes")
}

/// The probe stylesheets, by name (file stem), sorted.
pub(crate) fn probes() -> Result<Vec<String>, String> {
    let dir = probes_dir();
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|e| e == "xsl")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            names.push(stem.to_owned());
        }
    }
    names.sort();
    Ok(names)
}

pub(crate) fn expected_path(name: &str) -> PathBuf {
    probes_dir().join("expected").join(format!("{name}.txt"))
}
