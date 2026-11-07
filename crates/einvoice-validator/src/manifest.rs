//! The manifest of a rule-set bundle (`bundle.toml`): what the rule set is, which XML
//! schema and which stylesheets it runs, and the SHA-256 of each stylesheet.

use std::fmt;

use crate::Error;

/// The file name of the manifest in a bundle directory.
pub const MANIFEST_FILE: &str = "bundle.toml";

/// The manifest format this crate reads (the `format` key).
pub const MANIFEST_FORMAT: u32 = 1;

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
}

/// A rule set, as described by the manifest of its bundle: its name and version, the
/// XML schema to validate against first, and the stylesheets to run, in order, with
/// their checksums.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSet {
    name: String,
    title: String,
    version: String,
    schema: Option<SchemaId>,
    stylesheets: Vec<StylesheetEntry>,
}

impl RuleSet {
    /// Parses a bundle manifest (see the crate documentation for the format).
    ///
    /// # Errors
    ///
    /// [`Error::Bundle`] naming the line: a syntax outside the accepted TOML subset, an
    /// unknown or repeated key, a missing key, a value that is not valid for its key, an
    /// unknown schema, a format other than [`MANIFEST_FORMAT`].
    pub fn parse(manifest: &str) -> Result<Self, Error> {
        let tables = parse_tables(manifest)?;
        let mut tables = tables.into_iter();
        let Some(top) = tables.next() else {
            return Err(Error::bundle(format!("{MANIFEST_FILE}: empty")));
        };
        let mut top = Table::new(top);
        let format_line = top.line_of("format");
        top.allow(&["format", "name", "title", "version", "schema"])?;
        match top.take("format") {
            Some(Value::Integer(f)) if f == u64::from(MANIFEST_FORMAT) => {}
            Some(Value::Integer(f)) => {
                return Err(Error::bundle(format!(
                    "{MANIFEST_FILE}:{format_line}: format {f}: only format {MANIFEST_FORMAT} is supported"
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
        }
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
        for raw in tables {
            let mut table = Table::new(raw);
            table.allow(&["name", "version", "path", "sha256"])?;
            let entry_name = table.required_string("name")?;
            check_name(&entry_name).map_err(|m| table.error("name", &m))?;
            if stylesheets.iter().any(|s| s.name == entry_name) {
                return Err(table.error("name", &format!("stylesheet {entry_name:?} twice")));
            }
            let entry_version = match table.optional_string("version")? {
                Some(v) => {
                    check_version(&v).map_err(|m| table.error("version", &m))?;
                    v
                }
                None => version.clone(),
            };
            let path = table.required_string("path")?;
            check_path(&path).map_err(|m| table.error("path", &m))?;
            let sha256 = table.required_string("sha256")?;
            if sha256.len() != 64
                || !sha256
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            {
                return Err(table.error("sha256", "must be 64 lower-case hexadecimal digits"));
            }
            table.finish()?;
            stylesheets.push(StylesheetEntry {
                name: entry_name,
                version: entry_version,
                path,
                sha256,
            });
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

/// The keys of one table, in order, with their lines. The top-level table has header
/// line 0.
#[derive(Debug)]
struct RawTable {
    header_line: u32,
    entries: Vec<(String, Value, u32)>,
}

/// Reads the accepted TOML subset: comments, blank lines, `key = value` lines with a
/// bare key and a basic string (escapes `\"` and `\\` only), a literal string or a
/// non-negative integer, and `[[stylesheet]]` headers.
fn parse_tables(text: &str) -> Result<Vec<RawTable>, Error> {
    let mut tables = vec![RawTable {
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
            let header = rest.strip_prefix("[stylesheet]]").map(after_value);
            match header {
                Some(Ok(())) => {
                    tables.push(RawTable {
                        header_line: number,
                        entries: Vec::new(),
                    });
                    continue;
                }
                Some(Err(m)) => return Err(fail(&m)),
                None => return Err(fail("the only table allowed is [[stylesheet]]")),
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
    header_line: u32,
    /// Key, value (`None` once taken) and line.
    entries: Vec<(String, Option<Value>, u32)>,
}

impl Table {
    fn new(raw: RawTable) -> Self {
        Self {
            header_line: raw.header_line,
            entries: raw
                .entries
                .into_iter()
                .map(|(k, v, l)| (k, Some(v), l))
                .collect(),
        }
    }

    fn where_(&self) -> String {
        if self.header_line == 0 {
            MANIFEST_FILE.to_owned()
        } else {
            format!(
                "{MANIFEST_FILE}: [[stylesheet]] of line {}",
                self.header_line
            )
        }
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
                GOOD.replace("format = 1", "format = 2"),
                "bundle.toml:2: format 2",
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
                "bundle.toml:13: the only table",
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
