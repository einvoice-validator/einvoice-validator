//! Loading a rule set once, then validating documents with it.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use einvoice_xpath_eval::{Clock, DEFAULT_MAX_DEPTH, LimitExceeded};
use einvoice_xslt::LoadErrorKind;
use einvoice_xslt_exec::{
    DEFAULT_MAX_STACK, Executable, TransformError, TransformOptions, max_nesting_for_depth,
};
use roxmltree::ParsingOptions;
use sha2::{Digest, Sha256};

use crate::manifest::{MANIFEST_FILE, RuleSet, SchemaId, StylesheetEntry};
use crate::report::{Report, RuleSetVersion, StylesheetFailure, StylesheetReport};
use crate::{Error, builtin};

/// Options of one validation.
#[derive(Debug, Clone, Copy)]
pub struct ValidateOptions<'a> {
    document_uri: Option<&'a str>,
    clock: Option<Clock>,
    max_depth: u32,
    max_stack: u32,
}

impl Default for ValidateOptions<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> ValidateOptions<'a> {
    /// The default options: the document has no URI, the clock is the system clock in
    /// UTC, the maximum element depth is [`DEFAULT_MAX_DEPTH`] and the stack the
    /// stylesheets may use is [`DEFAULT_MAX_STACK`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            document_uri: None,
            clock: None,
            max_depth: DEFAULT_MAX_DEPTH,
            max_stack: DEFAULT_MAX_STACK,
        }
    }

    /// The maximum element depth of the document, and of any document a stylesheet
    /// loads with `document()`: the document element is at depth 1, its children at
    /// depth 2, and so on. A document with an element nested deeper is refused with
    /// [`Error::Limit`] before its schema is checked. Default: [`DEFAULT_MAX_DEPTH`]
    /// (256, libxml2's default).
    ///
    /// The stylesheets apply their template rules one element level inside another, so
    /// their stack use grows with the depth: with the default, a validation runs on a
    /// thread with the default 2 MiB stack, debug builds included. The nesting of
    /// template and function invocations the stylesheets are allowed grows with the
    /// maximum ([`max_nesting_for_depth`](crate::max_nesting_for_depth)), and so does the
    /// stack they need: a larger maximum depth needs a larger
    /// [`ValidateOptions::max_stack`], and a thread with that much stack.
    #[must_use]
    pub const fn max_depth(mut self, max_depth: u32) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// The most stack, in bytes, the stylesheets' template and function invocations may
    /// use, measured from the start of each stylesheet: beyond it, the validation fails
    /// with [`Error::Limit`] ([`LimitExceeded::Stack`]) instead of overflowing the
    /// stack, which would abort the process. Default: [`DEFAULT_MAX_STACK`] (1.5 MiB), for
    /// the default 2 MiB stack of a spawned thread. The thread that validates must have
    /// this much stack and at least 512 KiB more, for its caller and the deepest
    /// invocation.
    #[must_use]
    pub const fn max_stack(mut self, bytes: u32) -> Self {
        self.max_stack = bytes;
        self
    }

    /// The clock of the stylesheets: the implicit timezone and the value of
    /// `fn:current-dateTime()` (read once per validation, as the reference engine reads
    /// it once per transformation). By default, the system clock in UTC; required on
    /// targets without a system clock, such as `wasm32-unknown-unknown`, where
    /// validating without it fails with [`Error::Unsupported`].
    #[must_use]
    pub const fn clock(mut self, clock: Clock) -> Self {
        self.clock = Some(clock);
        self
    }

    /// The URI of the document: what `document-uri(/)` returns in the stylesheets, and
    /// what the SVRL report states in `svrl:active-pattern/@documents`. For a file,
    /// [`file_uri`] gives the URI the reference engine uses.
    #[must_use]
    pub const fn document_uri(mut self, uri: &'a str) -> Self {
        self.document_uri = Some(uri);
        self
    }
}

/// The `file:` URI that the reference engine (Saxon-HE) gives a document read from the
/// file at `path` (absolute). `None` for a relative path or one that is not Unicode.
#[must_use]
pub fn file_uri(path: &Path) -> Option<String> {
    einvoice_xpath_eval::file_uri(path)
}

/// A rule set ready to validate documents: its schema loaded and its stylesheets
/// prepared, once. Validating a document only reads it, so one `Validator` serves any
/// number of documents, from any number of threads at once (`Send + Sync`); nothing of
/// the rule set is copied per document.
#[derive(Debug)]
pub struct Validator {
    rule_set: RuleSet,
    schema: Option<einvoice_xsd::Schema>,
    /// The prepared stylesheets, in the order of `rule_set.stylesheets()`.
    stylesheets: Vec<Executable>,
}

impl Validator {
    /// The built-in rule set `name` (see [`BUILTIN_RULE_SETS`](crate::BUILTIN_RULE_SETS)).
    ///
    /// # Errors
    ///
    /// [`Error::Bundle`] for an unknown name; otherwise as [`Validator::from_bundle`],
    /// which none of the built-in rule sets fails in practice (a test loads each).
    pub fn builtin(name: &str) -> Result<Self, Error> {
        let (manifest, files) = builtin::get(name)?;
        Self::from_bundle(manifest, files)
    }

    /// The built-in rule set `cen-ubl`: the CEN EN 16931 validation artefacts for UBL,
    /// release 1.3.17 (EUPL-1.2), embedded unmodified, with the UBL 2.1 schemas.
    ///
    /// # Errors
    ///
    /// None in practice (a test loads it); the error is returned rather than hidden.
    #[cfg(feature = "cen-ubl")]
    pub fn cen_ubl() -> Result<Self, Error> {
        Self::builtin("cen-ubl")
    }

    /// The built-in rule set `cen-cii`: the CEN EN 16931 validation artefacts for UN/CEFACT
    /// CII D16B, release 1.3.17 (EUPL-1.2), embedded unmodified, without an XML schema
    /// (the CII D16B schemas are not embedded).
    ///
    /// # Errors
    ///
    /// None in practice (a test loads it); the error is returned rather than hidden.
    #[cfg(feature = "cen-cii")]
    pub fn cen_cii() -> Result<Self, Error> {
        Self::builtin("cen-cii")
    }

    /// Loads a bundle from memory: its manifest and its files, by their paths in the
    /// manifest. Files not named by the manifest are ignored.
    ///
    /// # Errors
    ///
    /// [`Error::Bundle`] for an invalid manifest, a missing file or a stylesheet that is
    /// not well-formed; [`Error::Checksum`] for a file whose SHA-256 differs from the
    /// manifest's; [`Error::Unsupported`] for a stylesheet or schema outside the
    /// supported subset.
    pub fn from_bundle(manifest: &str, files: &[(&str, &[u8])]) -> Result<Self, Error> {
        let rule_set = RuleSet::parse(manifest)?;
        let mut stylesheets = Vec::with_capacity(rule_set.stylesheets().len());
        for entry in rule_set.stylesheets() {
            let Some((_, bytes)) = files.iter().find(|(path, _)| *path == entry.path()) else {
                return Err(Error::bundle(format!(
                    "stylesheet {}: file {} missing from the bundle",
                    entry.name(),
                    entry.path()
                )));
            };
            stylesheets.push(prepare(entry, bytes, None)?);
        }
        Self::with(rule_set, stylesheets)
    }

    /// Loads the bundle in the directory `dir`: its manifest [`MANIFEST_FILE`] and the
    /// files it names, relative to `dir`.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when a file cannot be read, otherwise as
    /// [`Validator::from_bundle`].
    pub fn from_bundle_dir(dir: impl AsRef<Path>) -> Result<Self, Error> {
        let dir = dir.as_ref();
        let manifest_path = dir.join(MANIFEST_FILE);
        let manifest = read(&manifest_path)?;
        let manifest = std::str::from_utf8(&manifest)
            .map_err(|e| Error::bundle(format!("{}: not UTF-8: {e}", manifest_path.display())))?;
        let rule_set = RuleSet::parse(manifest)?;
        let mut stylesheets = Vec::with_capacity(rule_set.stylesheets().len());
        for entry in rule_set.stylesheets() {
            let mut path = dir.to_path_buf();
            path.extend(entry.path().split('/'));
            let bytes = read(&path)?;
            let base_uri = std::path::absolute(&path).ok().and_then(|p| file_uri(&p));
            stylesheets.push(prepare(entry, &bytes, base_uri)?);
        }
        Self::with(rule_set, stylesheets)
    }

    fn with(rule_set: RuleSet, stylesheets: Vec<Executable>) -> Result<Self, Error> {
        let schema = match rule_set.schema() {
            None => None,
            Some(SchemaId::Ubl21) => Some(einvoice_xsd::Schema::ubl_2_1().map_err(|e| {
                if e.kind == einvoice_xsd::LoadErrorKind::Unsupported {
                    Error::unsupported(format!("schema ubl-2.1: {e}"))
                } else {
                    Error::internal(format!("the embedded schema ubl-2.1 does not load: {e}"))
                }
            })?),
        };
        Ok(Self {
            rule_set,
            schema,
            stylesheets,
        })
    }

    /// The rule set: name, version, schema, stylesheets and their checksums.
    #[must_use]
    pub const fn rule_set(&self) -> &RuleSet {
        &self.rule_set
    }

    /// Validates a document with the default options (no document URI).
    ///
    /// # Errors
    ///
    /// As [`Validator::validate_with`].
    pub fn validate(&self, document: &[u8]) -> Result<Report, Error> {
        self.validate_with(document, &ValidateOptions::new())
    }

    /// Validates a document given as a buffer, with the default options (no document
    /// URI).
    ///
    /// # Errors
    ///
    /// As [`Validator::validate_with`].
    pub fn validate_owned(&self, document: Vec<u8>) -> Result<Report, Error> {
        self.validate_owned_with(document, &ValidateOptions::new())
    }

    /// [`Validator::validate_with`] on a document given as a buffer, which the
    /// validation keeps instead of copying: the buffer becomes the text without a copy,
    /// and the tree of the last stylesheet refers to it, so that while that stylesheet
    /// runs the document is in memory once rather than twice. Same report as
    /// [`Validator::validate_with`]; for a caller that has no further use for the bytes
    /// (a file just read).
    ///
    /// # Errors
    ///
    /// As [`Validator::validate_with`].
    pub fn validate_owned_with(
        &self,
        document: Vec<u8>,
        options: &ValidateOptions<'_>,
    ) -> Result<Report, Error> {
        self.validate_text(Cow::Owned(decode_owned(document)?), options)
    }

    /// Validates a document: checks that it is well-formed XML, validates it against
    /// the rule set's schema, then runs every stylesheet of the rule set on it. The
    /// stylesheets run even when the schema reports errors, so that the report is
    /// complete. When a stylesheet raises a dynamic error on a document that has schema
    /// errors (a non-numeric amount the rules compute with, ...), the report is returned
    /// all the same, invalid, with the error in [`StylesheetReport::failure`]; without
    /// schema errors, the error is returned.
    ///
    /// The document must be UTF-8 (a byte order mark is allowed) and have no document
    /// type declaration.
    ///
    /// # Errors
    ///
    /// [`Error::NotXml`] when the document is not well-formed XML (or not UTF-8);
    /// [`Error::Unsupported`] for another encoding, a document type declaration, or a
    /// construct the engine refuses; [`Error::Transform`] when a stylesheet raises a
    /// dynamic error on the document; [`Error::Bundle`] when a stylesheet's result is
    /// not an SVRL report; [`Error::Internal`] for a bug.
    pub fn validate_with(
        &self,
        document: &[u8],
        options: &ValidateOptions<'_>,
    ) -> Result<Report, Error> {
        self.validate_text(Cow::Borrowed(decode(document)?), options)
    }

    /// The validation of a decoded document: borrowed by the schema and every stylesheet
    /// but the last, which is given it when it is owned.
    fn validate_text(
        &self,
        text: Cow<'_, str>,
        options: &ValidateOptions<'_>,
    ) -> Result<Report, Error> {
        check_well_formed(&text, options.max_depth)?;
        let schema_errors = match &self.schema {
            None => Vec::new(),
            Some(schema) => {
                schema
                    .validate(&text)
                    .map_err(|e| match e {
                        einvoice_xsd::DocumentError::Malformed {
                            line,
                            column,
                            message,
                        } => Error::NotXml {
                            line,
                            column,
                            message,
                        },
                        einvoice_xsd::DocumentError::Unsupported {
                            line,
                            column,
                            message,
                        } => Error::unsupported(format!(
                            "document, line {line}, column {column}: {message}"
                        )),
                    })?
                    .errors
            }
        };
        let transform_options = TransformOptions {
            implicit_timezone: options.clock.map(|c| c.implicit_timezone()),
            current_date_time: options.clock.map(|c| c.current_date_time()),
            max_depth: options.max_depth,
            max_nesting: max_nesting_for_depth(options.max_depth),
            max_stack: options.max_stack,
            ..TransformOptions::default()
        };
        let mut stylesheets = Vec::with_capacity(self.stylesheets.len());
        let count = self.stylesheets.len();
        // The text, until the last stylesheet is given it.
        let mut text = Some(text);
        for (index, (entry, executable)) in self
            .rule_set
            .stylesheets()
            .iter()
            .zip(&self.stylesheets)
            .enumerate()
        {
            let source = if index + 1 == count {
                text.take()
            } else {
                text.as_deref().map(Cow::Borrowed)
            }
            .ok_or_else(|| Error::internal("the document text was given away early"))?;
            let result =
                executable.transform_with(source, options.document_uri, &transform_options);
            let report = match result {
                Ok(result) => StylesheetReport::new(
                    entry,
                    result.output,
                    result.warnings,
                    options.document_uri,
                )?,
                Err(e) => match transform_error(entry, &e) {
                    // The schema errors already make the document invalid: the verdict
                    // stands, the report says which stylesheet could not finish.
                    Error::Transform {
                        code,
                        line,
                        message,
                        ..
                    } if !schema_errors.is_empty() => StylesheetReport::failed(
                        entry,
                        StylesheetFailure {
                            code,
                            line,
                            message,
                        },
                        options.document_uri,
                    ),
                    other => return Err(other),
                },
            };
            stylesheets.push(report);
        }
        Ok(Report {
            rule_set: RuleSetVersion {
                name: self.rule_set.name().to_owned(),
                title: self.rule_set.title().to_owned(),
                version: self.rule_set.version().to_owned(),
                schema: self.rule_set.schema(),
            },
            document_uri: options.document_uri.map(str::to_owned),
            schema_errors,
            stylesheets,
        })
    }
}

fn read(path: &Path) -> Result<Vec<u8>, Error> {
    std::fs::read(path).map_err(|source| Error::Io {
        path: PathBuf::from(path),
        source,
    })
}

/// Lower-case hexadecimal SHA-256.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push(char::from(HEX[usize::from(byte >> 4)]));
        hex.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    hex
}

/// Checks the checksum of a stylesheet, loads and prepares it.
fn prepare(
    entry: &StylesheetEntry,
    bytes: &[u8],
    base_uri: Option<String>,
) -> Result<Executable, Error> {
    let actual = sha256_hex(bytes);
    if actual != entry.sha256() {
        return Err(Error::Checksum {
            path: entry.path().to_owned(),
            expected: entry.sha256().to_owned(),
            actual,
        });
    }
    let what = format!("stylesheet {} ({})", entry.name(), entry.path());
    let text = decode(bytes).map_err(|e| Error::bundle(format!("{what}: {e}")))?;
    let sheet = einvoice_xslt::load(text).map_err(|errors| {
        let unsupported = errors.0.iter().any(|e| {
            matches!(
                e.kind,
                LoadErrorKind::UnsupportedElement { .. }
                    | LoadErrorKind::UnsupportedAttribute { .. }
                    | LoadErrorKind::UnsupportedValue { .. }
                    | LoadErrorKind::XPath { .. }
            )
        });
        let message = format!("{what}: {}", first_lines(&errors.to_string()));
        if unsupported {
            Error::unsupported(message)
        } else {
            Error::bundle(message)
        }
    })?;
    let executable = Executable::new(&sheet).map_err(|errors| {
        Error::unsupported(format!("{what}: {}", first_lines(&errors.to_string())))
    })?;
    Ok(match base_uri {
        Some(uri) => executable.with_base_uri(uri),
        None => executable,
    })
}

/// At most the first ten lines of a list of errors, with the number left out.
fn first_lines(text: &str) -> String {
    const MAX: usize = 10;
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= MAX {
        return lines.join("; ");
    }
    format!(
        "{}; and {} more",
        lines[..MAX].join("; "),
        lines.len() - MAX
    )
}

fn transform_error(entry: &StylesheetEntry, e: &TransformError) -> Error {
    let at = e.line().map_or_else(String::new, |l| format!(", line {l}"));
    if let Some(limit) = e.limit() {
        return Error::Limit {
            limit,
            message: format!("stylesheet {}{at}: {}", entry.name(), e.message()),
        };
    }
    match e.code() {
        "UNSUPPORTED" => {
            Error::unsupported(format!("stylesheet {}{at}: {}", entry.name(), e.message()))
        }
        // The document was checked before: a parse error here means two parsers
        // disagree, which is a bug.
        "INTERNAL" | "SXXP0003" => Error::internal(format!(
            "stylesheet {}{at}: {}: {}",
            entry.name(),
            e.code(),
            e.message()
        )),
        code => Error::Transform {
            stylesheet: entry.name().to_owned(),
            code: code.to_owned(),
            line: e.line(),
            message: e.message().to_owned(),
        },
    }
}

/// The UTF-8 byte order mark.
const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// The text of a document given as bytes: UTF-8, with or without a byte order mark,
/// and an XML declaration, if any, naming UTF-8 or US-ASCII.
fn decode(bytes: &[u8]) -> Result<&str, Error> {
    refuse_utf16(bytes)?;
    let bytes = bytes.strip_prefix(BOM).unwrap_or(bytes);
    check_encoding(bytes)?;
    std::str::from_utf8(bytes).map_err(|e| not_utf8(bytes, &e))
}

/// [`decode`] on a buffer, which becomes the text without being copied (the byte order
/// mark, if any, is removed in place).
fn decode_owned(mut bytes: Vec<u8>) -> Result<String, Error> {
    refuse_utf16(&bytes)?;
    if bytes.starts_with(BOM) {
        bytes.drain(..BOM.len());
    }
    check_encoding(&bytes)?;
    String::from_utf8(bytes).map_err(|e| not_utf8(e.as_bytes(), &e.utf8_error()))
}

/// The error for bytes (after the byte order mark) that are not UTF-8.
fn not_utf8(bytes: &[u8], e: &std::str::Utf8Error) -> Error {
    bad_at(bytes, e.valid_up_to(), format!("not UTF-8: {e}"))
}

/// A [`Error::NotXml`] at the byte `valid_up_to` of `bytes` (after the byte order mark).
fn bad_at(bytes: &[u8], valid_up_to: usize, message: String) -> Error {
    let before = &bytes[..valid_up_to];
    let line_start = before
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |i| i + 1);
    let line = before.split(|&b| b == b'\n').count();
    let column = String::from_utf8_lossy(&before[line_start..])
        .chars()
        .count()
        + 1;
    Error::NotXml {
        line: u32::try_from(line).unwrap_or(u32::MAX),
        column: u32::try_from(column).unwrap_or(u32::MAX),
        message,
    }
}

/// Refuses a document that starts with a UTF-16 byte order mark.
fn refuse_utf16(bytes: &[u8]) -> Result<(), Error> {
    if bytes.starts_with(&[0xFE, 0xFF]) || bytes.starts_with(&[0xFF, 0xFE]) {
        return Err(Error::unsupported(
            "a UTF-16 document: only UTF-8 is supported",
        ));
    }
    Ok(())
}

/// Checks the XML declaration of a document given as bytes (after the byte order mark,
/// if any): its encoding, if any, must be UTF-8 or US-ASCII, and with US-ASCII every
/// byte must be ASCII. Whether the bytes are UTF-8 is checked by the caller.
fn check_encoding(bytes: &[u8]) -> Result<(), Error> {
    let ascii = match declared_encoding(bytes) {
        None => false,
        Some(e) if e.eq_ignore_ascii_case("utf-8") || e.eq_ignore_ascii_case("utf8") => false,
        Some(e) if e.eq_ignore_ascii_case("us-ascii") || e.eq_ignore_ascii_case("ascii") => true,
        Some(e) => {
            return Err(Error::unsupported(format!(
                "the {e} encoding: only UTF-8 is supported"
            )));
        }
    };
    if ascii && let Some(i) = bytes.iter().position(|b| !b.is_ascii()) {
        return Err(bad_at(
            bytes,
            i,
            "a byte outside US-ASCII, the encoding the XML declaration names".to_owned(),
        ));
    }
    Ok(())
}

/// The `encoding` pseudo-attribute of the XML declaration, if any.
fn declared_encoding(bytes: &[u8]) -> Option<&str> {
    let declaration = bytes.strip_prefix(b"<?xml")?;
    if !declaration.first().is_some_and(u8::is_ascii_whitespace) {
        return None;
    }
    let end = declaration.windows(2).position(|w| w == b"?>")?;
    let declaration = std::str::from_utf8(&declaration[..end]).ok()?;
    let rest = &declaration[declaration.find("encoding")? + "encoding".len()..];
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let value = &rest[1..];
    value.find(quote).map(|end| &value[..end])
}

/// Checks that the text is namespace-well-formed XML without a document type
/// declaration, as the stylesheets' parser requires, and without an element nested
/// deeper than `max_depth`.
fn check_well_formed(text: &str, max_depth: u32) -> Result<(), Error> {
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: u32::MAX,
        ..ParsingOptions::default()
    };
    match roxmltree::Document::parse_with_options(text, options) {
        Ok(document) => check_depth(&document, max_depth),
        Err(roxmltree::Error::DtdDetected) => Err(Error::unsupported(
            "a document type declaration (DTD): its entities and default attributes could change the document, so it is refused",
        )),
        Err(e) => Err(Error::NotXml {
            line: e.pos().row,
            column: e.pos().col,
            message: e.to_string(),
        }),
    }
}

/// [`Error::Limit`] for the first element, in document order, nested deeper than
/// `max_depth` (the document element is at depth 1).
fn check_depth(document: &roxmltree::Document<'_>, max_depth: u32) -> Result<(), Error> {
    // The elements still open, innermost last: their number is the depth of the parent
    // of the next element.
    let mut open: Vec<roxmltree::NodeId> = Vec::new();
    for node in document
        .root()
        .descendants()
        .filter(roxmltree::Node::is_element)
    {
        let parent = node.parent().map(|p| p.id());
        while open.last().is_some_and(|&id| Some(id) != parent) {
            open.pop();
        }
        if u32::try_from(open.len()).map_or(true, |depth| depth >= max_depth) {
            let at = document.text_pos_at(node.range().start);
            let limit = LimitExceeded::Depth {
                max: max_depth,
                line: at.row,
                column: at.col,
            };
            return Err(Error::Limit {
                limit,
                message: format!("document, {limit}"),
            });
        }
        open.push(node.id());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoding_a_buffer_gives_the_same_result() {
        for bytes in [
            &b"<a/>"[..],
            b"\xEF\xBB\xBF<a>\xC3\xA9</a>",
            b"<?xml version='1.0' encoding='UTF-8'?><a/>",
            b"\xFF\xFE<\0a\0/\0>\0",
            b"\xFE\xFF\0<\0a\0/\0>",
            b"<?xml version='1.0' encoding='ISO-8859-1'?><a/>",
            b"<a>\n  \xC3\xA9\xFF</a>",
            b"\xEF\xBB\xBF<a>\n\xFF</a>",
            b"<?xml version=\"1.0\" encoding=\"US-ASCII\"?>\n<a>\xC3\xA9</a>",
            b"\xEF\xBB\xBF",
            b"",
        ] {
            let borrowed = decode(bytes).map(str::to_owned).map_err(|e| e.to_string());
            let owned = decode_owned(bytes.to_vec()).map_err(|e| e.to_string());
            assert_eq!(owned, borrowed, "{bytes:?}");
        }
    }

    #[test]
    fn decoding() {
        assert_eq!(decode(b"\xEF\xBB\xBF<a/>").unwrap(), "<a/>");
        assert_eq!(
            decode(b"<?xml version='1.0' encoding='UTF-8'?><a/>").unwrap(),
            "<?xml version='1.0' encoding='UTF-8'?><a/>"
        );
        assert!(matches!(
            decode(b"\xFF\xFE<\0a\0/\0>\0"),
            Err(Error::Unsupported { .. })
        ));
        assert!(matches!(
            decode(b"<?xml version='1.0' encoding='ISO-8859-1'?><a/>"),
            Err(Error::Unsupported { .. })
        ));
        match decode(b"<a>\n  \xC3\xA9\xFF</a>") {
            Err(Error::NotXml { line, column, .. }) => assert_eq!((line, column), (2, 4)),
            other => panic!("{other:?}"),
        }
        match decode(b"<?xml version=\"1.0\" encoding=\"US-ASCII\"?>\n<a>\xC3\xA9</a>") {
            Err(Error::NotXml { line, column, .. }) => assert_eq!((line, column), (2, 4)),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            declared_encoding(b"<?xml-stylesheet encoding='x'?><a/>"),
            None
        );
    }

    #[test]
    fn well_formedness() {
        assert!(check_well_formed("<a/>", 1).is_ok());
        match check_well_formed("<a>\n<b></a>", DEFAULT_MAX_DEPTH) {
            Err(Error::NotXml { line, .. }) => assert_eq!(line, 2),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            check_well_formed(
                "<!DOCTYPE a [<!ENTITY e 'x'>]><a>&e;</a>",
                DEFAULT_MAX_DEPTH
            ),
            Err(Error::Unsupported { .. })
        ));
        assert!(matches!(
            check_well_formed("<p:a/>", DEFAULT_MAX_DEPTH),
            Err(Error::NotXml { .. })
        ));
    }

    #[test]
    fn checksums() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn errors_are_summarised() {
        let many: Vec<String> = (1..=12).map(|i| format!("line {i}: x")).collect();
        let text = first_lines(&many.join("\n"));
        assert!(text.ends_with("line 10: x; and 2 more"), "{text}");
    }
}
