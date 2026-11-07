//! The node model: immutable XML trees stored in an arena, one per document.
//!
//! A [`Document`] holds its nodes in document order: the document node first, then each
//! element followed by its attributes and then by its children. The index of a node in the
//! arena is therefore its position in document order, which makes document order a
//! comparison of two integers and every axis a walk over neighbouring records. Nodes of
//! different documents are ordered by the documents' creation order.
//!
//! Trees are built with a [`TreeBuilder`] (the XSLT layer builds its temporary trees with
//! it) or parsed from XML text with [`Document::parse`], which keeps every text node,
//! whitespace-only ones included, as Saxon does with `-strip:none`, and the namespace
//! prefixes as written in the source (they appear in `fn:name`).
//!
//! The model has no namespace nodes and no type annotations: every element and attribute is
//! untyped, so its typed value is `xs:untypedAtomic`.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt::{self, Write as _};
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use einvoice_xdm::AtomicValue;
use einvoice_xpath::{QName, XML_NAMESPACE};

/// "No node" in the parent, previous sibling and name fields.
const NONE: u32 = u32::MAX;

/// Creation order of documents, for the document order of nodes in different trees.
static NEXT_SERIAL: AtomicU64 = AtomicU64::new(0);

/// The kind of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// The document node, root of every tree.
    Document,
    /// An element.
    Element,
    /// An attribute.
    Attribute,
    /// A text node (never empty, never adjacent to another text node).
    Text,
    /// A comment.
    Comment,
    /// A processing instruction.
    ProcessingInstruction,
}

/// The axes, in the sense of XPath 2.0 section 3.2.1.1, that the evaluator walks.
/// `descendant` is not in the supported XPath subset but is used internally for `//name`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeAxis {
    /// `child`
    Child,
    /// `attribute`
    Attribute,
    /// `self`
    SelfAxis,
    /// `descendant`
    Descendant,
    /// `descendant-or-self`
    DescendantOrSelf,
    /// `following-sibling`
    FollowingSibling,
    /// `parent`
    Parent,
    /// `ancestor`
    Ancestor,
    /// `ancestor-or-self`
    AncestorOrSelf,
    /// `preceding-sibling`
    PrecedingSibling,
    /// `preceding`
    Preceding,
}

impl NodeAxis {
    /// Whether the axis delivers nodes in reverse document order.
    #[must_use]
    pub const fn is_reverse(self) -> bool {
        matches!(
            self,
            Self::Parent
                | Self::Ancestor
                | Self::AncestorOrSelf
                | Self::PrecedingSibling
                | Self::Preceding
        )
    }
}

/// One node of the arena: 32 bytes.
///
/// Every field is read directly by the axis walks and node tests, which run in chains of
/// dependent loads: packing the kind with the name, or sharing fields between the subtree
/// range of an element and the text range of the other nodes, measured 2 to 3% slower
/// over a whole validation.
#[derive(Debug, Clone, Copy)]
struct Record {
    kind: NodeKind,
    /// Index in [`Document::names`], or [`NONE`].
    name: u32,
    parent: u32,
    prev_sibling: u32,
    /// The index just after the node's attributes: its first child if it has one. For a
    /// node that cannot have attributes, the index just after it.
    first_child: u32,
    /// The index just after the node's subtree (attributes and descendants).
    end: u32,
    /// The range of the text of a text node, comment, processing instruction or
    /// attribute in the document's text space (see [`Document::own_text`]): start and
    /// length.
    text_start: u32,
    text_len: u32,
}

impl Record {
    /// A node at `index` without children (yet), with its text range.
    const fn new(
        kind: NodeKind,
        name: u32,
        parent: u32,
        prev_sibling: u32,
        index: u32,
        (text_start, text_len): (u32, u32),
    ) -> Self {
        Self {
            kind,
            name,
            parent,
            prev_sibling,
            first_child: index + 1,
            end: index + 1,
            text_start,
            text_len,
        }
    }
}

/// A problem building or parsing a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentError {
    message: String,
}

impl DocumentError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The explanation.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for DocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DocumentError {}

/// An immutable XML tree.
///
/// The texts of its nodes live in one text space: the source text the document was
/// parsed from, when the parser was given it as an owned `String` (offsets below its
/// length), followed by the texts that are not written verbatim in it (offsets from its
/// length on).
pub struct Document {
    serial: u64,
    uri: Option<String>,
    base_uri: Option<String>,
    has_xml_base: bool,
    records: Vec<Record>,
    names: Vec<QName>,
    /// The source text, or empty when the texts were copied out of it.
    source: String,
    /// The texts that are not in `source`.
    text: String,
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Document")
            .field("serial", &self.serial)
            .field("uri", &self.uri)
            .field("nodes", &self.records.len())
            .finish_non_exhaustive()
    }
}

impl Document {
    /// Parses XML text. Every text node is kept, whitespace-only ones included; a document
    /// type declaration is refused (its default attributes and entities would change the
    /// tree without notice). `base_uri`, if given, is the base URI of every node (the
    /// location the text was read from, Saxon's system id).
    ///
    /// The text can be borrowed (`&str`) or given (`String`). Borrowed, the texts of the
    /// nodes are copied into the tree. Given, the tree keeps it and refers to it for every
    /// text written verbatim in it (text without entity or character references, line
    /// ends or, in attributes, whitespace to normalize): only the others are copied.
    ///
    /// The document has no document URI: in Saxon-HE 12.10, `fn:document-uri` only knows
    /// the documents of its document pool, those that `fn:document` or `fn:doc` loaded,
    /// and is empty for a document given to the transformation, such as the one being
    /// validated. (Saxon 9.9 fell back to the system id.) A document loaded by
    /// `fn:document` is parsed with [`Document::parse_loaded`].
    ///
    /// # Errors
    ///
    /// XML that is not well-formed or not namespace-well-formed, or that has a document
    /// type declaration.
    pub fn parse<'t>(
        text: impl Into<Cow<'t, str>>,
        base_uri: Option<&str>,
    ) -> Result<Rc<Self>, DocumentError> {
        Self::parse_with_uris(text.into(), None, base_uri)
    }

    /// Parses XML text as [`Document::parse`] does, for a document that `fn:document`
    /// loaded from `uri` (the absolute URI it resolved): `uri` is both its document URI
    /// and the base URI of its nodes, as for a document in Saxon's document pool.
    ///
    /// # Errors
    ///
    /// Those of [`Document::parse`].
    pub fn parse_loaded<'t>(
        text: impl Into<Cow<'t, str>>,
        uri: &str,
    ) -> Result<Rc<Self>, DocumentError> {
        Self::parse_with_uris(text.into(), Some(uri), Some(uri))
    }

    fn parse_with_uris(
        text: Cow<'_, str>,
        document_uri: Option<&str>,
        base_uri: Option<&str>,
    ) -> Result<Rc<Self>, DocumentError> {
        let keep_source = matches!(text, Cow::Owned(_));
        let builder = build_parsed(&text, keep_source)?;
        let source = match text {
            Cow::Owned(source) => source,
            Cow::Borrowed(_) => String::new(),
        };
        builder.finish_with_source(
            source,
            document_uri.map(str::to_owned),
            base_uri.map(str::to_owned),
        )
    }

    /// The document node.
    #[must_use]
    pub fn root(self: &Rc<Self>) -> Node {
        Node {
            doc: Rc::clone(self),
            index: 0,
        }
    }

    /// The document URI (`fn:document-uri`), if any: only a document loaded by
    /// `fn:document` has one (see [`Document::parse`]).
    #[must_use]
    pub fn document_uri(&self) -> Option<&str> {
        self.uri.as_deref()
    }

    /// The base URI of the document's nodes, if any.
    #[must_use]
    pub fn base_uri(&self) -> Option<&str> {
        self.base_uri.as_deref()
    }

    /// Whether some element has an `xml:base` attribute (whose effect on base URIs is not
    /// implemented).
    #[must_use]
    pub const fn has_xml_base(&self) -> bool {
        self.has_xml_base
    }

    /// The number of nodes, the document node and attributes included.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.records.len()
    }

    fn record(&self, index: u32) -> &Record {
        // Indices only ever come from this document's own records.
        &self.records[index as usize]
    }

    pub(crate) fn kind(&self, index: u32) -> NodeKind {
        self.record(index).kind
    }

    pub(crate) fn name(&self, index: u32) -> Option<&QName> {
        let name = self.record(index).name;
        (name != NONE).then(|| &self.names[name as usize])
    }

    pub(crate) fn parent_index(&self, index: u32) -> Option<u32> {
        let parent = self.record(index).parent;
        (parent != NONE).then_some(parent)
    }

    /// The text of a text node, comment, processing instruction or attribute: a range of
    /// the text space, in `source` or, from the length of `source` on, in `text`.
    fn own_text(&self, index: u32) -> &str {
        let r = self.record(index);
        let start = r.text_start as usize;
        let end = start + r.text_len as usize;
        match start.checked_sub(self.source.len()) {
            None => &self.source[start..end],
            Some(start) => &self.text[start..end - self.source.len()],
        }
    }

    pub(crate) fn string_value(&self, index: u32) -> Cow<'_, str> {
        match self.kind(index) {
            NodeKind::Text
            | NodeKind::Comment
            | NodeKind::ProcessingInstruction
            | NodeKind::Attribute => Cow::Borrowed(self.own_text(index)),
            NodeKind::Element | NodeKind::Document => {
                let mut texts = self
                    .cursor(index, NodeAxis::Descendant)
                    .walk(self)
                    .filter(|&i| self.kind(i) == NodeKind::Text);
                let Some(first) = texts.next() else {
                    return Cow::Borrowed("");
                };
                let Some(second) = texts.next() else {
                    return Cow::Borrowed(self.own_text(first));
                };
                let mut value = String::from(self.own_text(first));
                value.push_str(self.own_text(second));
                for i in texts {
                    value.push_str(self.own_text(i));
                }
                Cow::Owned(value)
            }
        }
    }

    /// The start of an axis walk from the node at `index`.
    pub(crate) fn cursor(&self, index: u32, axis: NodeAxis) -> Cursor {
        let r = self.record(index);
        let kind = r.kind;
        let is_attribute = kind == NodeKind::Attribute;
        match axis {
            NodeAxis::SelfAxis => Cursor::One(index),
            NodeAxis::Parent => match r.parent {
                NONE => Cursor::Done,
                parent => Cursor::One(parent),
            },
            NodeAxis::Child => Cursor::Siblings {
                next: r.first_child,
                end: r.end,
            },
            NodeAxis::Attribute => {
                if kind == NodeKind::Element {
                    Cursor::Range {
                        next: index + 1,
                        end: r.first_child,
                    }
                } else {
                    Cursor::Done
                }
            }
            NodeAxis::Descendant => Cursor::Descendants {
                next: r.first_child,
                end: r.end,
            },
            NodeAxis::DescendantOrSelf => Cursor::Descendants {
                next: index,
                end: r.end,
            },
            NodeAxis::FollowingSibling => match r.parent {
                NONE => Cursor::Done,
                _ if is_attribute => Cursor::Done,
                parent => Cursor::Siblings {
                    next: r.end,
                    end: self.record(parent).end,
                },
            },
            NodeAxis::Ancestor => Cursor::Ancestors { next: r.parent },
            NodeAxis::AncestorOrSelf => Cursor::Ancestors { next: index },
            NodeAxis::PrecedingSibling => {
                if is_attribute {
                    Cursor::Done
                } else {
                    Cursor::PrecedingSiblings {
                        next: r.prev_sibling,
                    }
                }
            }
            NodeAxis::Preceding => Cursor::Preceding {
                next: index,
                ancestor: r.parent,
            },
        }
    }
}

/// The state of an axis walk over node indices, in axis order.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Cursor {
    Done,
    One(u32),
    /// Consecutive indices (attributes).
    Range {
        next: u32,
        end: u32,
    },
    /// Nodes whose subtrees follow each other (children, following siblings).
    Siblings {
        next: u32,
        end: u32,
    },
    /// Every node of a range except attributes, in document order.
    Descendants {
        next: u32,
        end: u32,
    },
    Ancestors {
        next: u32,
    },
    PrecedingSiblings {
        next: u32,
    },
    /// `next` is one past the next candidate; `ancestor` the nearest ancestor not yet
    /// passed, skipped when reached.
    Preceding {
        next: u32,
        ancestor: u32,
    },
}

impl Cursor {
    pub(crate) fn next(&mut self, doc: &Document) -> Option<u32> {
        match self {
            Self::Done => None,
            Self::One(index) => {
                let index = *index;
                *self = Self::Done;
                Some(index)
            }
            Self::Range { next, end } => {
                if *next >= *end {
                    return None;
                }
                let index = *next;
                *next += 1;
                Some(index)
            }
            Self::Siblings { next, end } => {
                if *next >= *end {
                    return None;
                }
                let index = *next;
                *next = doc.record(index).end;
                Some(index)
            }
            Self::Descendants { next, end } => {
                if *next >= *end {
                    return None;
                }
                let index = *next;
                // Skips the node's attributes (they directly follow an element).
                *next = doc.record(index).first_child;
                Some(index)
            }
            Self::Ancestors { next } => {
                if *next == NONE {
                    return None;
                }
                let index = *next;
                *next = doc.record(index).parent;
                Some(index)
            }
            Self::PrecedingSiblings { next } => {
                if *next == NONE {
                    return None;
                }
                let index = *next;
                *next = doc.record(index).prev_sibling;
                Some(index)
            }
            Self::Preceding { next, ancestor } => loop {
                if *next == 0 {
                    return None;
                }
                *next -= 1;
                let index = *next;
                if index == *ancestor {
                    *ancestor = doc.record(index).parent;
                    continue;
                }
                if doc.record(index).kind != NodeKind::Attribute {
                    return Some(index);
                }
            },
        }
    }

    /// The walk as an iterator borrowing the document.
    pub(crate) fn walk(self, doc: &Document) -> impl Iterator<Item = u32> + '_ {
        let mut cursor = self;
        std::iter::from_fn(move || cursor.next(doc))
    }
}

/// A node: a document and the position of the node in it.
///
/// Two `Node` values are equal when they are the same node (XPath `is`).
#[derive(Clone)]
pub struct Node {
    doc: Rc<Document>,
    index: u32,
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && Rc::ptr_eq(&self.doc, &other.doc)
    }
}

impl Eq for Node {}

impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Node({:?}", self.kind())?;
        if let Some(name) = self.name() {
            write!(f, " {name}")?;
        }
        write!(f, " #{} of document {})", self.index, self.doc.serial)
    }
}

impl Node {
    pub(crate) fn new(doc: Rc<Document>, index: u32) -> Self {
        Self { doc, index }
    }

    pub(crate) const fn index(&self) -> u32 {
        self.index
    }

    /// The document the node belongs to.
    #[must_use]
    pub fn document(&self) -> &Rc<Document> {
        &self.doc
    }

    /// The node kind.
    #[must_use]
    pub fn kind(&self) -> NodeKind {
        self.doc.kind(self.index)
    }

    /// The name of an element or attribute, with the prefix it was written with, or the
    /// target of a processing instruction (as a local name); `None` for other nodes.
    #[must_use]
    pub fn name(&self) -> Option<&QName> {
        self.doc.name(self.index)
    }

    /// The parent node.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        self.doc
            .parent_index(self.index)
            .map(|p| Self::new(Rc::clone(&self.doc), p))
    }

    /// The root of the tree: always the document node.
    #[must_use]
    pub fn root(&self) -> Self {
        Self::new(Rc::clone(&self.doc), 0)
    }

    /// The string value (XDM `dm:string-value`).
    #[must_use]
    pub fn string_value(&self) -> Cow<'_, str> {
        self.doc.string_value(self.index)
    }

    /// The typed value of the node in an untyped tree: `xs:untypedAtomic`, except for
    /// comments and processing instructions, whose typed value is an `xs:string`.
    #[must_use]
    pub fn typed_value(&self) -> AtomicValue {
        let value = self.string_value().into_owned();
        match self.kind() {
            NodeKind::Comment | NodeKind::ProcessingInstruction => AtomicValue::String(value),
            _ => AtomicValue::UntypedAtomic(value),
        }
    }

    /// The nodes on an axis from this node, in axis order (reverse document order for
    /// the reverse axes).
    #[must_use]
    pub fn axis(&self, axis: NodeAxis) -> Axis {
        Axis {
            cursor: self.doc.cursor(self.index, axis),
            doc: Rc::clone(&self.doc),
        }
    }

    /// The children, in document order.
    #[must_use]
    pub fn children(&self) -> Axis {
        self.axis(NodeAxis::Child)
    }

    /// The attributes, in document order (the order of the source).
    #[must_use]
    pub fn attributes(&self) -> Axis {
        self.axis(NodeAxis::Attribute)
    }

    /// Document order. Nodes of different documents are ordered by the creation of their
    /// documents.
    #[must_use]
    pub fn cmp_document_order(&self, other: &Self) -> Ordering {
        self.doc
            .serial
            .cmp(&other.doc.serial)
            .then(self.index.cmp(&other.index))
    }
}

/// The nodes on an axis from a node, in axis order.
#[derive(Debug, Clone)]
pub struct Axis {
    cursor: Cursor,
    doc: Rc<Document>,
}

impl Iterator for Axis {
    type Item = Node;

    fn next(&mut self) -> Option<Node> {
        let index = self.cursor.next(&self.doc)?;
        Some(Node::new(Rc::clone(&self.doc), index))
    }
}

/// Builds a [`Document`] from events, in document order.
///
/// Adjacent text is merged into one text node and empty text is dropped, as the XDM
/// requires. Attributes must come right after their element's start, before its content;
/// an attribute whose expanded name is already on the element replaces it.
#[derive(Debug)]
pub struct TreeBuilder {
    records: Vec<Record>,
    names: Vec<QName>,
    name_ids: HashMap<QName, u32>,
    /// The length of the source text the text space starts with (see [`Document`]): the
    /// texts added are stored from that offset on.
    source_len: usize,
    text: String,
    /// The open nodes, the document node first.
    open: Vec<u32>,
    /// The last child of each open node, or [`NONE`].
    last_child: Vec<u32>,
    has_xml_base: bool,
}

impl Default for TreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

fn to_u32(value: usize) -> Result<u32, DocumentError> {
    u32::try_from(value)
        .ok()
        .filter(|&v| v != NONE)
        .ok_or_else(|| DocumentError::new("document too large"))
}

/// Where the text of a node comes from.
#[derive(Debug, Clone, Copy)]
enum NodeText<'s> {
    /// A range of the source text: offset and length.
    Source(u32, u32),
    /// Text to store.
    Copy(&'s str),
}

impl TreeBuilder {
    /// A builder holding only the document node.
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: vec![Record::new(NodeKind::Document, NONE, NONE, NONE, 0, (0, 0))],
            names: Vec::new(),
            name_ids: HashMap::new(),
            source_len: 0,
            text: String::new(),
            open: vec![0],
            last_child: vec![NONE],
            has_xml_base: false,
        }
    }

    /// A builder whose text space starts with a source text of `source_len` bytes, with
    /// room for exactly `nodes` nodes (the document node included) and `text` bytes of
    /// copied text.
    fn with_capacity(nodes: usize, text: usize, source_len: usize) -> Self {
        let mut builder = Self::new();
        builder.records.reserve_exact(nodes.saturating_sub(1));
        builder.text.reserve_exact(text);
        builder.source_len = source_len;
        builder
    }

    fn intern(&mut self, name: QName) -> Result<u32, DocumentError> {
        if let Some(&id) = self.name_ids.get(&name) {
            return Ok(id);
        }
        let id = self.push_name(name.clone())?;
        self.name_ids.insert(name, id);
        Ok(id)
    }

    /// Adds a name to the table, without interning it.
    fn push_name(&mut self, name: QName) -> Result<u32, DocumentError> {
        let id = to_u32(self.names.len())?;
        self.names.push(name);
        Ok(id)
    }

    /// The range of the text space a text occupies: its offset and length.
    fn store_text(&mut self, value: NodeText<'_>) -> Result<(u32, u32), DocumentError> {
        match value {
            NodeText::Source(start, len) => Ok((start, len)),
            NodeText::Copy(value) => {
                let start = to_u32(self.source_len + self.text.len())?;
                let len = to_u32(value.len())?;
                to_u32(self.source_len + self.text.len() + value.len())?;
                self.text.push_str(value);
                Ok((start, len))
            }
        }
    }

    fn top(&self) -> u32 {
        self.open.last().copied().unwrap_or(0)
    }

    /// Appends a child of the innermost open node and returns its index: an element (its
    /// text is ignored), or a node with a text.
    fn append(
        &mut self,
        kind: NodeKind,
        name: u32,
        text: NodeText<'_>,
    ) -> Result<u32, DocumentError> {
        let index = to_u32(self.records.len())?;
        to_u32(self.records.len() + 1)?;
        let text = if kind == NodeKind::Element {
            (0, 0)
        } else {
            self.store_text(text)?
        };
        let parent = self.top();
        let last = self.last_child.last_mut().map_or(NONE, |l| {
            let previous = *l;
            *l = index;
            previous
        });
        self.records
            .push(Record::new(kind, name, parent, last, index, text));
        Ok(index)
    }

    /// Opens an element.
    ///
    /// # Errors
    ///
    /// A document too large for the arena.
    pub fn start_element(&mut self, name: QName) -> Result<(), DocumentError> {
        let name = self.intern(name)?;
        self.open_element(name)
    }

    fn open_element(&mut self, name: u32) -> Result<(), DocumentError> {
        let index = self.append(NodeKind::Element, name, NodeText::Copy(""))?;
        self.open.push(index);
        self.last_child.push(NONE);
        Ok(())
    }

    /// Adds an attribute to the element just opened.
    ///
    /// # Errors
    ///
    /// No element is open, or the element already has content.
    pub fn attribute(&mut self, name: QName, value: &str) -> Result<(), DocumentError> {
        let element = self.top();
        let has_content = self.last_child.last().is_some_and(|&l| l != NONE);
        if element == 0 || has_content {
            return Err(DocumentError::new(format!(
                "attribute {name} outside an element start"
            )));
        }
        let first = element as usize + 1;
        let existing = (first..self.records.len()).find(|&i| {
            let id = self.records[i].name;
            self.names[id as usize].same_expanded_name(&name)
        });
        let xml_base = is_xml_base(name.namespace.as_deref(), &name.local);
        let id = self.intern(name)?;
        if let Some(i) = existing {
            let (text_start, text_len) = self.store_text(NodeText::Copy(value))?;
            let record = &mut self.records[i];
            record.name = id;
            record.text_start = text_start;
            record.text_len = text_len;
            self.has_xml_base |= xml_base;
            return Ok(());
        }
        self.push_attribute(id, xml_base, NodeText::Copy(value))
    }

    /// Adds an attribute, known not to be on the element yet, to the element just opened.
    fn push_attribute(
        &mut self,
        name: u32,
        xml_base: bool,
        value: NodeText<'_>,
    ) -> Result<(), DocumentError> {
        let element = self.top();
        let index = to_u32(self.records.len())?;
        to_u32(self.records.len() + 1)?;
        let previous = if index == element + 1 {
            NONE
        } else {
            index - 1
        };
        let text = self.store_text(value)?;
        self.records.push(Record::new(
            NodeKind::Attribute,
            name,
            element,
            previous,
            index,
            text,
        ));
        self.records[element as usize].first_child = index + 1;
        self.has_xml_base |= xml_base;
        Ok(())
    }

    /// Adds text, merged with text added just before.
    ///
    /// # Errors
    ///
    /// A document too large for the arena.
    pub fn text(&mut self, value: &str) -> Result<(), DocumentError> {
        if value.is_empty() {
            return Ok(());
        }
        let last = self.last_child.last().copied().unwrap_or(NONE);
        // Text merges with the text node just before when that node's text is the end of
        // the copied texts (always so for a built tree, whose texts are all copied).
        let mergeable = last != NONE
            && last as usize + 1 == self.records.len()
            && self.records[last as usize].kind == NodeKind::Text
            && {
                let r = &self.records[last as usize];
                r.text_start as usize + r.text_len as usize == self.source_len + self.text.len()
                    && r.text_start as usize >= self.source_len
            };
        if mergeable {
            self.store_text(NodeText::Copy(value))?;
            let record = &mut self.records[last as usize];
            record.text_len = to_u32(record.text_len as usize + value.len())?;
            return Ok(());
        }
        self.append(NodeKind::Text, NONE, NodeText::Copy(value))
            .map(drop)
    }

    /// Adds a comment.
    ///
    /// # Errors
    ///
    /// A document too large for the arena.
    pub fn comment(&mut self, value: &str) -> Result<(), DocumentError> {
        self.append(NodeKind::Comment, NONE, NodeText::Copy(value))
            .map(drop)
    }

    /// Adds a processing instruction.
    ///
    /// # Errors
    ///
    /// A document too large for the arena.
    pub fn processing_instruction(
        &mut self,
        target: &str,
        value: &str,
    ) -> Result<(), DocumentError> {
        let name = self.intern(QName::local(target))?;
        self.append(NodeKind::ProcessingInstruction, name, NodeText::Copy(value))
            .map(drop)
    }

    /// Closes the innermost open element.
    ///
    /// # Errors
    ///
    /// No element is open.
    pub fn end_element(&mut self) -> Result<(), DocumentError> {
        if self.open.len() < 2 {
            return Err(DocumentError::new("end of element without a start"));
        }
        let element = self.top();
        self.open.pop();
        self.last_child.pop();
        let end = to_u32(self.records.len())?;
        self.records[element as usize].end = end;
        Ok(())
    }

    /// The finished document, with its document URI and the base URI of its nodes.
    ///
    /// # Errors
    ///
    /// An element is still open.
    pub fn finish(
        self,
        document_uri: Option<String>,
        base_uri: Option<String>,
    ) -> Result<Rc<Document>, DocumentError> {
        self.finish_with_source(String::new(), document_uri, base_uri)
    }

    /// The finished document, whose text space starts with `source` (as long as the
    /// builder was told).
    fn finish_with_source(
        mut self,
        source: String,
        document_uri: Option<String>,
        base_uri: Option<String>,
    ) -> Result<Rc<Document>, DocumentError> {
        if self.open.len() != 1 {
            return Err(DocumentError::new("unclosed element"));
        }
        if source.len() != self.source_len {
            return Err(DocumentError::new("source text of another length"));
        }
        self.records[0].end = to_u32(self.records.len())?;
        // No spare capacity is kept: a parsed tree is sized exactly, a built one is
        // trimmed once.
        self.records.shrink_to_fit();
        self.names.shrink_to_fit();
        self.text.shrink_to_fit();
        Ok(Rc::new(Document {
            serial: NEXT_SERIAL.fetch_add(1, AtomicOrdering::Relaxed),
            uri: document_uri,
            base_uri,
            has_xml_base: self.has_xml_base,
            records: self.records,
            names: self.names,
            source,
            text: self.text,
        }))
    }
}

fn is_xml_base(namespace: Option<&str>, local: &str) -> bool {
    local == "base" && namespace == Some(XML_NAMESPACE)
}

/// The offset of `part` in `input`, if `part` is a slice of it.
fn offset_in(input: &str, part: &str) -> Option<usize> {
    let start = part.as_ptr().addr().checked_sub(input.as_ptr().addr())?;
    (start.checked_add(part.len())? <= input.len()).then_some(start)
}

/// A name as the parser sees it: prefix as written (empty for none), namespace URI and
/// local part, all borrowed from the text or the parsed document.
type NameKey<'a> = (&'a str, Option<&'a str>, &'a str);

/// Builds the tree of a parsed document, in two passes over the parser's tree: the first
/// counts the nodes and the bytes of text to copy, so that the arena is allocated once at
/// its exact size; the second fills it. With `keep_source`, every text that is a slice of
/// `input` is stored as a range of it instead of being copied (the document then keeps
/// `input`).
fn build_parsed(input: &str, keep_source: bool) -> Result<TreeBuilder, DocumentError> {
    let options = roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: u32::MAX,
        ..roxmltree::ParsingOptions::default()
    };
    let parsed = roxmltree::Document::parse_with_options(input, options)
        .map_err(|e| DocumentError::new(format!("XML: {e}")))?;
    let (nodes, text_bytes) = sizes(&parsed, input, keep_source);
    let mut builder =
        TreeBuilder::with_capacity(nodes, text_bytes, if keep_source { input.len() } else { 0 });
    let mut names: HashMap<NameKey<'_>, u32> = HashMap::new();
    let node_text = |text| node_text(input, keep_source, text);
    // Elements still open, innermost last: closed when the walk in document order
    // reaches a node that is not inside them.
    let mut open: Vec<roxmltree::NodeId> = Vec::new();
    for node in parsed.root().descendants().skip(1) {
        let parent = node.parent().map(|p| p.id());
        while open.last().is_some_and(|&id| Some(id) != parent) {
            open.pop();
            builder.end_element()?;
        }
        match node.node_type() {
            roxmltree::NodeType::Root => {}
            roxmltree::NodeType::Element => {
                let tag = node.tag_name();
                let key = (
                    element_prefix(input, node),
                    tag.namespace().filter(|ns| !ns.is_empty()),
                    tag.name(),
                );
                let id = name_id(&mut builder, &mut names, key)?;
                builder.open_element(id)?;
                // The parser refuses duplicate attributes: each one is new.
                for attribute in node.attributes() {
                    let namespace = attribute.namespace().filter(|ns| !ns.is_empty());
                    let local = attribute.name();
                    let prefix = input
                        .get(attribute.range_qname())
                        .and_then(|q| q.split_once(':'))
                        .map_or("", |(prefix, _)| prefix);
                    let id = name_id(&mut builder, &mut names, (prefix, namespace, local))?;
                    builder.push_attribute(
                        id,
                        is_xml_base(namespace, local),
                        node_text(attribute.value())?,
                    )?;
                }
                open.push(node.id());
            }
            roxmltree::NodeType::Text => {
                // The parser merges adjacent text, so the text nodes it gives are never
                // adjacent.
                if is_in_element(node) {
                    let text = node.text().unwrap_or_default();
                    if !text.is_empty() {
                        builder.append(NodeKind::Text, NONE, node_text(text)?)?;
                    }
                }
            }
            roxmltree::NodeType::Comment => {
                let text = node_text(node.text().unwrap_or_default())?;
                builder.append(NodeKind::Comment, NONE, text)?;
            }
            roxmltree::NodeType::PI => {
                if let Some(pi) = node.pi() {
                    let id = name_id(&mut builder, &mut names, ("", None, pi.target))?;
                    let text = node_text(pi.value.unwrap_or(""))?;
                    builder.append(NodeKind::ProcessingInstruction, id, text)?;
                }
            }
        }
    }
    for _ in open {
        builder.end_element()?;
    }
    Ok(builder)
}

/// The number of nodes of the tree of a parsed document (the document node included) and
/// the bytes of text to copy into it: all of them, or with `keep_source` those that are
/// not slices of `input`.
fn sizes(parsed: &roxmltree::Document<'_>, input: &str, keep_source: bool) -> (usize, usize) {
    let copied = |text: &str| {
        if keep_source && offset_in(input, text).is_some() {
            0
        } else {
            text.len()
        }
    };
    let (mut nodes, mut text_bytes) = (1usize, 0usize);
    for node in parsed.root().descendants().skip(1) {
        match node.node_type() {
            roxmltree::NodeType::Root => {}
            roxmltree::NodeType::Element => {
                nodes += 1;
                for attribute in node.attributes() {
                    nodes += 1;
                    text_bytes += copied(attribute.value());
                }
            }
            roxmltree::NodeType::Text => {
                if is_in_element(node) {
                    nodes += 1;
                    text_bytes += copied(node.text().unwrap_or_default());
                }
            }
            roxmltree::NodeType::Comment => {
                nodes += 1;
                text_bytes += copied(node.text().unwrap_or_default());
            }
            roxmltree::NodeType::PI => {
                nodes += 1;
                text_bytes += node.pi().map_or(0, |pi| copied(pi.value.unwrap_or("")));
            }
        }
    }
    (nodes, text_bytes)
}

/// The identity of a name in the builder's table, added on first sight.
fn name_id<'a>(
    builder: &mut TreeBuilder,
    names: &mut HashMap<NameKey<'a>, u32>,
    key: NameKey<'a>,
) -> Result<u32, DocumentError> {
    if let Some(&id) = names.get(&key) {
        return Ok(id);
    }
    let (prefix, namespace, local) = key;
    let id = builder.push_name(QName {
        prefix: (!prefix.is_empty()).then(|| prefix.to_owned()),
        namespace: namespace.map(str::to_owned),
        local: local.to_owned(),
    })?;
    names.insert(key, id);
    Ok(id)
}

/// Where the text of a parsed node is stored: as a range of `input` if `keep_source` and
/// it is a slice of it, else copied.
fn node_text<'s>(
    input: &str,
    keep_source: bool,
    text: &'s str,
) -> Result<NodeText<'s>, DocumentError> {
    Ok(
        match keep_source.then(|| offset_in(input, text)).flatten() {
            Some(start) => NodeText::Source(to_u32(start)?, to_u32(text.len())?),
            None => NodeText::Copy(text),
        },
    )
}

/// Whether a node is inside an element: text between the top-level nodes is not part of
/// the data model.
fn is_in_element(node: roxmltree::Node<'_, '_>) -> bool {
    node.parent().is_some_and(|p| !p.is_root())
}

/// The prefix an element name is written with, empty for none.
fn element_prefix<'a>(input: &'a str, node: roxmltree::Node<'_, '_>) -> &'a str {
    let start = node.range().start + 1;
    let written = input
        .get(start..)
        .map(|rest| {
            let end = rest
                .find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>')
                .unwrap_or(rest.len());
            &rest[..end]
        })
        .unwrap_or_default();
    written.split_once(':').map_or("", |(prefix, _)| prefix)
}

/// The `file:` URI that Saxon-HE gives a document read from a file: Java's
/// `File.toURI().toString()` of the absolute path. ASCII characters outside the URI path
/// syntax, Unicode space separators and control characters are percent-encoded (UTF-8,
/// upper-case hex); other non-ASCII characters are kept. On Windows, `C:\a\b` becomes
/// `file:/C:/a/b`.
///
/// Returns `None` for a relative path or one that is not valid Unicode.
#[must_use]
pub fn file_uri(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return None;
    }
    let text = path.to_str()?.replace('\\', "/");
    let mut uri = String::from("file:");
    if !text.starts_with('/') {
        uri.push('/');
    }
    for c in text.chars() {
        let keep = if c.is_ascii() {
            c.is_ascii_alphanumeric() || "-_.!~*'():@&=+$,;/".contains(c)
        } else {
            // java.lang.Character.isSpaceChar (Zs, Zl, Zp) and isISOControl.
            !(matches!(
                c,
                '\u{A0}' | '\u{1680}' | '\u{2000}'
                    ..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
            ) || c.is_control())
        };
        if keep {
            uri.push(c);
        } else {
            let mut buffer = [0; 4];
            for byte in c.encode_utf8(&mut buffer).bytes() {
                let _ = write!(uri, "%{byte:02X}");
            }
        }
    }
    Some(uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(xml: &str) -> Rc<Document> {
        Document::parse(xml, Some("file:/t.xml")).unwrap()
    }

    fn describe(node: &Node) -> String {
        match node.kind() {
            NodeKind::Document => "/".to_owned(),
            NodeKind::Element => node.name().unwrap().to_string(),
            NodeKind::Attribute => format!("@{}", node.name().unwrap()),
            NodeKind::Text => format!("{:?}", node.string_value()),
            NodeKind::Comment => format!("<!--{}-->", node.string_value()),
            NodeKind::ProcessingInstruction => format!("<?{}?>", node.name().unwrap()),
        }
    }

    fn walk(node: &Node, axis: NodeAxis) -> Vec<String> {
        node.axis(axis).map(|n| describe(&n)).collect()
    }

    fn find(tree: &Rc<Document>, local: &str) -> Node {
        tree.root()
            .axis(NodeAxis::Descendant)
            .find(|n| n.name().is_some_and(|name| name.local == local))
            .unwrap()
    }

    const XML: &str = "<?xml version='1.0'?><!--c0--><r xmlns:p='urn:p' a='1' p:b='2'> \
        <x>1<y q='3'>2</y>3</x><p:z/><!--c1--><?pi data?><w><v/></w></r>";

    const NONE: [&str; 0] = [];

    #[test]
    fn document_order_and_axes() {
        let tree = parse(XML);
        let root = tree.root();
        assert_eq!(walk(&root, NodeAxis::Child), ["<!--c0-->", "r"]);
        let top = find(&tree, "r");
        assert_eq!(walk(&top, NodeAxis::Attribute), ["@a", "@p:b"]);
        assert_eq!(
            walk(&top, NodeAxis::Child),
            ["\" \"", "x", "p:z", "<!--c1-->", "<?pi?>", "w"]
        );
        assert_eq!(
            walk(&top, NodeAxis::Descendant),
            [
                "\" \"",
                "x",
                "\"1\"",
                "y",
                "\"2\"",
                "\"3\"",
                "p:z",
                "<!--c1-->",
                "<?pi?>",
                "w",
                "v"
            ]
        );
        let y = find(&tree, "y");
        assert_eq!(walk(&y, NodeAxis::Ancestor), ["x", "r", "/"]);
        assert_eq!(walk(&y, NodeAxis::AncestorOrSelf), ["y", "x", "r", "/"]);
        assert_eq!(walk(&y, NodeAxis::PrecedingSibling), ["\"1\""]);
        assert_eq!(walk(&y, NodeAxis::FollowingSibling), ["\"3\""]);
        assert_eq!(
            walk(&y, NodeAxis::Preceding),
            ["\"1\"", "\" \"", "<!--c0-->"]
        );
        let v = find(&tree, "v");
        assert_eq!(
            walk(&v, NodeAxis::Preceding),
            [
                "<?pi?>",
                "<!--c1-->",
                "p:z",
                "\"3\"",
                "\"2\"",
                "y",
                "\"1\"",
                "x",
                "\" \"",
                "<!--c0-->"
            ]
        );
        let q = y.attributes().next().unwrap();
        assert_eq!(walk(&q, NodeAxis::Parent), ["y"]);
        assert_eq!(
            walk(&q, NodeAxis::Preceding),
            ["\"1\"", "\" \"", "<!--c0-->"]
        );
        assert_eq!(walk(&q, NodeAxis::FollowingSibling), NONE);
        assert_eq!(walk(&q, NodeAxis::PrecedingSibling), NONE);
        assert_eq!(walk(&q, NodeAxis::DescendantOrSelf), ["@q"]);
        assert_eq!(walk(&q, NodeAxis::Child), NONE);
        let all: Vec<Node> = root.axis(NodeAxis::DescendantOrSelf).collect();
        assert!(
            all.windows(2)
                .all(|w| w[0].cmp_document_order(&w[1]) == Ordering::Less)
        );
    }

    #[test]
    fn values_names_and_prefixes() {
        let tree = parse(XML);
        let top = find(&tree, "r");
        assert_eq!(top.string_value(), " 123");
        assert_eq!(find(&tree, "x").string_value(), "123");
        assert_eq!(find(&tree, "y").string_value(), "2");
        assert_eq!(find(&tree, "w").string_value(), "");
        let z = find(&tree, "z");
        assert_eq!(z.name().unwrap().to_string(), "p:z");
        assert_eq!(z.name().unwrap().namespace.as_deref(), Some("urn:p"));
        let prefixed = top.attributes().nth(1).unwrap();
        assert_eq!(prefixed.name().unwrap().prefix.as_deref(), Some("p"));
        assert!(matches!(prefixed.typed_value(), AtomicValue::UntypedAtomic(s) if s == "2"));
        let comment = tree.root().children().next().unwrap();
        assert!(matches!(comment.typed_value(), AtomicValue::String(s) if s == "c0"));
        assert_eq!(tree.document_uri(), None);
        assert_eq!(tree.base_uri(), Some("file:/t.xml"));
        let loaded = Document::parse_loaded("<a/>", "file:/t.xml").unwrap();
        assert_eq!(loaded.document_uri(), Some("file:/t.xml"));
        assert_eq!(loaded.base_uri(), Some("file:/t.xml"));
        assert_eq!(z.root(), tree.root());
        assert_ne!(z, top);
    }

    #[test]
    fn undeclared_default_namespace_is_no_namespace() {
        let tree = parse("<r xmlns='urn:d'><n xmlns=''><m/></n></r>");
        assert_eq!(
            find(&tree, "r").name().unwrap().namespace.as_deref(),
            Some("urn:d")
        );
        assert_eq!(find(&tree, "n").name().unwrap().namespace, None);
        assert_eq!(find(&tree, "m").name().unwrap().namespace, None);
    }

    #[test]
    fn same_namespace_two_prefixes() {
        let tree = parse("<a:r xmlns:a='urn:x' xmlns:b='urn:x'><b:c a:d='1'/></a:r>");
        let inner = find(&tree, "c");
        assert_eq!(inner.name().unwrap().to_string(), "b:c");
        assert_eq!(find(&tree, "r").name().unwrap().to_string(), "a:r");
        let attribute = inner.attributes().next().unwrap();
        assert_eq!(attribute.name().unwrap().to_string(), "a:d");
    }

    #[test]
    fn builder_merges_text_and_replaces_attributes() {
        let mut builder = TreeBuilder::new();
        builder.start_element(QName::local("e")).unwrap();
        builder.attribute(QName::local("a"), "1").unwrap();
        builder.attribute(QName::local("a"), "2").unwrap();
        builder.text("x").unwrap();
        builder.text("").unwrap();
        builder.text("y").unwrap();
        assert!(builder.attribute(QName::local("late"), "1").is_err());
        builder.end_element().unwrap();
        assert!(builder.end_element().is_err());
        let tree = builder.finish(None, None).unwrap();
        let element = tree.root().children().next().unwrap();
        let attributes: Vec<_> = element
            .attributes()
            .map(|a| a.string_value().into_owned())
            .collect();
        assert_eq!(attributes, ["2"]);
        let texts: Vec<_> = element
            .children()
            .map(|t| t.string_value().into_owned())
            .collect();
        assert_eq!(texts, ["xy"]);
    }

    #[test]
    fn whitespace_text_kept_and_dtd_refused() {
        let tree = parse("<r>\n  <a/>\n</r>\n");
        let top = tree.root().children().next().unwrap();
        assert_eq!(top.children().count(), 3);
        assert!(Document::parse("<!DOCTYPE r [<!ENTITY e 'x'>]><r>&e;</r>", None).is_err());
        assert!(Document::parse("<r><a></r>", None).is_err());
    }

    #[test]
    fn records_are_compact() {
        assert_eq!(std::mem::size_of::<Record>(), 32);
    }

    /// Kind, name, string value and parent of every node, in document order.
    fn dump(tree: &Rc<Document>) -> Vec<String> {
        (0..tree.node_count())
            .map(|i| {
                let node = Node::new(Rc::clone(tree), u32::try_from(i).unwrap());
                let name = node.name().map(|n| {
                    format!(
                        "{}{{{}}}{}",
                        n,
                        n.namespace.as_deref().unwrap_or(""),
                        n.local
                    )
                });
                format!(
                    "{:?} {name:?} {:?} {:?}",
                    node.kind(),
                    node.string_value(),
                    node.parent().map(|p| p.index())
                )
            })
            .collect()
    }

    /// Texts written verbatim in an owned source are ranges of it; the others (character
    /// and entity references, line ends, attribute whitespace, CDATA sections merged with
    /// text) are copied. The tree is the same as from the borrowed text, which is copied
    /// whole, and its arenas are sized exactly.
    #[test]
    fn owned_source_is_referred_to() {
        let xml = "<?xml version='1.0'?><!--top--><?pi top data?>\
            <p:r xmlns:p='urn:p' xmlns='urn:d' a='plain' b='x&amp;y' c='t\tu'>\
            verbatim<e p:f='1'>a&#65;b</e>line\r\nend<![CDATA[<&>]]>more<!--c-->\
            <?pi in?><e/>tail</p:r>";
        let borrowed = Document::parse(xml, None).unwrap();
        let owned = Document::parse(xml.to_owned(), None).unwrap();
        assert_eq!(dump(&borrowed), dump(&owned));
        assert_eq!(borrowed.source, "");
        assert_eq!(owned.source, xml);
        // Copied: "x&y", "t u", "aAb", "line\nend<&>more"; everything else is a range.
        assert_eq!(owned.text, "x&yt uaAbline\nend<&>more");
        let copied_whole: usize = (0..borrowed.node_count())
            .map(|i| {
                let i = u32::try_from(i).unwrap();
                match borrowed.kind(i) {
                    NodeKind::Element | NodeKind::Document => 0,
                    _ => borrowed.own_text(i).len(),
                }
            })
            .sum();
        assert_eq!(borrowed.text.len(), copied_whole);
        for tree in [&borrowed, &owned] {
            assert_eq!(tree.records.capacity(), tree.records.len());
            assert_eq!(tree.text.capacity(), tree.text.len());
            assert_eq!(tree.names.capacity(), tree.names.len());
        }
        let r = find(&owned, "r");
        let values: Vec<_> = r
            .attributes()
            .map(|a| a.string_value().into_owned())
            .collect();
        assert_eq!(values, ["plain", "x&y", "t u"]);
        assert_eq!(r.string_value(), "verbatimaAbline\nend<&>moretail");
        assert_eq!(r.name().unwrap().to_string(), "p:r");
        let e = find(&owned, "e");
        assert_eq!(e.name().unwrap().namespace.as_deref(), Some("urn:d"));
        let f = e.attributes().next().unwrap();
        assert_eq!(f.name().unwrap().to_string(), "p:f");
        assert_eq!(f.name().unwrap().namespace.as_deref(), Some("urn:p"));
        // Names are interned: the two `e` elements share one entry.
        assert_eq!(owned.names.iter().filter(|n| n.local == "e").count(), 1);
    }

    #[test]
    fn built_trees_merge_text_after_a_parsed_shape() {
        // A built tree has no source: all its texts are copied, and adjacent text merges
        // even after other copied texts.
        let mut builder = TreeBuilder::new();
        builder.start_element(QName::local("e")).unwrap();
        builder.attribute(QName::local("a"), "v").unwrap();
        builder.comment("c").unwrap();
        builder.text("x").unwrap();
        builder.text("y").unwrap();
        builder.processing_instruction("t", "d").unwrap();
        builder.text("z").unwrap();
        builder.end_element().unwrap();
        let tree = builder.finish(None, None).unwrap();
        let e = tree.root().children().next().unwrap();
        let children: Vec<_> = e
            .children()
            .map(|c| (c.kind(), c.string_value().into_owned()))
            .collect();
        assert_eq!(
            children,
            [
                (NodeKind::Comment, "c".to_owned()),
                (NodeKind::Text, "xy".to_owned()),
                (NodeKind::ProcessingInstruction, "d".to_owned()),
                (NodeKind::Text, "z".to_owned()),
            ]
        );
        assert_eq!(tree.text, "vcxydz");
    }

    #[test]
    fn file_uris() {
        if cfg!(windows) {
            return;
        }
        assert_eq!(
            file_uri(Path::new("/a b/c,d%é#[1]\u{a0}.xml")).as_deref(),
            Some("file:/a%20b/c,d%25é%23%5B1%5D%C2%A0.xml")
        );
        assert_eq!(file_uri(Path::new("rel.xml")), None);
    }
}
