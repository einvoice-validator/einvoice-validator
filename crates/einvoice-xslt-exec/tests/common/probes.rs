//! The probes of `tests/probes/`: synthetic stylesheets run on `input.xml`, and the
//! format of what Saxon-HE gave for each (`expected/<name>.txt`):
//!
//! ```text
//! result                  or: error <CODE>
//! warnings <N>            the number of XTDE0540 (ambiguous rule match) warnings, each
//!                         distinct warning once, as Saxon's error reporter prints them
//! pattern-errors <L:CODE> ...
//!                         the dynamic errors recovered while matching a pattern, each
//!                         reported as a warning: the distinct pairs of the line of the
//!                         pattern's instruction and the error code, sorted (the
//!                         messages are not compared: Saxon's quote values where ours
//!                         do not, so that their distinct messages are not ours)
//! messages "<TEXT>"       only when `xsl:message` printed something: what it printed,
//!                         each message followed by a line feed, with `\`, `"`, line
//!                         feeds, carriage returns and tabs escaped as in Rust
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
    /// `(line, code)` of the errors recovered while matching patterns, sorted.
    pub(crate) pattern_errors: Vec<(u32, String)>,
    /// What `xsl:message` printed (see the format).
    pub(crate) messages: String,
    pub(crate) output: String,
}

/// `text` with `\`, `"`, line feeds, carriage returns and tabs escaped.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

/// Undoes [`escape`].
pub(crate) fn unescape(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        out.push(match chars.next() {
            Some('\\') => '\\',
            Some('"') => '"',
            Some('n') => '\n',
            Some('r') => '\r',
            Some('t') => '\t',
            other => return Err(format!("bad escape {other:?} in {text:?}")),
        });
    }
    Ok(out)
}

impl Expected {
    pub(crate) fn render(&self) -> String {
        let status = match &self.error {
            Some(code) => format!("error {code}"),
            None => "result".to_owned(),
        };
        let pattern_errors: String = std::iter::once("pattern-errors".to_owned())
            .chain(
                self.pattern_errors
                    .iter()
                    .map(|(line, code)| format!("{line}:{code}")),
            )
            .collect::<Vec<_>>()
            .join(" ");
        let messages = if self.messages.is_empty() {
            String::new()
        } else {
            format!("messages \"{}\"\n", escape(&self.messages))
        };
        format!(
            "{status}\nwarnings {}\n{pattern_errors}\n{messages}---\n{}",
            self.warnings, self.output
        )
    }

    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let mut rest = text;
        let mut line = || -> Result<&str, String> {
            let (line, after) = rest
                .split_once('\n')
                .ok_or_else(|| "expected file: bad header".to_owned())?;
            rest = after;
            Ok(line)
        };
        let (status, warnings, pattern_errors) = (line()?, line()?, line()?);
        let messages = match line()? {
            "---" => String::new(),
            messages => {
                let quoted = messages
                    .strip_prefix("messages \"")
                    .and_then(|l| l.strip_suffix('"'))
                    .ok_or_else(|| format!("bad messages line {messages:?}"))?;
                if line()? != "---" {
                    return Err("expected file: no --- after the messages".to_owned());
                }
                unescape(quoted)?
            }
        };
        let output = rest;
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
        let bad = || format!("bad pattern-errors line {pattern_errors:?}");
        let pattern_errors = pattern_errors
            .strip_prefix("pattern-errors")
            .ok_or_else(bad)?
            .split_whitespace()
            .map(|pair| {
                let (line, code) = pair.split_once(':').ok_or_else(bad)?;
                Ok((line.parse().map_err(|_| bad())?, code.to_owned()))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            error,
            warnings,
            pattern_errors,
            messages,
            output: output.to_owned(),
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

/// The `(line, code)` of a warning of this crate for an error recovered while matching
/// a pattern (`line <L>: <CODE>: An error occurred matching pattern ...`), `None` for
/// any other warning.
pub(crate) fn pattern_error(warning: &str) -> Option<(u32, String)> {
    let rest = warning.strip_prefix("line ")?;
    let (line, rest) = rest.split_once(": ")?;
    let (code, rest) = rest.split_once(": ")?;
    rest.starts_with("An error occurred matching pattern ")
        .then(|| Some((line.parse().ok()?, code.to_owned())))
        .flatten()
}

pub(crate) fn expected_path(name: &str) -> PathBuf {
    probes_dir().join("expected").join(format!("{name}.txt"))
}
