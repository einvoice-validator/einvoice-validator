//! The manifest of a rule-set bundle (`bundle.toml`): what the rule set is, which XML
//! schema and which stylesheets it runs, the resources the stylesheets read with
//! `document()`, the SHA-256 of each file, and how the findings of the stylesheets are
//! read: the severity of an unflagged failed assert, and the severities that scenarios
//! give rule ids.

use std::fmt;

use crate::{Error, Severity};

/// The file name of the manifest in a bundle directory.
pub const MANIFEST_FILE: &str = "bundle.toml";

/// The newest manifest format this crate reads (the `format` key): format 2 adds to
/// format 1, which is read too, the `[[resource]]` and `[[scenario]]` tables and the
/// `unflagged-assert` key of `[[stylesheet]]`.
pub const MANIFEST_FORMAT: u32 = 2;

/// The XML schema a rule set validates documents against before running its
/// stylesheets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SchemaId {
    /// The OASIS UBL 2.1 schemas for Invoice and `CreditNote`, embedded in
    /// [`einvoice_xsd`].
    Ubl21,
}

impl SchemaId {
    /// The name used in manifests: `ubl-2.1`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ubl21 => "ubl-2.1",
        }
    }

    /// The schema named `name` in a manifest.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "ubl-2.1" => Some(Self::Ubl21),
            _ => None,
        }
    }
}

impl fmt::Display for SchemaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One stylesheet of a rule set, as its manifest describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StylesheetEntry {
    name: String,
    version: String,
    path: String,
    sha256: String,
    unflagged_assert: Severity,
}

impl StylesheetEntry {
    /// Its name, unique within the rule set (`EN16931-UBL`, ...).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The version of the rule set it implements: the manifest's `version` of the
    /// stylesheet, else the version of the bundle.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Its path in the bundle: relative, `/`-separated.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The SHA-256 of the file, lower-case hexadecimal.
    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// The severity of its failed asserts that have no flag (format 2,
    /// `unflagged-assert`): [`Severity::Other`] unless the manifest says otherwise. The
    /// rule sets whose reference validators reject a document for an unflagged failed
    /// assert (Factur-X, XRechnung) say [`Severity::Fatal`].
    #[must_use]
    pub const fn unflagged_assert(&self) -> Severity {
        self.unflagged_assert
    }
}

/// The severities that a scenario gives rule ids, whatever their flags, in the documents
/// it matches (format 2, `[[scenario]]`): the `customLevel` elements of a scenario of a
/// KoSIT validator configuration. A document matches when its root element has the
/// scenario's namespace and local name, and its specification identifier (BT-24) is the
/// scenario's: the text of a `cbc:CustomizationID` child of the root of a UBL document,
/// or of `rsm:ExchangedDocumentContext/ram:GuidelineSpecifiedDocumentContextParameter/ram:ID`
/// in a CII document, compared as it is written. The first scenario of the manifest
/// that matches applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scenario {
    name: String,
    namespace: String,
    root: String,
    identifier: String,
    levels: Vec<(String, Severity)>,
}

impl Scenario {
    /// Its name (`EN16931 XRechnung (UBL Invoice)`).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The namespace of the root element of the documents it matches.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The local name of the root element of the documents it matches (`Invoice`).
    #[must_use]
    pub fn root(&self) -> &str {
        &self.root
    }

    /// The specification identifier (BT-24) of the documents it matches.
    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// The rule ids it gives a severity, with that severity, in the order of the
    /// manifest (the ids of `fatal`, then `warning`, then `other`).
    #[must_use]
    pub fn levels(&self) -> &[(String, Severity)] {
        &self.levels
    }

    /// The severity it gives the rule `id`, if any.
    #[must_use]
    pub fn level(&self, id: &str) -> Option<Severity> {
        self.levels
            .iter()
            .find(|(rule, _)| rule == id)
            .map(|(_, severity)| *severity)
    }
}

/// The namespace of a UBL 2 invoice.
pub(crate) const UBL_INVOICE: &str = "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2";
/// The namespace of a UBL 2 credit note.
pub(crate) const UBL_CREDIT_NOTE: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2";
/// The namespace of a UN/CEFACT Cross Industry Invoice (D16B and the other releases of
/// version 100).
pub(crate) const CII: &str = "urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100";

/// The root elements whose specification identifier (BT-24) is known, which a scenario
/// may name: namespace, local name.
const IDENTIFIED_ROOTS: [(&str, &str); 3] = [
    (UBL_INVOICE, "Invoice"),
    (UBL_CREDIT_NOTE, "CreditNote"),
    (CII, "CrossIndustryInvoice"),
];

/// A file of the bundle that the stylesheets read with `document()` (a code list, ...),
/// as the manifest describes it (format 2, `[[resource]]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceEntry {
    path: String,
    sha256: String,
}

impl ResourceEntry {
    /// Its path in the bundle: relative, `/`-separated. A stylesheet asks for it with
    /// `document()` by its URI, relative to the stylesheet's own.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The SHA-256 of the file, lower-case hexadecimal.
    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

/// A rule set, as described by the manifest of its bundle: its name and version, the
/// XML schema to validate against first, the stylesheets to run, in order, and the
/// resources they read, with their checksums.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSet {
    name: String,
    title: String,
    version: String,
    schema: Option<SchemaId>,
    stylesheets: Vec<StylesheetEntry>,
    resources: Vec<ResourceEntry>,
    scenarios: Vec<Scenario>,
}

impl RuleSet {
    /// Parses a bundle manifest (see the crate documentation for the format).
    ///
    /// # Errors
    ///
    /// [`Error::Bundle`] naming the line: a syntax outside the accepted TOML subset, an
    /// unknown or repeated key, a missing key, a value that is not valid for its key, an
    /// unknown schema, a format other than 1 and [`MANIFEST_FORMAT`], a `[[resource]]`,
    /// a `[[scenario]]` or an `unflagged-assert` in format 1, a path given twice, a
    /// scenario named twice, for a root element other than a UBL `Invoice` or
    /// `CreditNote` or a CII `CrossIndustryInvoice`, without a rule id, or giving a rule
    /// id two severities.
    pub fn parse(manifest: &str) -> Result<Self, Error> {
        let tables = parse_tables(manifest)?;
        let mut tables = tables.into_iter();
        let Some(top) = tables.next() else {
            return Err(Error::bundle(format!("{MANIFEST_FILE}: empty")));
        };
        let mut top = Table::new(top);
        let format_line = top.line_of("format");
        top.allow(&["format", "name", "title", "version", "schema"])?;
        let format = match top.take("format") {
            Some(Value::Integer(f)) if (1..=u64::from(MANIFEST_FORMAT)).contains(&f) => f,
            Some(Value::Integer(f)) => {
                return Err(Error::bundle(format!(
                    "{MANIFEST_FILE}:{format_line}: format {f}: only formats 1 and {MANIFEST_FORMAT} are supported"
                )));
            }
            Some(Value::String(_)) => {
                return Err(Error::bundle(format!(
                    "{MANIFEST_FILE}:{format_line}: `format` must be an integer"
                )));
            }
            None => {
                return Err(Error::bundle(format!(
                    "{MANIFEST_FILE}: the `format` key is missing (format = {MANIFEST_FORMAT})"
                )));
            }
        };
        let name = top.required_string("name")?;
        check_name(&name).map_err(|m| top.error("name", &m))?;
        let title = top.required_string("title")?;
        if title.trim().is_empty() || title.chars().any(char::is_control) {
            return Err(top.error("title", "must be non-empty text on one line"));
        }
        let version = top.required_string("version")?;
        check_version(&version).map_err(|m| top.error("version", &m))?;
        let schema = match top.optional_string("schema")? {
            None => None,
            Some(s) => Some(SchemaId::from_name(&s).ok_or_else(|| {
                top.error("schema", &format!("unknown schema {s:?} (known: ubl-2.1)"))
            })?),
        };
        top.finish()?;

        let mut stylesheets: Vec<StylesheetEntry> = Vec::new();
        let mut resources: Vec<ResourceEntry> = Vec::new();
        let mut scenarios: Vec<Scenario> = Vec::new();
        for raw in tables {
            let kind = raw.kind;
            let table = Table::new(raw);
            if kind == TableKind::Resource {
                let resource = resource(table, format, &stylesheets, &resources)?;
                resources.push(resource);
                continue;
            }
            if kind == TableKind::Scenario {
                let scenario = scenario(table, format, &scenarios)?;
                scenarios.push(scenario);
                continue;
            }
            let entry = stylesheet(table, format, &version, &stylesheets, &resources)?;
            stylesheets.push(entry);
        }
        if stylesheets.is_empty() {
            return Err(Error::bundle(format!(
                "{MANIFEST_FILE}: no [[stylesheet]]: a rule set runs at least one"
            )));
        }
        Ok(Self {
            name,
            title,
            version,
            schema,
            stylesheets,
            resources,
            scenarios,
        })
    }

    /// The manifest of the built-in rule set `name` (see
    /// [`BUILTIN_RULE_SETS`](crate::BUILTIN_RULE_SETS)), without loading it.
    ///
    /// # Errors
    ///
    /// [`Error::Bundle`] for an unknown name.
    pub fn builtin(name: &str) -> Result<Self, Error> {
        Self::parse(crate::builtin::get(name)?.0)
    }

    /// Its short name (`cen-ubl`): ASCII letters, digits, `-`, `.` and `_`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its title (`CEN EN 16931 UBL`).
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The version of the official artefacts (`1.3.17`).
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The XML schema documents are validated against first, if any.
    #[must_use]
    pub const fn schema(&self) -> Option<SchemaId> {
        self.schema
    }

    /// The stylesheets, in the order they run.
    #[must_use]
    pub fn stylesheets(&self) -> &[StylesheetEntry] {
        &self.stylesheets
    }

    /// The resources the stylesheets read with `document()` (format 2), in the order of
    /// the manifest.
    #[must_use]
    pub fn resources(&self) -> &[ResourceEntry] {
        &self.resources
    }

    /// The scenarios (format 2), in the order of the manifest, which is the order they
    /// are tried in.
    #[must_use]
    pub fn scenarios(&self) -> &[Scenario] {
        &self.scenarios
    }
}

/// A `[[stylesheet]]` table, whose name no stylesheet has yet and whose path no file of
/// the bundle has yet; `version` is the bundle's.
fn stylesheet(
    mut table: Table,
    format: u64,
    version: &str,
    stylesheets: &[StylesheetEntry],
    resources: &[ResourceEntry],
) -> Result<StylesheetEntry, Error> {
    table.allow(&["name", "version", "path", "sha256", "unflagged-assert"])?;
    let name = table.required_string("name")?;
    check_name(&name).map_err(|m| table.error("name", &m))?;
    if stylesheets.iter().any(|s| s.name == name) {
        return Err(table.error("name", &format!("stylesheet {name:?} twice")));
    }
    let version = match table.optional_string("version")? {
        Some(v) => {
            check_version(&v).map_err(|m| table.error("version", &m))?;
            v
        }
        None => version.to_owned(),
    };
    let path = table.required_string("path")?;
    check_path(&path).map_err(|m| table.error("path", &m))?;
    if resources.iter().any(|r| r.path == path) {
        return Err(table.error("path", &format!("{path:?} twice in the bundle")));
    }
    let sha256 = table.sha256()?;
    let unflagged_assert = match table.optional_string("unflagged-assert")? {
        None => Severity::Other,
        Some(_) if format < 2 => {
            return Err(table.error("unflagged-assert", "needs format = 2"));
        }
        Some(s) => severity(&s).map_err(|m| table.error("unflagged-assert", &m))?,
    };
    table.finish()?;
    Ok(StylesheetEntry {
        name,
        version,
        path,
        sha256,
        unflagged_assert,
    })
}

/// A severity named in a manifest: `fatal`, `warning` or `other`.
fn severity(name: &str) -> Result<Severity, String> {
    match name {
        "fatal" => Ok(Severity::Fatal),
        "warning" => Ok(Severity::Warning),
        "other" => Ok(Severity::Other),
        _ => Err(format!("{name:?}: expected fatal, warning or other")),
    }
}

/// A `[[scenario]]` table (format 2).
fn scenario(mut table: Table, format: u64, scenarios: &[Scenario]) -> Result<Scenario, Error> {
    if format < 2 {
        return Err(Error::bundle(format!(
            "{MANIFEST_FILE}:{}: [[scenario]] needs format = 2",
            table.header_line
        )));
    }
    table.allow(&[
        "name",
        "namespace",
        "root",
        "identifier",
        "fatal",
        "warning",
        "other",
    ])?;
    let name = table.required_string("name")?;
    if name.trim().is_empty() || name.chars().any(char::is_control) {
        return Err(table.error("name", "must be non-empty text on one line"));
    }
    if scenarios.iter().any(|s| s.name == name) {
        return Err(table.error("name", &format!("scenario {name:?} twice")));
    }
    let namespace = table.required_string("namespace")?;
    let root = table.required_string("root")?;
    if !IDENTIFIED_ROOTS.contains(&(namespace.as_str(), root.as_str())) {
        return Err(table.error(
            "root",
            &format!(
                "{{{namespace}}}{root}: a scenario matches a UBL Invoice or CreditNote, or a CII CrossIndustryInvoice"
            ),
        ));
    }
    let identifier = table.required_string("identifier")?;
    if identifier.is_empty() || identifier.chars().any(char::is_control) {
        return Err(table.error("identifier", "must be non-empty text on one line"));
    }
    let mut levels: Vec<(String, Severity)> = Vec::new();
    for key in ["fatal", "warning", "other"] {
        let Some(ids) = table.optional_string(key)? else {
            continue;
        };
        let level = severity(key).map_err(|m| table.error(key, &m))?;
        let before = levels.len();
        for id in ids.split([' ', '\t']).filter(|id| !id.is_empty()) {
            if levels.iter().any(|(rule, _)| rule == id) {
                return Err(table.error(key, &format!("rule id {id:?} given twice")));
            }
            levels.push((id.to_owned(), level));
        }
        if levels.len() == before {
            return Err(table.error(key, "no rule id (rule ids separated by spaces)"));
        }
    }
    if levels.is_empty() {
        return Err(Error::bundle(format!(
            "{}: no rule id (the keys fatal, warning and other)",
            table.where_()
        )));
    }
    table.finish()?;
    Ok(Scenario {
        name,
        namespace,
        root,
        identifier,
        levels,
    })
}

/// A `[[resource]]` table (format 2), whose path no file of the bundle has yet.
fn resource(
    mut table: Table,
    format: u64,
    stylesheets: &[StylesheetEntry],
    resources: &[ResourceEntry],
) -> Result<ResourceEntry, Error> {
    if format < 2 {
        return Err(Error::bundle(format!(
            "{MANIFEST_FILE}:{}: [[resource]] needs format = 2",
            table.header_line
        )));
    }
    table.allow(&["path", "sha256"])?;
    let path = table.required_string("path")?;
    check_path(&path).map_err(|m| table.error("path", &m))?;
    let taken =
        resources.iter().any(|r| r.path == path) || stylesheets.iter().any(|s| s.path == path);
    if taken {
        return Err(table.error("path", &format!("{path:?} twice in the bundle")));
    }
    let sha256 = table.sha256()?;
    table.finish()?;
    Ok(ResourceEntry { path, sha256 })
}

fn check_name(name: &str) -> Result<(), String> {
    let mut bytes = name.bytes();
    let first_ok = bytes.next().is_some_and(|b| b.is_ascii_alphanumeric());
    if first_ok && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_')) {
        Ok(())
    } else {
        Err(format!(
            "{name:?}: ASCII letters, digits, `-`, `.` and `_` only, starting with a letter or digit"
        ))
    }
}

fn check_version(version: &str) -> Result<(), String> {
    if !version.is_empty()
        && version
            .chars()
            .all(|c| !c.is_whitespace() && !c.is_control())
    {
        Ok(())
    } else {
        Err(format!("{version:?}: must be non-empty, without spaces"))
    }
}

/// A path inside the bundle: relative, `/`-separated, no empty, `.` or `..` segment, no
/// backslash or colon, so that it can never designate a file outside the bundle.
fn check_path(path: &str) -> Result<(), String> {
    let bad = path.is_empty()
        || path.contains(['\\', ':'])
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..");
    if bad {
        Err(format!(
            "{path:?}: must be a relative `/`-separated path inside the bundle (no empty, `.` or `..` segment, no `\\` or `:`)"
        ))
    } else {
        Ok(())
    }
}

/// A value of the accepted TOML subset.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    String(String),
    Integer(u64),
}

/// The kind of a table: the top-level one, `[[stylesheet]]`, `[[resource]]` or
/// `[[scenario]]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableKind {
    Top,
    Stylesheet,
    Resource,
    Scenario,
}

/// The keys of one table, in order, with their lines. The top-level table has header
/// line 0.
#[derive(Debug)]
struct RawTable {
    kind: TableKind,
    header_line: u32,
    entries: Vec<(String, Value, u32)>,
}

/// Reads the accepted TOML subset: comments, blank lines, `key = value` lines with a
/// bare key and a basic string (escapes `\"` and `\\` only), a literal string or a
/// non-negative integer, and `[[stylesheet]]`, `[[resource]]` and `[[scenario]]`
/// headers.
fn parse_tables(text: &str) -> Result<Vec<RawTable>, Error> {
    let mut tables = vec![RawTable {
        kind: TableKind::Top,
        header_line: 0,
        entries: Vec::new(),
    }];
    for (index, line) in text.split('\n').enumerate() {
        let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        let fail = |message: &str| Error::bundle(format!("{MANIFEST_FILE}:{number}: {message}"));
        let line = line.strip_suffix('\r').unwrap_or(line);
        let content = line.trim_matches([' ', '\t']);
        if content.is_empty() || content.starts_with('#') {
            continue;
        }
        if let Some(rest) = content.strip_prefix('[') {
            let header = [
                ("[stylesheet]]", TableKind::Stylesheet),
                ("[resource]]", TableKind::Resource),
                ("[scenario]]", TableKind::Scenario),
            ]
            .into_iter()
            .find_map(|(h, kind)| rest.strip_prefix(h).map(|after| (after_value(after), kind)));
            match header {
                Some((Ok(()), kind)) => {
                    tables.push(RawTable {
                        kind,
                        header_line: number,
                        entries: Vec::new(),
                    });
                    continue;
                }
                Some((Err(m), _)) => return Err(fail(&m)),
                None => {
                    return Err(fail(
                        "the only tables allowed are [[stylesheet]], [[resource]] and [[scenario]]",
                    ));
                }
            }
        }
        let Some((key, value)) = content.split_once('=') else {
            return Err(fail("expected `key = value`"));
        };
        let key = key.trim_matches([' ', '\t']);
        if key.is_empty()
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(fail(&format!("{key:?}: expected a bare key")));
        }
        let value = parse_value(value.trim_start_matches([' ', '\t'])).map_err(|m| fail(&m))?;
        let Some(table) = tables.last_mut() else {
            return Err(Error::internal("manifest: no table"));
        };
        if table.entries.iter().any(|(k, _, _)| k == key) {
            return Err(fail(&format!("key `{key}` repeated")));
        }
        table.entries.push((key.to_owned(), value, number));
    }
    Ok(tables)
}

fn parse_value(text: &str) -> Result<Value, String> {
    if let Some(rest) = text.strip_prefix('"') {
        let mut value = String::new();
        let mut chars = rest.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => {
                    after_value(&rest[i + 1..])?;
                    return Ok(Value::String(value));
                }
                '\\' => match chars.next() {
                    Some((_, '"')) => value.push('"'),
                    Some((_, '\\')) => value.push('\\'),
                    _ => return Err("only the escapes \\\" and \\\\ are accepted".to_owned()),
                },
                c if c.is_control() && c != '\t' => {
                    return Err("control character in a string".to_owned());
                }
                c => value.push(c),
            }
        }
        Err("unterminated string".to_owned())
    } else if let Some(rest) = text.strip_prefix('\'') {
        let end = rest
            .find('\'')
            .ok_or_else(|| "unterminated string".to_owned())?;
        let value = &rest[..end];
        if value.chars().any(|c| c.is_control() && c != '\t') {
            return Err("control character in a string".to_owned());
        }
        after_value(&rest[end + 1..])?;
        Ok(Value::String(value.to_owned()))
    } else {
        let end = text
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len());
        let digits = &text[..end];
        if digits.is_empty() || (digits.len() > 1 && digits.starts_with('0')) {
            return Err("expected a string or a non-negative integer".to_owned());
        }
        let value = digits
            .parse()
            .map_err(|_| "integer out of range".to_owned())?;
        after_value(&text[end..])?;
        Ok(Value::Integer(value))
    }
}

/// What may follow a value or a header on its line: blanks and a comment.
fn after_value(rest: &str) -> Result<(), String> {
    let rest = rest.trim_start_matches([' ', '\t']);
    if rest.is_empty() || rest.starts_with('#') {
        Ok(())
    } else {
        Err(format!("unexpected {rest:?} after the value"))
    }
}

/// A table whose keys are taken one by one; [`Table::finish`] fails on any left.
struct Table {
    kind: TableKind,
    header_line: u32,
    /// Key, value (`None` once taken) and line.
    entries: Vec<(String, Option<Value>, u32)>,
}

impl Table {
    fn new(raw: RawTable) -> Self {
        Self {
            kind: raw.kind,
            header_line: raw.header_line,
            entries: raw
                .entries
                .into_iter()
                .map(|(k, v, l)| (k, Some(v), l))
                .collect(),
        }
    }

    fn where_(&self) -> String {
        match self.kind {
            TableKind::Top => MANIFEST_FILE.to_owned(),
            TableKind::Stylesheet => format!(
                "{MANIFEST_FILE}: [[stylesheet]] of line {}",
                self.header_line
            ),
            TableKind::Resource => {
                format!("{MANIFEST_FILE}: [[resource]] of line {}", self.header_line)
            }
            TableKind::Scenario => {
                format!("{MANIFEST_FILE}: [[scenario]] of line {}", self.header_line)
            }
        }
    }

    /// The `sha256` key: 64 lower-case hexadecimal digits.
    fn sha256(&mut self) -> Result<String, Error> {
        let sha256 = self.required_string("sha256")?;
        if sha256.len() != 64
            || !sha256
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(self.error("sha256", "must be 64 lower-case hexadecimal digits"));
        }
        Ok(sha256)
    }

    /// The line of `key`, taken or not; the header's line when absent.
    fn line_of(&self, key: &str) -> u32 {
        self.entries
            .iter()
            .find(|(k, _, _)| k == key)
            .map_or(self.header_line, |(_, _, l)| *l)
    }

    fn error(&self, key: &str, message: &str) -> Error {
        Error::bundle(format!(
            "{MANIFEST_FILE}:{}: `{key}`: {message}",
            self.line_of(key)
        ))
    }

    /// Fails on a key outside `allowed`.
    fn allow(&self, allowed: &[&str]) -> Result<(), Error> {
        match self
            .entries
            .iter()
            .find(|(k, _, _)| !allowed.contains(&k.as_str()))
        {
            Some((key, _, line)) => Err(Error::bundle(format!(
                "{MANIFEST_FILE}:{line}: unknown key `{key}` (expected one of: {})",
                allowed.join(", ")
            ))),
            None => Ok(()),
        }
    }

    fn take(&mut self, key: &str) -> Option<Value> {
        self.entries
            .iter_mut()
            .find(|(k, _, _)| k == key)
            .and_then(|(_, v, _)| v.take())
    }

    fn optional_string(&mut self, key: &str) -> Result<Option<String>, Error> {
        match self.take(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s)),
            Some(Value::Integer(_)) => Err(self.error(key, "must be a string")),
        }
    }

    fn required_string(&mut self, key: &str) -> Result<String, Error> {
        let where_ = self.where_();
        self.optional_string(key)?
            .ok_or_else(|| Error::bundle(format!("{where_}: the `{key}` key is missing")))
    }

    fn finish(self) -> Result<(), Error> {
        match self.entries.iter().find(|(_, v, _)| v.is_some()) {
            Some((key, _, line)) => Err(Error::bundle(format!(
                "{MANIFEST_FILE}:{line}: unexpected key `{key}`"
            ))),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"# comment
format = 1
name = "cen-ubl"   # trailing comment
title = 'CEN EN 16931 UBL'
version = "1.3.17"
schema = "ubl-2.1"

[[stylesheet]]
name = "EN16931-UBL"
path = "ubl/xslt/EN16931-UBL-validation.xslt"
sha256 = "fc4e6cfa8596bba0c2abd19038a98960c6ee9fc1f44566787c115efd4f4aca7c"

[[stylesheet]]  # second
name = "other"
version = "3.0.22"
path = "x.xsl"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
"#;

    #[test]
    fn parses_a_manifest() {
        let rule_set = RuleSet::parse(GOOD).unwrap();
        assert_eq!(rule_set.name(), "cen-ubl");
        assert_eq!(rule_set.title(), "CEN EN 16931 UBL");
        assert_eq!(rule_set.version(), "1.3.17");
        assert_eq!(rule_set.schema(), Some(SchemaId::Ubl21));
        let s = rule_set.stylesheets();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].name(), "EN16931-UBL");
        assert_eq!(s[0].version(), "1.3.17");
        assert_eq!(s[0].path(), "ubl/xslt/EN16931-UBL-validation.xslt");
        assert_eq!(s[1].version(), "3.0.22");
    }

    #[test]
    fn the_embedded_manifest_parses() {
        let rule_set = RuleSet::parse(include_str!("../bundles/cen-ubl/bundle.toml")).unwrap();
        assert_eq!(rule_set.name(), "cen-ubl");
    }

    #[test]
    fn format_2_lists_resources() {
        let zeros = "0".repeat(64);
        let text = format!(
            "{}[[resource]]\npath = \"codes/a.xml\"\nsha256 = \"{zeros}\"\n\n[[resource]] # b\npath = \"b.xml\"\nsha256 = \"{}\"\n",
            GOOD.replace("format = 1", "format = 2"),
            "1".repeat(64)
        );
        let rule_set = RuleSet::parse(&text).unwrap();
        let resources: Vec<(&str, &str)> = rule_set
            .resources()
            .iter()
            .map(|r| (r.path(), r.sha256()))
            .collect();
        assert_eq!(
            resources,
            [
                ("codes/a.xml", zeros.as_str()),
                ("b.xml", "1".repeat(64).as_str())
            ]
        );
        assert_eq!(rule_set.stylesheets().len(), 2);
        assert_eq!(RuleSet::parse(GOOD).unwrap().resources(), []);
        for (bad, expected) in [
            (text.replace("codes/a.xml", "../a.xml"), "`path`"),
            (text.replace("codes/a.xml", "b.xml"), "twice in the bundle"),
            (text.replace("codes/a.xml", "x.xsl"), "twice in the bundle"),
            (
                text.replace(&format!("sha256 = \"{zeros}\""), "sha256 = \"x\""),
                "`sha256`",
            ),
            (
                text.replace("path = \"b.xml\"", "name = \"b\"\npath = \"b.xml\""),
                "unknown key `name`",
            ),
            (
                text.replace("path = \"b.xml\"\n", ""),
                "[[resource]] of line 22: the `path` key is missing",
            ),
        ] {
            let message = error(&bad);
            assert!(
                message.contains(expected),
                "{expected:?} not in {message:?}"
            );
        }
    }

    /// [`GOOD`] in format 2, the second stylesheet failing closed on unflagged asserts,
    /// with two scenarios.
    fn with_severities() -> String {
        format!(
            "{}
[[scenario]]
name = \"S 1\"
namespace = \"{UBL_INVOICE}\"
root = \"Invoice\"
identifier = \"urn:x\"
fatal = \"R-1  R-2\"
other = \"R-3\"

[[scenario]]  # CII
name = \"S 2\"
namespace = \"{CII}\"
root = \"CrossIndustryInvoice\"
identifier = \"urn:y#z\"
warning = \"R-1\"
",
            GOOD.replace("format = 1", "format = 2").replace(
                "path = \"x.xsl\"",
                "path = \"x.xsl\"\nunflagged-assert = \"fatal\""
            )
        )
    }

    #[test]
    fn format_2_reads_severities() {
        let text = with_severities();
        let rule_set = RuleSet::parse(&text).unwrap();
        let unflagged: Vec<Severity> = rule_set
            .stylesheets()
            .iter()
            .map(StylesheetEntry::unflagged_assert)
            .collect();
        assert_eq!(unflagged, [Severity::Other, Severity::Fatal]);
        let [s1, s2] = rule_set.scenarios() else {
            panic!("{:?}", rule_set.scenarios());
        };
        assert_eq!(
            (s1.name(), s1.namespace(), s1.root(), s1.identifier()),
            ("S 1", UBL_INVOICE, "Invoice", "urn:x")
        );
        assert_eq!(
            s1.levels(),
            [
                ("R-1".to_owned(), Severity::Fatal),
                ("R-2".to_owned(), Severity::Fatal),
                ("R-3".to_owned(), Severity::Other)
            ]
        );
        assert_eq!(s1.level("R-2"), Some(Severity::Fatal));
        assert_eq!(s1.level("R-4"), None);
        assert_eq!(
            (s2.root(), s2.identifier()),
            ("CrossIndustryInvoice", "urn:y#z")
        );
        assert_eq!(s2.level("R-1"), Some(Severity::Warning));
        // Format 1 and format 2 without them: as before.
        assert_eq!(RuleSet::parse(GOOD).unwrap().scenarios(), []);
        assert!(
            RuleSet::parse(GOOD)
                .unwrap()
                .stylesheets()
                .iter()
                .all(|s| s.unflagged_assert() == Severity::Other)
        );
    }

    #[test]
    fn severities_are_refused_with_the_line() {
        let text = with_severities();
        let in_format_1 = text.replace("format = 2", "format = 1");
        assert!(
            error(&in_format_1).contains("bundle.toml:17: `unflagged-assert`: needs format = 2")
        );
        let scenario_in_format_1 = in_format_1.replace("unflagged-assert = \"fatal\"\n", "");
        assert!(
            error(&scenario_in_format_1).contains("bundle.toml:19: [[scenario]] needs format = 2"),
            "{}",
            error(&scenario_in_format_1)
        );
        for (bad, expected) in [
            (
                text.replace(
                    "unflagged-assert = \"fatal\"",
                    "unflagged-assert = \"error\"",
                ),
                "`unflagged-assert`: \"error\": expected fatal, warning or other",
            ),
            (
                text.replace("name = \"S 2\"", "name = \"S 1\""),
                "scenario \"S 1\" twice",
            ),
            (
                text.replace("root = \"Invoice\"", "root = \"CreditNote\""),
                "`root`: {urn:oasis:names:specification:ubl:schema:xsd:Invoice-2}CreditNote: a scenario matches",
            ),
            (
                text.replace("root = \"Invoice\"", "root = \"Order\""),
                "a scenario matches a UBL Invoice or CreditNote, or a CII CrossIndustryInvoice",
            ),
            (
                text.replace("other = \"R-3\"", "other = \"R-2\""),
                "rule id \"R-2\" given twice",
            ),
            (
                text.replace("warning = \"R-1\"", "warning = \" \""),
                "`warning`: no rule id",
            ),
            (
                text.replace("warning = \"R-1\"\n", ""),
                "[[scenario]] of line 28: no rule id (the keys fatal, warning and other)",
            ),
            (
                text.replace("identifier = \"urn:x\"", "identifier = \"\""),
                "`identifier`",
            ),
            (
                text.replace("name = \"S 1\"", "name = \" \""),
                "`name`: must be non-empty",
            ),
            (
                text.replace("identifier = \"urn:x\"\n", ""),
                "[[scenario]] of line 20: the `identifier` key is missing",
            ),
            (
                text.replace("other = \"R-3\"", "information = \"R-3\""),
                "unknown key `information`",
            ),
        ] {
            let message = error(&bad);
            assert!(
                message.contains(expected),
                "{expected:?} not in {message:?}"
            );
        }
    }

    #[test]
    fn schema_is_optional() {
        let text = GOOD.replace("schema = \"ubl-2.1\"\n", "");
        assert_eq!(RuleSet::parse(&text).unwrap().schema(), None);
    }

    fn error(text: &str) -> String {
        RuleSet::parse(text).unwrap_err().to_string()
    }

    #[test]
    fn refusals_name_the_line() {
        let cases = [
            (
                GOOD.replace("format = 1", "format = 3"),
                "bundle.toml:2: format 3",
            ),
            (
                format!(
                    "{GOOD}[[resource]]\npath = \"r.xml\"\nsha256 = \"{}\"\n",
                    "0".repeat(64)
                ),
                "bundle.toml:18: [[resource]] needs format = 2",
            ),
            (
                GOOD.replace("format = 1", "format = \"1\""),
                "bundle.toml:2:",
            ),
            (
                GOOD.replace("format = 1\n", ""),
                "the `format` key is missing",
            ),
            (
                GOOD.replace("ubl-2.1", "ubl-2.2"),
                "bundle.toml:6: `schema`: unknown schema",
            ),
            (
                GOOD.replace("title =", "titel ="),
                "bundle.toml:4: unknown key `titel`",
            ),
            (
                GOOD.replace("name = \"other\"", "name = \"EN16931-UBL\""),
                "twice",
            ),
            (GOOD.replace("x.xsl", "../x.xsl"), "`path`"),
            (GOOD.replace("x.xsl", "/x.xsl"), "`path`"),
            (GOOD.replace("x.xsl", "C:/x.xsl"), "`path`"),
            (GOOD.replace("x.xsl", "a//x.xsl"), "`path`"),
            (GOOD.replace("x.xsl", "a\\\\x.xsl"), "`path`"),
            (GOOD.replace("\"0000", "\"A000"), "`sha256`"),
            (
                GOOD.replace("version = \"1.3.17\"", "version = \"1 3\""),
                "`version`",
            ),
            (
                GOOD.replace("[[stylesheet]]  #", "[stylesheet] #"),
                "bundle.toml:13: the only tables allowed",
            ),
            (
                GOOD.replace("name = \"cen-ubl\"", "name = \"cen\\nubl\""),
                "bundle.toml:3: only the escapes",
            ),
            (
                GOOD.replace("name = \"cen-ubl\"", "name = \"cen-ubl"),
                "unterminated",
            ),
            (
                GOOD.replace("version = \"1.3.17\"", "version = \"1.3.17\" x"),
                "after the value",
            ),
            (
                GOOD.replace("format = 1", "format = 1\nformat = 1"),
                "repeated",
            ),
            (
                GOOD.replace("format = 1", "format = 01"),
                "bundle.toml:2: expected a string",
            ),
            (
                GOOD.replace("path = \"x.xsl\"\n", ""),
                "the `path` key is missing",
            ),
            (
                GOOD.split("[[stylesheet]]").next().unwrap().to_owned(),
                "no [[stylesheet]]",
            ),
            (String::new(), "the `format` key is missing"),
            (
                "format\n".to_owned(),
                "bundle.toml:1: expected `key = value`",
            ),
        ];
        for (text, expected) in cases {
            let message = error(&text);
            assert!(
                message.contains(expected),
                "{expected:?} not in {message:?}"
            );
        }
    }
}
