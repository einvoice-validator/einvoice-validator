//! The result of one document, written in each output format.

use std::fmt::Write as _;

use einvoice_validator::{Error, ErrorKind, Finding, Report, RuleSet, Severity, Verdict};

use crate::json::Json;

/// The version of the JSON report format (`format_version`).
pub(crate) const JSON_FORMAT_VERSION: u64 = 1;

/// One validated document: its name as given on the command line (`-` for standard
/// input) and the result.
#[derive(Debug)]
pub(crate) struct Outcome {
    pub(crate) name: String,
    pub(crate) result: Result<Report, Error>,
}

/// What became of a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Valid,
    /// Schema errors, fatal findings, or not XML.
    Invalid,
    /// No verdict: the document could not be read or validated.
    Error(ErrorKind),
}

impl Status {
    pub(crate) fn of(result: &Result<Report, Error>) -> Self {
        match result {
            Ok(report) => match report.verdict() {
                Verdict::Valid => Self::Valid,
                Verdict::Invalid => Self::Invalid,
            },
            Err(e) if e.kind() == ErrorKind::NotXml => Self::Invalid,
            Err(e) => Self::Error(e.kind()),
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Invalid => "invalid",
            Self::Error(_) => "error",
        }
    }

    /// The exit code this status calls for (see the help text).
    pub(crate) const fn exit_code(self) -> u8 {
        match self {
            Self::Valid => 0,
            Self::Invalid => 1,
            Self::Error(ErrorKind::Io | ErrorKind::Bundle) => 2,
            Self::Error(_) => 3,
        }
    }
}

/// The exit code of a rule set that could not be loaded.
pub(crate) const fn load_exit_code(error: &Error) -> u8 {
    Status::Error(error.kind()).exit_code()
}

/// The message of an error, without the file name the output already shows.
pub(crate) fn error_message(error: &Error) -> String {
    match error {
        Error::Io { source, .. } => source.to_string(),
        other => other.to_string(),
    }
}

/// Terminal colours, or none.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Style {
    pub(crate) colour: bool,
}

impl Style {
    fn paint(self, code: &str, text: &str) -> String {
        if self.colour {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }
}

const RED: &str = "1;31";
const GREEN: &str = "1;32";
const YELLOW: &str = "33";
const MAGENTA: &str = "1;35";

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The text form: a line for the document, then a line per schema error and two per
/// finding.
pub(crate) fn text(outcome: &Outcome, style: Style) -> String {
    let mut out = String::new();
    let name = &outcome.name;
    match &outcome.result {
        Err(e) if e.kind() == ErrorKind::NotXml => {
            let _ = writeln!(out, "{name}: {}: {e}", style.paint(RED, "invalid"));
        }
        Err(e) => {
            let _ = writeln!(
                out,
                "{name}: {} ({}): {}",
                style.paint(MAGENTA, "error"),
                e.kind(),
                error_message(e)
            );
        }
        Ok(report) => report_text(name, report, style, &mut out),
    }
    out
}

/// The text form of a report.
fn report_text(name: &str, report: &Report, style: Style, out: &mut String) {
    let failed = report
        .stylesheets
        .iter()
        .filter(|s| s.failure.is_some())
        .count();
    let counts: Vec<String> = [
        (report.schema_errors.len(), "schema error", "schema errors"),
        (report.count(Severity::Fatal), "fatal", "fatal"),
        (report.count(Severity::Warning), "warning", "warnings"),
        (
            report.count(Severity::Other),
            "other finding",
            "other findings",
        ),
        (failed, "stylesheet error", "stylesheet errors"),
    ]
    .into_iter()
    .filter(|(n, _, _)| *n > 0)
    .map(|(n, one, many)| plural(n, one, many))
    .collect();
    let verdict = match report.verdict() {
        Verdict::Valid => style.paint(GREEN, "valid"),
        Verdict::Invalid => style.paint(RED, "invalid"),
    };
    if counts.is_empty() {
        let _ = writeln!(out, "{name}: {verdict}");
    } else {
        let _ = writeln!(out, "{name}: {verdict}: {}", counts.join(", "));
    }
    for e in &report.schema_errors {
        let _ = writeln!(
            out,
            "  {} {}",
            style.paint(RED, "schema  "),
            abbreviate(&e.to_string())
        );
    }
    for s in &report.stylesheets {
        if let Some(failure) = &s.failure {
            let at = failure
                .line
                .map_or_else(String::new, |l| format!(", line {l}"));
            let _ = writeln!(
                out,
                "  {} stylesheet {} stopped{at}: {}: {}",
                style.paint(MAGENTA, "error   "),
                s.name,
                failure.code,
                failure.message
            );
        }
    }
    for f in report.findings() {
        let label = format!("{:<8}", label(f));
        let label = match f.severity {
            Severity::Fatal => style.paint(RED, &label),
            Severity::Warning => style.paint(YELLOW, &label),
            Severity::Other => label,
        };
        let _ = writeln!(
            out,
            "  {label} {}: {}",
            f.id.as_deref().unwrap_or("(no id)"),
            one_line(&f.text)
        );
        let _ = writeln!(out, "           at {}", f.location);
    }
    for s in &report.stylesheets {
        for w in &s.warnings {
            let _ = writeln!(out, "  note     stylesheet {}: {w}", s.name);
        }
    }
}

/// The UBL namespaces and the XML Schema namespace, written `{uri}` in names, and the
/// prefixes the official UBL rule sets declare for them.
const UBL_PREFIXES: &[(&str, &str)] = &[
    (
        "{urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2}",
        "cbc:",
    ),
    (
        "{urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2}",
        "cac:",
    ),
    (
        "{urn:oasis:names:specification:ubl:schema:xsd:CommonExtensionComponents-2}",
        "ext:",
    ),
    (
        "{urn:oasis:names:specification:ubl:schema:xsd:Invoice-2}",
        "ubl:",
    ),
    (
        "{urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2}",
        "cn:",
    ),
    ("{http://www.w3.org/2001/XMLSchema}", "xs:"),
];

/// Writes the namespaces of [`UBL_PREFIXES`] in `{uri}local` names with their usual
/// prefixes, to keep the text form short (the JSON form keeps the full names).
fn abbreviate(text: &str) -> String {
    let mut out = text.to_owned();
    for (uri, prefix) in UBL_PREFIXES {
        if out.contains(uri) {
            out = out.replace(uri, prefix);
        }
    }
    out
}

/// The flag of a finding, or what it is when it has none.
fn label(f: &Finding) -> &str {
    f.flag.as_deref().unwrap_or(match f.kind {
        einvoice_validator::FindingKind::FailedAssert => "assert",
        einvoice_validator::FindingKind::SuccessfulReport => "report",
    })
}

/// A message on one line: runs of whitespace (the stylesheets keep line breaks and
/// indentation in their message texts) become one space, with none at either end. Only the
/// text form does this; the JSON and SVRL forms keep the text as the stylesheet wrote it.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The closing line of the text form when there was more than one document.
pub(crate) fn text_summary(statuses: &[Status]) -> String {
    let count = |f: fn(&Status) -> bool| statuses.iter().filter(|s| f(s)).count();
    format!(
        "{}: {} valid, {} invalid, {} not validated\n",
        plural(statuses.len(), "document", "documents"),
        count(|s| *s == Status::Valid),
        count(|s| *s == Status::Invalid),
        count(|s| matches!(s, Status::Error(_))),
    )
}

/// The SVRL form: the report of each stylesheet as written, on standard output; the
/// problems on standard error.
pub(crate) fn svrl(outcome: &Outcome) -> (String, String) {
    match &outcome.result {
        Ok(report) => {
            let mut out = String::new();
            let mut err = String::new();
            for s in &report.stylesheets {
                if let Some(failure) = &s.failure {
                    let _ = writeln!(
                        err,
                        "{}: stylesheet {} stopped: {}: {}",
                        outcome.name, s.name, failure.code, failure.message
                    );
                    continue;
                }
                out.push_str(&s.svrl);
                if !s.svrl.ends_with('\n') {
                    out.push('\n');
                }
            }
            (out, err)
        }
        Err(_) => (String::new(), text(outcome, Style { colour: false })),
    }
}

/// The rule set, in the JSON report.
pub(crate) fn json_rule_set(rule_set: &RuleSet) -> Json {
    Json::Object(vec![
        ("name", Json::string(rule_set.name())),
        ("title", Json::string(rule_set.title())),
        ("version", Json::string(rule_set.version())),
        (
            "schema",
            Json::optional(rule_set.schema().map(einvoice_validator::SchemaId::as_str)),
        ),
        (
            "stylesheets",
            Json::Array(
                rule_set
                    .stylesheets()
                    .iter()
                    .map(|s| {
                        Json::Object(vec![
                            ("name", Json::string(s.name())),
                            ("version", Json::string(s.version())),
                            ("path", Json::string(s.path())),
                            ("sha256", Json::string(s.sha256())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// One document, in the JSON report.
pub(crate) fn json(outcome: &Outcome) -> Json {
    let status = Status::of(&outcome.result);
    let (error, report) = match &outcome.result {
        Ok(report) => (Json::Null, Some(report)),
        Err(e) => {
            let (line, column) = match e {
                Error::NotXml { line, column, .. } => (
                    Json::Number(u64::from(*line)),
                    Json::Number(u64::from(*column)),
                ),
                _ => (Json::Null, Json::Null),
            };
            (
                Json::Object(vec![
                    ("kind", Json::string(e.kind().as_str())),
                    ("message", Json::string(error_message(e))),
                    ("line", line),
                    ("column", column),
                ]),
                None,
            )
        }
    };
    let count = |s: Severity| Json::count(report.map_or(0, |r| r.count(s)));
    let schema_errors: Vec<Json> = report
        .map(|r| r.schema_errors.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|e| {
            Json::Object(vec![
                ("kind", Json::string(e.kind.as_str())),
                ("line", Json::Number(u64::from(e.line))),
                ("column", Json::Number(u64::from(e.column))),
                ("element", Json::string(&e.element)),
                ("attribute", Json::optional(e.attribute.as_deref())),
                ("message", Json::string(&e.message)),
            ])
        })
        .collect();
    let mut findings = Vec::new();
    let mut warnings = Vec::new();
    let mut stylesheet_errors = Vec::new();
    for s in report.map(|r| r.stylesheets.as_slice()).unwrap_or_default() {
        for f in &s.findings {
            findings.push(Json::Object(vec![
                ("stylesheet", Json::string(&s.name)),
                ("kind", Json::string(f.kind.as_str())),
                ("id", Json::optional(f.id.as_deref())),
                ("flag", Json::optional(f.flag.as_deref())),
                ("severity", Json::string(f.severity.as_str())),
                ("role", Json::optional(f.role.as_deref())),
                ("location", Json::string(&f.location)),
                ("context", Json::optional(f.context.as_deref())),
                ("pattern", Json::optional(f.pattern.as_deref())),
                ("test", Json::string(&f.test)),
                ("text", Json::string(&f.text)),
            ]));
        }
        for w in &s.warnings {
            warnings.push(Json::Object(vec![
                ("stylesheet", Json::string(&s.name)),
                ("message", Json::string(w)),
            ]));
        }
        if let Some(failure) = &s.failure {
            stylesheet_errors.push(Json::Object(vec![
                ("stylesheet", Json::string(&s.name)),
                ("code", Json::string(&failure.code)),
                (
                    "line",
                    failure
                        .line
                        .map_or(Json::Null, |l| Json::Number(u64::from(l))),
                ),
                ("message", Json::string(&failure.message)),
            ]));
        }
    }
    Json::Object(vec![
        ("path", Json::string(&outcome.name)),
        ("status", Json::string(status.as_str())),
        ("error", error),
        (
            "counts",
            Json::Object(vec![
                (
                    "schema_errors",
                    Json::count(report.map_or(0, |r| r.schema_errors.len())),
                ),
                ("fatal", count(Severity::Fatal)),
                ("warning", count(Severity::Warning)),
                ("other", count(Severity::Other)),
                ("stylesheet_errors", Json::count(stylesheet_errors.len())),
            ]),
        ),
        ("schema_errors", Json::Array(schema_errors)),
        ("findings", Json::Array(findings)),
        ("stylesheet_errors", Json::Array(stylesheet_errors)),
        ("warnings", Json::Array(warnings)),
    ])
}

/// The JSON summary of the run.
pub(crate) fn json_summary(statuses: &[Status]) -> Json {
    let count = |f: fn(&Status) -> bool| Json::count(statuses.iter().filter(|s| f(s)).count());
    Json::Object(vec![
        ("documents", Json::count(statuses.len())),
        ("valid", count(|s| *s == Status::Valid)),
        ("invalid", count(|s| *s == Status::Invalid)),
        ("errors", count(|s| matches!(s, Status::Error(_)))),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_on_one_line() {
        assert_eq!(
            one_line("[BR-01] An Invoice shall have (BT-24).   "),
            "[BR-01] An Invoice shall have (BT-24)."
        );
        assert_eq!(one_line("\n  a\t\tb \r\n c  "), "a b c");
        assert_eq!(one_line("   "), "");
        assert_eq!(one_line("é\u{a0}x"), "é x");
    }

    #[test]
    fn exit_codes() {
        assert_eq!(Status::Valid.exit_code(), 0);
        assert_eq!(Status::Invalid.exit_code(), 1);
        assert_eq!(Status::Error(ErrorKind::Io).exit_code(), 2);
        assert_eq!(Status::Error(ErrorKind::Bundle).exit_code(), 2);
        assert_eq!(Status::Error(ErrorKind::Unsupported).exit_code(), 3);
        assert_eq!(Status::Error(ErrorKind::Transform).exit_code(), 3);
        assert_eq!(Status::Error(ErrorKind::Internal).exit_code(), 3);
    }

    #[test]
    fn colours_only_when_asked() {
        assert_eq!(Style { colour: false }.paint(RED, "x"), "x");
        assert_eq!(Style { colour: true }.paint(RED, "x"), "\x1b[1;31mx\x1b[0m");
    }

    #[test]
    fn ubl_namespaces_are_abbreviated() {
        assert_eq!(
            abbreviate(
                "{urn:oasis:names:specification:ubl:schema:xsd:Invoice-2}Invoice: expected {urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2}ID, {urn:x}y"
            ),
            "ubl:Invoice: expected cbc:ID, {urn:x}y"
        );
    }

    #[test]
    fn summary() {
        assert_eq!(
            text_summary(&[Status::Valid, Status::Invalid, Status::Error(ErrorKind::Io)]),
            "3 documents: 1 valid, 1 invalid, 1 not validated\n"
        );
    }
}
