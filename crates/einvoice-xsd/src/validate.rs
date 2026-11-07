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

enum FrameContent<'s> {
    Empty {
        stopped: bool,
    },
    Simple {
        type_id: TypeId,
        text: String,
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
    content: FrameContent<'s>,
}

struct Validator<'s, 'd, 'i> {
    schema: &'s Schema,
    doc: &'d roxmltree::Document<'i>,
    errors: Vec<ValidationError>,
    ids: HashSet<String>,
    line_starts: Option<Vec<usize>>,
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
        line_starts: None,
    };
    validator.run()?;
    let mut errors = validator.errors;
    errors.sort_by_key(|e| (e.line, e.column));
    Ok(Report { errors })
}

impl<'s, 'd, 'i> Validator<'s, 'd, 'i> {
    fn position(&mut self, node: Node<'_, '_>) -> (u32, u32) {
        let text = self.doc.input_text();
        let starts = self.line_starts.get_or_insert_with(|| {
            std::iter::once(0)
                .chain(text.match_indices('\n').map(|(i, _)| i + 1))
                .collect()
        });
        let offset = node.range().start;
        let line = starts.partition_point(|&s| s <= offset);
        let start = starts[line.saturating_sub(1)];
        let column = text.get(start..offset).map_or(0, |s| s.chars().count()) + 1;
        (
            u32::try_from(line).unwrap_or(u32::MAX),
            u32::try_from(column).unwrap_or(u32::MAX),
        )
    }

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
        let (line, column) = self.position(node);
        self.errors.push(ValidationError {
            kind,
            line,
            column,
            element: element_name(node),
            attribute,
            message,
        });
    }

    fn unsupported(&mut self, node: Node<'_, '_>, message: String) -> DocumentError {
        let (line, column) = self.position(node);
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
                text: String::new(),
                stopped: false,
            },
            TypeDef::Complex(complex) => match &complex.content {
                Content::Empty => FrameContent::Empty { stopped: false },
                Content::Simple(simple) => FrameContent::Simple {
                    type_id: *simple,
                    text: String::new(),
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

    /// Checks a value against a simple type; when valid, registers `xs:ID` values.
    fn check_value(&mut self, type_id: TypeId, raw: &str) -> Option<ValueProblem> {
        let schema = self.schema;
        let TypeDef::Simple(simple) = &schema.types[type_id] else {
            return Some(ValueProblem::Invalid(format!(
                "{} is not a simple type",
                self.type_name(type_id)
            )));
        };
        let value = datatypes::normalize(raw, simple.builtin.white_space());
        if !simple.builtin.is_valid(&value) {
            return Some(ValueProblem::Invalid(format!(
                "{} is not a valid value of {}",
                quote(raw),
                self.type_name(type_id)
            )));
        }
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
        if simple.builtin == Builtin::Id && !self.ids.insert(value.into_owned()) {
            return Some(ValueProblem::DuplicateId(format!(
                "the ID {} is already used in the document",
                quote(raw)
            )));
        }
        None
    }

    fn attributes(&mut self, node: Node<'_, '_>, type_id: TypeId) {
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
            let name = clark(namespace, local);
            if let Some(attribute_use) = uses.iter().find(|u| u.name.matches(namespace, local)) {
                match self.check_value(attribute_use.type_id, attribute.value()) {
                    None => {}
                    Some(ValueProblem::Invalid(problem)) => {
                        self.report_attribute(
                            node,
                            ErrorKind::InvalidAttributeValue,
                            Some(name),
                            problem,
                        );
                    }
                    Some(ValueProblem::DuplicateId(problem)) => {
                        self.report_attribute(node, ErrorKind::DuplicateId, Some(name), problem);
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
                            Some(name),
                            "no global attribute declaration, required by a strict wildcard"
                                .to_owned(),
                        );
                    }
                }
                _ => self.report_attribute(
                    node,
                    ErrorKind::AttributeNotAllowed,
                    Some(name),
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

    fn text(&mut self, frame: &mut Frame<'s, 'd, 'i>, node: Node<'_, '_>) {
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
            } => value.push_str(text),
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
        let child_name = element_name(child);
        match &mut frame.content {
            FrameContent::Empty { stopped } => {
                if !*stopped {
                    *stopped = true;
                    self.report(
                        parent,
                        ErrorKind::ContentNotEmpty,
                        format!("child element {child_name} in empty content"),
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
                        format!("child element {child_name} in simple content"),
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
