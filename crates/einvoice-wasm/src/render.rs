//! The values returned to JavaScript, as JSON: the report of a document, mirroring a
//! document of the JSON format of `einvoice-validate` (version 1), the raw SVRL reports,
//! the rule set, the licences, and the details of an error.

use einvoice_validator::{Error, LimitExceeded, Report, RuleSet, Severity, Verdict};

use crate::json::Json;

/// The name of the package, in the `tool` member of a report.
pub(crate) const PACKAGE: &str = "einvoice-validator";

/// The version of this crate and of the package.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The `format` member of a report.
pub(crate) const FORMAT: &str = "einvoice-validator";

/// The version of the report format (`format_version`): the document members are those of
/// a document of the JSON format of `einvoice-validate`, version 1.
pub(crate) const FORMAT_VERSION: u64 = 1;

/// The rule set, as `einvoice-validate` writes it in its JSON report.
pub(crate) fn rule_set(rule_set: &RuleSet) -> Json {
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

/// The report of one validation: the header (format, tool, rule set, document URI), then
/// the members of a document of the JSON format of `einvoice-validate`, version 1, in the
/// same order, without `path`. A document that is not well-formed XML has the status
/// `invalid` and its `error` set, as there; any other error is returned, to be thrown.
///
/// # Errors
///
/// Every error of `result` but [`Error::NotXml`].
pub(crate) fn report(
    rule_set: &RuleSet,
    document_uri: Option<&str>,
    result: Result<Report, Error>,
) -> Result<Json, Error> {
    let (status, error, report) = match result {
        Ok(report) => {
            let status = match report.verdict() {
                Verdict::Valid => "valid",
                Verdict::Invalid => "invalid",
            };
            (status, Json::Null, Some(report))
        }
        Err(e @ Error::NotXml { .. }) => ("invalid", error_json(&e), None),
        Err(e) => return Err(e),
    };
    let report = report.as_ref();
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
                ("line", Json::line(failure.line)),
                ("message", Json::string(&failure.message)),
            ]));
        }
    }
    Ok(Json::Object(vec![
        ("format", Json::string(FORMAT)),
        ("format_version", Json::Number(FORMAT_VERSION)),
        ("tool", tool()),
        ("rule_set", self::rule_set(rule_set)),
        ("document_uri", Json::optional(document_uri)),
        ("status", Json::string(status)),
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
    ]))
}

/// The `tool` member of a report.
fn tool() -> Json {
    Json::Object(vec![
        ("name", Json::string(PACKAGE)),
        ("version", Json::string(VERSION)),
    ])
}

/// The SVRL report of each stylesheet, in the order they ran, exactly as the stylesheet
/// wrote it.
///
/// # Errors
///
/// Every error of `result`, and [`Error::Transform`] for a stylesheet that a dynamic
/// error stopped (which only happens on a document with schema errors): there is no SVRL
/// report to return.
pub(crate) fn svrl(result: Result<Report, Error>) -> Result<Json, Error> {
    let report = result?;
    let mut out = Vec::with_capacity(report.stylesheets.len());
    for s in report.stylesheets {
        if let Some(failure) = s.failure {
            return Err(Error::Transform {
                stylesheet: s.name,
                code: failure.code,
                line: failure.line,
                message: failure.message,
            });
        }
        let svrl = s.svrl().to_text();
        out.push(Json::Object(vec![
            ("stylesheet", Json::String(s.name)),
            ("version", Json::String(s.version)),
            ("sha256", Json::String(s.sha256)),
            ("svrl", Json::String(svrl)),
        ]));
    }
    Ok(Json::Array(out))
}

/// What the MIT licence covers in the package (the text is that of the library crates,
/// which `einvoice_validator::licences` gives for their code).
const MIT_COVERS: &str = "the code of the npm package einvoice-validator (crate einvoice-wasm) \
    and of the crates of the project it is built from (einvoice-validator, einvoice-bigint, \
    einvoice-svrl, einvoice-xdm, einvoice-xml, einvoice-xpath, einvoice-xpath-eval, \
    einvoice-xsd, einvoice-xslt, einvoice-xslt-exec)";

/// The licences of what the package embeds, with their texts: those of
/// `einvoice_validator::licences`, the MIT licence covering this crate's code too.
pub(crate) fn licences() -> Json {
    Json::Array(
        einvoice_validator::licences()
            .into_iter()
            .map(|l| {
                let covers = if l.id == "MIT" { MIT_COVERS } else { l.covers };
                Json::Object(vec![
                    ("id", Json::string(l.id)),
                    ("covers", Json::string(covers)),
                    ("text", Json::string(l.text)),
                ])
            })
            .collect(),
    )
}

/// The names of the built-in rule sets.
pub(crate) fn builtin_rule_sets() -> Json {
    Json::Array(
        einvoice_validator::BUILTIN_RULE_SETS
            .iter()
            .map(|&name| Json::string(name))
            .collect(),
    )
}

/// The `error` member of a report, `{ kind, message, line, column }` as
/// `einvoice-validate` writes it; line and column are set for `not-xml` only.
fn error_json(error: &Error) -> Json {
    let (line, column) = match error {
        Error::NotXml { line, column, .. } => (Json::line(Some(*line)), Json::line(Some(*column))),
        _ => (Json::Null, Json::Null),
    };
    Json::Object(vec![
        ("kind", Json::string(error.kind().as_str())),
        ("message", Json::string(error.to_string())),
        ("line", line),
        ("column", column),
    ])
}

/// What a thrown error carries besides its message: the kind of the error, then its
/// details (`line` and `column` for `not-xml`; `stylesheet`, `code` and `line` for
/// `transform`; `path` for `io` and for a checksum mismatch).
pub(crate) fn error_details(error: &Error) -> Vec<(&'static str, Json)> {
    let mut out = vec![("kind", Json::string(error.kind().as_str()))];
    match error {
        Error::NotXml { line, column, .. } => {
            out.push(("line", Json::line(Some(*line))));
            out.push(("column", Json::line(Some(*column))));
        }
        Error::Transform {
            stylesheet,
            code,
            line,
            ..
        } => {
            out.push(("stylesheet", Json::string(stylesheet)));
            out.push(("code", Json::string(code)));
            out.push(("line", Json::line(*line)));
        }
        Error::Limit { limit, .. } => {
            let name = match limit {
                LimitExceeded::Depth { line, column, .. } => {
                    out.push(("line", Json::line(Some(*line))));
                    out.push(("column", Json::line(Some(*column))));
                    "depth"
                }
                LimitExceeded::Nesting { .. } => "nesting",
                LimitExceeded::Stack { .. } => "stack",
                // A limit of a later version of the library.
                _ => "other",
            };
            out.insert(1, ("limit", Json::string(name)));
            out.push(("max", Json::line(Some(limit.max()))));
        }
        Error::Checksum { path, .. } => out.push(("path", Json::string(path))),
        Error::Io { path, .. } => out.push(("path", Json::string(path.display().to_string()))),
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use einvoice_validator::{Clock, ValidateOptions, Validator};

    use super::*;

    /// An official example, valid.
    const VALID: &[u8] = include_bytes!(
        "../../../artefacts/cen-en16931-ubl-1.3.17/ubl/examples/ubl-tc434-example1.xml"
    );

    /// VEFA test BR-02.002: no invoice number, BR-02 fails, and the schema misses it.
    const BR_02: &[u8] = include_bytes!(
        "../../../corpus/cen-en16931-ubl-1.3.17/vefa/Invoice-unit-UBL/BR-02.002.xml"
    );

    fn validate(document: &[u8], uri: Option<&str>) -> (Validator, Result<Report, Error>) {
        let validator = Validator::cen_ubl().unwrap();
        let clock = Clock::from_unix_millis(60, 1_789_000_123_456).unwrap();
        let mut options = ValidateOptions::new().clock(clock);
        if let Some(uri) = uri {
            options = options.document_uri(uri);
        }
        let result = validator.validate_with(document, &options);
        (validator, result)
    }

    /// A member of an object, by name.
    fn member<'a>(json: &'a Json, name: &str) -> &'a Json {
        match json {
            Json::Object(members) => &members.iter().find(|(k, _)| *k == name).unwrap().1,
            other => panic!("not an object: {other:?}"),
        }
    }

    fn keys(json: &Json) -> Vec<&'static str> {
        match json {
            Json::Object(members) => members.iter().map(|(k, _)| *k).collect(),
            other => panic!("not an object: {other:?}"),
        }
    }

    #[test]
    fn a_valid_document() {
        let (validator, result) = validate(VALID, Some("urn:example:1"));
        let json = report(validator.rule_set(), Some("urn:example:1"), result).unwrap();
        assert_eq!(
            keys(&json),
            [
                "format",
                "format_version",
                "tool",
                "rule_set",
                "document_uri",
                "status",
                "error",
                "counts",
                "schema_errors",
                "findings",
                "stylesheet_errors",
                "warnings"
            ]
        );
        assert_eq!(member(&json, "format"), &Json::string("einvoice-validator"));
        assert_eq!(member(&json, "format_version"), &Json::Number(1));
        assert_eq!(member(&json, "status"), &Json::string("valid"));
        assert_eq!(member(&json, "error"), &Json::Null);
        assert_eq!(
            member(&json, "document_uri"),
            &Json::string("urn:example:1")
        );
        assert_eq!(
            member(&json, "counts").text(),
            r#"{"schema_errors":0,"fatal":0,"warning":0,"other":0,"stylesheet_errors":0}"#
        );
        assert_eq!(
            member(&json, "tool").text(),
            format!(r#"{{"name":"einvoice-validator","version":"{VERSION}"}}"#)
        );
        let rule_set = member(&json, "rule_set");
        assert_eq!(
            keys(rule_set),
            ["name", "title", "version", "schema", "stylesheets"]
        );
        assert_eq!(member(rule_set, "name"), &Json::string("cen-ubl"));
        assert_eq!(member(rule_set, "version"), &Json::string("1.3.17"));
        assert_eq!(member(rule_set, "schema"), &Json::string("ubl-2.1"));
        let Json::Array(stylesheets) = member(rule_set, "stylesheets") else {
            panic!("stylesheets")
        };
        assert_eq!(keys(&stylesheets[0]), ["name", "version", "path", "sha256"]);
    }

    #[test]
    fn an_invalid_document() {
        let (validator, result) = validate(BR_02, None);
        let json = report(validator.rule_set(), None, result).unwrap();
        assert_eq!(member(&json, "status"), &Json::string("invalid"));
        assert_eq!(member(&json, "document_uri"), &Json::Null);
        let Json::Array(findings) = member(&json, "findings") else {
            panic!("findings")
        };
        let br02 = findings
            .iter()
            .find(|f| member(f, "id") == &Json::string("BR-02"))
            .unwrap();
        assert_eq!(
            keys(br02),
            [
                "stylesheet",
                "kind",
                "id",
                "flag",
                "severity",
                "role",
                "location",
                "context",
                "pattern",
                "test",
                "text"
            ]
        );
        assert_eq!(member(br02, "kind"), &Json::string("failed-assert"));
        assert_eq!(member(br02, "flag"), &Json::string("fatal"));
        assert_eq!(member(br02, "severity"), &Json::string("fatal"));
        assert_eq!(
            member(br02, "text"),
            &Json::string("[BR-02]-An Invoice shall have an Invoice number (BT-1).")
        );
        let Json::Array(schema_errors) = member(&json, "schema_errors") else {
            panic!("schema_errors")
        };
        assert_eq!(schema_errors.len(), 1);
        assert_eq!(
            keys(&schema_errors[0]),
            ["kind", "line", "column", "element", "attribute", "message"]
        );
        assert_eq!(
            member(&schema_errors[0], "kind"),
            &Json::string("missing-element")
        );
    }

    #[test]
    fn a_document_that_is_not_xml_is_invalid() {
        let (validator, result) = validate(b"<Invoice>\n<a></Invoice>", None);
        let json = report(validator.rule_set(), None, result).unwrap();
        assert_eq!(member(&json, "status"), &Json::string("invalid"));
        let error = member(&json, "error");
        assert_eq!(keys(error), ["kind", "message", "line", "column"]);
        assert_eq!(member(error, "kind"), &Json::string("not-xml"));
        assert_eq!(member(error, "line"), &Json::Number(2));
        assert_eq!(member(&json, "findings"), &Json::Array(Vec::new()));
        // validateSvrl has no report to give: it throws.
        let (_, result) = validate(b"<Invoice>\n<a></Invoice>", None);
        let e = svrl(result).unwrap_err();
        assert_eq!(e.kind().as_str(), "not-xml");
        assert_eq!(
            Json::Object(error_details(&e)).text(),
            r#"{"kind":"not-xml","line":2,"column":4}"#
        );
    }

    #[test]
    fn other_errors_are_returned() {
        let (validator, result) = validate(b"<?xml version='1.0' encoding='Shift_JIS'?><a/>", None);
        let e = report(validator.rule_set(), None, result).unwrap_err();
        assert_eq!(e.kind().as_str(), "unsupported");
        assert_eq!(
            Json::Object(error_details(&e)).text(),
            r#"{"kind":"unsupported"}"#
        );
        let e = Validator::builtin("no-such-rule-set").unwrap_err();
        assert_eq!(e.kind().as_str(), "bundle");
        let transform = Error::Transform {
            stylesheet: "S".to_owned(),
            code: "FORG0001".to_owned(),
            line: None,
            message: "m".to_owned(),
        };
        assert_eq!(
            Json::Object(error_details(&transform)).text(),
            r#"{"kind":"transform","stylesheet":"S","code":"FORG0001","line":null}"#
        );
    }

    #[test]
    fn raw_svrl() {
        let (validator, result) = validate(BR_02, Some("urn:example:2"));
        let Json::Array(reports) = svrl(result).unwrap() else {
            panic!("not an array")
        };
        assert_eq!(reports.len(), validator.rule_set().stylesheets().len());
        assert_eq!(
            keys(&reports[0]),
            ["stylesheet", "version", "sha256", "svrl"]
        );
        let Json::String(text) = member(&reports[0], "svrl") else {
            panic!("svrl")
        };
        assert!(text.contains("schematron-output"), "{text}");
        assert!(text.contains(r#"id="BR-02""#), "{text}");
        assert_eq!(
            member(&reports[0], "sha256"),
            &Json::string(validator.rule_set().stylesheets()[0].sha256())
        );
    }

    #[test]
    fn licences_and_rule_sets() {
        let Json::Array(licences) = licences() else {
            panic!("not an array")
        };
        let ids: Vec<String> = licences.iter().map(|l| member(l, "id").text()).collect();
        assert_eq!(
            ids,
            [
                r#""MIT""#,
                r#""EUPL-1.2""#,
                r#""LicenseRef-OASIS-Copyright-Notice""#,
                r#""LicenseRef-UN-CEFACT-Copyright-Notice""#,
                r#""W3C-19980720""#
            ]
        );
        // The rule sets of the features of this build: both by default.
        let names: Vec<String> = features().iter().map(|f| format!("{f:?}")).collect();
        assert_eq!(builtin_rule_sets().text(), format!("[{}]", names.join(",")));
        if cfg!(all(feature = "cen-ubl", feature = "cen-cii")) {
            assert_eq!(builtin_rule_sets().text(), r#"["cen-ubl","cen-cii"]"#);
        }
    }

    #[test]
    fn limits_are_errors_with_their_details() {
        let validator = Validator::cen_ubl().unwrap();
        let clock = Clock::from_unix_millis(0, 0).unwrap();
        let options = ValidateOptions::new().clock(clock).max_depth(2);
        let result = validator.validate_with(b"<a>\n <b>\n  <c/></b></a>", &options);
        let e = report(validator.rule_set(), None, result).unwrap_err();
        assert_eq!(e.kind().as_str(), "unsupported");
        assert_eq!(
            Json::Object(error_details(&e)).text(),
            r#"{"kind":"unsupported","limit":"depth","line":3,"column":3,"max":2}"#
        );
        let stack = Error::Limit {
            limit: LimitExceeded::Stack { max: 655_360 },
            message: "m".to_owned(),
        };
        assert_eq!(
            Json::Object(error_details(&stack)).text(),
            r#"{"kind":"unsupported","limit":"stack","max":655360}"#
        );
        let nesting = Error::Limit {
            limit: LimitExceeded::Nesting { max: 576 },
            message: "m".to_owned(),
        };
        assert_eq!(
            Json::Object(error_details(&nesting)).text(),
            r#"{"kind":"unsupported","limit":"nesting","max":576}"#
        );
    }

    /// The features of this build of the crate.
    fn features() -> Vec<&'static str> {
        let mut features = Vec::new();
        if cfg!(feature = "cen-ubl") {
            features.push("cen-ubl");
        }
        if cfg!(feature = "cen-cii") {
            features.push("cen-cii");
        }
        features
    }

    /// The MIT entry names this crate and exactly the workspace crates it is built from
    /// with the features of this build (`cargo tree`), each shipping the repository's
    /// LICENSE, which is its text; every licence term of those crates is listed, and the
    /// crate's licence expression is the union of those terms.
    #[test]
    fn the_mit_licence_names_exactly_the_crates_of_the_package() {
        use einvoice_testkit::workspace::{self, Features};

        let Json::Array(licences) = licences() else {
            panic!("not an array")
        };
        let Json::String(covers) = member(&licences[0], "covers") else {
            panic!("covers")
        };
        assert!(
            covers.starts_with(
                "the code of the npm package einvoice-validator (crate einvoice-wasm) and of the crates "
            ),
            "{covers}"
        );
        let features = features();
        let crates =
            workspace::dependencies(env!("CARGO_PKG_NAME"), Features::Only(&features)).unwrap();
        let others: Vec<&str> = crates
            .iter()
            .map(|c| c.name.as_str())
            .filter(|name| *name != env!("CARGO_PKG_NAME"))
            .collect();
        assert_eq!(workspace::named_crates(covers).unwrap(), others, "{covers}");
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let licence = std::fs::read_to_string(dir.join("../../LICENSE")).unwrap();
        assert_eq!(member(&licences[0], "text"), &Json::String(licence.clone()));
        for c in crates.iter().filter(|c| c.name != env!("CARGO_PKG_NAME")) {
            assert_eq!(
                std::fs::read_to_string(c.dir.join("LICENSES/MIT.txt")).unwrap(),
                licence,
                "{}",
                c.name
            );
        }
        let ids: std::collections::BTreeSet<String> = licences
            .iter()
            .map(|l| {
                let Json::String(id) = member(l, "id") else {
                    panic!("id")
                };
                id.clone()
            })
            .collect();
        let mut terms = std::collections::BTreeSet::new();
        for c in &crates {
            for term in c.license_terms().unwrap() {
                terms.insert(term.to_owned());
            }
        }
        assert_eq!(ids, terms);
        assert_eq!(
            env!("CARGO_PKG_LICENSE")
                .split(" AND ")
                .map(str::to_owned)
                .collect::<std::collections::BTreeSet<_>>(),
            terms
        );
    }
}
