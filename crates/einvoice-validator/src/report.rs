//! The result of validating one document.

use std::fmt;

use einvoice_xpath::QName;
use einvoice_xslt_exec::{CompactOutput, ResultObserver};

pub use einvoice_xsd::{ErrorKind as SchemaErrorKind, ValidationError as SchemaError};

use crate::Error;

/// The SVRL namespace.
const SVRL_NS: &str = "http://purl.oclc.org/dsdl/svrl";

/// The overall verdict on a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verdict {
    /// No schema error and no finding with severity [`Severity::Fatal`].
    Valid,
    /// At least one schema error or one finding with severity [`Severity::Fatal`].
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
/// it, and from the manifest of the rule set: a failed assert without a flag has the
/// severity of its stylesheet's `unflagged-assert`
/// ([`StylesheetEntry::unflagged_assert`](crate::StylesheetEntry::unflagged_assert)), and
/// the scenario a document matches gives the rule ids it lists their severities, whatever
/// their flags ([`Scenario`](crate::Scenario)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    /// `flag="fatal"`: the document does not conform; the verdict is
    /// [`Verdict::Invalid`].
    Fatal,
    /// `flag="warning"`: reported, the document still conforms.
    Warning,
    /// Any other flag, or none: reported, the document still conforms (unless the
    /// validation fails closed: [`ValidateOptions::fail_closed`](crate::ValidateOptions::fail_closed)).
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

    /// The severity a Schematron flag stands for when failing closed: as
    /// [`Severity::from_flag`], but any other flag, or none, is [`Severity::Fatal`].
    #[must_use]
    pub fn from_flag_fail_closed(flag: Option<&str>) -> Self {
        match Self::from_flag(flag) {
            Self::Other => Self::Fatal,
            severity => severity,
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
    /// The severity of the finding (see [`Severity`]): the one the flag stands for,
    /// unless the manifest of the rule set gives another.
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
    /// The SVRL report, exactly as the stylesheet wrote it, kept compactly (see
    /// [`StylesheetReport::svrl`]).
    svrl: CompactOutput,
    /// Its failed asserts and successful reports, in report order.
    pub findings: Vec<Finding>,
    /// The number of rules that fired (its `svrl:fired-rule` elements): none means the
    /// stylesheet matched nothing in the document (a document of another syntax, ...).
    pub fired_rules: usize,
    /// Recoverable conditions the transformation reported (ambiguous rule matches,
    /// dynamic errors raised while matching a pattern, after which the rule did not
    /// match), as the reference engine reports them: each distinct message once.
    pub warnings: Vec<String>,
    /// The dynamic error that stopped the stylesheet, if it did not finish: then
    /// [`StylesheetReport::svrl`] is empty and there are no findings. This only happens
    /// in a report with schema errors (see
    /// [`Validator::validate_with`](crate::Validator::validate_with)).
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
    /// The SVRL report, exactly as the stylesheet wrote it: empty when the stylesheet
    /// did not finish.
    ///
    /// It is kept compactly, each distinct piece of its text once (a compiled Schematron
    /// writes the same `svrl:fired-rule` element each time a rule fires): its text is
    /// made only when asked for, with [`CompactOutput::to_text`], `to_string()` or
    /// written with `{}`; it compares equal to a `&str`.
    #[must_use]
    pub const fn svrl(&self) -> &CompactOutput {
        &self.svrl
    }

    /// The SVRL report in the canonical text form of the `einvoice-svrl` crate (version
    /// 1, or 2 for a report with `svrl:metadata`), in which the document's URI is written
    /// `{document}`: the form the expected results of the reference engine are stored
    /// in.
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
        einvoice_svrl::canonicalize(&self.svrl.to_text(), &|u| Some(u) == uri)
            .map_err(|e| Error::internal(format!("stylesheet {}: {e}", self.name)))
    }

    /// The report of a stylesheet that finished: its SVRL report, as `reader` read it
    /// while it was written.
    pub(crate) fn new(
        entry: &crate::StylesheetEntry,
        svrl: CompactOutput,
        reader: SvrlReader,
        warnings: Vec<String>,
        document_uri: Option<&str>,
    ) -> Result<Self, Error> {
        // A stylesheet that does not write SVRL is not a validation stylesheet: the
        // bundle is wrong.
        let (findings, fired_rules) = reader.finish(&svrl).map_err(|m| {
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
            fired_rules,
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
            svrl: CompactOutput::default(),
            findings: Vec::new(),
            fired_rules: 0,
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
    /// The name of the scenario of the rule set the document matched, whose severities
    /// apply to its findings ([`RuleSet::scenarios`](crate::RuleSet::scenarios)), if
    /// any.
    pub scenario: Option<String>,
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

/// How the findings of one stylesheet get their severities: from their flags, the
/// stylesheet's severity of an unflagged failed assert, the levels of the scenario the
/// document matched, and whether the validation fails closed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Severities<'a> {
    /// See [`crate::ValidateOptions::fail_closed`].
    pub(crate) fail_closed: bool,
    /// See [`crate::StylesheetEntry::unflagged_assert`].
    pub(crate) unflagged_assert: Severity,
    /// The scenario the document matched, if any.
    pub(crate) scenario: Option<&'a crate::Scenario>,
}

impl Default for Severities<'_> {
    fn default() -> Self {
        Self {
            fail_closed: false,
            unflagged_assert: Severity::Other,
            scenario: None,
        }
    }
}

impl Severities<'_> {
    /// The severity of a finding: the scenario's level of its rule id, if it gives one;
    /// otherwise the one its flag stands for ([`Severity::from_flag`]), a failed assert
    /// without a flag taking the stylesheet's; and when the validation fails closed,
    /// [`Severity::Fatal`] in place of [`Severity::Other`].
    pub(crate) fn of(self, kind: FindingKind, flag: Option<&str>, id: Option<&str>) -> Severity {
        let severity = id
            .and_then(|id| self.scenario.and_then(|s| s.level(id)))
            .unwrap_or_else(|| match (kind, flag) {
                (FindingKind::FailedAssert, None) => self.unflagged_assert,
                _ => Severity::from_flag(flag),
            });
        if self.fail_closed && severity == Severity::Other {
            Severity::Fatal
        } else {
            severity
        }
    }
}

/// Reads the failed asserts and successful reports of an SVRL report as the stylesheet
/// writes it, with the context of the last fired rule and the id of the last active
/// pattern before each, and counts the fired rules: what a parser of the report's text
/// reads, without the text.
#[derive(Debug, Default)]
pub(crate) struct SvrlReader<'a> {
    /// How the findings get their severities.
    severities: Severities<'a>,
    /// The number of open elements.
    depth: usize,
    /// The number of elements at the top of the report: a document has one.
    roots: usize,
    /// Whether the first of them is `svrl:schematron-output`.
    root_is_svrl: bool,
    /// Whether a comment holds `--` or ends with `-`, which XML forbids.
    bad_comment: bool,
    findings: Vec<Finding>,
    fired_rules: usize,
    pattern: Option<String>,
    context: Option<String>,
    /// The finding being read.
    finding: Option<Reading>,
}

/// A finding being read.
#[derive(Debug)]
struct Reading {
    finding: Finding,
    /// The text of its `svrl:text` children so far.
    text: String,
    /// Whether the text read is in one of them.
    in_text: bool,
}

impl<'a> SvrlReader<'a> {
    pub(crate) fn new(severities: Severities<'a>) -> Self {
        Self {
            severities,
            ..Self::default()
        }
    }

    /// The findings and the number of fired rules; or why `svrl`, the text read, is not
    /// an SVRL report: the error of a parser of the text when it is not well-formed.
    fn finish(self, svrl: &CompactOutput) -> Result<(Vec<Finding>, usize), String> {
        if self.roots != 1 || self.bad_comment {
            // Not a well-formed document: the parser says where.
            einvoice_xml::check(&svrl.to_text()).map_err(|e| e.to_string())?;
            if self.roots != 1 {
                return Err(format!(
                    "{} elements at the top of the document, instead of one",
                    self.roots
                ));
            }
        }
        if !self.root_is_svrl {
            return Err("the root is not svrl:schematron-output".to_owned());
        }
        Ok((self.findings, self.fired_rules))
    }
}

/// Whether `name` is the SVRL element `local`.
fn is_svrl(name: &QName, local: &str) -> bool {
    name.namespace.as_deref() == Some(SVRL_NS) && name.local == local
}

/// The value of the attribute in no namespace named `local`.
fn attribute(attributes: &[(QName, String)], local: &str) -> Option<String> {
    attributes
        .iter()
        .find(|(name, _)| {
            name.namespace.as_deref().is_none_or(str::is_empty) && name.local == local
        })
        .map(|(_, value)| value.clone())
}

impl ResultObserver for SvrlReader<'_> {
    fn start_element(&mut self, name: &QName, attributes: &[(QName, String)]) {
        self.depth += 1;
        match self.depth {
            1 => {
                self.roots += 1;
                if self.roots == 1 {
                    self.root_is_svrl = is_svrl(name, "schematron-output");
                }
            }
            // The children of the root.
            2 if self.roots == 1 && name.namespace.as_deref() == Some(SVRL_NS) => {
                let kind = match name.local.as_str() {
                    "active-pattern" => {
                        self.pattern =
                            attribute(attributes, "id").or_else(|| attribute(attributes, "name"));
                        self.context = None;
                        return;
                    }
                    "fired-rule" => {
                        self.fired_rules += 1;
                        self.context = attribute(attributes, "context");
                        return;
                    }
                    "failed-assert" => FindingKind::FailedAssert,
                    "successful-report" => FindingKind::SuccessfulReport,
                    _ => return,
                };
                let flag = attribute(attributes, "flag");
                let id = attribute(attributes, "id");
                let severity = self.severities.of(kind, flag.as_deref(), id.as_deref());
                let finding = Finding {
                    kind,
                    id,
                    severity,
                    flag,
                    role: attribute(attributes, "role"),
                    location: attribute(attributes, "location").unwrap_or_default(),
                    context: self.context.clone(),
                    pattern: self.pattern.clone(),
                    test: attribute(attributes, "test").unwrap_or_default(),
                    text: String::new(),
                };
                self.finding = Some(Reading {
                    finding,
                    text: String::new(),
                    in_text: false,
                });
            }
            // The children of a finding.
            3 if is_svrl(name, "text") => {
                if let Some(reading) = &mut self.finding {
                    reading.in_text = true;
                }
            }
            _ => {}
        }
    }

    fn end_element(&mut self) {
        match self.depth {
            2 => {
                if let Some(Reading {
                    mut finding, text, ..
                }) = self.finding.take()
                {
                    finding.text = einvoice_svrl::normalize_space(&text);
                    self.findings.push(finding);
                }
            }
            3 => {
                if let Some(reading) = &mut self.finding {
                    reading.in_text = false;
                }
            }
            _ => {}
        }
        self.depth = self.depth.saturating_sub(1);
    }

    fn text(&mut self, text: &str) {
        if let Some(reading) = &mut self.finding
            && reading.in_text
        {
            reading.text.push_str(text);
        }
    }

    fn comment(&mut self, text: &str) {
        if text.contains("--") || text.ends_with('-') {
            self.bad_comment = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The findings of an SVRL report read from its text, the parser's events given to
    /// [`SvrlReader`] as the serializer gives them.
    fn findings(svrl: &str, fail_closed: bool) -> Result<(Vec<Finding>, usize), String> {
        let qname = |name: einvoice_xml::Name<'_, '_>| QName {
            prefix: (!name.prefix.is_empty()).then(|| name.prefix.to_owned()),
            namespace: name.namespace.map(str::to_owned),
            local: name.local.to_owned(),
        };
        let mut reader = SvrlReader::new(Severities {
            fail_closed,
            ..Severities::default()
        });
        let mut parser = einvoice_xml::Reader::new(svrl);
        let mut depth = 0;
        while let Ok(Some(event)) = parser.next_event() {
            match event {
                einvoice_xml::Event::Start(start) => {
                    let attributes: Vec<(QName, String)> = start
                        .attributes()
                        .map(|a| (qname(a.name), a.value.to_string()))
                        .collect();
                    reader.start_element(&qname(start.name()), &attributes);
                    depth += 1;
                }
                einvoice_xml::Event::End(_) => {
                    reader.end_element();
                    depth -= 1;
                }
                einvoice_xml::Event::Text(text) if depth > 0 => reader.text(&text.value),
                einvoice_xml::Event::Comment(comment) => reader.comment(&comment.value),
                _ => {}
            }
        }
        reader.finish(&CompactOutput::from(svrl))
    }

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
        let (f, fired_rules) = findings(svrl, false).unwrap();
        assert_eq!(fired_rules, 2);
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
        let (closed, _) = findings(svrl, true).unwrap();
        let severities: Vec<Severity> = closed.iter().map(|f| f.severity).collect();
        assert_eq!(
            severities,
            [Severity::Fatal, Severity::Warning, Severity::Fatal]
        );
    }

    /// The error of the parser that read the reports before (`roxmltree`), which the
    /// streaming parser reproduces.
    fn parser_error(text: &str) -> String {
        let options = roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: u32::MAX,
            ..roxmltree::ParsingOptions::default()
        };
        let roxmltree = roxmltree::Document::parse_with_options(text, options)
            .unwrap_err()
            .to_string();
        assert_eq!(
            einvoice_xml::check(text).unwrap_err().to_string(),
            roxmltree
        );
        roxmltree
    }

    fn svrl_name(local: &str) -> QName {
        QName {
            prefix: Some("svrl".to_owned()),
            namespace: Some(SVRL_NS.to_owned()),
            local: local.to_owned(),
        }
    }

    #[test]
    fn refuses_what_is_not_svrl() {
        assert_eq!(
            findings("<a/>", false).unwrap_err(),
            "the root is not svrl:schematron-output"
        );
        // Not well-formed: the parser's error.
        for text in ["<a", ""] {
            assert_eq!(
                findings(text, false).unwrap_err(),
                parser_error(text),
                "{text}"
            );
        }
        // What the serializer can write and a parser refuses (the parser stops before
        // giving the events): two elements at the top, comments that XML forbids.
        let root = svrl_name("schematron-output");
        let mut reader = SvrlReader::default();
        for _ in 0..2 {
            reader.start_element(&root, &[]);
            reader.end_element();
        }
        let text = format!("<svrl:schematron-output xmlns:svrl=\"{SVRL_NS}\"/><a/>");
        assert_eq!(
            reader.finish(&CompactOutput::from(text.as_str())),
            Err(parser_error(&text))
        );
        for comment in ["x -- y", "x-"] {
            let mut reader = SvrlReader::default();
            reader.start_element(&root, &[]);
            reader.comment(comment);
            reader.end_element();
            let text = format!(
                "<svrl:schematron-output xmlns:svrl=\"{SVRL_NS}\"><!--{comment}--></svrl:schematron-output>"
            );
            assert_eq!(
                reader.finish(&CompactOutput::from(text.as_str())),
                Err(parser_error(&text)),
                "{comment}"
            );
        }
        // Events the text does not have, which the parser accepts.
        let mut reader = SvrlReader::default();
        for _ in 0..2 {
            reader.start_element(&root, &[]);
            reader.end_element();
        }
        assert_eq!(
            reader.finish(&CompactOutput::from("<a/>")).unwrap_err(),
            "2 elements at the top of the document, instead of one"
        );
    }

    #[test]
    fn severities() {
        assert_eq!(Severity::from_flag(Some("fatal")), Severity::Fatal);
        assert_eq!(Severity::from_flag(Some("warning")), Severity::Warning);
        assert_eq!(Severity::from_flag(Some("information")), Severity::Other);
        assert_eq!(Severity::from_flag(None), Severity::Other);
        assert_eq!(
            Severity::from_flag_fail_closed(Some("fatal")),
            Severity::Fatal
        );
        assert_eq!(
            Severity::from_flag_fail_closed(Some("warning")),
            Severity::Warning
        );
        assert_eq!(
            Severity::from_flag_fail_closed(Some("information")),
            Severity::Fatal
        );
        assert_eq!(Severity::from_flag_fail_closed(None), Severity::Fatal);
    }

    /// The severities of the findings of a stylesheet, as the manifest and the options
    /// set them.
    #[test]
    fn severities_of_a_stylesheet() {
        use FindingKind::{FailedAssert as A, SuccessfulReport as R};
        let manifest = "format = 2\nname = \"n\"\ntitle = \"t\"\nversion = \"1\"\n\n[[stylesheet]]\nname = \"s\"\npath = \"s.xsl\"\nsha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\n\n[[scenario]]\nname = \"x\"\nnamespace = \"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\"\nroot = \"Invoice\"\nidentifier = \"urn:x\"\nfatal = \"UP\"\nother = \"DOWN\"\n";
        let rule_set = crate::RuleSet::parse(manifest).unwrap();
        let scenario = rule_set.scenarios().first();
        let findings = [
            (A, Some("fatal"), Some("R")),
            (A, Some("warning"), Some("R")),
            (A, Some("information"), Some("R")),
            (A, None, Some("R")),
            (A, None, None),
            (R, None, Some("R")),
            (R, Some("fatal"), None),
            (A, Some("warning"), Some("UP")),
            (R, None, Some("UP")),
            (A, Some("fatal"), Some("DOWN")),
            (A, None, Some("DOWN")),
        ];
        let severities = |severities: Severities<'_>| -> Vec<&str> {
            findings
                .iter()
                .map(|(kind, flag, id)| severities.of(*kind, *flag, *id).as_str())
                .collect()
        };
        let base = Severities::default();
        assert_eq!(
            severities(base),
            [
                "fatal", "warning", "other", "other", "other", "other", "fatal", "warning",
                "other", "fatal", "other"
            ]
        );
        // An unflagged failed assert is fatal: an unflagged successful report is not.
        let unflagged = Severities {
            unflagged_assert: Severity::Fatal,
            ..base
        };
        assert_eq!(
            severities(unflagged),
            [
                "fatal", "warning", "other", "fatal", "fatal", "other", "fatal", "warning",
                "other", "fatal", "fatal"
            ]
        );
        // The scenario's levels win over the flags and over the unflagged severity.
        let with_scenario = Severities {
            scenario,
            ..unflagged
        };
        assert_eq!(
            severities(with_scenario),
            [
                "fatal", "warning", "other", "fatal", "fatal", "other", "fatal", "fatal", "fatal",
                "other", "other"
            ]
        );
        // Failing closed: whatever is still other is fatal.
        let closed = Severities {
            fail_closed: true,
            ..with_scenario
        };
        assert_eq!(
            severities(closed),
            [
                "fatal", "warning", "fatal", "fatal", "fatal", "fatal", "fatal", "fatal", "fatal",
                "fatal", "fatal"
            ]
        );
    }
}
