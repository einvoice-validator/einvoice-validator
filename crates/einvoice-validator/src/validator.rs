//! Loading a rule set once, then validating documents with it.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use einvoice_xpath_eval::{
    Clock, CompiledExpr, Context, DEFAULT_MAX_DEPTH, Document, DynamicContext, ErrorCode, Item,
    LimitExceeded, NoHost, SimpleStaticContext,
};
use einvoice_xslt::LoadErrorKind;
use einvoice_xslt_exec::{
    DEFAULT_MAX_STACK, Executable, TransformError, TransformOptions, max_nesting_for_depth,
};
use sha2::{Digest, Sha256};

use crate::manifest::{
    CII, MANIFEST_FILE, RuleSet, Scenario, SchemaDocumentEntry, SchemaId, StylesheetEntry,
    UBL_CREDIT_NOTE, UBL_INVOICE, namespace_map,
};
use crate::report::{
    NoScenario, Report, RuleSetVersion, SchemaError, Severities, StylesheetFailure,
    StylesheetReport, SvrlReader,
};
use crate::resources::{BUNDLE_URI, Resources};
use crate::{Error, builtin};

/// Options of one validation.
#[derive(Debug, Clone, Copy)]
pub struct ValidateOptions<'a> {
    document_uri: Option<&'a str>,
    clock: Option<Clock>,
    max_depth: u32,
    max_stack: u32,
    fail_closed: bool,
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
            fail_closed: false,
        }
    }

    /// Fail closed: a finding that would otherwise be of severity
    /// [`Severity::Other`](crate::Severity::Other) (a flag other than `fatal` and
    /// `warning`, or none, unless the manifest of the rule set gives it a severity) is of
    /// severity [`Severity::Fatal`](crate::Severity::Fatal): it makes the document
    /// invalid. Off by default. For rule sets whose flags the caller does not know in
    /// advance: every finding that is not a warning then rejects the document.
    #[must_use]
    pub const fn fail_closed(mut self, fail_closed: bool) -> Self {
        self.fail_closed = fail_closed;
        self
    }

    /// The maximum element depth of the document, and of any document a stylesheet
    /// loads with `document()`: the document element is at depth 1, its children at
    /// depth 2, and so on. A document with an element nested deeper is refused with
    /// [`Error::Limit`] before its schema is checked. Default: [`DEFAULT_MAX_DEPTH`]
    /// (256, libxml2's default).
    ///
    /// The stylesheets apply their template rules one element level inside another, on
    /// the heap, not on the call stack: the stack a validation uses does not grow with
    /// the depth, and a validation runs on a thread with the default 2 MiB stack, debug
    /// builds included. The nesting of template and function invocations the stylesheets
    /// are allowed grows with the maximum
    /// ([`max_nesting_for_depth`](crate::max_nesting_for_depth)), and so does the memory
    /// they may take, a few hundred bytes per level.
    ///
    /// On WebAssembly, at most [`WASM_MAX_DEPTH`]: a validation asking for more fails with
    /// [`Error::OptionOutOfRange`].
    #[must_use]
    pub const fn max_depth(mut self, max_depth: u32) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// The most call stack, in bytes, the stylesheets may use, measured from the start of
    /// each stylesheet: beyond it, the validation fails with [`Error::Limit`]
    /// ([`LimitExceeded::Stack`]) instead of overflowing the stack, which would abort the
    /// process. What uses it is the evaluation of their expressions and what that calls:
    /// stylesheet functions, values computed when first read; their template invocations
    /// do not. Default: [`DEFAULT_MAX_STACK`] (1.5 MiB), for the default 2 MiB stack of a
    /// spawned thread. The thread that validates must have this much stack and at least
    /// 512 KiB more, for its caller and the deepest invocation.
    ///
    /// On WebAssembly, the default is 256 KiB, and so is the most allowed,
    /// [`WASM_MAX_STACK`]: a validation asking for more fails with
    /// [`Error::OptionOutOfRange`].
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

/// The largest [`ValidateOptions::max_depth`] on WebAssembly: 400.
///
/// A WebAssembly engine runs the module's functions on a native stack of its own, beside
/// the module's stack in linear memory, which is the only one the library measures
/// ([`ValidateOptions::max_stack`]). When the native stack runs out first, the engine
/// throws (a `RangeError` in V8, an `InternalError` in Firefox) and leaves the module's
/// memory inconsistent, so that every later call fails. What keeps the native stack from
/// running out is [`WASM_MAX_STACK`]. The stylesheets' template invocations take no call
/// stack (they are kept on the heap), so that every document within this maximum depth
/// gets a report: measured with the npm package (opt-level 3, `wasm-opt -O3`) in Chrome
/// (main thread and Web Worker) and Node.js, and with the `wasm-validate` example in
/// wasmtime, the stylesheets of the built-in rule sets validate documents 256 and 400
/// levels deep, nested inside a UBL or a CII invoice, a rule firing and reporting on
/// every nested element, with the default 256 KiB of stack; natively, with the same
/// limits, the stylesheets of Peppol BIS Billing 3.0.22, XRechnung 3.0.2, Factur-X and
/// the French rules do too. The nesting of template and function invocations that comes
/// with this maximum ([`max_nesting_for_depth`](crate::max_nesting_for_depth): 864) takes
/// a few hundred bytes of memory per level.
pub const WASM_MAX_DEPTH: u32 = 400;

/// The largest [`ValidateOptions::max_stack`] on WebAssembly, and its default there
/// ([`DEFAULT_MAX_STACK`]): 256 KiB.
///
/// The native stack of a JavaScript engine can run out before the module's stack in
/// linear memory reaches [`ValidateOptions::max_stack`] (see [`WASM_MAX_DEPTH`]).
/// Measured with the npm package, the largest `max_stack`, in KiB, with which a
/// recursion still ends with [`Error::Limit`] ([`LimitExceeded::Stack`]) rather than with
/// the engine's stack overflow, for the stylesheets of the built-in rule sets and of
/// Peppol BIS Billing 3.0.22 on a document 8192 levels deep / a named template calling
/// itself (not as a tail call) / a template rule applying itself / a function calling
/// itself:
///
/// | Engine | Official | Named template | Template rule | Function |
/// |---|---|---|---|---|
/// | Chrome 154, Web Worker | 320 | 342 | 348 | 543 |
/// | Chrome 154, main thread | 522 | 512 | 545 | 863 |
/// | Firefox 153, Web Worker | 630 | 674 | 689 | 1021 |
/// | Firefox 153, main thread | 1021 | 1021 | 1021 | 1021 |
/// | Node.js 22 | 640 | 679 | 685 | 1021 |
///
/// (1021 KiB is the module's own stack, 1 MiB, running out.) 256 KiB keeps a margin of a
/// fifth below the smallest, a Chrome Web Worker.
pub const WASM_MAX_STACK: u32 = 256 << 10;

/// What [`ValidateOptions`] may ask for on this target: the largest maximum depth and the
/// largest stack, or `None` for no bound.
#[cfg(target_family = "wasm")]
const CAPS: Option<(u32, u32)> = Some((WASM_MAX_DEPTH, WASM_MAX_STACK));
#[cfg(not(target_family = "wasm"))]
const CAPS: Option<(u32, u32)> = None;

// The defaults are within the caps.
#[cfg(target_family = "wasm")]
const _: () = assert!(DEFAULT_MAX_STACK <= WASM_MAX_STACK && DEFAULT_MAX_DEPTH <= WASM_MAX_DEPTH);

impl ValidateOptions<'_> {
    /// [`Error::OptionOutOfRange`] when the options ask for more than `caps` (the largest
    /// maximum depth and the largest stack) allow.
    const fn check(&self, caps: Option<(u32, u32)>) -> Result<(), Error> {
        let Some((max_depth, max_stack)) = caps else {
            return Ok(());
        };
        if self.max_depth > max_depth {
            return Err(Error::OptionOutOfRange {
                option: "max_depth",
                value: self.max_depth,
                max: max_depth,
            });
        }
        if self.max_stack > max_stack {
            return Err(Error::OptionOutOfRange {
                option: "max_stack",
                value: self.max_stack,
                max: max_stack,
            });
        }
        Ok(())
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
/// the rule set is copied per document. A clone shares the loaded rule set (it is
/// reference-counted): cloning is cheap, and the clones can be moved to other threads.
#[derive(Debug, Clone)]
pub struct Validator {
    rule_set: RuleSet,
    schema: Option<Arc<einvoice_xsd::Schema>>,
    /// The prepared stylesheets, in the order of `rule_set.stylesheets()`.
    stylesheets: Arc<[Executable]>,
    /// The resources the stylesheets read with `document()`.
    resources: Arc<Resources>,
    /// In a rule set that selects by scenario, what each scenario selects, in the order
    /// of `rule_set.scenarios()`; empty otherwise.
    selections: Arc<[Selection]>,
    /// The indices of every stylesheet, in order: what runs without a selection.
    all: Arc<[usize]>,
}

/// A scenario of a rule set that selects by scenario, prepared: its `match` expression
/// compiled, and the stylesheets it runs.
#[derive(Debug)]
struct Selection {
    matches: CompiledExpr,
    /// Indices in `rule_set.stylesheets()`, in the order they run.
    stylesheets: Vec<usize>,
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
    /// CII D16B, release 1.3.17 (EUPL-1.2), embedded unmodified, with the UN/CEFACT CII
    /// D16B schemas embedded in [`einvoice_xsd`] ([`SchemaId::CiiD16b`]).
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
        let file = |path: &str| files.iter().find(|(p, _)| *p == path).map(|(_, b)| *b);
        let mut stylesheets = Vec::with_capacity(rule_set.stylesheets().len());
        for entry in rule_set.stylesheets() {
            let Some(bytes) = file(entry.path()) else {
                return Err(Error::bundle(format!(
                    "stylesheet {}: file {} missing from the bundle",
                    entry.name(),
                    entry.path()
                )));
            };
            // In memory, the files of the bundle have URIs of their own.
            let base_uri = format!("{BUNDLE_URI}{}", entry.path());
            stylesheets.push(prepare(entry, bytes, Some(base_uri))?);
        }
        let mut resources = Resources::default();
        for entry in rule_set.resources() {
            let Some(bytes) = file(entry.path()) else {
                return Err(Error::bundle(format!(
                    "resource {}: file missing from the bundle",
                    entry.path()
                )));
            };
            resources.add(entry, bytes, format!("{BUNDLE_URI}{}", entry.path()))?;
        }
        let mut schema_documents = Vec::with_capacity(rule_set.schema_documents().len());
        for entry in rule_set.schema_documents() {
            let Some(bytes) = file(entry.path()) else {
                return Err(Error::bundle(format!(
                    "schema document {}: file missing from the bundle",
                    entry.path()
                )));
            };
            schema_documents.push(schema_document(entry, bytes)?);
        }
        Self::with(rule_set, stylesheets, resources, &schema_documents)
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
        let mut resources = Resources::default();
        for entry in rule_set.resources() {
            let mut path = dir.to_path_buf();
            path.extend(entry.path().split('/'));
            let bytes = read(&path)?;
            let uri = std::path::absolute(&path)
                .ok()
                .and_then(|p| file_uri(&p))
                .ok_or_else(|| {
                    Error::bundle(format!(
                        "resource {}: no file URI for {}",
                        entry.path(),
                        path.display()
                    ))
                })?;
            resources.add(entry, &bytes, uri)?;
        }
        let mut schema_documents = Vec::with_capacity(rule_set.schema_documents().len());
        for entry in rule_set.schema_documents() {
            let mut path = dir.to_path_buf();
            path.extend(entry.path().split('/'));
            schema_documents.push(schema_document(entry, &read(&path)?)?);
        }
        Self::with(rule_set, stylesheets, resources, &schema_documents)
    }

    /// The validator of `rule_set`, its stylesheets prepared, its resources and the
    /// documents of its schema, if the bundle has one (path and text), read.
    fn with(
        rule_set: RuleSet,
        stylesheets: Vec<Executable>,
        resources: Resources,
        schema_documents: &[(String, String)],
    ) -> Result<Self, Error> {
        let schema = match rule_set.schema() {
            None => None,
            Some(id) => Some(load_schema(id, schema_documents)?),
        };
        let selections = if rule_set.selects_by_scenario() {
            rule_set
                .scenarios()
                .iter()
                .map(|scenario| selection(&rule_set, scenario))
                .collect::<Result<Vec<_>, Error>>()?
        } else {
            Vec::new()
        };
        Ok(Self {
            rule_set,
            schema: schema.map(Arc::new),
            all: (0..stylesheets.len()).collect(),
            stylesheets: stylesheets.into(),
            resources: Arc::new(resources),
            selections: selections.into(),
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
        options.check(CAPS)?;
        self.validate_text(Cow::Owned(decode_owned(document)?), options)
    }

    /// Validates a document: checks that it is well-formed XML, validates it against
    /// the rule set's schema, then runs every stylesheet of the rule set on it. The
    /// stylesheets run even when the schema reports errors, so that the report is
    /// complete.
    ///
    /// In a rule set that selects by scenario
    /// ([`RuleSet::selects_by_scenario`](crate::RuleSet::selects_by_scenario)), the
    /// `match` expression of each scenario is then evaluated on the document, as the
    /// KoSIT validator evaluates it (an expression that raises a dynamic error is false):
    /// when exactly one is true, the stylesheets of that scenario run, in its order;
    /// otherwise the report has no stylesheet and no schema error, only
    /// [`Report::no_scenario`], and the document is invalid.
    ///
    /// When a stylesheet raises a dynamic error on a document that has schema
    /// errors (a non-numeric amount the rules compute with, ...), the report is returned
    /// all the same, invalid, with the error in [`StylesheetReport::failure`]; without
    /// schema errors, the error is returned.
    ///
    /// The document must be in an encoding the validator reads (UTF-8, UTF-16, US-ASCII, ISO-8859-1, ISO-8859-15 or windows-1252,
    /// see [`einvoice_xml::decode`]) and have no document type declaration.
    ///
    /// # Errors
    ///
    /// [`Error::NotXml`] when the document is not well-formed XML (bytes that are not
    /// characters of its encoding included); [`Error::Unsupported`] for an encoding the
    /// validator does not read, a document type declaration, or a
    /// construct the engine refuses; [`Error::Limit`] for a document beyond a limit;
    /// [`Error::OptionOutOfRange`] for options beyond what the target allows (on
    /// WebAssembly); [`Error::Transform`] when a stylesheet raises a dynamic error on
    /// the document; [`Error::Bundle`] when a stylesheet's result is not an SVRL report;
    /// [`Error::Internal`] for a bug.
    pub fn validate_with(
        &self,
        document: &[u8],
        options: &ValidateOptions<'_>,
    ) -> Result<Report, Error> {
        options.check(CAPS)?;
        self.validate_text(decode(document)?, options)
    }

    /// The validation of a decoded document: borrowed by the schema and every stylesheet
    /// but the last, which is given it when it is owned.
    fn validate_text(
        &self,
        text: Cow<'_, str>,
        options: &ValidateOptions<'_>,
    ) -> Result<Report, Error> {
        let selects = self.rule_set.selects_by_scenario();
        let identify = !selects && !self.rule_set.scenarios().is_empty();
        let checked = check(&text, options.max_depth, self.schema.as_deref(), identify)?;
        let schema_errors = checked.schema_errors;
        let (scenario, run) = if selects {
            match self.select(&text, options)? {
                Ok(index) => (
                    self.rule_set.scenarios().get(index),
                    self.selections
                        .get(index)
                        .map(|s| s.stylesheets.as_slice())
                        .unwrap_or_default(),
                ),
                // As the KoSIT validator's fallback scenario: no schema, no stylesheet,
                // rejected.
                Err(no_scenario) => return Ok(self.without_scenario(no_scenario, options)),
            }
        } else {
            (
                checked.identification.scenario(self.rule_set.scenarios()),
                self.all.as_ref(),
            )
        };
        let transform_options = TransformOptions {
            implicit_timezone: options.clock.map(|c| c.implicit_timezone()),
            current_date_time: options.clock.map(|c| c.current_date_time()),
            max_depth: options.max_depth,
            max_nesting: max_nesting_for_depth(options.max_depth),
            max_stack: options.max_stack,
            resolver: Some(&*self.resources),
            ..TransformOptions::default()
        };
        let mut stylesheets = Vec::with_capacity(run.len());
        let count = run.len();
        // The text, until the last stylesheet is given it.
        let mut text = Some(text);
        for (index, &stylesheet) in run.iter().enumerate() {
            let (Some(entry), Some(executable)) = (
                self.rule_set.stylesheets().get(stylesheet),
                self.stylesheets.get(stylesheet),
            ) else {
                return Err(Error::internal(
                    "a scenario names no stylesheet of the rule set",
                ));
            };
            let source = if index + 1 == count {
                text.take()
            } else {
                text.as_deref().map(Cow::Borrowed)
            }
            .ok_or_else(|| Error::internal("the document text was given away early"))?;
            // The findings are read as the stylesheet writes its report.
            let mut reader = SvrlReader::new(Severities {
                fail_closed: options.fail_closed,
                unflagged_assert: entry.unflagged_assert(),
                scenario,
            });
            let result = executable.transform_compact(
                source,
                options.document_uri,
                &transform_options,
                &mut reader,
            );
            let report = match result {
                Ok(result) => StylesheetReport::new(
                    entry,
                    result.output,
                    reader,
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
            rule_set: self.version(),
            document_uri: options.document_uri.map(str::to_owned),
            scenario: scenario.map(|s| s.name().to_owned()),
            no_scenario: None,
            schema_errors,
            stylesheets,
        })
    }

    /// The report on a document that has no scenario, in a rule set that selects by
    /// scenario.
    fn without_scenario(&self, no_scenario: NoScenario, options: &ValidateOptions<'_>) -> Report {
        Report {
            rule_set: self.version(),
            document_uri: options.document_uri.map(str::to_owned),
            scenario: None,
            no_scenario: Some(no_scenario),
            schema_errors: Vec::new(),
            stylesheets: Vec::new(),
        }
    }

    /// The rule set, as a report names it.
    fn version(&self) -> RuleSetVersion {
        RuleSetVersion {
            name: self.rule_set.name().to_owned(),
            title: self.rule_set.title().to_owned(),
            version: self.rule_set.version().to_owned(),
            schema: self.rule_set.schema().cloned(),
            unchecked_schema: self.rule_set.unchecked_schema().map(str::to_owned),
        }
    }

    /// The scenario of a rule set that selects by scenario that a document (checked
    /// well-formed, and not nested deeper than `options.max_depth`) matches: the index of
    /// the one whose `match` expression is true on it, or why there is none. The
    /// expressions are evaluated as the KoSIT validator evaluates them: with the document
    /// node as the context item, the effective boolean value; one that raises a dynamic
    /// error is false (the validator logs the error and goes on). A construct the engine
    /// refuses, or a limit, is an error rather than a false.
    fn select(
        &self,
        text: &str,
        options: &ValidateOptions<'_>,
    ) -> Result<Result<usize, NoScenario>, Error> {
        let document =
            Document::parse_with_max_depth(text, None, options.max_depth).map_err(|e| {
                // The XML check accepted the text: two parsers disagree, a bug.
                Error::internal(format!("the document for the scenarios: {}", e.message()))
            })?;
        let clock = match options.clock {
            Some(clock) => clock,
            None => system_clock()?,
        };
        let dc = DynamicContext::new(&NoHost, clock);
        let context = Context::for_item(Item::Node(document.root()));
        let mut matched = Vec::new();
        for (index, (scenario, selection)) in self
            .rule_set
            .scenarios()
            .iter()
            .zip(self.selections.iter())
            .enumerate()
        {
            match selection.matches.effective_boolean_value(&dc, &context) {
                Ok(true) => matched.push(index),
                Ok(false) => {}
                Err(e) => {
                    let what = format!("scenario {:?}: match: {}", scenario.name(), e.message());
                    if let Some(limit) = e.limit() {
                        return Err(Error::Limit {
                            limit,
                            message: what,
                        });
                    }
                    match e.code() {
                        ErrorCode::Unsupported => return Err(Error::unsupported(what)),
                        ErrorCode::Internal => return Err(Error::internal(what)),
                        // A dynamic error: the KoSIT validator takes the scenario not to
                        // match.
                        _ => {}
                    }
                }
            }
        }
        Ok(match matched[..] {
            [index] => Ok(index),
            [] => Err(NoScenario::NoneMatches),
            _ => Err(NoScenario::SeveralMatch(
                matched
                    .iter()
                    .filter_map(|&i| self.rule_set.scenarios().get(i))
                    .map(|s| s.name().to_owned())
                    .collect(),
            )),
        })
    }
}

/// The clock of the `match` expressions when the options give none: the system clock in
/// UTC, as for the stylesheets.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
fn system_clock() -> Result<Clock, Error> {
    Clock::system(0).map_err(|e| Error::internal(format!("the system clock: {}", e.message())))
}

/// No system clock on this target: the caller must give one.
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
fn system_clock() -> Result<Clock, Error> {
    Err(Error::unsupported(
        "no system clock on this target: give ValidateOptions::clock",
    ))
}

/// A scenario of a rule set that selects by scenario, prepared: its `match` expression
/// parsed with its namespaces and compiled, as an XPath expression without the XSLT
/// functions; its stylesheets by index, every one of the rule set when it names none.
fn selection(rule_set: &RuleSet, scenario: &Scenario) -> Result<Selection, Error> {
    let what = format!("scenario {:?}: match", scenario.name());
    let Some(text) = scenario.match_expression() else {
        return Err(Error::internal(format!("{what}: none")));
    };
    let expr = einvoice_xpath::parse_expr(text, &namespace_map(scenario.namespaces()))
        .map_err(|e| Error::bundle(format!("{what}: {e}")))?;
    // A function or construct the engine does not have: refused, not evaluated
    // differently.
    let matches = einvoice_xpath_eval::compile(&expr, &SimpleStaticContext::new())
        .map_err(|e| Error::unsupported(format!("{what}: {}", e.message())))?;
    let stylesheets = match scenario.stylesheets() {
        None => (0..rule_set.stylesheets().len()).collect(),
        Some(names) => names
            .iter()
            .map(|name| {
                rule_set
                    .stylesheets()
                    .iter()
                    .position(|s| s.name() == name)
                    .ok_or_else(|| Error::internal(format!("{what}: no stylesheet {name}")))
            })
            .collect::<Result<_, Error>>()?,
    };
    Ok(Selection {
        matches,
        stylesheets,
    })
}

fn read(path: &Path) -> Result<Vec<u8>, Error> {
    std::fs::read(path).map_err(|source| Error::Io {
        path: PathBuf::from(path),
        source,
    })
}

/// A document of the schema of a bundle: its SHA-256 checked against the manifest's, its
/// text decoded as a document is (any supported encoding); its path in the bundle and its text.
fn schema_document(entry: &SchemaDocumentEntry, bytes: &[u8]) -> Result<(String, String), Error> {
    let actual = sha256_hex(bytes);
    if actual != entry.sha256() {
        return Err(Error::Checksum {
            path: entry.path().to_owned(),
            expected: entry.sha256().to_owned(),
            actual,
        });
    }
    let text = decode(bytes)
        .map_err(|e| Error::bundle(format!("schema document {}: {e}", entry.path())))?;
    Ok((entry.path().to_owned(), text.into_owned()))
}

/// Loads the schema `id`: an embedded one, or that of the bundle from its documents
/// (path and text).
fn load_schema(
    id: &SchemaId,
    documents: &[(String, String)],
) -> Result<einvoice_xsd::Schema, Error> {
    let loaded = match id {
        SchemaId::Ubl21 => einvoice_xsd::Schema::ubl_2_1(),
        SchemaId::CiiD16b => einvoice_xsd::Schema::cii_d16b(),
        SchemaId::Bundle(entry_point) => {
            let files: Vec<(&str, &str)> = documents
                .iter()
                .map(|(path, text)| (path.as_str(), text.as_str()))
                .collect();
            einvoice_xsd::Schema::load(&files, &[entry_point.as_str()])
        }
    };
    loaded.map_err(|e| {
        if e.kind == einvoice_xsd::LoadErrorKind::Unsupported {
            Error::unsupported(format!("schema {id}: {e}"))
        } else if matches!(id, SchemaId::Bundle(_)) {
            Error::bundle(format!("schema {id}: {e}"))
        } else {
            Error::internal(format!("the embedded schema {id} does not load: {e}"))
        }
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
    let sheet = einvoice_xslt::load(&text).map_err(|errors| {
        let unsupported = errors.0.iter().any(|e| {
            matches!(
                e.kind,
                LoadErrorKind::UnsupportedElement { .. }
                    | LoadErrorKind::UnsupportedAttribute { .. }
                    | LoadErrorKind::UnsupportedValue { .. }
                    | LoadErrorKind::XPath { .. }
                    | LoadErrorKind::TooDeep { .. }
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

/// The text of a document given as bytes, decoded with [`einvoice_xml::decode`]: UTF-8
/// (borrowed), UTF-16, US-ASCII, ISO-8859-1, ISO-8859-15 or windows-1252, as its byte
/// order mark, first bytes and XML declaration say.
pub(crate) fn decode(bytes: &[u8]) -> Result<Cow<'_, str>, Error> {
    einvoice_xml::decode(bytes).map_err(decoding_error)
}

/// [`decode`] on a buffer, which becomes the text without being copied when it is UTF-8
/// (the byte order mark, if any, is removed in place).
fn decode_owned(bytes: Vec<u8>) -> Result<String, Error> {
    einvoice_xml::decode_owned(bytes).map_err(decoding_error)
}

/// The error for a document that cannot be decoded: [`Error::Unsupported`] for an
/// encoding that is not read, [`Error::NotXml`] otherwise (bytes that are not a character
/// of the encoding, first bytes and an XML declaration that disagree, a second byte
/// order mark).
fn decoding_error(e: einvoice_xml::Error) -> Error {
    match e {
        einvoice_xml::Error::UnsupportedEncoding(name) => Error::unsupported(format!(
            "the {name} encoding: the encodings read are UTF-8, UTF-16, US-ASCII, ISO-8859-1, ISO-8859-15 and windows-1252"
        )),
        e => Error::NotXml {
            line: e.pos().row,
            column: e.pos().col,
            message: e.to_string(),
        },
    }
}

/// The namespace of the UBL basic components (`cbc:`).
const UBL_CBC: &str = "urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2";
/// The namespace of the CII aggregate components (`ram:`).
const CII_RAM: &str =
    "urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100";

/// The element that holds the specification identifier (BT-24) of a document with this
/// root element: its path from the root, by namespace and local name.
fn identifier_path(namespace: &str, local: &str) -> &'static [(&'static str, &'static str)] {
    match (namespace, local) {
        (UBL_INVOICE, "Invoice") | (UBL_CREDIT_NOTE, "CreditNote") => {
            &[(UBL_CBC, "CustomizationID")]
        }
        (CII, "CrossIndustryInvoice") => &[
            (CII, "ExchangedDocumentContext"),
            (CII_RAM, "GuidelineSpecifiedDocumentContextParameter"),
            (CII_RAM, "ID"),
        ],
        _ => &[],
    }
}

/// The root element of a document and its specification identifiers (BT-24), read with
/// the events of the XML check: what picks the scenario of the rule set that applies.
#[derive(Debug, Default)]
struct Identification {
    /// The namespace and local name of the root element.
    root: Option<(String, String)>,
    /// The path of the identifier element from the root (see [`identifier_path`]).
    path: &'static [(&'static str, &'static str)],
    /// How many steps of the path the open elements below the root match.
    matched: usize,
    /// The text of the identifier element being read.
    text: Option<String>,
    /// The string value of every identifier element, in document order.
    identifiers: Vec<String>,
}

impl Identification {
    /// A start tag, the element at `depth` (the root at 1).
    fn start(&mut self, name: &einvoice_xml::Name<'_, '_>, depth: u32) {
        let Ok(depth) = usize::try_from(depth) else {
            return;
        };
        let namespace = name.namespace.unwrap_or("");
        if depth == 1 {
            self.path = identifier_path(namespace, name.local);
            self.root = Some((namespace.to_owned(), name.local.to_owned()));
            return;
        }
        // The parent matches the path up to its own step: this element may be the next.
        if self.matched + 2 == depth
            && self
                .path
                .get(self.matched)
                .is_some_and(|&(ns, local)| ns == namespace && local == name.local)
        {
            self.matched += 1;
            if self.matched == self.path.len() {
                self.text = Some(String::new());
            }
        }
    }

    /// The end of the element at `depth`.
    fn end(&mut self, depth: u32) {
        let Ok(depth) = usize::try_from(depth) else {
            return;
        };
        if depth >= 2 && self.matched + 1 == depth {
            if self.matched == self.path.len()
                && let Some(text) = self.text.take()
            {
                self.identifiers.push(text);
            }
            self.matched -= 1;
        }
    }

    /// Text, within the identifier element or not.
    fn text(&mut self, text: &str) {
        if let Some(value) = &mut self.text {
            value.push_str(text);
        }
    }

    /// The first of `scenarios` (without `match` expressions) the document matches.
    fn scenario<'s>(&self, scenarios: &'s [Scenario]) -> Option<&'s Scenario> {
        let (namespace, local) = self.root.as_ref()?;
        scenarios.iter().find(|s| {
            s.namespace() == Some(namespace.as_str())
                && s.root() == Some(local.as_str())
                && self
                    .identifiers
                    .iter()
                    .any(|i| Some(i.as_str()) == s.identifier())
        })
    }
}

/// What the XML check found.
#[derive(Debug)]
struct Checked {
    /// The schema errors, sorted by position.
    schema_errors: Vec<SchemaError>,
    /// The root element and the specification identifiers, when asked for.
    identification: Identification,
}

/// Reads the text once, with the streaming reader of `einvoice-xml`: checks that it is
/// namespace-well-formed XML without a document type declaration, as the stylesheets'
/// parser requires, and without an element nested deeper than `max_depth`, and
/// validates it against `schema`, if any, in the same pass; with `identify`, reads the
/// root element and the specification identifiers (BT-24) too. The schema errors,
/// sorted by position.
///
/// A well-formedness error comes first, wherever it is in the text; then an element
/// nested too deep, the first in document order (the schema is not checked from it
/// on); then a construct of the schema stage outside its subset.
fn check(
    text: &str,
    max_depth: u32,
    schema: Option<&einvoice_xsd::Schema>,
    identify: bool,
) -> Result<Checked, Error> {
    let mut reader = einvoice_xml::Reader::new(text);
    let mut validation = schema.map(|schema| schema.validation(text));
    let mut identification = Identification::default();
    let mut depth = 0u32;
    let mut too_deep = None;
    loop {
        let event = match reader.next_event() {
            Ok(Some(event)) => event,
            Ok(None) => break,
            Err(einvoice_xml::Error::DtdDetected) => {
                return Err(Error::unsupported(
                    "a document type declaration (DTD): its entities and default attributes could change the document, so it is refused",
                ));
            }
            Err(e) => {
                return Err(Error::NotXml {
                    line: e.pos().row,
                    column: e.pos().col,
                    message: e.to_string(),
                });
            }
        };
        match &event {
            einvoice_xml::Event::Start(start) => {
                depth = depth.saturating_add(1);
                if identify {
                    identification.start(&start.name(), depth);
                }
                if depth > max_depth && too_deep.is_none() {
                    let at = einvoice_xml::TextPos::at(text, start.range().start);
                    too_deep = Some(LimitExceeded::Depth {
                        max: max_depth,
                        line: at.row,
                        column: at.col,
                    });
                    // The rest is only checked for well-formedness.
                    validation = None;
                }
            }
            einvoice_xml::Event::End(_) => {
                if identify {
                    identification.end(depth);
                }
                depth = depth.saturating_sub(1);
            }
            einvoice_xml::Event::Text(t) if identify => identification.text(&t.value),
            _ => {}
        }
        if let Some(validation) = &mut validation {
            validation.event(event);
        }
    }
    if let Some(limit) = too_deep {
        return Err(Error::Limit {
            limit,
            message: format!("document, {limit}"),
        });
    }
    let schema_errors = match validation {
        None => Vec::new(),
        Some(validation) => schema_errors(validation)?,
    };
    Ok(Checked {
        schema_errors,
        identification,
    })
}

/// The schema errors of a validation that read the whole document.
fn schema_errors(validation: einvoice_xsd::Validation<'_, '_>) -> Result<Vec<SchemaError>, Error> {
    match validation.finish() {
        Ok(report) => Ok(report.errors),
        Err(einvoice_xsd::DocumentError::Malformed {
            line,
            column,
            message,
        }) => Err(Error::NotXml {
            line,
            column,
            message,
        }),
        Err(einvoice_xsd::DocumentError::Unsupported {
            line,
            column,
            message,
        }) => Err(Error::unsupported(format!(
            "document, line {line}, column {column}: {message}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The schema errors of the XML check, without the identification.
    fn check(
        text: &str,
        max_depth: u32,
        schema: Option<&einvoice_xsd::Schema>,
    ) -> Result<Vec<SchemaError>, Error> {
        super::check(text, max_depth, schema, false).map(|c| c.schema_errors)
    }

    /// The root element and the specification identifiers the XML check reads.
    fn identify(text: &str) -> (Option<(String, String)>, Vec<String>) {
        let checked = super::check(text, DEFAULT_MAX_DEPTH, None, true).unwrap();
        (
            checked.identification.root,
            checked.identification.identifiers,
        )
    }

    #[test]
    fn specification_identifiers() {
        let root = |ns: &str, local: &str| Some((ns.to_owned(), local.to_owned()));
        let ubl = format!(
            "<Invoice xmlns='{UBL_INVOICE}' xmlns:cbc='{UBL_CBC}'><cbc:CustomizationID> urn:a</cbc:CustomizationID>\
             <x><cbc:CustomizationID>no</cbc:CustomizationID></x>\
             <cbc:CustomizationID>u<!-- c -->rn:<![CDATA[b]]><i>c</i></cbc:CustomizationID><CustomizationID>no</CustomizationID></Invoice>"
        );
        assert_eq!(
            identify(&ubl),
            (
                root(UBL_INVOICE, "Invoice"),
                vec![" urn:a".to_owned(), "urn:bc".to_owned()]
            )
        );
        let credit_note = format!(
            "<c:CreditNote xmlns:c='{UBL_CREDIT_NOTE}'><b:CustomizationID xmlns:b='{UBL_CBC}'>urn:x</b:CustomizationID></c:CreditNote>"
        );
        assert_eq!(
            identify(&credit_note),
            (
                root(UBL_CREDIT_NOTE, "CreditNote"),
                vec!["urn:x".to_owned()]
            )
        );
        let cii = format!(
            "<rsm:CrossIndustryInvoice xmlns:rsm='{CII}' xmlns:ram='{CII_RAM}'>\
             <rsm:ExchangedDocumentContext><ram:ID>no</ram:ID>\
             <ram:GuidelineSpecifiedDocumentContextParameter><ram:ID>urn:y</ram:ID><ram:Other>no</ram:Other></ram:GuidelineSpecifiedDocumentContextParameter>\
             <ram:GuidelineSpecifiedDocumentContextParameter><ram:ID>urn:z</ram:ID></ram:GuidelineSpecifiedDocumentContextParameter>\
             </rsm:ExchangedDocumentContext><ram:ID>no</ram:ID></rsm:CrossIndustryInvoice>"
        );
        assert_eq!(
            identify(&cii),
            (
                root(CII, "CrossIndustryInvoice"),
                vec!["urn:y".to_owned(), "urn:z".to_owned()]
            )
        );
        // Another root element: no identifier is read.
        assert_eq!(
            identify(&format!(
                "<Order xmlns='{UBL_INVOICE}'><CustomizationID xmlns='{UBL_CBC}'>x</CustomizationID></Order>"
            )),
            (root(UBL_INVOICE, "Order"), vec![])
        );
        assert_eq!(identify("<a/>"), (root("", "a"), vec![]));
    }

    /// The caps of WebAssembly, checked on every target.
    #[test]
    fn options_within_the_caps_of_webassembly() {
        let caps = Some((WASM_MAX_DEPTH, WASM_MAX_STACK));
        let within = ValidateOptions::new()
            .max_depth(WASM_MAX_DEPTH)
            .max_stack(WASM_MAX_STACK);
        assert!(within.check(caps).is_ok());
        assert!(
            ValidateOptions::new()
                .max_stack(WASM_MAX_STACK)
                .check(caps)
                .is_ok()
        );
        let error = within
            .max_depth(WASM_MAX_DEPTH + 1)
            .check(caps)
            .unwrap_err();
        assert!(matches!(
            error,
            Error::OptionOutOfRange {
                option: "max_depth",
                value: 401,
                max: 400
            }
        ));
        assert_eq!(error.kind(), crate::ErrorKind::Unsupported);
        assert_eq!(
            error.to_string(),
            "option max_depth is 401, more than 400, the most this target allows"
        );
        let error = within
            .max_stack(WASM_MAX_STACK + 1)
            .check(caps)
            .unwrap_err();
        assert!(matches!(
            error,
            Error::OptionOutOfRange {
                option: "max_stack",
                value: 262_145,
                max: 262_144
            }
        ));
        // Without caps (the other targets), anything goes.
        let anything = ValidateOptions::new()
            .max_depth(u32::MAX)
            .max_stack(u32::MAX);
        assert!(anything.check(None).is_ok());
        assert_eq!(CAPS, None);
    }

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
            b"<?xml version='1.0' encoding='Shift_JIS'?><a/>",
            b"\xEF\xBB\xBF\xEF\xBB\xBF<a/>",
            b"\xEF\xBB\xBF",
            b"",
        ] {
            let borrowed = decode(bytes)
                .map(Cow::into_owned)
                .map_err(|e| e.to_string());
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
        assert!(matches!(decode(b"<a/>").unwrap(), Cow::Borrowed("<a/>")));
        assert_eq!(decode(b"\xFF\xFE<\0a\0/\0>\0").unwrap(), "<a/>");
        assert_eq!(
            decode(b"<?xml version='1.0' encoding='ISO-8859-1'?><a>\xE9</a>").unwrap(),
            "<?xml version='1.0' encoding='ISO-8859-1'?><a>\u{e9}</a>"
        );
        match decode(b"<?xml version='1.0' encoding='Shift_JIS'?><a/>") {
            Err(Error::Unsupported { message }) => assert_eq!(
                message,
                "the Shift_JIS encoding: the encodings read are UTF-8, UTF-16, US-ASCII, ISO-8859-1, ISO-8859-15 and windows-1252"
            ),
            other => panic!("{other:?}"),
        }
        match decode(b"<?xml version='1.0' encoding='UTF-16'?><a/>") {
            Err(Error::NotXml {
                line: 1,
                column: 1,
                message,
            }) => assert_eq!(
                message,
                "UTF-8 (no byte order mark), but the XML declaration names the encoding 'UTF-16'"
            ),
            other => panic!("{other:?}"),
        }
        match decode(b"<a>\n  \xC3\xA9\xFF</a>") {
            Err(Error::NotXml { line, column, .. }) => assert_eq!((line, column), (2, 4)),
            other => panic!("{other:?}"),
        }
        match decode(b"<?xml version=\"1.0\" encoding=\"US-ASCII\"?>\n<a>\xC3\xA9</a>") {
            Err(Error::NotXml {
                line,
                column,
                message,
            }) => {
                assert_eq!((line, column), (2, 4));
                assert_eq!(message, "bytes that are not US-ASCII at 2:4");
            }
            other => panic!("{other:?}"),
        }
        // Not an XML declaration: UTF-8.
        assert!(decode(b"<?xml-stylesheet encoding='x'?><a/>").is_ok());
    }

    #[test]
    fn well_formedness() {
        assert!(check("<a/>", 1, None).is_ok());
        match check("<a>\n<b></a>", DEFAULT_MAX_DEPTH, None) {
            Err(Error::NotXml {
                line,
                column,
                message,
            }) => assert_eq!(
                (line, column, message.as_str()),
                (2, 4, "expected 'b' tag, not 'a' at 2:4")
            ),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            check(
                "<!DOCTYPE a [<!ENTITY e 'x'>]><a>&e;</a>",
                DEFAULT_MAX_DEPTH,
                None
            ),
            Err(Error::Unsupported { .. })
        ));
        assert!(matches!(
            check("<p:a/>", DEFAULT_MAX_DEPTH, None),
            Err(Error::NotXml { .. })
        ));
    }

    #[test]
    fn one_pass_order_of_errors() {
        let schema = einvoice_xsd::Schema::ubl_2_1().unwrap();
        let invoice = "<Invoice xmlns='urn:oasis:names:specification:ubl:schema:xsd:Invoice-2' \
                       xmlns:xsi='http://www.w3.org/2001/XMLSchema-instance'>";
        let depth = |text: &str, max| match check(text, max, Some(&schema)) {
            Err(Error::Limit {
                limit: LimitExceeded::Depth { line, column, .. },
                ..
            }) => Some((line, column)),
            _ => None,
        };
        // Schema errors, in one pass with the well-formedness check.
        let errors = check(&format!("{invoice}</Invoice>"), 3, Some(&schema)).unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind.as_str(), "missing-element");
        // An element too deep: a limit, from the first one; the schema is not checked.
        let deep = format!("{invoice}\n<a><b><c/></b></a><d><e><f/></e></d></Invoice>");
        assert_eq!(depth(&deep, 2), Some((2, 4)));
        assert_eq!(depth(&deep, 3), Some((2, 7)));
        assert_eq!(depth(&deep, 4), None);
        // A well-formedness error comes first, even after an element too deep.
        let malformed = format!("{deep}<");
        assert!(matches!(
            check(&malformed, 2, Some(&schema)),
            Err(Error::NotXml { .. })
        ));
        assert!(matches!(
            check(&malformed, 2, None),
            Err(Error::NotXml { .. })
        ));
        // A schema error comes after the depth limit, and after any well-formedness
        // error: an xsi:type naming a built-in type not derived from the declared one.
        let retyped = invoice.replace(
            "<Invoice ",
            "<Invoice xsi:type='xs:IDREF' xmlns:xs='http://www.w3.org/2001/XMLSchema' ",
        );
        let errors = check(&format!("{retyped}</Invoice>"), 3, Some(&schema)).unwrap();
        assert!(
            errors
                .iter()
                .any(|e| e.kind == einvoice_xsd::ErrorKind::InvalidXsiType),
            "{errors:?}"
        );
        assert!(matches!(
            check(&format!("{retyped}<b><c/></b></Invoice>"), 2, Some(&schema)),
            Err(Error::Limit { .. })
        ));
        assert!(matches!(
            check(&format!("{retyped}</Invoice><"), 3, Some(&schema)),
            Err(Error::NotXml { .. })
        ));
        // A document type declaration is refused, also with a schema.
        assert!(matches!(
            check("<!DOCTYPE Invoice><Invoice/>", 3, Some(&schema)),
            Err(Error::Unsupported { .. })
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
