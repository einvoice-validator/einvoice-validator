//! Validation of an instance document against a loaded [`Schema`].
//!
//! The document is read as a stream of events ([`einvoice_xml::Reader`]): no tree is
//! built, so that the memory used does not grow with the document, but with the depth
//! of its elements. The validation keeps one frame per open element whose content is
//! examined, and skips the content of the others to the end of their element. Error
//! recovery follows the model of libxml2, which makes error lists comparable with
//! xmllint:
//!
//! * a document element without a declaration, an element matched by a strict
//!   wildcard without a declaration, and an element whose type is abstract are
//!   reported and their content is not examined;
//! * within an element, the first child element that the content type does not
//!   allow (unexpected in the content model, or any element in simple or empty
//!   content) is reported and the remaining children of that element are not
//!   examined; text errors before it are reported once per text node (the text
//!   between two pieces of markup other than character data and CDATA sections);
//! * an `xsi:type` that cannot be used is reported and the declared type is used.
//!
//! Nothing is copied out of the text but what an error spells out: the text of an
//! element with simple content is a slice of the text (copied only when references
//! were replaced in it or comments or processing instructions split it), values are
//! checked on that slice (whitespace processing builds a new string only when a check
//! needs the processed value and it differs from a slice of the text), and the
//! `xs:ID` values seen are slices too. Errors record the byte offset of their
//! element's start tag; lines and columns are computed at the end, in one pass over
//! the text.
//!
//! The reader refuses documents with a document type declaration, whose internal
//! subset may declare entities: [`validate`] reads those with `roxmltree` instead (the
//! entities expanded, attribute defaults not applied), and gives the validation the
//! same events from its tree.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use einvoice_xml::{Event, StartElement};

use crate::automaton::{Automaton, State, Term};
use crate::datatypes::{self, Builtin, UNIMPLEMENTED_BUILTINS, WhiteSpace};
use crate::error::{DocumentError, ErrorKind, Report, ValidationError};
use crate::model::{
    ANY_TYPE, ComplexType, ConstraintKind, Content, ElementId, ProcessContents, Schema, TypeDef,
    TypeId, ValueConstraint, XS, XSI, clark, facet_problem, namespace_name, quote,
};

/// How a child element is to be validated.
#[derive(Clone, Copy)]
enum Assignment {
    /// The document element: looked up among the global declarations.
    Root,
    /// Against this element declaration.
    Declared(ElementId),
    /// Matched by a lax wildcard without a global declaration: `xs:anyType`.
    Lax,
}

/// A value of the document: a slice of its text `'d`, or a value the parser built
/// (references replaced, whitespace normalised), which lives only as long as the
/// event it comes with.
#[derive(Clone, Copy)]
enum Value<'v, 'd> {
    Document(&'d str),
    Other(&'v str),
}

impl Value<'_, '_> {
    #[expect(
        clippy::match_same_arms,
        reason = "the two slices have different lifetimes: one pattern cannot bind both"
    )]
    fn as_str(&self) -> &str {
        match *self {
            Self::Document(s) => s,
            Self::Other(s) => s,
        }
    }
}

/// An attribute of an element (namespace declarations are not attributes).
struct Attribute<'v, 'd> {
    namespace: Option<&'v str>,
    local: &'v str,
    value: Value<'v, 'd>,
}

/// The start of an element, as the validation reads it: from the streaming reader,
/// or from a `roxmltree` node.
trait Start<'d> {
    /// The namespace name, `None` (or empty) for none.
    fn namespace(&self) -> Option<&str>;
    /// The local name.
    fn local(&self) -> &'d str;
    /// The byte offset of the start tag in the text.
    fn offset(&self) -> usize;
    /// The value of the attribute `local` in the `xsi` namespace.
    fn xsi(&self, local: &str) -> Option<&str>;
    /// The namespace name bound to `prefix` (`None` for the default namespace) in the
    /// scope of the element: as `roxmltree` resolves it, never for the `xml` and
    /// `xmlns` prefixes, and empty or `None` when the default namespace is undeclared.
    fn lookup_namespace(&self, prefix: Option<&str>) -> Option<&str>;
    /// The attributes, in document order.
    fn attributes(&self) -> impl Iterator<Item = Attribute<'_, 'd>>;
}

impl<'a> Start<'a> for StartElement<'_, 'a> {
    fn namespace(&self) -> Option<&str> {
        self.name().namespace
    }

    fn local(&self) -> &'a str {
        self.name().local
    }

    fn offset(&self) -> usize {
        self.range().start
    }

    fn xsi(&self, local: &str) -> Option<&str> {
        self.attribute(Some(XSI), local).map(AsRef::as_ref)
    }

    fn lookup_namespace(&self, prefix: Option<&str>) -> Option<&str> {
        match prefix {
            Some("xml" | "xmlns") => None,
            Some(prefix) => StartElement::lookup_namespace(self, prefix),
            None => StartElement::lookup_namespace(self, ""),
        }
    }

    fn attributes(&self) -> impl Iterator<Item = Attribute<'_, 'a>> {
        StartElement::attributes(self).map(|attribute| Attribute {
            namespace: attribute.name.namespace,
            local: attribute.name.local,
            value: match attribute.value {
                Cow::Borrowed(value) => Value::Document(value),
                Cow::Owned(value) => Value::Other(value),
            },
        })
    }
}

impl<'d> Start<'d> for roxmltree::Node<'d, '_> {
    fn namespace(&self) -> Option<&str> {
        self.tag_name().namespace()
    }

    fn local(&self) -> &'d str {
        self.tag_name().name()
    }

    fn offset(&self) -> usize {
        self.range().start
    }

    fn xsi(&self, local: &str) -> Option<&str> {
        self.attribute((XSI, local))
    }

    fn lookup_namespace(&self, prefix: Option<&str>) -> Option<&str> {
        self.lookup_namespace_uri(prefix)
    }

    fn attributes(&self) -> impl Iterator<Item = Attribute<'_, 'd>> {
        roxmltree::Node::attributes(self).map(|attribute| Attribute {
            namespace: attribute.namespace(),
            local: attribute.name(),
            value: Value::Document(attribute.value()),
        })
    }
}

enum FrameContent<'s, 'd> {
    Empty {
        stopped: bool,
    },
    Simple {
        type_id: TypeId,
        /// The text so far: a slice of the document while there is one text node.
        text: Cow<'d, str>,
        /// The value constraint of the element's declaration, if any.
        constraint: Option<&'s ValueConstraint>,
        stopped: bool,
    },
    Children {
        automaton: &'s Automaton,
        mixed: bool,
        state: State,
        stopped: bool,
    },
}

/// An open element whose content is examined.
struct Frame<'s, 'd> {
    /// The byte offset of its start tag.
    offset: usize,
    /// Its namespace name ([`Namespaces::id`]) and local name, for its errors.
    namespace: Option<usize>,
    local: &'d str,
    content: FrameContent<'s, 'd>,
}

/// Up to this many distinct namespace names, one is looked up by a linear scan;
/// beyond, through the index.
const SCAN_LIMIT: usize = 16;

/// The namespace names of the open elements, numbered, so that a frame can name its
/// element in errors after the reader has moved on (the reader's names only live
/// until its next event).
#[derive(Default)]
struct Namespaces {
    names: Vec<Box<str>>,
    index: HashMap<Box<str>, usize>,
}

impl Namespaces {
    fn id(&mut self, namespace: Option<&str>) -> Option<usize> {
        let namespace = namespace?;
        let found = if self.names.len() <= SCAN_LIMIT {
            self.names.iter().position(|n| **n == *namespace)
        } else {
            self.index.get(namespace).copied()
        };
        Some(found.unwrap_or_else(|| {
            let id = self.names.len();
            self.names.push(namespace.into());
            self.index.insert(namespace.into(), id);
            id
        }))
    }

    fn name(&self, id: Option<usize>) -> Option<&str> {
        id.and_then(|id| self.names.get(id)).map(AsRef::as_ref)
    }
}

struct Validator<'s, 'd> {
    schema: &'s Schema,
    /// The text of the document.
    text: &'d str,
    /// Each error with the byte offset of its element; its line and column are
    /// filled in by [`positions`].
    errors: Vec<(usize, ValidationError)>,
    /// The `xs:ID` values seen, whitespace-processed.
    ids: HashSet<Cow<'d, str>>,
    namespaces: Namespaces,
}

/// Lines and columns from byte offsets, walking forward through a text.
struct Cursor<'t> {
    text: &'t [u8],
    offset: usize,
    /// 1-based.
    line: usize,
    /// Characters since the start of the line.
    column: usize,
}

impl<'t> Cursor<'t> {
    fn new(text: &'t str) -> Self {
        Self {
            text: text.as_bytes(),
            offset: 0,
            line: 1,
            column: 0,
        }
    }

    /// The 1-based line and column of the character at byte `offset` (a character
    /// boundary at or after the previous offset): the lines are counted by line
    /// feeds, the columns in characters.
    fn advance(&mut self, offset: usize) -> (u32, u32) {
        let end = offset.min(self.text.len());
        for &b in self.text.get(self.offset..end).unwrap_or(&[]) {
            if b == b'\n' {
                self.line += 1;
                self.column = 0;
            } else if b & 0xC0 != 0x80 {
                // Not a UTF-8 continuation byte: a character starts here.
                self.column += 1;
            }
        }
        self.offset = self.offset.max(end);
        (
            u32::try_from(self.line).unwrap_or(u32::MAX),
            u32::try_from(self.column + 1).unwrap_or(u32::MAX),
        )
    }
}

/// The errors in document order (by the position of their element, in the order
/// they were found for the same element), with their lines and columns.
fn positions(text: &str, mut errors: Vec<(usize, ValidationError)>) -> Vec<ValidationError> {
    errors.sort_by_key(|(offset, _)| *offset);
    let mut cursor = Cursor::new(text);
    errors
        .into_iter()
        .map(|(offset, mut error)| {
            (error.line, error.column) = cursor.advance(offset);
            error
        })
        .collect()
}

/// Why a value is not valid.
enum ValueProblem {
    /// Not in the lexical space or not allowed by a facet.
    Invalid(String),
    /// A valid `xs:ID` that an earlier attribute or element of the document uses.
    DuplicateId(String),
}

fn element_name<'d>(element: &impl Start<'d>) -> String {
    clark(namespace_name(element.namespace()), element.local())
}

/// The error of a text the reader refuses.
fn malformed(e: &einvoice_xml::Error) -> DocumentError {
    DocumentError::Malformed {
        line: e.pos().row,
        column: e.pos().col,
        message: e.to_string(),
    }
}

/// Validates the text of a document: read with the streaming reader, or, when it has
/// a document type declaration, with `roxmltree`.
pub(crate) fn validate(schema: &Schema, xml: &str) -> Result<Report, DocumentError> {
    let mut reader = einvoice_xml::Reader::new(xml);
    let mut validation = Validation::new(schema, xml);
    loop {
        match reader.next_event() {
            Ok(Some(event)) => validation.event(event),
            Ok(None) => return validation.finish(),
            Err(einvoice_xml::Error::DtdDetected) => return validate_tree(schema, xml),
            Err(e) => return Err(malformed(&e)),
        }
    }
}

/// [`validate`] on a document with a document type declaration, read with `roxmltree`
/// (internal subset allowed for entities): the validation is given the events of its
/// tree, in document order.
fn validate_tree(schema: &Schema, xml: &str) -> Result<Report, DocumentError> {
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..roxmltree::ParsingOptions::default()
    };
    let doc = roxmltree::Document::parse_with_options(xml, options).map_err(|e| {
        DocumentError::Malformed {
            line: e.pos().row,
            column: e.pos().col,
            message: e.to_string(),
        }
    })?;
    let mut walk = Walk::new(schema, xml);
    // The elements open, innermost last: closed when the walk in document order
    // reaches a node that is not inside them.
    let mut open: Vec<roxmltree::NodeId> = Vec::new();
    for node in doc.root().descendants().skip(1) {
        let parent = node.parent().map(|p| p.id());
        while open.last().is_some_and(|&id| Some(id) != parent) {
            open.pop();
            walk.end();
        }
        match node.node_type() {
            roxmltree::NodeType::Element => {
                walk.start(&node);
                open.push(node.id());
            }
            roxmltree::NodeType::Text => walk.text(Cow::Borrowed(node.text().unwrap_or(""))),
            roxmltree::NodeType::Comment | roxmltree::NodeType::PI => walk.split_text(),
            roxmltree::NodeType::Root => {}
        }
    }
    for _ in open {
        walk.end();
    }
    walk.finish()
}

/// A validation in progress, given the events of a document one by one, in document
/// order, as [`einvoice_xml::Reader`] reads them: [`Schema::validation`].
///
/// This is what [`Schema::validate`] does, for a caller that reads the document
/// itself, so that one pass over the text checks it for well-formedness (and for
/// anything else the caller wants to check) and validates it. The events must be
/// those of a reader of the text given to [`Schema::validation`], every one of them,
/// up to the end of the document: the validation does not check well-formedness, and
/// the positions of its errors are byte offsets of that text. An event of a document
/// the reader then refuses, or a validation finished early, gives a report that
/// means nothing; it never panics.
pub struct Validation<'s, 'a> {
    walk: Walk<'s, 'a>,
}

impl std::fmt::Debug for Validation<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Validation")
            .field("errors", &self.walk.validator.errors.len())
            .field("open", &self.walk.frames.len())
            .finish_non_exhaustive()
    }
}

impl<'s, 'a> Validation<'s, 'a> {
    pub(crate) fn new(schema: &'s Schema, text: &'a str) -> Self {
        Self {
            walk: Walk::new(schema, text),
        }
    }

    /// Takes the next event of the document.
    pub fn event(&mut self, event: Event<'_, 'a>) {
        match event {
            Event::Start(start) => self.walk.start(&start),
            Event::End(_) => self.walk.end(),
            Event::Text(text) => self.walk.text(text.value),
            Event::Comment(_) | Event::ProcessingInstruction(_) => self.walk.split_text(),
        }
    }

    /// The report, once every event is given.
    ///
    /// # Errors
    ///
    /// [`DocumentError::Unsupported`] when the document needs a feature outside the
    /// implemented subset (`xsi:type` naming a built-in type the schemas do not use):
    /// the first such element in document order.
    pub fn finish(self) -> Result<Report, DocumentError> {
        self.walk.finish()
    }
}

/// The state of a validation between two events.
struct Walk<'s, 'd> {
    validator: Validator<'s, 'd>,
    /// The open elements whose content is examined, innermost last.
    frames: Vec<Frame<'s, 'd>>,
    /// The number of open elements inside one whose content is not examined, that
    /// one included; 0 when the innermost open element's content is examined.
    skipped: usize,
    /// The text node being read: character data and CDATA sections not separated by
    /// other markup make one.
    text: Option<Cow<'d, str>>,
    /// The first construct outside the supported subset: the rest is not examined.
    unsupported: Option<DocumentError>,
}

impl<'s, 'd> Walk<'s, 'd> {
    fn new(schema: &'s Schema, text: &'d str) -> Self {
        Self {
            validator: Validator {
                schema,
                text,
                errors: Vec::new(),
                ids: HashSet::new(),
                namespaces: Namespaces::default(),
            },
            frames: Vec::new(),
            skipped: 0,
            text: None,
            unsupported: None,
        }
    }

    fn start(&mut self, element: &impl Start<'d>) {
        self.split_text();
        if self.unsupported.is_some() {
            return;
        }
        if self.skipped > 0 {
            self.skipped += 1;
            return;
        }
        let assignment = match self.frames.last_mut() {
            None => Assignment::Root,
            Some(top) => {
                let Some(assignment) = self.validator.child(top, element) else {
                    self.skipped = 1;
                    return;
                };
                assignment
            }
        };
        match self.validator.start(element, assignment) {
            Ok(Some(frame)) => self.frames.push(frame),
            Ok(None) => self.skipped = 1,
            Err(e) => self.unsupported = Some(e),
        }
    }

    fn end(&mut self) {
        self.split_text();
        if self.unsupported.is_some() {
            return;
        }
        if self.skipped > 0 {
            self.skipped -= 1;
        } else if let Some(frame) = self.frames.pop() {
            self.validator.end(&frame);
        }
    }

    fn text(&mut self, value: Cow<'d, str>) {
        if self.unsupported.is_some() || self.skipped > 0 || self.frames.is_empty() {
            return;
        }
        match &mut self.text {
            None => self.text = Some(value),
            Some(text) => text.to_mut().push_str(&value),
        }
    }

    /// Ends the text node being read, if any (markup other than character data and
    /// CDATA sections, or the end of the element).
    fn split_text(&mut self) {
        if let Some(text) = self.text.take()
            && let Some(top) = self.frames.last_mut()
        {
            self.validator.text(top, text);
        }
    }

    fn finish(mut self) -> Result<Report, DocumentError> {
        self.split_text();
        if let Some(e) = self.unsupported {
            return Err(e);
        }
        Ok(Report {
            errors: positions(self.validator.text, self.validator.errors),
        })
    }
}

impl<'s, 'd> Validator<'s, 'd> {
    fn report(&mut self, offset: usize, element: String, kind: ErrorKind, message: String) {
        self.report_attribute(offset, element, kind, None, message);
    }

    fn report_attribute(
        &mut self,
        offset: usize,
        element: String,
        kind: ErrorKind,
        attribute: Option<String>,
        message: String,
    ) {
        self.errors.push((
            offset,
            ValidationError {
                kind,
                line: 0,
                column: 0,
                element,
                attribute,
                message,
            },
        ));
    }

    /// The name of the element of a frame (its namespace and local name), in Clark
    /// notation.
    fn frame_name(&self, namespace: Option<usize>, local: &str) -> String {
        clark(self.namespaces.name(namespace), local)
    }

    fn unsupported(&self, offset: usize, message: String) -> DocumentError {
        let (line, column) = Cursor::new(self.text).advance(offset);
        DocumentError::Unsupported {
            line,
            column,
            message,
        }
    }

    fn type_name(&self, type_id: TypeId) -> String {
        self.schema.types[type_id]
            .name()
            .map_or_else(|| "an anonymous type".to_owned(), ToString::to_string)
    }

    /// Starts an element: finds its type, checks `xsi:nil`, `xsi:type` and the
    /// attributes. `None` when its content is not to be examined.
    fn start(
        &mut self,
        element: &impl Start<'d>,
        assignment: Assignment,
    ) -> Result<Option<Frame<'s, 'd>>, DocumentError> {
        let schema = self.schema;
        let namespace = namespace_name(element.namespace());
        let local = element.local();
        let offset = element.offset();
        let declaration = match assignment {
            Assignment::Root => {
                let Some(id) = schema.global_element(namespace, local) else {
                    self.report(
                        offset,
                        element_name(element),
                        ErrorKind::UnknownRoot,
                        "no global element declaration for the document element".to_owned(),
                    );
                    return Ok(None);
                };
                Some(id)
            }
            Assignment::Declared(id) => Some(id),
            Assignment::Lax => None,
        };
        let declared_type = declaration.map_or(ANY_TYPE, |id| schema.elements[id].type_id);
        if declaration.is_some()
            && let Some(nil) = element.xsi("nil")
        {
            let value = datatypes::normalize(nil, WhiteSpace::Collapse);
            let attribute = Some(clark(Some(XSI), "nil"));
            if Builtin::Boolean.is_valid(&value) {
                self.report_attribute(
                    offset,
                    element_name(element),
                    ErrorKind::NotNillable,
                    attribute,
                    "xsi:nil on an element whose declaration is not nillable".to_owned(),
                );
            } else {
                self.report_attribute(
                    offset,
                    element_name(element),
                    ErrorKind::InvalidXsiNil,
                    attribute,
                    format!("xsi:nil {} is not a boolean", quote(nil)),
                );
            }
        }
        let mut type_id = declared_type;
        if let Some(xsi_type) = element.xsi("type")
            && let Some(local_type) = self.xsi_type(element, xsi_type, declared_type)?
        {
            type_id = local_type;
        }
        if let TypeDef::Complex(ComplexType {
            is_abstract: true, ..
        }) = &schema.types[type_id]
        {
            let name = self.type_name(type_id);
            self.report(
                offset,
                element_name(element),
                ErrorKind::AbstractType,
                format!("the type {name} is abstract"),
            );
            return Ok(None);
        }
        self.attributes(element, type_id);
        let constraint = declaration.and_then(|id| schema.element_constraint(id));
        let content = match &schema.types[type_id] {
            TypeDef::Simple(_) => FrameContent::Simple {
                type_id,
                text: Cow::Borrowed(""),
                constraint,
                stopped: false,
            },
            TypeDef::Complex(complex) => match &complex.content {
                Content::Empty => FrameContent::Empty { stopped: false },
                Content::Simple(simple) => FrameContent::Simple {
                    type_id: *simple,
                    text: Cow::Borrowed(""),
                    constraint,
                    stopped: false,
                },
                Content::ElementOnly(automaton) => FrameContent::Children {
                    automaton,
                    mixed: false,
                    state: None,
                    stopped: false,
                },
                Content::Mixed(automaton) => FrameContent::Children {
                    automaton,
                    mixed: true,
                    state: None,
                    stopped: false,
                },
            },
        };
        Ok(Some(Frame {
            offset,
            namespace: self.namespaces.id(namespace),
            local,
            content,
        }))
    }

    /// Resolves `xsi:type`: the type to use, or `None` (error reported) to keep the
    /// declared type.
    fn xsi_type(
        &mut self,
        element: &impl Start<'d>,
        value: &str,
        declared: TypeId,
    ) -> Result<Option<TypeId>, DocumentError> {
        let schema = self.schema;
        let offset = element.offset();
        let collapsed = datatypes::normalize(value, WhiteSpace::Collapse);
        let Some((prefix, local)) = datatypes::split_qname(&collapsed) else {
            self.report_attribute(
                offset,
                element_name(element),
                ErrorKind::InvalidXsiType,
                Some(clark(Some(XSI), "type")),
                format!("xsi:type {} is not a QName", quote(value)),
            );
            return Ok(None);
        };
        let namespace = if let Some(p) = prefix {
            let Some(ns) = element.lookup_namespace(Some(p)) else {
                self.report_attribute(
                    offset,
                    element_name(element),
                    ErrorKind::InvalidXsiType,
                    Some(clark(Some(XSI), "type")),
                    format!("xsi:type {}: prefix {p:?} is not declared", quote(value)),
                );
                return Ok(None);
            };
            Some(ns)
        } else {
            namespace_name(element.lookup_namespace(None))
        };
        let Some(type_id) = schema.global_type(namespace, local) else {
            if namespace == Some(XS) && UNIMPLEMENTED_BUILTINS.contains(&local) {
                let message = format!(
                    "xsi:type names the built-in type xs:{local}, which this crate does not implement"
                );
                return Err(self.unsupported(offset, message));
            }
            self.report_attribute(
                offset,
                element_name(element),
                ErrorKind::InvalidXsiType,
                Some(clark(Some(XSI), "type")),
                format!(
                    "xsi:type {} does not resolve to a type definition",
                    quote(value)
                ),
            );
            return Ok(None);
        };
        if !schema.is_derived_from(type_id, declared) {
            let (name, declared_name) = (self.type_name(type_id), self.type_name(declared));
            self.report_attribute(
                offset,
                element_name(element),
                ErrorKind::InvalidXsiType,
                Some(clark(Some(XSI), "type")),
                format!("xsi:type {name} is not derived from the declared type {declared_name}"),
            );
            return Ok(None);
        }
        Ok(Some(type_id))
    }

    /// Checks a value, before whitespace processing, against a simple type; when
    /// valid, registers `xs:ID` values. The processed value is only built when an
    /// `enumeration` facet or the `xs:ID` registry needs it; a value that is a slice
    /// of the text is registered as a slice of it.
    fn check_value(&mut self, type_id: TypeId, raw: Value<'_, 'd>) -> Option<ValueProblem> {
        let schema = self.schema;
        let TypeDef::Simple(simple) = &schema.types[type_id] else {
            return Some(ValueProblem::Invalid(format!(
                "{} is not a simple type",
                self.type_name(type_id)
            )));
        };
        if !simple.builtin.is_valid_raw(raw.as_str()) {
            return Some(ValueProblem::Invalid(format!(
                "{} is not a valid value of {}",
                quote(raw.as_str()),
                self.type_name(type_id)
            )));
        }
        let is_id = simple.builtin == Builtin::Id;
        if !is_id && !schema.has_facets(type_id) {
            return None;
        }
        let white_space = simple.builtin.white_space();
        let value: Cow<'d, str> = match raw {
            Value::Document(raw) => datatypes::normalize(raw, white_space),
            Value::Other(raw) => Cow::Owned(datatypes::normalize(raw, white_space).into_owned()),
        };
        if let Some(problem) = facet_problem(&schema.types, type_id, &value) {
            return Some(ValueProblem::Invalid(format!(
                "{} is {}",
                quote(raw.as_str()),
                problem.describe(&self.type_name(type_id))
            )));
        }
        if is_id && !self.ids.insert(value) {
            return Some(ValueProblem::DuplicateId(format!(
                "the ID {} is already used in the document",
                quote(raw.as_str())
            )));
        }
        None
    }

    /// Whether two valid values of a simple type, before whitespace processing,
    /// are the same value (Part 2, 2.2.1): the test of a fixed value constraint.
    fn same_value(&self, type_id: TypeId, a: &str, b: &str) -> bool {
        let TypeDef::Simple(simple) = &self.schema.types[type_id] else {
            return false;
        };
        let builtin = simple.builtin;
        let white_space = builtin.white_space();
        let a = datatypes::normalize(a, white_space);
        let b = datatypes::normalize(b, white_space);
        // The loader refuses fixed values on the types without a value key.
        match (builtin.value_key(&a), builtin.value_key(&b)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }

    /// Why a value constraint of an element's declaration is not a valid value of
    /// the simple type that the element's actual type gives its content, if it is
    /// not (it is valid for the declared type: the loader checks it).
    fn constraint_problem(&self, type_id: TypeId, value: &str) -> Option<String> {
        let TypeDef::Simple(simple) = &self.schema.types[type_id] else {
            return Some(format!("{} is not a simple type", self.type_name(type_id)));
        };
        if !simple.builtin.is_valid_raw(value) {
            return Some(format!("not a valid value of {}", self.type_name(type_id)));
        }
        let processed = datatypes::normalize(value, simple.builtin.white_space());
        facet_problem(&self.schema.types, type_id, &processed)
            .map(|problem| problem.describe(&self.type_name(type_id)))
    }

    fn attributes(&mut self, element: &impl Start<'d>, type_id: TypeId) {
        let schema = self.schema;
        let offset = element.offset();
        let (uses, wildcard) = match &schema.types[type_id] {
            TypeDef::Simple(_) => (&[][..], None),
            TypeDef::Complex(complex) => {
                (&complex.attributes[..], complex.attribute_wildcard.as_ref())
            }
        };
        for attribute in element.attributes() {
            let namespace = namespace_name(attribute.namespace);
            let local = attribute.local;
            if namespace == Some(XSI)
                && matches!(
                    local,
                    "type" | "nil" | "schemaLocation" | "noNamespaceSchemaLocation"
                )
            {
                continue;
            }
            // Spelt out only for an error.
            let name = || Some(clark(namespace, local));
            if let Some(attribute_use) = uses.iter().find(|u| u.name.matches(namespace, local)) {
                match self.check_value(attribute_use.type_id, attribute.value) {
                    None => {
                        let value = attribute.value.as_str();
                        if let Some(fixed) = attribute_use.fixed.map(|f| schema.fixed_value(f))
                            && !self.same_value(attribute_use.type_id, value, fixed)
                        {
                            self.report_attribute(
                                offset,
                                element_name(element),
                                ErrorKind::InvalidAttributeValue,
                                name(),
                                format!(
                                    "{} is not the fixed value {} of the attribute",
                                    quote(value),
                                    quote(fixed)
                                ),
                            );
                        }
                    }
                    Some(ValueProblem::Invalid(problem)) => {
                        self.report_attribute(
                            offset,
                            element_name(element),
                            ErrorKind::InvalidAttributeValue,
                            name(),
                            problem,
                        );
                    }
                    Some(ValueProblem::DuplicateId(problem)) => {
                        self.report_attribute(
                            offset,
                            element_name(element),
                            ErrorKind::DuplicateId,
                            name(),
                            problem,
                        );
                    }
                }
                continue;
            }
            match wildcard {
                Some(w) if w.allows(namespace) => {
                    // No global attribute declarations exist in the supported
                    // subset: lax and skip accept, strict cannot be satisfied.
                    if w.process == ProcessContents::Strict {
                        self.report_attribute(
                            offset,
                            element_name(element),
                            ErrorKind::UndeclaredAttribute,
                            name(),
                            "no global attribute declaration, required by a strict wildcard"
                                .to_owned(),
                        );
                    }
                }
                _ => self.report_attribute(
                    offset,
                    element_name(element),
                    ErrorKind::AttributeNotAllowed,
                    name(),
                    "the attribute is not allowed".to_owned(),
                ),
            }
        }
        for attribute_use in uses.iter().filter(|u| u.required) {
            let present = element.attributes().any(|a| {
                attribute_use
                    .name
                    .matches(namespace_name(a.namespace), a.local)
            });
            if !present {
                let name = attribute_use.name.to_string();
                self.report_attribute(
                    offset,
                    element_name(element),
                    ErrorKind::MissingAttribute,
                    Some(name),
                    "the required attribute is missing".to_owned(),
                );
            }
        }
    }

    /// A text node of the element of `frame`.
    fn text(&mut self, frame: &mut Frame<'s, 'd>, text: Cow<'d, str>) {
        let (offset, namespace, local) = (frame.offset, frame.namespace, frame.local);
        match &mut frame.content {
            FrameContent::Empty { stopped: false } => {
                let element = self.frame_name(namespace, local);
                self.report(
                    offset,
                    element,
                    ErrorKind::ContentNotEmpty,
                    format!("text {} in empty content", quote(&text)),
                );
            }
            FrameContent::Simple {
                text: value,
                stopped: false,
                ..
            } => {
                if value.is_empty() {
                    *value = text;
                } else {
                    value.to_mut().push_str(&text);
                }
            }
            FrameContent::Children {
                mixed: false,
                stopped: false,
                ..
            } if !text.chars().all(datatypes::is_xml_space) => {
                let element = self.frame_name(namespace, local);
                self.report(
                    offset,
                    element,
                    ErrorKind::TextNotAllowed,
                    format!("text {} in element-only content", quote(text.trim())),
                );
            }
            _ => {}
        }
    }

    /// Matches a child element against its parent's content; `None` when the child
    /// is not to be validated (error reported, or skipped by a wildcard).
    fn child(&mut self, frame: &mut Frame<'s, 'd>, child: &impl Start<'d>) -> Option<Assignment> {
        let schema = self.schema;
        let (offset, namespace, local) = (frame.offset, frame.namespace, frame.local);
        match &mut frame.content {
            FrameContent::Empty { stopped } => {
                if !*stopped {
                    *stopped = true;
                    let parent = self.frame_name(namespace, local);
                    self.report(
                        offset,
                        parent,
                        ErrorKind::ContentNotEmpty,
                        format!("child element {} in empty content", element_name(child)),
                    );
                }
                None
            }
            FrameContent::Simple { stopped, .. } => {
                if !*stopped {
                    *stopped = true;
                    let parent = self.frame_name(namespace, local);
                    self.report(
                        offset,
                        parent,
                        ErrorKind::ElementInSimpleContent,
                        format!("child element {} in simple content", element_name(child)),
                    );
                }
                None
            }
            FrameContent::Children {
                automaton,
                state,
                stopped,
                ..
            } => {
                if *stopped {
                    return None;
                }
                let (namespace, local) = (namespace_name(child.namespace()), child.local());
                let Some((position, term)) =
                    automaton.step(*state, namespace, local, &schema.elements)
                else {
                    *stopped = true;
                    let expected = automaton.expected(*state, &schema.elements);
                    let message = if expected.is_empty() {
                        "not expected: no more child elements are allowed".to_owned()
                    } else {
                        format!("not expected; expected: {}", expected.join(", "))
                    };
                    self.report(
                        child.offset(),
                        element_name(child),
                        ErrorKind::UnexpectedElement,
                        message,
                    );
                    return None;
                };
                *state = Some(position);
                match term {
                    Term::Element(id) => Some(Assignment::Declared(*id)),
                    Term::Wildcard(wildcard) => {
                        match (wildcard.process, schema.global_element(namespace, local)) {
                            (ProcessContents::Skip, _) => None,
                            (_, Some(id)) => Some(Assignment::Declared(id)),
                            (ProcessContents::Lax, None) => Some(Assignment::Lax),
                            (ProcessContents::Strict, None) => {
                                self.report(
                                    child.offset(),
                                    element_name(child),
                                    ErrorKind::UndeclaredElement,
                                    "no global element declaration, required by a strict wildcard"
                                        .to_owned(),
                                );
                                None
                            }
                        }
                    }
                }
            }
        }
    }

    fn end(&mut self, frame: &Frame<'s, 'd>) {
        let element = |v: &Self| v.frame_name(frame.namespace, frame.local);
        match &frame.content {
            FrameContent::Simple {
                type_id,
                text,
                constraint,
                stopped: false,
            } => {
                let type_id = *type_id;
                if let Some(constraint) = constraint
                    && text.is_empty()
                {
                    // cvc-elt.5.1: an element without any character or element
                    // children takes the default or fixed value, which must be
                    // valid for the actual type (an xsi:type may change it).
                    if let Some(problem) = self.constraint_problem(type_id, &constraint.value) {
                        let element = element(self);
                        self.report(
                            frame.offset,
                            element,
                            ErrorKind::InvalidValue,
                            format!(
                                "the {} value {} of the declaration is {problem}",
                                constraint.kind.as_str(),
                                quote(&constraint.value)
                            ),
                        );
                    }
                    return;
                }
                let value = match text {
                    Cow::Borrowed(text) => Value::Document(text),
                    Cow::Owned(text) => Value::Other(text.as_str()),
                };
                match self.check_value(type_id, value) {
                    None => {
                        // cvc-elt.5.2.2.2.2: a fixed value is compared in the value
                        // space.
                        if let Some(constraint) = constraint
                            && constraint.kind == ConstraintKind::Fixed
                            && !self.same_value(type_id, text, &constraint.value)
                        {
                            let element = element(self);
                            self.report(
                                frame.offset,
                                element,
                                ErrorKind::InvalidValue,
                                format!(
                                    "{} is not the fixed value {} of the declaration",
                                    quote(text),
                                    quote(&constraint.value)
                                ),
                            );
                        }
                    }
                    Some(ValueProblem::Invalid(problem)) => {
                        let element = element(self);
                        self.report(frame.offset, element, ErrorKind::InvalidValue, problem);
                    }
                    Some(ValueProblem::DuplicateId(problem)) => {
                        let element = element(self);
                        self.report(frame.offset, element, ErrorKind::DuplicateId, problem);
                    }
                }
            }
            FrameContent::Children {
                automaton,
                state,
                stopped: false,
                ..
            } if !automaton.accepts(*state) => {
                let expected = automaton.expected(*state, &self.schema.elements);
                let element = element(self);
                self.report(
                    frame.offset,
                    element,
                    ErrorKind::MissingElement,
                    format!("content incomplete; expected: {}", expected.join(", ")),
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Line and column as a table of line starts gives them: the line is the number
    /// of line starts at or before the offset, the column one more than the number
    /// of characters between the start of that line and the offset.
    fn reference_position(text: &str, offset: usize) -> (u32, u32) {
        let starts: Vec<usize> = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        let line = starts.partition_point(|&s| s <= offset);
        let start = starts[line - 1];
        let column = text[start..offset].chars().count() + 1;
        (u32::try_from(line).unwrap(), u32::try_from(column).unwrap())
    }

    proptest::proptest! {
        #[test]
        fn cursor_positions(
            text in "[a\n\ré€😀<]{0,40}",
            picks in proptest::collection::vec(proptest::prelude::any::<proptest::sample::Index>(), 0..8),
        ) {
            let boundaries: Vec<usize> = text
                .char_indices()
                .map(|(i, _)| i)
                .chain(std::iter::once(text.len()))
                .collect();
            let mut offsets: Vec<usize> = picks.iter().map(|i| boundaries[i.index(boundaries.len())]).collect();
            offsets.sort_unstable();
            let mut cursor = Cursor::new(&text);
            for offset in offsets {
                proptest::prop_assert_eq!(cursor.advance(offset), reference_position(&text, offset));
            }
        }
    }
}
