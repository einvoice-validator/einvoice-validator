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
//! prefixes as written in the source (they appear in `fn:name`). Parsing reads the text
//! twice with the streaming reader of `einvoice-xml`, which builds no tree of its own:
//! once to check it and count its nodes, then to fill an arena of exactly that size.
//!
//! A built tree may have no document node: a parentless element, attribute, text node or
//! comment ([`TreeBuilder::parentless`]), as the XSLT layer constructs them in a sequence
//! (the root of its tree is the node itself). The elements of a built tree have the
//! namespace nodes they were given ([`TreeBuilder::start_element_with_namespaces`]), and
//! those of their ancestors in the tree; a parsed document keeps none (only the namespaces
//! of the names of its elements and attributes). There are no type annotations: every
//! element and attribute is untyped, so its typed value is `xs:untypedAtomic`.
//!
//! A parsed document may be nested at most [`DEFAULT_MAX_DEPTH`] elements deep, or the
//! maximum given to [`Document::parse_with_max_depth`]: beyond it the parse fails with
//! [`LimitExceeded::Depth`], so that what walks the tree recursively (the template rules of
//! a stylesheet) has a bounded depth. Parsing, building and walking a tree use no
//! recursion.

use std::borrow::Cow;
use std::cell::{Cell, OnceCell};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt::{self, Write as _};
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use std::sync::Arc;

use einvoice_xdm::AtomicValue;
use einvoice_xpath::{NamespaceBinding, QName, XML_NAMESPACE};

use crate::limit::{DEFAULT_MAX_DEPTH, LimitExceeded};

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
    limit: Option<LimitExceeded>,
}

impl DocumentError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            limit: None,
        }
    }

    fn limit_exceeded(limit: LimitExceeded) -> Self {
        Self {
            message: limit.to_string(),
            limit: Some(limit),
        }
    }

    /// The explanation.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The limit the document exceeded, when that is the problem: an element nested
    /// deeper than the maximum depth ([`LimitExceeded::Depth`]).
    #[must_use]
    pub const fn limit(&self) -> Option<LimitExceeded> {
        self.limit
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
    /// The namespace nodes given to elements of a built tree, by element index, in
    /// document order.
    namespaces: Vec<(u32, Arc<[NamespaceBinding]>)>,
    /// The source text, or empty when the texts were copied out of it.
    source: String,
    /// The texts that are not in `source`.
    text: String,
    /// The elements by expanded name, built by the second search for elements of a
    /// name among many nodes ([`Document::elements_named`]).
    elements: OnceCell<ElementIndex>,
    /// The searches [`Document::elements_named`] answered without the index.
    searches: Cell<u32>,
}

/// The fewest nodes a search for elements of a name must span to use an index.
const INDEXED_SEARCH: u32 = 64;

/// The elements of a document by local name, then namespace, each list in document
/// order.
#[derive(Debug, Default)]
struct ElementIndex {
    by_local: HashMap<String, ByNamespace>,
}

/// The elements of one local name, by namespace.
type ByNamespace = Vec<(Option<String>, Vec<u32>)>;

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
    /// tree without notice). Line ends are normalised as XML 1.0 requires (section
    /// 2.11), in comments and processing instructions too. `base_uri`, if given, is the
    /// base URI of every node (the location the text was read from, Saxon's system id).
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
    /// An element may be nested at most [`DEFAULT_MAX_DEPTH`] deep (the document element
    /// is at depth 1); [`Document::parse_with_max_depth`] sets another maximum.
    ///
    /// # Errors
    ///
    /// XML that is not well-formed or not namespace-well-formed, or that has a document
    /// type declaration; an element nested deeper than the maximum depth
    /// ([`DocumentError::limit`] is then [`LimitExceeded::Depth`]).
    pub fn parse<'t>(
        text: impl Into<Cow<'t, str>>,
        base_uri: Option<&str>,
    ) -> Result<Rc<Self>, DocumentError> {
        Self::parse_with_uris(text.into(), None, base_uri, DEFAULT_MAX_DEPTH)
    }

    /// [`Document::parse`] with another maximum element depth than
    /// [`DEFAULT_MAX_DEPTH`]. What walks a tree recursively (the template rules of a
    /// stylesheet) needs stack in proportion to its depth: a larger maximum needs a
    /// larger stack.
    ///
    /// # Errors
    ///
    /// Those of [`Document::parse`].
    pub fn parse_with_max_depth<'t>(
        text: impl Into<Cow<'t, str>>,
        base_uri: Option<&str>,
        max_depth: u32,
    ) -> Result<Rc<Self>, DocumentError> {
        Self::parse_with_uris(text.into(), None, base_uri, max_depth)
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
        Self::parse_with_uris(text.into(), Some(uri), Some(uri), DEFAULT_MAX_DEPTH)
    }

    /// [`Document::parse_loaded`] with another maximum element depth than
    /// [`DEFAULT_MAX_DEPTH`].
    ///
    /// # Errors
    ///
    /// Those of [`Document::parse`].
    pub fn parse_loaded_with_max_depth<'t>(
        text: impl Into<Cow<'t, str>>,
        uri: &str,
        max_depth: u32,
    ) -> Result<Rc<Self>, DocumentError> {
        Self::parse_with_uris(text.into(), Some(uri), Some(uri), max_depth)
    }

    fn parse_with_uris(
        text: Cow<'_, str>,
        document_uri: Option<&str>,
        base_uri: Option<&str>,
        max_depth: u32,
    ) -> Result<Rc<Self>, DocumentError> {
        let keep_source = matches!(text, Cow::Owned(_));
        let builder = build_parsed(&text, keep_source, max_depth)?;
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

    /// A new document with the same tree, whose text space starts with `source`, which
    /// must be the text this one starts with.
    fn copy_with_source(&self, source: String) -> Self {
        debug_assert_eq!(source, self.source);
        Self {
            serial: NEXT_SERIAL.fetch_add(1, AtomicOrdering::Relaxed),
            uri: self.uri.clone(),
            base_uri: self.base_uri.clone(),
            has_xml_base: self.has_xml_base,
            records: self.records.clone(),
            names: self.names.clone(),
            namespaces: self.namespaces.clone(),
            source,
            text: self.text.clone(),
            elements: OnceCell::new(),
            searches: Cell::new(0),
        }
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

    /// The number of distinct names of its nodes ([`Node::name_id`]).
    #[must_use]
    pub fn name_count(&self) -> usize {
        self.names.len()
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

    /// The elements named `{namespace}local` among the nodes from `start` (included) to
    /// `end` (excluded), in document order, read from an index of the document's
    /// elements by name, which the second search of [`INDEXED_SEARCH`] nodes or more
    /// builds. `None` when there is no index (yet): the caller searches the nodes.
    pub(crate) fn elements_named(
        &self,
        namespace: Option<&str>,
        local: &str,
        start: u32,
        end: u32,
    ) -> Option<&[u32]> {
        if end.saturating_sub(start) < INDEXED_SEARCH {
            return None;
        }
        let index = if let Some(index) = self.elements.get() {
            index
        } else {
            let searches = self.searches.get().saturating_add(1);
            self.searches.set(searches);
            if searches < 2 {
                return None;
            }
            self.elements.get_or_init(|| self.index_elements())
        };
        let all = index
            .by_local
            .get(local)
            .and_then(|by_ns| by_ns.iter().find(|(ns, _)| ns.as_deref() == namespace))
            .map_or(&[][..], |(_, elements)| elements.as_slice());
        let from = all.partition_point(|&i| i < start);
        let to = all.partition_point(|&i| i < end);
        all.get(from..to)
    }

    /// The index of the elements by expanded name.
    fn index_elements(&self) -> ElementIndex {
        // The list of each name: names that differ only by prefix share one.
        let mut lists: Vec<Vec<u32>> = Vec::new();
        let mut keys: Vec<(Option<&str>, &str)> = Vec::new();
        let mut list_of_key: HashMap<(Option<&str>, &str), usize> = HashMap::new();
        let list_of_name: Vec<usize> = self
            .names
            .iter()
            .map(|name| {
                let key = (name.namespace.as_deref(), name.local.as_str());
                *list_of_key.entry(key).or_insert_with(|| {
                    keys.push(key);
                    lists.push(Vec::new());
                    lists.len() - 1
                })
            })
            .collect();
        for (index, record) in (0_u32..).zip(&self.records) {
            if record.kind == NodeKind::Element
                && let Some(&list) = list_of_name.get(record.name as usize)
                && let Some(elements) = lists.get_mut(list)
            {
                elements.push(index);
            }
        }
        let mut by_local: HashMap<String, ByNamespace> = HashMap::new();
        for ((namespace, local), elements) in keys.into_iter().zip(lists) {
            if !elements.is_empty() {
                by_local
                    .entry(local.to_owned())
                    .or_default()
                    .push((namespace.map(str::to_owned), elements));
            }
        }
        ElementIndex { by_local }
    }

    /// The first index after the attributes of the node at `index` and the first after
    /// its subtree: the range of its descendants.
    pub(crate) fn descendant_range(&self, index: u32) -> (u32, u32) {
        let r = self.record(index);
        (r.first_child, r.end)
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

    /// The number of the node's name in its document, `None` for a node without name:
    /// two nodes of a document have the same number when their names have the same
    /// prefix, local name and namespace. Numbers are below [`Document::name_count`].
    #[must_use]
    pub fn name_id(&self) -> Option<u32> {
        let name = self.doc.record(self.index).name;
        (name != NONE).then_some(name)
    }

    /// The parent node.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        self.doc
            .parent_index(self.index)
            .map(|p| Self::new(Rc::clone(&self.doc), p))
    }

    /// The root of the tree: the document node, or the parentless node of a tree built
    /// without one ([`TreeBuilder::parentless`]).
    #[must_use]
    pub fn root(&self) -> Self {
        Self::new(Rc::clone(&self.doc), 0)
    }

    /// The namespace nodes of an element of a built tree (XDM `dm:namespace-nodes`): those
    /// it was given and those of its ancestors, the nearest binding of a prefix winning,
    /// innermost element first. Empty for other nodes and for the elements of a parsed
    /// document.
    #[must_use]
    pub fn namespaces_in_scope(&self) -> Vec<NamespaceBinding> {
        let doc = &self.doc;
        let mut out: Vec<NamespaceBinding> = Vec::new();
        if doc.namespaces.is_empty() || self.kind() != NodeKind::Element {
            return out;
        }
        let mut index = self.index;
        while index != NONE {
            if let Ok(i) = doc.namespaces.binary_search_by_key(&index, |(e, _)| *e) {
                for binding in doc.namespaces[i].1.iter() {
                    if !out.iter().any(|b| b.prefix == binding.prefix) {
                        out.push(binding.clone());
                    }
                }
            }
            index = doc.record(index).parent;
        }
        out
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

    fn count(self) -> usize {
        self.cursor.walk(&self.doc).count()
    }
}

/// Documents that `fn:document` loads, parsed once and given to every transformation that
/// loads them again: the code lists a stylesheet reads, the same text under the same URI
/// for each document it validates.
///
/// [`DocumentCache::parse_loaded`] returns the tree [`Document::parse_loaded_with_max_depth`]
/// would build from the text, without parsing it again when the same text was parsed under
/// the same URI and maximum depth before: a copy of that tree, which is a new document
/// (created now, as a parse now would create it, for the document order of nodes in
/// different trees). The cache is shared between threads; it keeps at most
/// [`DocumentCache::MAX_DOCUMENTS`] documents of at most [`DocumentCache::MAX_BYTES`] of
/// text in all, and parses the others each time.
#[derive(Debug, Default)]
pub struct DocumentCache {
    entries: std::sync::Mutex<Vec<CachedDocument>>,
}

/// A parsed document kept by a [`DocumentCache`], with what it was parsed under.
#[derive(Debug)]
struct CachedDocument {
    max_depth: u32,
    document: Document,
}

impl DocumentCache {
    /// The most documents a cache keeps.
    pub const MAX_DOCUMENTS: usize = 64;
    /// The most text, in bytes, of the documents a cache keeps.
    pub const MAX_BYTES: usize = 64 << 20;

    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The document [`Document::parse_loaded_with_max_depth`] parses from `text`, loaded
    /// from `uri`: a copy of the one parsed before from the same text, URI and maximum
    /// depth, or parsed now (and kept, room permitting).
    ///
    /// # Errors
    ///
    /// Those of [`Document::parse`]; a text that does not parse is not kept.
    pub fn parse_loaded(
        &self,
        text: String,
        uri: &str,
        max_depth: u32,
    ) -> Result<Rc<Document>, DocumentError> {
        {
            let entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(entry) = entries.iter().find(|e| {
                e.max_depth == max_depth
                    && e.document.uri.as_deref() == Some(uri)
                    && e.document.source == text
            }) {
                return Ok(Rc::new(entry.document.copy_with_source(text)));
            }
        }
        let document = Document::parse_loaded_with_max_depth(text, uri, max_depth)?;
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let bytes: usize = entries.iter().map(|e| e.document.source.len()).sum();
        if entries.len() < Self::MAX_DOCUMENTS
            && bytes.saturating_add(document.source.len()) <= Self::MAX_BYTES
            && !entries.iter().any(|e| {
                e.max_depth == max_depth
                    && e.document.uri.as_deref() == Some(uri)
                    && e.document.source == document.source
            })
        {
            entries.push(CachedDocument {
                max_depth,
                document: document.copy_with_source(document.source.clone()),
            });
        }
        Ok(document)
    }
}

/// Builds a [`Document`] from events, in document order.
///
/// Adjacent text is merged into one text node and empty text is dropped, as the XDM
/// requires (except at the top of a parentless tree, see [`TreeBuilder::parentless`]). Attributes must come right after their element's start, before its content;
/// an attribute whose expanded name is already on the element replaces it.
#[derive(Debug)]
pub struct TreeBuilder {
    records: Vec<Record>,
    names: Vec<QName>,
    name_ids: HashMap<QName, u32>,
    /// The namespace nodes given to elements, by element index.
    namespaces: Vec<(u32, Arc<[NamespaceBinding]>)>,
    /// Whether the tree has no document node: its one top-level node is its root.
    parentless: bool,
    /// The length of the source text the text space starts with (see [`Document`]): the
    /// texts added are stored from that offset on.
    source_len: usize,
    text: String,
    /// The open nodes, the document node first (none for a parentless tree).
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
            namespaces: Vec::new(),
            parentless: false,
            source_len: 0,
            text: String::new(),
            open: vec![0],
            last_child: vec![NONE],
            has_xml_base: false,
        }
    }

    /// A builder of a tree without document node, whose root is the one node added at
    /// the top: a parentless element (with its content), attribute, text node or comment
    /// (XDM nodes without parent, as XSLT constructs them in a sequence). Text added at
    /// the top makes one text node; anything added after the top-level node is an error.
    #[must_use]
    pub fn parentless() -> Self {
        Self {
            records: Vec::new(),
            open: Vec::new(),
            parentless: true,
            ..Self::new()
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

    /// The innermost open node, or [`NONE`] at the top of a parentless tree.
    fn top(&self) -> u32 {
        self.open.last().copied().unwrap_or(NONE)
    }

    /// The number of open nodes that are not elements (the document node).
    const fn base(&self) -> usize {
        if self.parentless { 0 } else { 1 }
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
        if self.parentless && self.open.is_empty() && !self.records.is_empty() {
            return Err(DocumentError::new(
                "a second node at the top of a parentless tree",
            ));
        }
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

    /// Opens an element with namespace nodes (besides those it inherits from its
    /// ancestors in the tree, see [`Node::namespaces_in_scope`]).
    ///
    /// # Errors
    ///
    /// A document too large for the arena.
    pub fn start_element_with_namespaces(
        &mut self,
        name: QName,
        namespaces: Arc<[NamespaceBinding]>,
    ) -> Result<(), DocumentError> {
        self.start_element(name)?;
        if !namespaces.is_empty() {
            let element = self.top();
            self.namespaces.push((element, namespaces));
        }
        Ok(())
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
        if self.parentless && self.records.is_empty() {
            // A parentless attribute: the root of its tree.
            let xml_base = is_xml_base(name.namespace.as_deref(), &name.local);
            let id = self.intern(name)?;
            let text = self.store_text(NodeText::Copy(value))?;
            self.records
                .push(Record::new(NodeKind::Attribute, id, NONE, NONE, 0, text));
            self.has_xml_base |= xml_base;
            return Ok(());
        }
        let element = self.top();
        let has_content = self.last_child.last().is_some_and(|&l| l != NONE);
        if element == NONE
            || self.records[element as usize].kind != NodeKind::Element
            || has_content
        {
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
            // At the top of a parentless tree, a zero-length text node (XSLT constructs
            // them in sequences); dropped anywhere else.
            if self.parentless && self.records.is_empty() {
                return self
                    .append(NodeKind::Text, NONE, NodeText::Copy(""))
                    .map(drop);
            }
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
        if self.open.len() <= self.base() {
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
        if self.open.len() != self.base() {
            return Err(DocumentError::new("unclosed element"));
        }
        if source.len() != self.source_len {
            return Err(DocumentError::new("source text of another length"));
        }
        if self.parentless {
            if self.records.is_empty() {
                return Err(DocumentError::new("a parentless tree without a node"));
            }
        } else {
            self.records[0].end = to_u32(self.records.len())?;
        }
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
            namespaces: self.namespaces,
            source,
            text: self.text,
            elements: OnceCell::new(),
            searches: Cell::new(0),
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

/// The error of a document the reader refuses.
fn xml_error(e: &einvoice_xml::Error) -> DocumentError {
    DocumentError::new(format!("XML: {e}"))
}

/// The namespace names of a parsed document, numbered in order of appearance, so that
/// a name is looked up by its prefix, local part and namespace number: the namespace
/// names the reader gives only live until its next event.
struct Namespaces<'a> {
    input: &'a str,
    ids: HashMap<Box<str>, u32>,
    /// The last namespace name looked up, when it is a slice of the input (whose
    /// content its address and length then identify): its offset, length and number.
    last: Option<(usize, usize, u32)>,
}

impl<'a> Namespaces<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            ids: HashMap::new(),
            last: None,
        }
    }

    /// The number of a namespace name, [`NONE`] for no namespace.
    fn id(&mut self, namespace: Option<&str>) -> Result<u32, DocumentError> {
        let Some(namespace) = namespace else {
            return Ok(NONE);
        };
        let at = offset_in(self.input, namespace);
        if let (Some(at), Some((last, len, id))) = (at, self.last)
            && at == last
            && namespace.len() == len
        {
            return Ok(id);
        }
        let id = if let Some(&id) = self.ids.get(namespace) {
            id
        } else {
            let id = to_u32(self.ids.len())?;
            self.ids.insert(namespace.into(), id);
            id
        };
        if let Some(at) = at {
            self.last = Some((at, namespace.len(), id));
        }
        Ok(id)
    }
}

/// A name as the reader gives it: prefix as written (empty for none), local part (both
/// slices of the text) and the number of its namespace name ([`Namespaces::id`]).
type NameKey<'a> = (&'a str, &'a str, u32);

/// The names of a parsed document in the builder's table, by [`NameKey`].
struct Names<'a> {
    namespaces: Namespaces<'a>,
    ids: HashMap<NameKey<'a>, u32>,
}

impl<'a> Names<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            namespaces: Namespaces::new(input),
            ids: HashMap::new(),
        }
    }

    /// The identity of a name in the builder's table, added on first sight.
    fn id(
        &mut self,
        builder: &mut TreeBuilder,
        name: einvoice_xml::Name<'_, 'a>,
    ) -> Result<u32, DocumentError> {
        let key = (name.prefix, name.local, self.namespaces.id(name.namespace)?);
        if let Some(&id) = self.ids.get(&key) {
            return Ok(id);
        }
        let id = builder.push_name(QName {
            prefix: (!name.prefix.is_empty()).then(|| name.prefix.to_owned()),
            namespace: name.namespace.map(str::to_owned),
            local: name.local.to_owned(),
        })?;
        self.ids.insert(key, id);
        Ok(id)
    }
}

/// What a pass over the events of a document does with its nodes, in document order.
/// Adjacent text events make one text node, given to [`Pass::text`] once complete and
/// only when not empty; text outside the document element is not part of the data
/// model and is not given.
trait Pass<'a> {
    fn start(&mut self, start: einvoice_xml::StartElement<'_, 'a>) -> Result<(), DocumentError>;
    fn end(&mut self) -> Result<(), DocumentError>;
    fn text(&mut self, text: &str) -> Result<(), DocumentError>;
    fn comment(&mut self, value: &str) -> Result<(), DocumentError>;
    fn processing_instruction(
        &mut self,
        pi: &einvoice_xml::ProcessingInstruction<'a>,
    ) -> Result<(), DocumentError>;
}

/// Reads `input` to the end, giving its nodes to `pass`. A well-formedness error comes
/// first, wherever it is in the text; then an element nested deeper than `max_depth`
/// (the first in document order), the elements after it not being given to `pass`.
fn read<'a>(input: &'a str, max_depth: u32, pass: &mut impl Pass<'a>) -> Result<(), DocumentError> {
    let mut reader = einvoice_xml::Reader::new(input);
    let mut depth = 0u32;
    // The text node being read: adjacent text events (character data and CDATA
    // sections) make one.
    let mut text: Option<Cow<'a, str>> = None;
    let mut too_deep: Option<LimitExceeded> = None;
    while let Some(event) = reader.next_event().map_err(|e| xml_error(&e))? {
        if let einvoice_xml::Event::Text(t) = event {
            if depth > 0 && too_deep.is_none() {
                match &mut text {
                    None => text = Some(t.value),
                    Some(joined) => joined.to_mut().push_str(&t.value),
                }
            }
            continue;
        }
        if let Some(joined) = text.take()
            && !joined.is_empty()
        {
            pass.text(&joined)?;
        }
        match event {
            einvoice_xml::Event::Start(start) => {
                depth = depth.saturating_add(1);
                if depth > max_depth && too_deep.is_none() {
                    // The rest is only checked for well-formedness.
                    let at = einvoice_xml::TextPos::at(input, start.range().start);
                    too_deep = Some(LimitExceeded::Depth {
                        max: max_depth,
                        line: at.row,
                        column: at.col,
                    });
                }
                if too_deep.is_none() {
                    pass.start(start)?;
                }
            }
            einvoice_xml::Event::End(_) => {
                depth = depth.saturating_sub(1);
                if too_deep.is_none() {
                    pass.end()?;
                }
            }
            einvoice_xml::Event::Comment(comment) if too_deep.is_none() => {
                pass.comment(&comment.value)?;
            }
            einvoice_xml::Event::ProcessingInstruction(pi) if too_deep.is_none() => {
                pass.processing_instruction(&pi)?;
            }
            _ => {}
        }
    }
    match too_deep {
        Some(limit) => Err(DocumentError::limit_exceeded(limit)),
        None => Ok(()),
    }
}

/// The first pass: the number of nodes of the tree (the document node included) and
/// the bytes of text to copy into it: all of them, or with `keep_source` those that
/// are not slices of the text.
struct Sizes<'a> {
    input: &'a str,
    keep_source: bool,
    nodes: usize,
    text_bytes: usize,
}

impl Sizes<'_> {
    fn add(&mut self, text: &str) {
        self.nodes += 1;
        if !(self.keep_source && offset_in(self.input, text).is_some()) {
            self.text_bytes += text.len();
        }
    }
}

impl<'a> Pass<'a> for Sizes<'a> {
    fn start(&mut self, start: einvoice_xml::StartElement<'_, 'a>) -> Result<(), DocumentError> {
        self.nodes += 1;
        for attribute in start.attributes() {
            self.add(attribute.value);
        }
        Ok(())
    }

    fn end(&mut self) -> Result<(), DocumentError> {
        Ok(())
    }

    fn text(&mut self, text: &str) -> Result<(), DocumentError> {
        self.add(text);
        Ok(())
    }

    fn comment(&mut self, value: &str) -> Result<(), DocumentError> {
        self.add(value);
        Ok(())
    }

    fn processing_instruction(
        &mut self,
        pi: &einvoice_xml::ProcessingInstruction<'a>,
    ) -> Result<(), DocumentError> {
        self.add(pi.value.as_deref().unwrap_or(""));
        Ok(())
    }
}

/// The second pass: the tree, in a builder sized by the first.
struct Fill<'a> {
    input: &'a str,
    keep_source: bool,
    builder: TreeBuilder,
    names: Names<'a>,
}

impl Fill<'_> {
    /// Where a text is stored: as a range of the input if `keep_source` and it is a
    /// slice of it, else copied.
    fn node_text<'s>(&self, text: &'s str) -> Result<NodeText<'s>, DocumentError> {
        Ok(
            match self
                .keep_source
                .then(|| offset_in(self.input, text))
                .flatten()
            {
                Some(start) => NodeText::Source(to_u32(start)?, to_u32(text.len())?),
                None => NodeText::Copy(text),
            },
        )
    }
}

impl<'a> Pass<'a> for Fill<'a> {
    fn start(&mut self, start: einvoice_xml::StartElement<'_, 'a>) -> Result<(), DocumentError> {
        let id = self.names.id(&mut self.builder, start.name())?;
        self.builder.open_element(id)?;
        // The reader refuses duplicate attributes: each one is new.
        for attribute in start.attributes() {
            let id = self.names.id(&mut self.builder, attribute.name)?;
            let text = self.node_text(attribute.value)?;
            self.builder.push_attribute(
                id,
                is_xml_base(attribute.name.namespace, attribute.name.local),
                text,
            )?;
        }
        Ok(())
    }

    fn end(&mut self) -> Result<(), DocumentError> {
        self.builder.end_element()
    }

    fn text(&mut self, text: &str) -> Result<(), DocumentError> {
        let text = self.node_text(text)?;
        self.builder.append(NodeKind::Text, NONE, text).map(drop)
    }

    fn comment(&mut self, value: &str) -> Result<(), DocumentError> {
        let text = self.node_text(value)?;
        self.builder.append(NodeKind::Comment, NONE, text).map(drop)
    }

    fn processing_instruction(
        &mut self,
        pi: &einvoice_xml::ProcessingInstruction<'a>,
    ) -> Result<(), DocumentError> {
        let target = einvoice_xml::Name {
            prefix: "",
            local: pi.target,
            namespace: None,
        };
        let id = self.names.id(&mut self.builder, target)?;
        let text = self.node_text(pi.value.as_deref().unwrap_or(""))?;
        self.builder
            .append(NodeKind::ProcessingInstruction, id, text)
            .map(drop)
    }
}

/// Builds the tree of a document, in two passes of the streaming reader over its text:
/// the first checks it (well-formedness, then depth) and counts the nodes and the bytes
/// of text to copy, so that the arena is allocated once at its exact size; the second
/// fills it. With `keep_source`, every text that is a slice of `input` is stored as a
/// range of it instead of being copied (the document then keeps `input`). An element
/// nested deeper than `max_depth` is refused by the first pass, before the arena is
/// allocated.
fn build_parsed(
    input: &str,
    keep_source: bool,
    max_depth: u32,
) -> Result<TreeBuilder, DocumentError> {
    let mut sizes = Sizes {
        input,
        keep_source,
        nodes: 1,
        text_bytes: 0,
    };
    read(input, max_depth, &mut sizes)?;
    let mut fill = Fill {
        input,
        keep_source,
        builder: TreeBuilder::with_capacity(
            sizes.nodes,
            sizes.text_bytes,
            if keep_source { input.len() } else { 0 },
        ),
        names: Names::new(input),
    };
    read(input, max_depth, &mut fill)?;
    Ok(fill.builder)
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
    fn adjacent_text_is_one_node() {
        let text = "<r>a<![CDATA[<b>]]>c&amp;d<!--x-->e\r\nf<s/></r>";
        for tree in [parse(text), Document::parse(text.to_owned(), None).unwrap()] {
            let top = tree.root().children().next().unwrap();
            let children: Vec<_> = top
                .children()
                .map(|t| (t.kind(), t.string_value().into_owned()))
                .collect();
            assert_eq!(
                children,
                [
                    (NodeKind::Text, "a<b>c&d".to_owned()),
                    (NodeKind::Comment, "x".to_owned()),
                    (NodeKind::Text, "e\nf".to_owned()),
                    (NodeKind::Element, String::new()),
                ]
            );
        }
        // Line ends are normalised in comments and processing instructions too (XML 1.0,
        // section 2.11).
        let tree = parse("<r><!--a\r\nb\rc--><?p a\r\nb?></r>");
        let values: Vec<_> = tree
            .root()
            .children()
            .next()
            .unwrap()
            .children()
            .map(|n| n.string_value().into_owned())
            .collect();
        assert_eq!(values, ["a\nb\nc", "a\nb"]);
        // A carriage return next to a reference is a line end like any other.
        assert_eq!(parse("<r>x\r&amp;y</r>").root().string_value(), "x\n&y");
    }

    #[test]
    fn errors_are_those_of_the_reader() {
        let error = Document::parse("<r>\n<a></r>", None).unwrap_err();
        assert_eq!(error.message(), "XML: expected 'a' tag, not 'r' at 2:4");
        assert_eq!(error.limit(), None);
        let error = Document::parse("<!DOCTYPE r><r/>", None).unwrap_err();
        assert_eq!(error.message(), "XML: XML with DTD detected");
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
    fn the_document_cache_parses_a_text_once() {
        fn shape(doc: &Rc<Document>) -> Vec<String> {
            doc.root()
                .axis(NodeAxis::Descendant)
                .flat_map(|n| std::iter::once(n.clone()).chain(n.attributes()))
                .map(|n| format!("{} {}", describe(&n), n.string_value()))
                .collect()
        }
        let cache = DocumentCache::new();
        let xml = "<cl id='1'><e v='a'/>text &amp; more<e v='b'/></cl>";
        let uri = "file:/codes.xml";
        let first = cache.parse_loaded(xml.to_owned(), uri, 8).unwrap();
        let between = parse("<x/>");
        let second = cache.parse_loaded(xml.to_owned(), uri, 8).unwrap();
        // The same tree, as a new document created after the one parsed in between.
        assert_eq!(shape(&first), shape(&second));
        assert_eq!(second.document_uri(), Some(uri));
        assert_eq!(second.base_uri(), Some(uri));
        assert_eq!(second.node_count(), first.node_count());
        assert_ne!(first.root(), second.root());
        assert_eq!(
            between.root().cmp_document_order(&second.root()),
            Ordering::Less
        );
        assert_eq!(
            first.root().cmp_document_order(&between.root()),
            Ordering::Less
        );
        assert_eq!(cache.entries.lock().unwrap().len(), 1);
        // Another text, URI or maximum depth is parsed, and kept apart.
        let other = cache.parse_loaded("<cl/>".to_owned(), uri, 8).unwrap();
        assert_eq!(other.node_count(), 2);
        let elsewhere = cache
            .parse_loaded(xml.to_owned(), "file:/other.xml", 8)
            .unwrap();
        assert_eq!(elsewhere.document_uri(), Some("file:/other.xml"));
        assert_eq!(shape(&elsewhere), shape(&first));
        let deeper = cache.parse_loaded(xml.to_owned(), uri, 1).unwrap_err();
        assert!(deeper.limit().is_some());
        assert_eq!(cache.entries.lock().unwrap().len(), 3);
        // A text that does not parse is not kept.
        assert!(cache.parse_loaded("<cl>".to_owned(), uri, 8).is_err());
        assert_eq!(cache.entries.lock().unwrap().len(), 3);
    }

    #[test]
    fn the_element_index_lists_the_elements_of_a_name_in_document_order() {
        let mut xml = String::from(r#"<r xmlns:a="urn:x" xmlns:b="urn:x" xmlns="urn:x">"#);
        for i in 0..40 {
            let prefix = ["a:", "b:", ""][i % 3];
            write!(xml, r#"<{prefix}e n="{i}"><f/><e/></{prefix}e><g>t</g>"#).unwrap();
        }
        xml.push_str("</r>");
        let doc = parse(&xml);
        let scan = |start: u32, end: u32| -> Vec<u32> {
            (start..end)
                .filter(|&i| {
                    doc.kind(i) == NodeKind::Element
                        && doc.name(i).is_some_and(|n| {
                            n.local == "e" && n.namespace.as_deref() == Some("urn:x")
                        })
                })
                .collect()
        };
        let (first, end) = doc.descendant_range(0);
        // The first search is answered without an index, the second builds it.
        assert_eq!(doc.elements_named(Some("urn:x"), "e", first, end), None);
        let all = doc.elements_named(Some("urn:x"), "e", first, end).unwrap();
        assert_eq!(all, scan(first, end).as_slice());
        assert_eq!(all.len(), 80);
        // A range inside the document, and names it does not have.
        let inner = doc.elements_named(Some("urn:x"), "e", 10, 200).unwrap();
        assert_eq!(inner, scan(10, 200).as_slice());
        assert_eq!(doc.elements_named(None, "e", first, end), Some(&[][..]));
        assert_eq!(
            doc.elements_named(Some("urn:x"), "zz", first, end),
            Some(&[][..])
        );
        // Small ranges are searched.
        assert_eq!(doc.elements_named(Some("urn:x"), "e", 10, 20), None);
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
