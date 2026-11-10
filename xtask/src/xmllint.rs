//! The XML Schema step of the reference pipelines: xmllint (libxml2), the
//! reference of `einvoice-xsd`, validating documents against the schema a KoSIT
//! scenario names (`validateWithXmlSchema`), as the configuration ships it.
//!
//! xmllint runs with `--huge`: without it, libxml2's parser refuses documents
//! nested deeper than 256 elements, a limit of that parser which the reference
//! validators do not have (`crates/einvoice-xsd/tests/oracle/cii/disagreements.tsv`,
//! `deep-nesting`), and the verdict on such a document would not be a schema
//! verdict. The program is `$XMLLINT`, else `xmllint` on the PATH.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::util::{Error, Result};

/// The verdict of the schema on one document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchemaVerdict {
    Valid,
    Invalid,
    /// Not well-formed for xmllint: no schema verdict.
    Malformed,
}

impl SchemaVerdict {
    /// The word of the `schema` column of the acceptance table.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Invalid => "invalid",
            Self::Malformed => "malformed",
        }
    }
}

/// What validates documents against an XML schema.
pub(crate) trait SchemaCheck {
    /// The verdict of the schema whose entry point is `schema` on each of
    /// `documents`, in order.
    fn verdicts(&self, schema: &Path, documents: &[PathBuf]) -> Result<Vec<SchemaVerdict>>;

    /// What runs the check, for the summary line (program and version).
    fn describe(&self) -> Result<String>;
}

/// xmllint, run on batches of documents.
pub(crate) struct Xmllint {
    program: OsString,
}

/// The number of documents given to one run of xmllint (the command line stays
/// far below the limits of every platform).
const BATCH: usize = 200;

impl Xmllint {
    /// `$XMLLINT`, else `xmllint`.
    pub(crate) fn from_env() -> Self {
        Self {
            program: std::env::var_os("XMLLINT").unwrap_or_else(|| OsString::from("xmllint")),
        }
    }

    fn command(&self) -> Command {
        Command::new(&self.program)
    }

    fn cannot_run(&self, e: &std::io::Error) -> Error {
        Error::new(format!(
            "{} (the schema step of the KoSIT scenarios; set XMLLINT to another program): cannot run: {e}",
            self.program.to_string_lossy()
        ))
    }
}

impl SchemaCheck for Xmllint {
    fn verdicts(&self, schema: &Path, documents: &[PathBuf]) -> Result<Vec<SchemaVerdict>> {
        let mut out = Vec::with_capacity(documents.len());
        for batch in documents.chunks(BATCH) {
            let output = self
                .command()
                .args(["--noout", "--nonet", "--huge", "--schema"])
                .arg(schema)
                .args(batch)
                .output()
                .map_err(|e| self.cannot_run(&e))?;
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("WXS schema") && stderr.contains("failed to compile") {
                return Err(Error::new(format!(
                    "xmllint: the schema {} does not compile:\n{stderr}",
                    schema.display()
                )));
            }
            // Status 0 (all valid), 3 (some invalid) or 1 (some not well-formed);
            // any other is a failure of xmllint itself.
            if !matches!(output.status.code(), Some(0 | 1 | 3)) {
                return Err(Error::new(format!(
                    "xmllint on {}: {}\n{stderr}",
                    schema.display(),
                    output.status
                )));
            }
            out.extend(parse(&stderr, batch)?);
        }
        Ok(out)
    }

    fn describe(&self) -> Result<String> {
        let output = self
            .command()
            .arg("--version")
            .output()
            .map_err(|e| self.cannot_run(&e))?;
        let text = String::from_utf8_lossy(&output.stderr);
        let first = text.lines().next().unwrap_or("").trim();
        if !output.status.success() || first.is_empty() {
            return Err(Error::new(format!(
                "{} --version: {}",
                self.program.to_string_lossy(),
                output.status
            )));
        }
        Ok(first.to_owned())
    }
}

/// The verdicts on `documents` in what xmllint wrote on its standard error: a
/// line `<path> validates` for a valid document, `<path> fails to validate` for
/// an invalid one, neither for one that is not well-formed (its parser errors
/// only). A document named by both lines, or validated with an internal error,
/// is an error.
pub(crate) fn parse(stderr: &str, documents: &[PathBuf]) -> Result<Vec<SchemaVerdict>> {
    documents
        .iter()
        .map(|document| {
            let path = document.to_string_lossy();
            let has = |suffix: &str| {
                let line = format!("{path} {suffix}");
                stderr.lines().any(|l| l == line)
            };
            match (
                has("validates"),
                has("fails to validate"),
                has("validation generated an internal error"),
            ) {
                (true, false, false) => Ok(SchemaVerdict::Valid),
                (false, true, false) => Ok(SchemaVerdict::Invalid),
                (false, false, false) => Ok(SchemaVerdict::Malformed),
                _ => Err(Error::new(format!(
                    "xmllint: no single verdict on {path}:\n{stderr}"
                ))),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{SchemaVerdict, parse};

    #[test]
    fn verdicts_are_read_from_the_result_lines_only() {
        let documents: Vec<PathBuf> = ["d/a.xml", "d/b.xml", "d/c.xml", "d/a b.xml"]
            .iter()
            .map(PathBuf::from)
            .collect();
        // An error message that starts with the path, and ends like a result line,
        // is not one.
        let stderr = "d/a.xml validates\n\
                      d/b.xml:3: element x: Schemas validity error : not d/a.xml fails to validate\n\
                      d/b.xml fails to validate\n\
                      d/c.xml:1: parser error : Start tag expected, '<' not found\n\
                      d/a b.xml validates\n";
        assert_eq!(
            parse(stderr, &documents).unwrap(),
            [
                SchemaVerdict::Valid,
                SchemaVerdict::Invalid,
                SchemaVerdict::Malformed,
                SchemaVerdict::Valid
            ]
        );
        // Both lines for one document, or an internal error: no verdict.
        assert!(
            parse(
                "d/a.xml validates\nd/a.xml fails to validate\n",
                &documents[..1]
            )
            .is_err()
        );
        assert!(
            parse(
                "d/a.xml validation generated an internal error\n",
                &documents[..1]
            )
            .is_err()
        );
    }
}
