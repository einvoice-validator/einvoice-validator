//! The result of validating one document.

use std::fmt;

use roxmltree::{Node, ParsingOptions};

pub use einvoice_xsd::{ErrorKind as SchemaErrorKind, ValidationError as SchemaError};

use crate::Error;

/// The SVRL namespace.
const SVRL_NS: &str = "http://purl.oclc.org/dsdl/svrl";

/// The overall verdict on a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verdict {
    /// No schema error and no finding with severity [`Severity::Fatal`].
    Valid,
    /// At least one schema error or one fatal finding.
    Invalid,
}

impl Verdict {
    /// `valid` or `invalid`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Invalid => "invalid",
        }
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The severity of a finding, from its Schematron `flag`, as the official rule sets use
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    /// `flag="fatal"`: the document does not conform; the verdict is
    /// [`Verdict::Invalid`].
    Fatal,
    /// `flag="warning"`: reported, the document still conforms.
    Warning,
    /// Any other flag, or none: reported, the document still conforms.
    Other,
}

impl Severity {
    /// The severity a Schematron flag stands for.
    #[must_use]
    pub fn from_flag(flag: Option<&str>) -> Self {
        match flag {
            Some("fatal") => Self::Fatal,
            Some("warning") => Self::Warning,
            _ => Self::Other,
        }
    }

    /// `fatal`, `warning` or `other`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fatal => "fatal",
            Self::Warning => "warning",
            Self::Other => "other",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a finding is in the SVRL report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FindingKind {
    /// `svrl:failed-assert`: an assertion that does not hold.
    FailedAssert,
    /// `svrl:successful-report`: a report whose condition holds.
    SuccessfulReport,
}

impl FindingKind {
    /// `failed-assert` or `successful-report`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FailedAssert => "failed-assert",
            Self::SuccessfulReport => "successful-report",
        }
    }
}

impl fmt::Display for FindingKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A failed assert or a successful report of a stylesheet, as the SVRL report states it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Finding {
    /// Failed assert or successful report.
    pub kind: FindingKind,
    /// The rule id (`BR-02`), if the rule has one.
    pub id: Option<String>,
    /// The Schematron flag (`fatal`, `warning`), if any.
    pub flag: Option<String>,
    /// The severity the flag stands for.
    pub severity: Severity,
    /// The Schematron role, if any.
    pub role: Option<String>,
    /// The XPath of the node concerned, as the stylesheet writes it.
    pub location: String,
    /// The context of the rule that fired (its `svrl:fired-rule/@context`).
    pub context: Option<String>,
    /// The id of the pattern the rule belongs to (`svrl:active-pattern/@id`).
    pub pattern: Option<String>,
    /// The XPath test of the assert or report.
    pub test: String,
    /// The message, whitespace normalised.
    pub text: String,
}

/// What one stylesheet of the rule set reported on a document.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StylesheetReport {
    /// The stylesheet, by its name in the bundle.
    pub name: String,
    /// The version of the rule set it implements.
    pub version: String,
    /// The SHA-256 of the stylesheet that ran.
    pub sha256: String,
    /// The SVRL report, exactly as the stylesheet wrote it.
    pub svrl: String,
    /// Its failed asserts and successful reports, in report order.
    pub findings: Vec<Finding>,
    /// Recoverable conditions the transformation reported (ambiguous rule matches,
    /// dynamic errors raised while matching a pattern, after which the rule did not
    /// match), as the reference engine reports them: each distinct message once.
    pub warnings: Vec<String>,
    /// The dynamic error that stopped the stylesheet, if it did not finish: then
    /// `svrl` is empty and there are no findings. This only happens in a report with
    /// schema errors (see [`Validator::validate_with`](crate::Validator::validate_with)).
    pub failure: Option<StylesheetFailure>,
    pub(crate) document_uri: Option<String>,
}

/// A dynamic error raised by a stylesheet on a document (the reference engine raises it
/// as well), which stopped the stylesheet.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StylesheetFailure {
    /// The XPath or XSLT error code (`FORG0001`, ...).
    pub code: String,
    /// The line of the stylesheet instruction, when known.
    pub line: Option<u32>,
    /// The explanation.
    pub message: String,
}

impl StylesheetReport {
    /// The SVRL report in the canonical text form of the `einvoice-svrl` crate (version
    /// 1), in which the document's URI is written `{document}`: the form the expected
    /// results of the reference engine are stored in.
    ///
    /// # Errors
    ///
    /// [`Error::Transform`] when the stylesheet did not finish (there is no report);
    /// [`Error::Internal`] when the report holds something the canonical form cannot
    /// represent.
    pub fn canonical_svrl(&self) -> Result<String, Error> {
        if let Some(failure) = &self.failure {
            return Err(Error::Transform {
                stylesheet: self.name.clone(),
                code: failure.code.clone(),
                line: failure.line,
                message: failure.message.clone(),
            });
        }
        let uri = self.document_uri.as_deref();
        einvoice_svrl::canonicalize(&self.svrl, &|u| Some(u) == uri)
            .map_err(|e| Error::internal(format!("stylesheet {}: {e}", self.name)))
    }

    pub(crate) fn new(
        entry: &crate::StylesheetEntry,
        svrl: String,
        warnings: Vec<String>,
        document_uri: Option<&str>,
    ) -> Result<Self, Error> {
        // A stylesheet that does not write SVRL is not a validation stylesheet: the
        // bundle is wrong.
        let findings = findings(&svrl).map_err(|m| {
            Error::bundle(format!(
                "stylesheet {}: its result is not an SVRL report: {m}",
                entry.name()
            ))
        })?;
        Ok(Self {
            name: entry.name().to_owned(),
            version: entry.version().to_owned(),
            sha256: entry.sha256().to_owned(),
            svrl,
            findings,
            warnings,
            failure: None,
            document_uri: document_uri.map(str::to_owned),
        })
    }

    pub(crate) fn failed(
        entry: &crate::StylesheetEntry,
        failure: StylesheetFailure,
        document_uri: Option<&str>,
    ) -> Self {
        Self {
            name: entry.name().to_owned(),
            version: entry.version().to_owned(),
            sha256: entry.sha256().to_owned(),
            svrl: String::new(),
            findings: Vec::new(),
            warnings: Vec::new(),
            failure: Some(failure),
            document_uri: document_uri.map(str::to_owned),
        }
    }
}

/// The rule set a report was produced with.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RuleSetVersion {
    /// Its short name (`cen-ubl`).
    pub name: String,
    /// Its title.
    pub title: String,
    /// Its version (`1.3.17`).
    pub version: String,
    /// The schema the document was validated against, if any.
    pub schema: Option<crate::SchemaId>,
}

/// The result of validating one document: the schema errors, then what each stylesheet
/// of the rule set reported.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Report {
    /// The rule set used.
    pub rule_set: RuleSetVersion,
    /// The URI the document was given, if any (`document-uri(/)` in the stylesheets).
    pub document_uri: Option<String>,
    /// The schema validity errors, sorted by position; empty when the document is valid
    /// against the schema, or when the rule set has no schema.
    pub schema_errors: Vec<SchemaError>,
    /// One report per stylesheet, in the order they ran.
    pub stylesheets: Vec<StylesheetReport>,
}

impl Report {
    /// [`Verdict::Valid`] if there is no schema error and no fatal finding.
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        if self.schema_errors.is_empty() && self.count(Severity::Fatal) == 0 {
            Verdict::Valid
        } else {
            Verdict::Invalid
        }
    }

    /// Whether every stylesheet ran to the end (see [`StylesheetReport::failure`]).
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.stylesheets.iter().all(|s| s.failure.is_none())
    }

    /// Whether the verdict is [`Verdict::Valid`].
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.verdict() == Verdict::Valid
    }

    /// Every finding of every stylesheet, in order.
    pub fn findings(&self) -> impl Iterator<Item = &Finding> {
        self.stylesheets.iter().flat_map(|s| s.findings.iter())
    }

    /// The number of findings with the given severity.
    #[must_use]
    pub fn count(&self, severity: Severity) -> usize {
        self.findings().filter(|f| f.severity == severity).count()
    }
}

/// The failed asserts and successful reports of an SVRL report, with the context of the
/// last fired rule and the id of the last active pattern before each.
fn findings(svrl: &str) -> Result<Vec<Finding>, String> {
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: u32::MAX,
        ..ParsingOptions::default()
    };
    let doc = roxmltree::Document::parse_with_options(svrl, options).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if !is_svrl(root, "schematron-output") {
        return Err("the root is not svrl:schematron-output".to_owned());
    }
    let mut out = Vec::new();
    let mut pattern: Option<String> = None;
    let mut context: Option<String> = None;
    for child in root.children().filter(Node::is_element) {
        if child.tag_name().namespace() != Some(SVRL_NS) {
            continue;
        }
        let attribute = |name: &str| child.attribute(name).map(str::to_owned);
        let kind = match child.tag_name().name() {
            "active-pattern" => {
                pattern = attribute("id").or_else(|| attribute("name"));
                context = None;
                continue;
            }
            "fired-rule" => {
                context = attribute("context");
                continue;
            }
            "failed-assert" => FindingKind::FailedAssert,
            "successful-report" => FindingKind::SuccessfulReport,
            _ => continue,
        };
        let text: String = child
            .children()
            .filter(|n| is_svrl(*n, "text"))
            .flat_map(|n| {
                n.descendants()
                    .filter(Node::is_text)
                    .filter_map(|d| d.text())
            })
            .collect();
        let flag = attribute("flag");
        out.push(Finding {
            kind,
            id: attribute("id"),
            severity: Severity::from_flag(flag.as_deref()),
            flag,
            role: attribute("role"),
            location: attribute("location").unwrap_or_default(),
            context: context.clone(),
            pattern: pattern.clone(),
            test: attribute("test").unwrap_or_default(),
            text: einvoice_svrl::normalize_space(&text),
        });
    }
    Ok(out)
}

fn is_svrl(node: Node<'_, '_>, name: &str) -> bool {
    node.is_element()
        && node.tag_name().namespace() == Some(SVRL_NS)
        && node.tag_name().name() == name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_findings_with_their_rule_and_pattern() {
        let svrl = r#"<svrl:schematron-output xmlns:svrl="http://purl.oclc.org/dsdl/svrl">
  <svrl:active-pattern id="P1"/>
  <svrl:fired-rule context="/a"/>
  <svrl:failed-assert id="R-1" flag="fatal" location="/a[1]" test="b">
    <svrl:text>  [R-1] two
      lines </svrl:text>
  </svrl:failed-assert>
  <svrl:active-pattern name="P2"/>
  <svrl:successful-report id="R-2" flag="warning" role="r" location="/" test="c"><svrl:text/></svrl:successful-report>
  <svrl:fired-rule context="/z"/>
  <svrl:failed-assert location="/" test="d"/>
</svrl:schematron-output>"#;
        let f = findings(svrl).unwrap();
        assert_eq!(f.len(), 3);
        assert_eq!(f[0].kind, FindingKind::FailedAssert);
        assert_eq!(f[0].id.as_deref(), Some("R-1"));
        assert_eq!(f[0].severity, Severity::Fatal);
        assert_eq!(f[0].context.as_deref(), Some("/a"));
        assert_eq!(f[0].pattern.as_deref(), Some("P1"));
        assert_eq!(f[0].text, "[R-1] two lines");
        assert_eq!(f[1].kind, FindingKind::SuccessfulReport);
        assert_eq!(f[1].severity, Severity::Warning);
        assert_eq!(f[1].role.as_deref(), Some("r"));
        assert_eq!(f[1].context, None);
        assert_eq!(f[1].pattern.as_deref(), Some("P2"));
        assert_eq!(f[2].severity, Severity::Other);
        assert_eq!(f[2].id, None);
        assert_eq!(f[2].context.as_deref(), Some("/z"));
    }

    #[test]
    fn refuses_what_is_not_svrl() {
        assert!(findings("<a/>").is_err());
        assert!(findings("<a").is_err());
    }

    #[test]
    fn severities() {
        assert_eq!(Severity::from_flag(Some("fatal")), Severity::Fatal);
        assert_eq!(Severity::from_flag(Some("warning")), Severity::Warning);
        assert_eq!(Severity::from_flag(Some("information")), Severity::Other);
        assert_eq!(Severity::from_flag(None), Severity::Other);
    }
}
