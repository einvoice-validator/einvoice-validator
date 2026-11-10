//! Where the result of a sequence constructor goes (XSLT 2.0 section 5.7).
//!
//! Instructions push events into a [`Sink`]; the instruction that owns the sequence
//! constructor chooses the sink, which gives the result its meaning:
//!
//! - complex content (the final result, written straight to XML by [`XmlWriter`] or
//!   built as a tree; a temporary tree, built by [`TreeSink`]): adjacent atomic values
//!   are separated by a space, nodes are copied, a document node is replaced by its
//!   children;
//! - simple content (attribute values, comments, processing instructions,
//!   `xsl:value-of`): [`TextSink`] concatenates the string values, adjacent text merged,
//!   with a separator between the other items;
//! - a sequence (`xsl:function` bodies, variables and parameters with `as`):
//!   [`SequenceSink`] keeps the items, and each node an instruction constructs at its top
//!   is a new parentless node (an element with its content, a document node with its
//!   content, an attribute, a text node, even a zero-length one, a comment or a
//!   processing instruction), as Saxon-HE 12.10 constructs them.

use std::cmp::Ordering;
use std::rc::Rc;
use std::sync::Arc;

use einvoice_xpath::{QName, XML_NAMESPACE};
use einvoice_xpath_eval::tree::Axis;
use einvoice_xpath_eval::{Document, Item, Node, NodeKind, TreeBuilder};
use einvoice_xslt::NamespaceBinding;

use crate::error::{Result, TransformError, err};
use crate::output::{CompactBuilder, ResultObserver};

/// The receiver of the events of a sequence constructor.
pub(crate) trait Sink {
    /// Opens an element with namespace nodes, besides those it inherits from its parent:
    /// those of a literal result element, of `xsl:element`.
    fn start_element(&mut self, name: &QName, namespaces: &Arc<[NamespaceBinding]>) -> Result<()>;
    /// Opens a copy of an element (`xsl:copy`, `xsl:copy-of`, a node of `xsl:sequence`):
    /// its namespace nodes are exactly `namespaces`, those it had, whatever its new parent
    /// has (Saxon copies the namespaces of an element as they are). Only a tree tells the
    /// two apart: written as XML, a binding of the parent that the copy lacks is not
    /// undeclared, as Saxon's serializer does not undeclare it.
    fn start_copied_element(
        &mut self,
        name: &QName,
        namespaces: &Arc<[NamespaceBinding]>,
    ) -> Result<()> {
        self.start_element(name, namespaces)
    }
    /// Adds a namespace node to the element just opened (`xsl:namespace`): `prefix`
    /// `None` for the default namespace.
    fn namespace(&mut self, prefix: Option<&str>, uri: &str) -> Result<()>;
    /// Adds an attribute to the element just opened.
    fn attribute(&mut self, name: &QName, value: &str) -> Result<()>;
    /// Closes the innermost open element.
    fn end_element(&mut self) -> Result<()>;
    /// A text node (possibly empty: it still separates atomic values).
    fn text(&mut self, text: &str) -> Result<()>;
    /// A comment.
    fn comment(&mut self, text: &str) -> Result<()>;
    /// A processing instruction.
    fn processing_instruction(&mut self, target: &str, value: &str) -> Result<()>;
    /// Opens a document node (a copied one, or `xsl:copy` of one): in complex content,
    /// its children are added in its place.
    fn start_document(&mut self) -> Result<()> {
        Ok(())
    }
    /// Closes the document node opened last.
    fn end_document(&mut self) -> Result<()> {
        Ok(())
    }
    /// An item of `xsl:sequence`: an atomic value or an existing node.
    fn item(&mut self, item: &Item) -> Result<()>;
}

/// Copies an existing node (deep copy, as XSLT 2.0 section 5.7.1 prescribes for nodes in
/// a sequence constructor, and as `xsl:copy-of` copies), each element with its namespace
/// nodes (`copy-namespaces="yes"`). The subtree is walked with an explicit stack, not a
/// recursion: its depth is only bounded by the document's.
pub(crate) fn copy_node(sink: &mut dyn Sink, node: &Node) -> Result<()> {
    // The document node or elements being copied, innermost last, each with its children
    // not copied yet, and for an element (to close) its namespaces in scope.
    let mut open: Vec<(Axis, Option<Arc<[NamespaceBinding]>>)> = Vec::new();
    let mut next = Some(node.clone());
    loop {
        if let Some(node) = next.take() {
            match node.kind() {
                NodeKind::Document => {
                    sink.start_document()?;
                    open.push((node.children(), None));
                }
                NodeKind::Element => {
                    let Some(name) = node.name() else {
                        return err("INTERNAL", "element without name");
                    };
                    // Those of the parent element, unless it has bindings of its own.
                    let inherited = open
                        .last()
                        .and_then(|(_, parent)| parent.as_ref())
                        .filter(|_| !node.has_own_namespaces());
                    let namespaces: Arc<[NamespaceBinding]> = match inherited {
                        Some(parent) => Arc::clone(parent),
                        None => node.namespaces_in_scope().into(),
                    };
                    sink.start_copied_element(name, &namespaces)?;
                    for attribute in node.attributes() {
                        copy_attribute(sink, &attribute)?;
                    }
                    open.push((node.children(), Some(namespaces)));
                }
                NodeKind::Attribute => copy_attribute(sink, &node)?,
                NodeKind::Text => sink.text(&node.string_value())?,
                NodeKind::Comment => sink.comment(&node.string_value())?,
                NodeKind::ProcessingInstruction => {
                    let target = node.name().map_or("", |n| n.local.as_str());
                    sink.processing_instruction(target, &node.string_value())?;
                }
            }
        }
        let Some((children, _)) = open.last_mut() else {
            return Ok(());
        };
        match children.next() {
            Some(child) => next = Some(child),
            None => {
                if let Some((_, element)) = open.pop() {
                    if element.is_some() {
                        sink.end_element()?;
                    } else {
                        sink.end_document()?;
                    }
                }
            }
        }
    }
}

fn copy_attribute(sink: &mut dyn Sink, attribute: &Node) -> Result<()> {
    let Some(name) = attribute.name() else {
        return err("INTERNAL", "attribute without name");
    };
    sink.attribute(name, &attribute.string_value())
}

/// Serialization parameters of the XML output method (`xsl:output`), as Saxon-HE 12.10
/// writes XML: UTF-8, namespace declarations before the attributes and sorted by prefix,
/// `&lt;`, `&gt;` and `&amp;` escaped (and quotes, tabs, line ends and carriage returns
/// in attribute values), carriage returns as `&#xD;`, U+007F to U+009F and U+2028 as
/// character references, three spaces of indentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Serialization {
    /// `indent="yes"`.
    pub indent: bool,
    /// `omit-xml-declaration="yes"`.
    pub omit_xml_declaration: bool,
    /// `Some(true)`: `standalone="yes"`, `Some(false)`: `"no"`, `None`: omitted.
    pub standalone: Option<bool>,
}

/// A namespace binding: the prefix (`None` for the default namespace) and the URI (empty:
/// the default namespace undeclared).
type Binding = (Option<String>, String);

/// A start tag once its namespaces are fixed up: the element's name, its attributes and
/// its own namespace nodes.
type FixedTag = (QName, Vec<(QName, String)>, Vec<Binding>);

/// An element whose start tag is not complete yet: attributes and namespaces may still
/// come.
#[derive(Debug)]
struct StartTag {
    name: QName,
    attributes: Vec<(QName, String)>,
    /// The namespace nodes it was given: those of a literal result element, or all those
    /// of a copied element.
    given: Arc<[NamespaceBinding]>,
    /// Those `xsl:namespace` added.
    added: Vec<Binding>,
}

impl StartTag {
    fn new(name: &QName, given: &Arc<[NamespaceBinding]>) -> Self {
        Self {
            name: name.clone(),
            attributes: Vec::new(),
            given: Arc::clone(given),
            added: Vec::new(),
        }
    }

    /// The URI the element's own namespace nodes bind `prefix` to.
    fn own(&self, prefix: Option<&str>) -> Option<&str> {
        self.added
            .iter()
            .find(|(p, _)| p.as_deref() == prefix)
            .map(|(_, u)| u.as_str())
            .or_else(|| {
                self.given
                    .iter()
                    .find(|b| b.prefix.as_deref() == prefix)
                    .map(|b| b.uri.as_str())
            })
    }

    /// Adds a namespace node (`xsl:namespace`): an error when the element's own namespace
    /// nodes bind the prefix to another URI (`XTDE0430`), or for a default namespace on
    /// an element in no namespace (`XTDE0440`). The namespaces it inherits do not count.
    fn namespace(&mut self, prefix: Option<&str>, uri: &str) -> Result<()> {
        if prefix.is_none() && self.name.namespace.is_none() {
            return err(
                "XTDE0440",
                format!(
                    "a default namespace node ({uri:?}) on the element {}, which is in no namespace",
                    self.name
                ),
            );
        }
        match self.own(prefix) {
            Some(existing) if existing == uri => Ok(()),
            Some(existing) => Err(namespace_conflict(prefix, uri, existing)),
            None => {
                self.added.push((prefix.map(str::to_owned), uri.to_owned()));
                Ok(())
            }
        }
    }

    /// Adds an attribute, replacing one of the same expanded name.
    fn attribute(&mut self, name: &QName, value: &str) {
        if let Some(existing) = self
            .attributes
            .iter_mut()
            .find(|(n, _)| n.same_expanded_name(name))
        {
            existing.0 = name.clone();
            value.clone_into(&mut existing.1);
        } else {
            self.attributes.push((name.clone(), value.to_owned()));
        }
    }

    /// The element's own namespace nodes when they are exactly those it was given: no
    /// `xsl:namespace` added any, and namespace fixup ([`StartTag::fix`]) adds none and
    /// renames nothing, as the namespace nodes given bind the prefix of the element and
    /// that of each attribute in a namespace already. `None` otherwise. Allocates
    /// nothing: the common case of a literal result element or a copy, whose names are
    /// bound by its own namespace nodes.
    fn given_unchanged(&self) -> Option<&Arc<[NamespaceBinding]>> {
        if !self.added.is_empty() {
            return None;
        }
        let bound = |prefix: Option<&str>| {
            self.given
                .iter()
                .find(|b| b.prefix.as_deref() == prefix)
                .map(|b| b.uri.as_str())
        };
        if self.name.prefix.as_deref() != Some("xml")
            && bound(self.name.prefix.as_deref())
                != Some(self.name.namespace.as_deref().unwrap_or(""))
        {
            return None;
        }
        for (attribute, _) in &self.attributes {
            let Some(namespace) = attribute.namespace.as_deref() else {
                continue;
            };
            match attribute.prefix.as_deref() {
                Some("xml") => {}
                Some(prefix) if bound(Some(prefix)) == Some(namespace) => {}
                _ => return None,
            }
        }
        Some(&self.given)
    }

    /// Namespace fixup, as Saxon-HE 12.10 does it when the start tag is complete: the
    /// element's name, then each attribute's, gets a namespace node for its prefix unless
    /// the element's own namespace nodes (those it was given, those added, those fixup
    /// added) bind it already. A prefix they bind to another namespace is replaced: by the
    /// prefix and `_0` for the element, by the prefix, `_` and the position of the
    /// attribute (from 1) for an attribute. A namespace the element inherits is no
    /// conflict: the new node overrides it. Returns the name, the attributes and the own
    /// namespace nodes: those given, then those added, then those of fixup.
    fn fix(self) -> Result<FixedTag> {
        let Self {
            mut name,
            mut attributes,
            given,
            added,
        } = self;
        let mut own: Vec<Binding> = given
            .iter()
            .map(|b| (b.prefix.clone(), b.uri.clone()))
            .filter(|(p, _)| !added.iter().any(|(q, _)| q == p))
            .collect();
        own.extend(added);
        let lookup = |own: &[Binding], prefix: Option<&str>| {
            own.iter()
                .find(|(p, _)| p.as_deref() == prefix)
                .map(|(_, u)| u.clone())
        };
        let uri = name.namespace.clone().unwrap_or_default();
        if name.prefix.as_deref() != Some("xml") {
            match lookup(&own, name.prefix.as_deref()) {
                Some(bound) if bound == uri => {}
                Some(_) => {
                    let Some(prefix) = name.prefix.clone() else {
                        return err(
                            "UNSUPPORTED",
                            format!(
                                "the element {name} in a namespace other than its default namespace node"
                            ),
                        );
                    };
                    let renamed = format!("{prefix}_0");
                    match lookup(&own, Some(&renamed)) {
                        Some(bound) if bound == uri => {}
                        Some(_) => {
                            return err(
                                "UNSUPPORTED",
                                format!("the prefix Saxon gives the element {name}"),
                            );
                        }
                        None => own.push((Some(renamed.clone()), uri.clone())),
                    }
                    name.prefix = Some(renamed);
                }
                None => own.push((name.prefix.clone(), uri)),
            }
        }
        for (seq, (attribute, _)) in (1_usize..).zip(attributes.iter_mut()) {
            let Some(namespace) = attribute.namespace.clone() else {
                continue;
            };
            let Some(prefix) = attribute.prefix.clone() else {
                return err(
                    "UNSUPPORTED",
                    format!("the prefix Saxon gives the attribute {attribute} in a namespace"),
                );
            };
            if prefix == "xml" {
                continue;
            }
            match lookup(&own, Some(&prefix)) {
                Some(bound) if bound == namespace => {}
                Some(_) => {
                    let renamed = format!("{prefix}_{seq}");
                    match lookup(&own, Some(&renamed)) {
                        Some(bound) if bound == namespace => {}
                        Some(_) => {
                            return err(
                                "UNSUPPORTED",
                                format!("the prefix Saxon gives the attribute {attribute}"),
                            );
                        }
                        None => own.push((Some(renamed.clone()), namespace)),
                    }
                    attribute.prefix = Some(renamed);
                }
                None => own.push((Some(prefix), namespace)),
            }
        }
        Ok((name, attributes, own))
    }
}

/// One open element of the result.
#[derive(Debug)]
struct Open {
    /// The written qualified name, for the end tag.
    tag: String,
    /// How many entries of the namespace scope this element added.
    bindings: usize,
    /// The namespace nodes the element was given, when they are its own namespace nodes
    /// ([`StartTag::given_unchanged`]): each is bound as it is in the scope after its
    /// start tag, and so for a child given the same set.
    given: Option<Arc<[NamespaceBinding]>>,
    has_elements: bool,
    has_text: bool,
}

/// Where [`XmlWriter`] puts the text it writes, and who reads the result as it is
/// written.
pub(crate) trait Target {
    /// The text written since the previous call is a complete piece, to keep and clear
    /// from `out`; or left in `out`, to which the writer appends.
    fn commit(&mut self, out: &mut String) -> Result<()>;
    /// Whether some text was taken from the writer.
    fn has_taken(&self) -> bool;
    /// The start of an element, its attributes as written.
    fn start_element(&mut self, name: &QName, attributes: &[(QName, String)]);
    /// The end of the innermost open element.
    fn end_element(&mut self);
    /// Character data inside the document element.
    fn text(&mut self, text: &str);
    /// A comment.
    fn comment(&mut self, text: &str);
    /// A processing instruction.
    fn processing_instruction(&mut self, target: &str, data: &str);
}

/// The result as one string: the writer's own buffer.
#[derive(Debug)]
pub(crate) struct WholeText;

impl Target for WholeText {
    fn commit(&mut self, _out: &mut String) -> Result<()> {
        Ok(())
    }

    fn has_taken(&self) -> bool {
        false
    }

    fn start_element(&mut self, _name: &QName, _attributes: &[(QName, String)]) {}

    fn end_element(&mut self) {}

    fn text(&mut self, _text: &str) {}

    fn comment(&mut self, _text: &str) {}

    fn processing_instruction(&mut self, _target: &str, _data: &str) {}
}

/// The result kept as a [`CompactOutput`], read by an observer as it is written.
pub(crate) struct Compact<'o> {
    pub(crate) builder: CompactBuilder,
    pub(crate) observer: &'o mut dyn ResultObserver,
}

impl std::fmt::Debug for Compact<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compact")
            .field("builder", &self.builder)
            .finish_non_exhaustive()
    }
}

impl Target for Compact<'_> {
    fn commit(&mut self, out: &mut String) -> Result<()> {
        self.builder.push(out)?;
        out.clear();
        Ok(())
    }

    fn has_taken(&self) -> bool {
        !self.builder.is_empty()
    }

    fn start_element(&mut self, name: &QName, attributes: &[(QName, String)]) {
        self.observer.start_element(name, attributes);
    }

    fn end_element(&mut self) {
        self.observer.end_element();
    }

    fn text(&mut self, text: &str) {
        self.observer.text(text);
    }

    fn comment(&mut self, text: &str) {
        self.observer.comment(text);
    }

    fn processing_instruction(&mut self, target: &str, data: &str) {
        self.observer.processing_instruction(target, data);
    }
}

/// The order of namespace declarations in a start tag, as Saxon-HE 12.10 writes them
/// (its `NamespaceMap` is sorted): by prefix, the default namespace first, prefixes
/// compared as Java compares strings (UTF-16 code units).
fn declaration_order(a: &(Option<String>, String), b: &(Option<String>, String)) -> Ordering {
    let key = |p: &Option<String>| {
        p.as_deref()
            .unwrap_or("")
            .encode_utf16()
            .collect::<Vec<u16>>()
    };
    key(&a.0).cmp(&key(&b.0))
}

/// Writes the final result tree as XML text as it is produced, without building it,
/// into its buffer, which the target keeps whole or takes a piece at a time: after an
/// end tag or an empty element, and after a start tag that content follows.
#[derive(Debug)]
pub(crate) struct XmlWriter<T> {
    out: String,
    target: T,
    settings: Serialization,
    /// Whether the result may be a fragment rather than a document: text and several
    /// elements at the top, as Saxon writes the content of `xsl:message`.
    fragment: bool,
    pending: Option<StartTag>,
    open: Vec<Open>,
    /// Namespace bindings in scope, innermost last: prefix (`None`: default) and URI.
    scope: Vec<(Option<String>, String)>,
    last_atomic: bool,
}

const INDENT: &str = "   ";

impl<T: Target> XmlWriter<T> {
    pub(crate) fn new(settings: Serialization, target: T) -> Self {
        let mut out = String::new();
        if !settings.omit_xml_declaration {
            out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"");
            match settings.standalone {
                Some(true) => out.push_str(" standalone=\"yes\""),
                Some(false) => out.push_str(" standalone=\"no\""),
                None => {}
            }
            out.push_str("?>");
        }
        Self {
            out,
            target,
            settings,
            fragment: false,
            pending: None,
            open: Vec::new(),
            scope: Vec::new(),
            last_atomic: false,
        }
    }

    /// The text not taken by the target (all of it for [`WholeText`]), and the target.
    pub(crate) fn finish(mut self) -> Result<(String, T)> {
        self.flush()?;
        if !self.open.is_empty() {
            return err("INTERNAL", "unclosed element at the end of the result");
        }
        if self.settings.indent {
            self.out.push('\n');
        }
        self.target.commit(&mut self.out)?;
        Ok((self.out, self.target))
    }

    fn bound(&self, prefix: Option<&str>) -> Option<&str> {
        self.scope
            .iter()
            .rev()
            .find(|(p, _)| p.as_deref() == prefix)
            .map(|(_, uri)| uri.as_str())
    }

    /// Whether each of the namespace nodes `given` is bound as it is in the scope, so
    /// that a start tag with these as its own namespace nodes declares none. Without a
    /// look at the scope when the parent element was given the same set.
    fn all_bound(&self, given: &Arc<[NamespaceBinding]>) -> bool {
        let parent_given = self.open.last().and_then(|o| o.given.as_ref());
        if parent_given.is_some_and(|p| Arc::ptr_eq(p, given)) {
            return true;
        }
        given.iter().all(|b| {
            b.prefix.as_deref() == Some("xml")
                || self.bound(b.prefix.as_deref()).unwrap_or("") == b.uri
        })
    }

    /// Writes the pending start tag, its namespaces fixed up ([`StartTag::fix`]): the
    /// declarations of the namespace nodes not in scope yet, sorted by prefix, then the
    /// attributes. Bindings in scope that the element lacks are not undeclared (Saxon's
    /// serializer does not), except the default namespace of an element in none.
    fn flush(&mut self) -> Result<()> {
        let Some(pending) = self.pending.take() else {
            return Ok(());
        };
        let given = pending.given_unchanged().map(Arc::clone);
        if given.as_ref().is_some_and(|g| self.all_bound(g)) {
            // Fixup would change nothing, and each namespace node of the element is in
            // scope already: no declaration, nothing to copy.
            let StartTag {
                name, attributes, ..
            } = pending;
            self.write_start_tag(&name, &attributes, &[], given);
            return Ok(());
        }
        let (name, attributes, own) = pending.fix()?;
        let mut declarations: Vec<Binding> = own
            .into_iter()
            .filter(|(prefix, uri)| {
                prefix.as_deref() != Some("xml")
                    && self.bound(prefix.as_deref()).unwrap_or("") != uri
            })
            .collect();
        declarations.sort_by(declaration_order);
        self.write_start_tag(&name, &attributes, &declarations, given);
        Ok(())
    }

    /// Writes a start tag: its namespace declarations (added to the scope), then its
    /// attributes; and opens the element. `given`: as [`Open::given`].
    fn write_start_tag(
        &mut self,
        name: &QName,
        attributes: &[(QName, String)],
        declarations: &[Binding],
        given: Option<Arc<[NamespaceBinding]>>,
    ) {
        let bindings_before = self.scope.len();
        self.scope.extend(declarations.iter().cloned());
        let mut tag = String::new();
        if let Some(prefix) = &name.prefix {
            tag.push_str(prefix);
            tag.push(':');
        }
        tag.push_str(&name.local);
        self.out.push('<');
        self.out.push_str(&tag);
        for (prefix, uri) in declarations {
            self.out.push_str(" xmlns");
            if let Some(prefix) = prefix {
                self.out.push(':');
                self.out.push_str(prefix);
            }
            self.out.push_str("=\"");
            escape_attribute(&mut self.out, uri);
            self.out.push('"');
        }
        for (name, value) in attributes {
            self.out.push(' ');
            if let Some(prefix) = &name.prefix {
                self.out.push_str(prefix);
                self.out.push(':');
            }
            self.out.push_str(&name.local);
            self.out.push_str("=\"");
            escape_attribute(&mut self.out, value);
            self.out.push('"');
        }
        self.out.push('>');
        self.target.start_element(name, attributes);
        self.open.push(Open {
            tag,
            bindings: self.scope.len() - bindings_before,
            given,
            has_elements: false,
            has_text: false,
        });
    }

    /// Writes the pending start tag, if any, as a piece of its own: content follows, the
    /// element is not empty.
    fn flush_tag(&mut self) -> Result<()> {
        if self.pending.is_some() {
            self.flush()?;
            self.target.commit(&mut self.out)?;
        }
        Ok(())
    }

    /// A line end and the indentation of the depth: whitespace in an element, the
    /// innermost open one or the one just closed.
    fn indent(&mut self) {
        let start = self.out.len();
        self.out.push('\n');
        for _ in 0..self.open.len() {
            self.out.push_str(INDENT);
        }
        self.target.text(&self.out[start..]);
    }

    /// Before an element, comment or processing instruction: indentation, unless in mixed
    /// content.
    fn before_markup(&mut self) {
        if let Some(parent) = self.open.last_mut() {
            parent.has_elements = true;
            if !parent.has_text && self.settings.indent {
                self.indent();
            }
        } else if self.settings.indent && (!self.out.is_empty() || self.target.has_taken()) {
            self.out.push('\n');
        }
    }

    fn write_text(&mut self, text: &str) -> Result<()> {
        self.flush_tag()?;
        if text.is_empty() {
            return Ok(());
        }
        let Some(parent) = self.open.last_mut() else {
            if self.fragment {
                escape_text(&mut self.out, text);
                return Ok(());
            }
            if !text.chars().all(|c| matches!(c, ' ' | '\t' | '\r' | '\n')) {
                return err(
                    "UNSUPPORTED",
                    "text outside the document element of the result",
                );
            }
            return Ok(());
        };
        parent.has_text = true;
        escape_text(&mut self.out, text);
        self.target.text(text);
        Ok(())
    }
}

/// The error for a namespace node whose prefix is already bound to another URI on its
/// element.
fn namespace_conflict(prefix: Option<&str>, uri: &str, other: &str) -> TransformError {
    TransformError::new(
        "XTDE0430",
        format!(
            "the namespace prefix {:?} is bound to {other:?} on the element, and a namespace node binds it to {uri:?}",
            prefix.unwrap_or("")
        ),
    )
}

impl<T: Target> Sink for XmlWriter<T> {
    fn start_element(&mut self, name: &QName, namespaces: &Arc<[NamespaceBinding]>) -> Result<()> {
        self.flush_tag()?;
        self.last_atomic = false;
        self.before_markup();
        self.pending = Some(StartTag::new(name, namespaces));
        Ok(())
    }

    fn namespace(&mut self, prefix: Option<&str>, uri: &str) -> Result<()> {
        self.last_atomic = false;
        let Some(pending) = self.pending.as_mut() else {
            return err(
                "XTDE0410",
                "a namespace node cannot be created after the children of the containing element",
            );
        };
        pending.namespace(prefix, uri)
    }

    fn attribute(&mut self, name: &QName, value: &str) -> Result<()> {
        self.last_atomic = false;
        let Some(pending) = self.pending.as_mut() else {
            return err(
                "XTDE0410",
                format!(
                    "an attribute node ({name}) cannot be created after the children of the containing element"
                ),
            );
        };
        pending.attribute(name, value);
        Ok(())
    }

    fn end_element(&mut self) -> Result<()> {
        self.last_atomic = false;
        if let Some(pending) = self.pending.take() {
            // An empty element: written as `<name .../>`.
            self.pending = Some(pending);
            self.flush()?;
            let Some(open) = self.open.pop() else {
                return err("INTERNAL", "end of an element that is not open");
            };
            self.out.pop();
            self.out.push_str("/>");
            self.scope.truncate(self.scope.len() - open.bindings);
            self.target.end_element();
            return self.target.commit(&mut self.out);
        }
        let Some(open) = self.open.pop() else {
            return err("INTERNAL", "end of an element that is not open");
        };
        if open.has_elements && !open.has_text && self.settings.indent {
            self.indent();
        }
        self.out.push_str("</");
        self.out.push_str(&open.tag);
        self.out.push('>');
        self.scope.truncate(self.scope.len() - open.bindings);
        self.target.end_element();
        self.target.commit(&mut self.out)
    }

    fn text(&mut self, text: &str) -> Result<()> {
        self.last_atomic = false;
        self.write_text(text)
    }

    fn comment(&mut self, text: &str) -> Result<()> {
        self.flush_tag()?;
        self.last_atomic = false;
        self.before_markup();
        self.out.push_str("<!--");
        self.out.push_str(text);
        self.out.push_str("-->");
        self.target.comment(text);
        Ok(())
    }

    fn processing_instruction(&mut self, target: &str, value: &str) -> Result<()> {
        self.flush_tag()?;
        self.last_atomic = false;
        self.before_markup();
        self.out.push_str("<?");
        self.out.push_str(target);
        if !value.is_empty() {
            self.out.push(' ');
            self.out.push_str(value);
        }
        self.out.push_str("?>");
        self.target.processing_instruction(target, value);
        Ok(())
    }

    fn item(&mut self, item: &Item) -> Result<()> {
        match item {
            Item::Atomic(_) => {
                if self.last_atomic {
                    self.write_text(" ")?;
                }
                self.write_text(&item.string_value())?;
                self.last_atomic = true;
                Ok(())
            }
            Item::Node(node) => {
                self.last_atomic = false;
                copy_node(self, node)
            }
        }
    }
}

/// Whether Saxon's XML serializer writes a character as a character reference wherever it
/// is: the C1 controls and DEL (U+007F to U+009F), and the line separator U+2028, which
/// XML 1.1 normalizes as a line end. The reference is in lower-case hexadecimal.
const fn written_as_reference(c: char) -> bool {
    matches!(c, '\u{7f}'..='\u{9f}' | '\u{2028}')
}

fn push_reference(out: &mut String, c: char) {
    use std::fmt::Write as _;
    let _ = write!(out, "&#x{:x};", u32::from(c));
}

fn escape_text(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '\r' => out.push_str("&#xD;"),
            c if written_as_reference(c) => push_reference(out, c),
            c => out.push(c),
        }
    }
}

fn escape_attribute(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&#34;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            c if written_as_reference(c) => push_reference(out, c),
            c => out.push(c),
        }
    }
}

/// Writes a tree as XML (`root` a document node or any node), with the serialization
/// parameters `settings`; as a fragment (text and several elements at the top) with
/// `fragment`.
pub(crate) fn serialize(root: &Node, settings: Serialization, fragment: bool) -> Result<String> {
    let mut writer = XmlWriter::new(settings, WholeText);
    writer.fragment = fragment;
    copy_node(&mut writer, root)?;
    Ok(writer.finish()?.0)
}

/// Builds a tree: a temporary tree (a variable whose value is a sequence constructor,
/// without `as`), a new document node; the final result tree; or the content of an
/// element or a document node constructed in a sequence, a parentless element. The
/// start tag of an element is kept until it is complete, then its namespaces fixed up as
/// Saxon does ([`StartTag::fix`]): an element's namespace nodes are those it inherits
/// from its parent and its own (a constructed element), or exactly its own (a copy).
#[derive(Debug)]
pub(crate) struct TreeSink {
    builder: TreeBuilder,
    last_atomic: bool,
    /// The start tag not added yet, and whether it is a copy's.
    pending: Option<(StartTag, bool)>,
    /// The namespaces in scope on each open element, innermost last.
    scopes: Vec<Scope>,
    /// The namespaces of the start tags whose own namespace nodes were those given, as
    /// worked out for the last of them (at most [`SCOPE_MEMO_SIZE`], the oldest dropped).
    memo: Vec<ScopeMemo>,
}

/// The namespaces in scope on an element of a tree being built: shared by the elements
/// of a tree that have the same, so that the scope of an element is known by its
/// address.
type Scope = Rc<[NamespaceBinding]>;

/// The namespaces of an element ([`TreeSink::flush`]) whose own namespace nodes are the
/// set `given` ([`StartTag::given_unchanged`]), copied or not, under a parent of scope
/// `parent` (`None`: no parent element): its scope, and the namespace entries of its
/// node. A function of these three alone, known by their addresses (kept alive here).
#[derive(Debug)]
struct ScopeMemo {
    parent: Option<Scope>,
    given: Arc<[NamespaceBinding]>,
    copied: bool,
    scope: Scope,
    entries: Arc<[NamespaceBinding]>,
}

/// How many [`ScopeMemo`] a tree keeps: the few combinations of the literal result
/// elements of a stylesheet that build it.
const SCOPE_MEMO_SIZE: usize = 16;

/// The namespaces in scope on an element, and the namespace entries of its node, from
/// those of its parent and its own namespace nodes `own` once fixed up: a constructed
/// element inherits its parent's (`copied` false), a copy has exactly its own. The
/// entries are the bindings of its scope its parent's lacks, and the prefixes of its
/// parent's it lacks undeclared (bound to the empty URI).
fn child_scope<'a>(
    parent: &[NamespaceBinding],
    own: impl Iterator<Item = (Option<&'a str>, &'a str)>,
    copied: bool,
) -> (Vec<NamespaceBinding>, Vec<NamespaceBinding>) {
    let mut scope: Vec<NamespaceBinding> = if copied { Vec::new() } else { parent.to_vec() };
    for (prefix, uri) in own {
        if prefix == Some("xml") {
            continue;
        }
        scope.retain(|b| b.prefix.as_deref() != prefix);
        scope.push(NamespaceBinding {
            prefix: prefix.map(str::to_owned),
            uri: uri.to_owned(),
        });
    }
    scope.retain(|b| !b.uri.is_empty());
    let mut entries: Vec<NamespaceBinding> = scope
        .iter()
        .filter(|b| !parent.contains(b))
        .cloned()
        .collect();
    for binding in parent {
        if !scope.iter().any(|b| b.prefix == binding.prefix) {
            entries.push(NamespaceBinding {
                prefix: binding.prefix.clone(),
                uri: String::new(),
            });
        }
    }
    (scope, entries)
}

impl TreeSink {
    /// A temporary tree: a document node.
    pub(crate) fn new() -> Self {
        Self::with(TreeBuilder::new())
    }

    /// A tree without document node, for one parentless node.
    fn parentless() -> Self {
        Self::with(TreeBuilder::parentless())
    }

    const fn with(builder: TreeBuilder) -> Self {
        Self {
            builder,
            last_atomic: false,
            pending: None,
            scopes: Vec::new(),
            memo: Vec::new(),
        }
    }

    pub(crate) fn finish(self) -> Result<Rc<Document>> {
        self.finish_with(None)
    }

    /// The finished tree, its nodes with the base URI `base_uri`.
    pub(crate) fn finish_with(mut self, base_uri: Option<String>) -> Result<Rc<Document>> {
        self.flush()?;
        self.builder
            .finish(None, base_uri)
            .map_err(|e| TransformError::new("INTERNAL", e.message()))
    }

    /// Adds the pending start tag to the tree: the element, with its namespace entries
    /// ([`child_scope`]); then its attributes.
    fn flush(&mut self) -> Result<()> {
        let Some((tag, copied)) = self.pending.take() else {
            return Ok(());
        };
        let parent = self.scopes.last().cloned();
        let (name, attributes, scope, entries) = if let Some(given) = tag.given_unchanged() {
            let given = Arc::clone(given);
            let (scope, entries) = self.memoized_scope(parent, given, copied);
            let StartTag {
                name, attributes, ..
            } = tag;
            (name, attributes, scope, entries)
        } else {
            let (name, attributes, own) = tag.fix()?;
            let own = own.iter().map(|(p, u)| (p.as_deref(), u.as_str()));
            let parent = parent.as_deref().unwrap_or_default();
            let (scope, entries) = child_scope(parent, own, copied);
            (name, attributes, scope.into(), entries.into())
        };
        self.scopes.push(scope);
        self.builder
            .start_element_with_namespaces(name, entries)
            .map_err(|e| tree_error(&e))?;
        for (name, value) in attributes {
            self.builder
                .attribute(name, &value)
                .map_err(|e| tree_error(&e))?;
        }
        Ok(())
    }

    /// The scope and the namespace entries of an element whose own namespace nodes are
    /// `given`, from [`TreeSink::memo`] or worked out ([`child_scope`]) and kept. A scope
    /// equal to the parent's is the parent's.
    fn memoized_scope(
        &mut self,
        parent: Option<Scope>,
        given: Arc<[NamespaceBinding]>,
        copied: bool,
    ) -> (Scope, Arc<[NamespaceBinding]>) {
        let found = self.memo.iter().find(|m| {
            m.copied == copied
                && Arc::ptr_eq(&m.given, &given)
                && match (&m.parent, &parent) {
                    (None, None) => true,
                    (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                    _ => false,
                }
        });
        if let Some(m) = found {
            return (Rc::clone(&m.scope), Arc::clone(&m.entries));
        }
        let own = given.iter().map(|b| (b.prefix.as_deref(), b.uri.as_str()));
        let parent_bindings = parent.as_deref().unwrap_or_default();
        let (scope, entries) = child_scope(parent_bindings, own, copied);
        let scope: Scope = match &parent {
            Some(p) if **p == *scope => Rc::clone(p),
            _ => scope.into(),
        };
        let entries: Arc<[NamespaceBinding]> = entries.into();
        let memo = ScopeMemo {
            parent,
            given,
            copied,
            scope: Rc::clone(&scope),
            entries: Arc::clone(&entries),
        };
        if self.memo.len() == SCOPE_MEMO_SIZE {
            // The oldest goes.
            self.memo.remove(0);
        }
        self.memo.push(memo);
        (scope, entries)
    }

    /// The start tag being built, or the error for an attribute or a namespace node that
    /// comes too late.
    fn start_tag(&mut self, what: &str) -> Result<&mut StartTag> {
        match self.pending.as_mut() {
            Some((tag, _)) => Ok(tag),
            None => err(
                "XTDE0410",
                format!(
                    "{what} cannot be created after the children of the containing element, or at the top of a temporary tree"
                ),
            ),
        }
    }
}

fn tree_error(e: &einvoice_xpath_eval::DocumentError) -> TransformError {
    TransformError::new("INTERNAL", e.message())
}

impl Sink for TreeSink {
    fn start_element(&mut self, name: &QName, namespaces: &Arc<[NamespaceBinding]>) -> Result<()> {
        self.flush()?;
        self.last_atomic = false;
        self.pending = Some((StartTag::new(name, namespaces), false));
        Ok(())
    }

    fn start_copied_element(
        &mut self,
        name: &QName,
        namespaces: &Arc<[NamespaceBinding]>,
    ) -> Result<()> {
        self.flush()?;
        self.last_atomic = false;
        self.pending = Some((StartTag::new(name, namespaces), true));
        Ok(())
    }

    fn namespace(&mut self, prefix: Option<&str>, uri: &str) -> Result<()> {
        self.last_atomic = false;
        self.start_tag("a namespace node")?.namespace(prefix, uri)
    }

    fn attribute(&mut self, name: &QName, value: &str) -> Result<()> {
        self.last_atomic = false;
        if let Some((tag, _)) = self.pending.as_mut() {
            tag.attribute(name, value);
            return Ok(());
        }
        // The error, its message written only now.
        self.start_tag(&format!("an attribute node ({name})"))
            .map(|_| ())
    }

    fn end_element(&mut self) -> Result<()> {
        self.flush()?;
        self.last_atomic = false;
        self.scopes.pop();
        self.builder.end_element().map_err(|e| tree_error(&e))
    }

    fn text(&mut self, text: &str) -> Result<()> {
        self.flush()?;
        self.last_atomic = false;
        self.builder.text(text).map_err(|e| tree_error(&e))
    }

    fn comment(&mut self, text: &str) -> Result<()> {
        self.flush()?;
        self.last_atomic = false;
        self.builder.comment(text).map_err(|e| tree_error(&e))
    }

    fn processing_instruction(&mut self, target: &str, value: &str) -> Result<()> {
        self.flush()?;
        self.last_atomic = false;
        self.builder
            .processing_instruction(target, value)
            .map_err(|e| tree_error(&e))
    }

    fn start_document(&mut self) -> Result<()> {
        self.flush()?;
        self.last_atomic = false;
        Ok(())
    }

    fn end_document(&mut self) -> Result<()> {
        self.flush()?;
        self.last_atomic = false;
        Ok(())
    }

    fn item(&mut self, item: &Item) -> Result<()> {
        match item {
            Item::Atomic(_) => {
                let was_atomic = self.last_atomic;
                if was_atomic {
                    self.text(" ")?;
                }
                self.text(&item.string_value())?;
                self.last_atomic = true;
                Ok(())
            }
            Item::Node(node) => {
                self.flush()?;
                self.last_atomic = false;
                copy_node(self, node)
            }
        }
    }
}

/// Simple content (XSLT 2.0 section 5.7.2): zero-length text nodes dropped, adjacent
/// text nodes merged, every item atomized to a string, the strings joined with a
/// separator.
#[derive(Debug, Default)]
pub(crate) struct TextSink {
    parts: Vec<String>,
    /// Whether the last part is a text node that the next one can merge with.
    last_text: bool,
    /// Depth inside a constructed element or document node, whose string value is one
    /// part.
    depth: usize,
}

impl TextSink {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn push_part(&mut self, value: &str, is_text: bool) {
        if is_text && self.last_text {
            if let Some(last) = self.parts.last_mut() {
                last.push_str(value);
            }
        } else {
            self.parts.push(value.to_owned());
        }
        self.last_text = is_text;
    }

    /// The joined value.
    pub(crate) fn finish(self, separator: &str) -> String {
        if self.parts.len() == 1 {
            return self.parts.into_iter().next().unwrap_or_default();
        }
        self.parts.join(separator)
    }

    fn open(&mut self) {
        if self.depth == 0 {
            self.parts.push(String::new());
            self.last_text = false;
        }
        self.depth += 1;
    }
}

impl Sink for TextSink {
    fn start_element(
        &mut self,
        _name: &QName,
        _namespaces: &Arc<[NamespaceBinding]>,
    ) -> Result<()> {
        self.open();
        Ok(())
    }

    fn namespace(&mut self, _prefix: Option<&str>, uri: &str) -> Result<()> {
        if self.depth == 0 {
            self.push_part(uri, false);
        }
        Ok(())
    }

    fn attribute(&mut self, _name: &QName, value: &str) -> Result<()> {
        if self.depth == 0 {
            self.push_part(value, false);
        }
        Ok(())
    }

    fn end_element(&mut self) -> Result<()> {
        self.depth = self.depth.saturating_sub(1);
        Ok(())
    }

    fn text(&mut self, text: &str) -> Result<()> {
        if self.depth > 0 {
            if let Some(last) = self.parts.last_mut() {
                last.push_str(text);
            }
        } else if !text.is_empty() {
            self.push_part(text, true);
        }
        Ok(())
    }

    fn comment(&mut self, text: &str) -> Result<()> {
        if self.depth == 0 {
            self.push_part(text, false);
        }
        Ok(())
    }

    fn processing_instruction(&mut self, _target: &str, value: &str) -> Result<()> {
        if self.depth == 0 {
            self.push_part(value, false);
        }
        Ok(())
    }

    fn start_document(&mut self) -> Result<()> {
        self.open();
        Ok(())
    }

    fn end_document(&mut self) -> Result<()> {
        self.depth = self.depth.saturating_sub(1);
        Ok(())
    }

    fn item(&mut self, item: &Item) -> Result<()> {
        if self.depth > 0 {
            return err(
                "INTERNAL",
                "an item inside an element constructed in simple content",
            );
        }
        let is_text = matches!(item, Item::Node(n) if n.kind() == NodeKind::Text);
        let value = item.string_value();
        if is_text && value.is_empty() {
            return Ok(());
        }
        self.push_part(&value, is_text);
        Ok(())
    }
}

/// A sequence of items (function bodies, variables and parameters with `as`). A node
/// constructed at the top of the sequence is a new parentless node: an element or a
/// document node is built with its content, as complex content; each attribute, text
/// (each instruction's, not merged with the next, and kept when of zero length, as Saxon
/// does), comment and processing instruction is a node of its own. Items of
/// `xsl:sequence` are kept as they are (nodes are not copied).
#[derive(Debug, Default)]
pub(crate) struct SequenceSink {
    pub(crate) items: Vec<Item>,
    /// The element or document node being constructed at the top, and how many
    /// elements and document nodes are open in it (boxed: a function call holds a sink
    /// on the stack of each level of recursion).
    open: Option<Box<(TreeSink, usize)>>,
}

impl SequenceSink {
    /// The tree being constructed, if any.
    fn tree(&mut self) -> Option<&mut TreeSink> {
        self.open.as_mut().map(|e| &mut e.0)
    }

    /// A parentless node built by `build`.
    fn push_parentless(
        &mut self,
        build: impl FnOnce(
            &mut TreeBuilder,
        ) -> std::result::Result<(), einvoice_xpath_eval::DocumentError>,
    ) -> Result<()> {
        let mut builder = TreeBuilder::parentless();
        build(&mut builder).map_err(|e| tree_error(&e))?;
        let tree = builder
            .finish(None, None)
            .map_err(|e| TransformError::new("INTERNAL", e.message()))?;
        self.items.push(Item::Node(tree.root()));
        Ok(())
    }

    /// One construction closed: the tree is an item once the outermost one is.
    fn close(&mut self) -> Result<()> {
        let Some(open) = self.open.as_mut() else {
            return err("INTERNAL", "end of a node that is not open");
        };
        open.1 -= 1;
        if open.1 == 0
            && let Some(open) = self.open.take()
        {
            self.items.push(Item::Node(open.0.finish()?.root()));
        }
        Ok(())
    }
}

impl Sink for SequenceSink {
    fn start_element(&mut self, name: &QName, namespaces: &Arc<[NamespaceBinding]>) -> Result<()> {
        let open = self
            .open
            .get_or_insert_with(|| Box::new((TreeSink::parentless(), 0)));
        open.1 += 1;
        open.0.start_element(name, namespaces)
    }

    fn start_copied_element(
        &mut self,
        name: &QName,
        namespaces: &Arc<[NamespaceBinding]>,
    ) -> Result<()> {
        let open = self
            .open
            .get_or_insert_with(|| Box::new((TreeSink::parentless(), 0)));
        open.1 += 1;
        open.0.start_copied_element(name, namespaces)
    }

    fn namespace(&mut self, prefix: Option<&str>, uri: &str) -> Result<()> {
        if let Some(tree) = self.tree() {
            return tree.namespace(prefix, uri);
        }
        err(
            "UNSUPPORTED",
            "a parentless namespace node constructed in a sequence is not supported",
        )
    }

    fn attribute(&mut self, name: &QName, value: &str) -> Result<()> {
        if let Some(tree) = self.tree() {
            return tree.attribute(name, value);
        }
        self.push_parentless(|b| b.attribute(name.clone(), value))
    }

    fn end_element(&mut self) -> Result<()> {
        let Some(open) = self.open.as_mut() else {
            return err("INTERNAL", "end of an element that is not open");
        };
        open.0.end_element()?;
        self.close()
    }

    fn text(&mut self, text: &str) -> Result<()> {
        if let Some(tree) = self.tree() {
            return tree.text(text);
        }
        self.push_parentless(|b| b.text(text))
    }

    fn comment(&mut self, text: &str) -> Result<()> {
        if let Some(tree) = self.tree() {
            return tree.comment(text);
        }
        self.push_parentless(|b| b.comment(text))
    }

    fn processing_instruction(&mut self, target: &str, value: &str) -> Result<()> {
        if let Some(tree) = self.tree() {
            return tree.processing_instruction(target, value);
        }
        self.push_parentless(|b| b.processing_instruction(target, value))
    }

    fn start_document(&mut self) -> Result<()> {
        let open = self
            .open
            .get_or_insert_with(|| Box::new((TreeSink::new(), 0)));
        open.1 += 1;
        open.0.start_document()
    }

    fn end_document(&mut self) -> Result<()> {
        let Some(open) = self.open.as_mut() else {
            return err("INTERNAL", "end of a document node that is not open");
        };
        open.0.end_document()?;
        self.close()
    }

    fn item(&mut self, item: &Item) -> Result<()> {
        if let Some(tree) = self.tree() {
            return tree.item(item);
        }
        self.items.push(item.clone());
        Ok(())
    }
}

/// Whether the name of a computed attribute or element is a lexical `QName`: its prefix
/// and local part.
fn lexical_qname(text: &str) -> Option<(Option<&str>, &str)> {
    let text = text.trim_matches(einvoice_xpath::is_xml_whitespace);
    let (prefix, local) = match text.split_once(':') {
        Some((p, l)) => (Some(p), l),
        None => (None, text),
    };
    (einvoice_xpath::is_ncname(local) && prefix.is_none_or(einvoice_xpath::is_ncname))
        .then_some((prefix, local))
}

/// The name of a computed attribute (`xsl:attribute`): a lexical `QName`, in the namespace
/// of `namespace` when given (a prefix of the name is kept; an empty namespace means no
/// namespace and no prefix), else in the namespace its prefix has among the namespaces
/// in scope on the instruction (no namespace without prefix).
pub(crate) fn attribute_name(
    text: &str,
    namespace: Option<&str>,
    namespaces: &[NamespaceBinding],
) -> Result<QName> {
    let Some((prefix, local)) = lexical_qname(text) else {
        return err(
            "XTDE0850",
            format!("invalid attribute name {text:?} in xsl:attribute"),
        );
    };
    if prefix.is_none() && local == "xmlns" {
        return err("XTDE0855", "an attribute cannot be named xmlns");
    }
    if let Some(uri) = namespace {
        if uri.is_empty() {
            return Ok(QName::local(local));
        }
        let Some(prefix) = prefix else {
            return err(
                "UNSUPPORTED",
                format!(
                    "the prefix Saxon chooses for an attribute {local:?} in the namespace {uri:?} given without one"
                ),
            );
        };
        return Ok(QName {
            prefix: Some(prefix.to_owned()),
            namespace: Some(uri.to_owned()),
            local: local.to_owned(),
        });
    }
    let namespace = match prefix {
        None => None,
        Some("xml") => Some(XML_NAMESPACE.to_owned()),
        Some(p) => Some(
            namespaces
                .iter()
                .find(|ns| ns.prefix.as_deref() == Some(p))
                .map(|ns| ns.uri.clone())
                .ok_or_else(|| {
                    TransformError::new(
                        "XTDE0860",
                        format!("prefix {p:?} of attribute name {text:?} is not declared"),
                    )
                })?,
        ),
    };
    Ok(QName {
        prefix: prefix.map(str::to_owned),
        namespace,
        local: local.to_owned(),
    })
}

/// The name of a computed element (`xsl:element`): a lexical `QName`, in the namespace of
/// `namespace` when given (a prefix of the name is kept; an empty namespace means no
/// namespace and no prefix), else in the namespace its prefix, or the default namespace
/// without one, has among the namespaces in scope on the instruction.
pub(crate) fn element_name(
    text: &str,
    namespace: Option<&str>,
    namespaces: &[NamespaceBinding],
) -> Result<QName> {
    let Some((prefix, local)) = lexical_qname(text) else {
        return err(
            "XTDE0820",
            format!("invalid element name {text:?} in xsl:element"),
        );
    };
    if let Some(uri) = namespace {
        return Ok(QName {
            prefix: prefix.filter(|_| !uri.is_empty()).map(str::to_owned),
            namespace: (!uri.is_empty()).then(|| uri.to_owned()),
            local: local.to_owned(),
        });
    }
    let namespace = match prefix {
        Some("xml") => Some(XML_NAMESPACE.to_owned()),
        _ => namespaces
            .iter()
            .find(|ns| ns.prefix.as_deref() == prefix)
            .map(|ns| ns.uri.clone())
            .filter(|uri| !uri.is_empty()),
    };
    if prefix.is_some() && namespace.is_none() {
        return err(
            "XTDE0830",
            format!("the prefix of element name {text:?} is not declared"),
        );
    }
    Ok(QName {
        prefix: prefix.map(str::to_owned),
        namespace,
        local: local.to_owned(),
    })
}
