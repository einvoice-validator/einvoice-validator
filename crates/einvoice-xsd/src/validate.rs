//! Validation of an instance document against a loaded [`Schema`].
//!
//! The document is walked in document order with an explicit stack (no recursion,
//! so deeply nested input cannot overflow the call stack). Error recovery follows
//! the model of libxml2, which makes error lists comparable with xmllint:
//!
//! * a document element without a declaration, an element matched by a strict
//!   wildcard without a declaration, and an element whose type is abstract are
//!   reported and their content is not examined;
//! * within an element, the first child element that the content type does not
//!   allow (unexpected in the content model, or any element in simple or empty
//!   content) is reported and the remaining children of that element are not
//!   examined; text errors before it are reported once per text node;
//! * an `xsi:type` that cannot be used is reported and the declared type is used.
//!
//! The walk copies nothing out of the parsed document: element and attribute names
//! are only spelt out in errors, the text of an element with simple content is a
//! slice of the parsed text (copied only when comments or processing instructions
//! split it into several text nodes), values are checked on that slice (whitespace
//! processing builds a new string only when a check needs the processed value and
//! it differs from a slice of the text), and the `xs:ID` values seen are slices
//! too. Errors record the byte offset of their element; lines and columns are
//! computed at the end, in one pass over the text.

use std::borrow::Cow;
use std::collections::HashSet;

use roxmltree::Node;

use crate::automaton::{Automaton, State, Term};
use crate::datatypes::{self, Builtin, UNIMPLEMENTED_BUILTINS, WhiteSpace};
use crate::error::{DocumentError, ErrorKind, Report, ValidationError};
use crate::model::{
    ANY_TYPE, ComplexType, Content, ElementId, ProcessContents, Schema, TypeDef, TypeId, XS, XSI,
    clark, namespace_name,
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

enum FrameContent<'s, 'd> {
    Empty {
        stopped: bool,
    },
    Simple {
        type_id: TypeId,
        /// The text so far: a slice of the document while there is one text node.
        text: Cow<'d, str>,
        stopped: bool,
    },
    Children {
        automaton: &'s Automaton,
        mixed: bool,
        state: State,
        stopped: bool,
    },
}

struct Frame<'s, 'd, 'i> {
    node: Node<'d, 'i>,
    next: Option<Node<'d, 'i>>,
    content: FrameContent<'s, 'd>,
}

struct Validator<'s, 'd, 'i> {
    schema: &'s Schema,
    doc: &'d roxmltree::Document<'i>,
    /// Each error with the byte offset of its element; its line and column are
    /// filled in by [`positions`].
    errors: Vec<(usize, ValidationError)>,
    /// The `xs:ID` values seen, whitespace-processed.
    ids: HashSet<Cow<'d, str>>,
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

/// Values quoted in messages are cut to this many characters.
const MAX_QUOTED: usize = 80;

fn quote(value: &str) -> String {
    let mut out: String = value.chars().take(MAX_QUOTED).collect();
    if value.chars().nth(MAX_QUOTED).is_some() {
        out.push_str("...");
    }
    format!("{out:?}")
}

fn element_name(node: Node<'_, '_>) -> String {
    clark(
        namespace_name(node.tag_name().namespace()),
        node.tag_name().name(),
    )
}

pub(crate) fn validate(schema: &Schema, xml: &str) -> Result<Report, DocumentError> {
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
    let mut validator = Validator {
        schema,
        doc: &doc,
        errors: Vec::new(),
        ids: HashSet::new(),
    };
    validator.run()?;
    let errors = positions(xml, validator.errors);
    Ok(Report { errors })
}

impl<'s, 'd, 'i> Validator<'s, 'd, 'i> {
    fn report(&mut self, node: Node<'_, '_>, kind: ErrorKind, message: String) {
        self.report_attribute(node, kind, None, message);
    }

    fn report_attribute(
        &mut self,
        node: Node<'_, '_>,
        kind: ErrorKind,
        attribute: Option<String>,
        message: String,
    ) {
        self.errors.push((
            node.range().start,
            ValidationError {
                kind,
                line: 0,
                column: 0,
                element: element_name(node),
                attribute,
                message,
            },
        ));
    }

    fn unsupported(&self, node: Node<'_, '_>, message: String) -> DocumentError {
        let (line, column) = Cursor::new(self.doc.input_text()).advance(node.range().start);
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

    fn run(&mut self) -> Result<(), DocumentError> {
        let root = self.doc.root_element();
        let mut stack: Vec<Frame<'s, 'd, 'i>> = Vec::new();
        if let Some(frame) = self.start(root, Assignment::Root)? {
            stack.push(frame);
        }
        while let Some(top) = stack.last_mut() {
            let Some(child) = top.next else {
                if let Some(frame) = stack.pop() {
                    self.end(frame);
                }
                continue;
            };
            top.next = child.next_sibling();
            if child.is_text() {
                self.text(top, child);
            } else if child.is_element()
                && let Some(assignment) = self.child(top, child)
                && let Some(frame) = self.start(child, assignment)?
            {
                stack.push(frame);
            }
        }
        Ok(())
    }

    /// Starts an element: finds its type, checks `xsi:nil`, `xsi:type` and the
    /// attributes. `None` when its content is not to be examined.
    fn start(
        &mut self,
        node: Node<'d, 'i>,
        assignment: Assignment,
    ) -> Result<Option<Frame<'s, 'd, 'i>>, DocumentError> {
        let schema = self.schema;
        let namespace = namespace_name(node.tag_name().namespace());
        let local = node.tag_name().name();
        let declaration = match assignment {
            Assignment::Root => {
                let Some(id) = schema.global_element(namespace, local) else {
                    self.report(
                        node,
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
            && let Some(nil) = node.attribute((XSI, "nil"))
        {
            let value = datatypes::normalize(nil, WhiteSpace::Collapse);
            let attribute = Some(clark(Some(XSI), "nil"));
            if Builtin::Boolean.is_valid(&value) {
                self.report_attribute(
                    node,
                    ErrorKind::NotNillable,
                    attribute,
                    "xsi:nil on an element whose declaration is not nillable".to_owned(),
                );
            } else {
                self.report_attribute(
                    node,
                    ErrorKind::InvalidXsiNil,
                    attribute,
                    format!("xsi:nil {} is not a boolean", quote(nil)),
                );
            }
        }
        let mut type_id = declared_type;
        if let Some(xsi_type) = node.attribute((XSI, "type"))
            && let Some(local_type) = self.xsi_type(node, xsi_type, declared_type)?
        {
            type_id = local_type;
        }
        if let TypeDef::Complex(ComplexType {
            is_abstract: true, ..
        }) = &schema.types[type_id]
        {
            let name = self.type_name(type_id);
            self.report(
                node,
                ErrorKind::AbstractType,
                format!("the type {name} is abstract"),
            );
            return Ok(None);
        }
        self.attributes(node, type_id);
        let content = match &schema.types[type_id] {
            TypeDef::Simple(_) => FrameContent::Simple {
                type_id,
                text: Cow::Borrowed(""),
                stopped: false,
            },
            TypeDef::Complex(complex) => match &complex.content {
                Content::Empty => FrameContent::Empty { stopped: false },
                Content::Simple(simple) => FrameContent::Simple {
                    type_id: *simple,
                    text: Cow::Borrowed(""),
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
            node,
            next: node.first_child(),
            content,
        }))
    }

    /// Resolves `xsi:type`: the type to use, or `None` (error reported) to keep the
    /// declared type.
    fn xsi_type(
        &mut self,
        node: Node<'_, '_>,
        value: &str,
        declared: TypeId,
    ) -> Result<Option<TypeId>, DocumentError> {
        let schema = self.schema;
        let collapsed = datatypes::normalize(value, WhiteSpace::Collapse);
        let Some((prefix, local)) = datatypes::split_qname(&collapsed) else {
            self.report_attribute(
                node,
                ErrorKind::InvalidXsiType,
                Some(clark(Some(XSI), "type")),
                format!("xsi:type {} is not a QName", quote(value)),
            );
            return Ok(None);
        };
        let namespace = if let Some(p) = prefix {
            let Some(ns) = node.lookup_namespace_uri(Some(p)) else {
                self.report_attribute(
                    node,
                    ErrorKind::InvalidXsiType,
                    Some(clark(Some(XSI), "type")),
                    format!("xsi:type {}: prefix {p:?} is not declared", quote(value)),
                );
                return Ok(None);
            };
            Some(ns)
        } else {
            namespace_name(node.lookup_namespace_uri(None))
        };
        let Some(type_id) = schema.global_type(namespace, local) else {
            if namespace == Some(XS) && UNIMPLEMENTED_BUILTINS.contains(&local) {
                let message = format!(
                    "xsi:type names the built-in type xs:{local}, which this crate does not implement"
                );
                return Err(self.unsupported(node, message));
            }
            self.report_attribute(
                node,
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
                node,
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
    /// `enumeration` facet or the `xs:ID` registry needs it.
    #[expect(
        clippy::ptr_arg,
        reason = "a value borrowed from the document (Cow::Borrowed) keeps its lifetime, so that the xs:ID registry holds a slice of the document rather than a copy"
    )]
    fn check_value(&mut self, type_id: TypeId, raw: &Cow<'d, str>) -> Option<ValueProblem> {
        let schema = self.schema;
        let TypeDef::Simple(simple) = &schema.types[type_id] else {
            return Some(ValueProblem::Invalid(format!(
                "{} is not a simple type",
                self.type_name(type_id)
            )));
        };
        if !simple.builtin.is_valid_raw(raw) {
            return Some(ValueProblem::Invalid(format!(
                "{} is not a valid value of {}",
                quote(raw),
                self.type_name(type_id)
            )));
        }
        let is_id = simple.builtin == Builtin::Id;
        if !is_id && !schema.has_enumeration(type_id) {
            return None;
        }
        let white_space = simple.builtin.white_space();
        let value: Cow<'d, str> = match raw {
            Cow::Borrowed(raw) => datatypes::normalize(raw, white_space),
            Cow::Owned(raw) => Cow::Owned(datatypes::normalize(raw, white_space).into_owned()),
        };
        let mut current = type_id;
        while let TypeDef::Simple(step) = &schema.types[current] {
            if let Some(values) = &step.enumeration
                && !values.iter().any(|v| *v == value)
            {
                return Some(ValueProblem::Invalid(format!(
                    "{} is not one of the values allowed by {}: {}",
                    quote(raw),
                    self.type_name(type_id),
                    values
                        .iter()
                        .map(|v| quote(v))
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            current = step.base;
        }
        if is_id && !self.ids.insert(value) {
            return Some(ValueProblem::DuplicateId(format!(
                "the ID {} is already used in the document",
                quote(raw)
            )));
        }
        None
    }

    fn attributes(&mut self, node: Node<'d, 'i>, type_id: TypeId) {
        let schema = self.schema;
        let (uses, wildcard) = match &schema.types[type_id] {
            TypeDef::Simple(_) => (&[][..], None),
            TypeDef::Complex(complex) => {
                (&complex.attributes[..], complex.attribute_wildcard.as_ref())
            }
        };
        for attribute in node.attributes() {
            let namespace = namespace_name(attribute.namespace());
            let local = attribute.name();
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
                match self.check_value(attribute_use.type_id, &Cow::Borrowed(attribute.value())) {
                    None => {}
                    Some(ValueProblem::Invalid(problem)) => {
                        self.report_attribute(
                            node,
                            ErrorKind::InvalidAttributeValue,
                            name(),
                            problem,
                        );
                    }
                    Some(ValueProblem::DuplicateId(problem)) => {
                        self.report_attribute(node, ErrorKind::DuplicateId, name(), problem);
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
                            node,
                            ErrorKind::UndeclaredAttribute,
                            name(),
                            "no global attribute declaration, required by a strict wildcard"
                                .to_owned(),
                        );
                    }
                }
                _ => self.report_attribute(
                    node,
                    ErrorKind::AttributeNotAllowed,
                    name(),
                    "the attribute is not allowed".to_owned(),
                ),
            }
        }
        for attribute_use in uses.iter().filter(|u| u.required) {
            let present = node.attributes().any(|a| {
                attribute_use
                    .name
                    .matches(namespace_name(a.namespace()), a.name())
            });
            if !present {
                let name = attribute_use.name.to_string();
                self.report_attribute(
                    node,
                    ErrorKind::MissingAttribute,
                    Some(name),
                    "the required attribute is missing".to_owned(),
                );
            }
        }
    }

    fn text(&mut self, frame: &mut Frame<'s, 'd, 'i>, node: Node<'d, 'i>) {
        let text = node.text().unwrap_or("");
        let parent = frame.node;
        match &mut frame.content {
            FrameContent::Empty { stopped: false } => {
                self.report(
                    parent,
                    ErrorKind::ContentNotEmpty,
                    format!("text {} in empty content", quote(text)),
                );
            }
            FrameContent::Simple {
                text: value,
                stopped: false,
                ..
            } => {
                if value.is_empty() {
                    *value = Cow::Borrowed(text);
                } else {
                    value.to_mut().push_str(text);
                }
            }
            FrameContent::Children {
                mixed: false,
                stopped: false,
                ..
            } if !text.chars().all(datatypes::is_xml_space) => {
                self.report(
                    parent,
                    ErrorKind::TextNotAllowed,
                    format!("text {} in element-only content", quote(text.trim())),
                );
            }
            _ => {}
        }
    }

    /// Matches a child element against its parent's content; `None` when the child
    /// is not to be validated (error reported, or skipped by a wildcard).
    fn child(&mut self, frame: &mut Frame<'s, 'd, 'i>, child: Node<'_, '_>) -> Option<Assignment> {
        let schema = self.schema;
        let parent = frame.node;
        match &mut frame.content {
            FrameContent::Empty { stopped } => {
                if !*stopped {
                    *stopped = true;
                    self.report(
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
                    self.report(
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
                let namespace = namespace_name(child.tag_name().namespace());
                let local = child.tag_name().name();
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
                    self.report(child, ErrorKind::UnexpectedElement, message);
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
                                    child,
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

    fn end(&mut self, frame: Frame<'s, 'd, 'i>) {
        match frame.content {
            FrameContent::Simple {
                type_id,
                text,
                stopped: false,
            } => match self.check_value(type_id, &text) {
                None => {}
                Some(ValueProblem::Invalid(problem)) => {
                    self.report(frame.node, ErrorKind::InvalidValue, problem);
                }
                Some(ValueProblem::DuplicateId(problem)) => {
                    self.report(frame.node, ErrorKind::DuplicateId, problem);
                }
            },
            FrameContent::Children {
                automaton,
                state,
                stopped: false,
                ..
            } if !automaton.accepts(state) => {
                let expected = automaton.expected(state, &self.schema.elements);
                self.report(
                    frame.node,
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
