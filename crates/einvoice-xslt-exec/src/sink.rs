//! Where the result of a sequence constructor goes (XSLT 2.0 section 5.7).
//!
//! Instructions push events into a [`Sink`]; the instruction that owns the sequence
//! constructor chooses the sink, which gives the result its meaning:
//!
//! - complex content (the final result, written straight to XML by [`XmlWriter`]; a
//!   temporary tree, built by [`TreeSink`]): adjacent atomic values are separated by a
//!   space, nodes are copied;
//! - simple content (attribute values, comments, `xsl:value-of`): [`TextSink`]
//!   concatenates the string values, adjacent text merged, with a separator between the
//!   other items;
//! - a sequence (`xsl:function` bodies, variables and parameters with `as`):
//!   [`SequenceSink`] keeps the items, and each node an instruction constructs at its top
//!   is a new parentless node (an element with its content, an attribute, a text node,
//!   even a zero-length one, or a comment), as Saxon-HE 12.10 constructs them.

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
    /// Opens an element with namespace nodes: those of a literal result element, or of a
    /// copied element.
    fn start_element(&mut self, name: &QName, namespaces: &Arc<[NamespaceBinding]>) -> Result<()>;
    /// Adds an attribute to the element just opened.
    fn attribute(&mut self, name: &QName, value: &str) -> Result<()>;
    /// Closes the innermost open element.
    fn end_element(&mut self) -> Result<()>;
    /// A text node (possibly empty: it still separates atomic values).
    fn text(&mut self, text: &str) -> Result<()>;
    /// A comment.
    fn comment(&mut self, text: &str) -> Result<()>;
    /// An item of `xsl:sequence`: an atomic value or an existing node.
    fn item(&mut self, item: &Item) -> Result<()>;
}

/// Copies an existing node into complex content (deep copy, as XSLT 2.0 section 5.7.1
/// prescribes for nodes in a sequence constructor), each element with its namespace
/// nodes (`copy-namespaces="yes"`; the elements of a parsed document have none, see
/// [`Node::namespaces_in_scope`]). The subtree is walked with an explicit stack, not a
/// recursion: its depth is only bounded by the document's.
fn copy_node(sink: &mut dyn Sink, node: &Node) -> Result<()> {
    // The document node or elements being copied, innermost last, each with its children
    // not copied yet, and whether it is an element (to close).
    let mut open: Vec<(Axis, bool)> = Vec::new();
    let mut next = Some(node.clone());
    loop {
        if let Some(node) = next.take() {
            match node.kind() {
                NodeKind::Document => open.push((node.children(), false)),
                NodeKind::Element => {
                    let Some(name) = node.name() else {
                        return err("INTERNAL", "element without name");
                    };
                    let namespaces: Arc<[NamespaceBinding]> = node.namespaces_in_scope().into();
                    sink.start_element(name, &namespaces)?;
                    for attribute in node.attributes() {
                        copy_attribute(sink, &attribute)?;
                    }
                    open.push((node.children(), true));
                }
                NodeKind::Attribute => copy_attribute(sink, &node)?,
                NodeKind::Text => sink.text(&node.string_value())?,
                NodeKind::Comment => sink.comment(&node.string_value())?,
                NodeKind::ProcessingInstruction => {
                    return err(
                        "UNSUPPORTED",
                        "copying a processing instruction to the result is not supported",
                    );
                }
            }
        }
        let Some((children, _)) = open.last_mut() else {
            return Ok(());
        };
        match children.next() {
            Some(child) => next = Some(child),
            None => {
                if let Some((_, true)) = open.pop() {
                    sink.end_element()?;
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

/// Serialization parameters (`xsl:output`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Serialization {
    pub(crate) indent: bool,
    pub(crate) omit_xml_declaration: bool,
    /// `Some(true)`: `standalone="yes"`, `Some(false)`: `"no"`, `None`: omitted.
    pub(crate) standalone: Option<bool>,
}

/// An element whose start tag is not written yet: attributes may still come.
#[derive(Debug)]
struct Pending {
    name: QName,
    attributes: Vec<(QName, String)>,
    /// Namespace declarations to write: prefix (`None`: default) and URI.
    declarations: Vec<(Option<String>, String)>,
}

/// One open element of the result.
#[derive(Debug)]
struct Open {
    /// The written qualified name, for the end tag.
    tag: String,
    /// How many entries of the namespace scope this element added.
    bindings: usize,
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
}

/// Writes the final result tree as XML text as it is produced, without building it,
/// into its buffer, which the target keeps whole or takes a piece at a time: after an
/// end tag or an empty element, and after a start tag that content follows.
#[derive(Debug)]
pub(crate) struct XmlWriter<T> {
    out: String,
    target: T,
    settings: Serialization,
    pending: Option<Pending>,
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
            pending: None,
            open: Vec::new(),
            scope: Vec::new(),
            last_atomic: false,
        }
    }

    /// The text not taken by the target (all of it for [`WholeText`]), and the target.
    pub(crate) fn finish(mut self) -> Result<(String, T)> {
        self.flush();
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

    /// Writes the pending start tag.
    fn flush(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let mut declarations = pending.declarations;
        let mut attributes = pending.attributes;
        let mut tag = String::new();
        let bindings_before = self.scope.len();
        for (prefix, uri) in &declarations {
            self.scope.push((prefix.clone(), uri.clone()));
        }
        // Namespace fixup: the element's own name, then the attributes'.
        self.fix_namespace(&pending.name, bindings_before, &mut declarations);
        for (name, _) in &mut attributes {
            self.fix_attribute_namespace(name, bindings_before, &mut declarations);
        }
        if let Some(prefix) = &pending.name.prefix {
            tag.push_str(prefix);
            tag.push(':');
        }
        tag.push_str(&pending.name.local);
        self.out.push('<');
        self.out.push_str(&tag);
        for (prefix, uri) in &declarations {
            self.out.push_str(" xmlns");
            if let Some(prefix) = prefix {
                self.out.push(':');
                self.out.push_str(prefix);
            }
            self.out.push_str("=\"");
            escape_attribute(&mut self.out, uri);
            self.out.push('"');
        }
        for (name, value) in &attributes {
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
        self.target.start_element(&pending.name, &attributes);
        self.open.push(Open {
            tag,
            bindings: self.scope.len() - bindings_before,
            has_elements: false,
            has_text: false,
        });
    }

    /// Writes the pending start tag, if any, as a piece of its own: content follows, the
    /// element is not empty.
    fn flush_tag(&mut self) -> Result<()> {
        if self.pending.is_some() {
            self.flush();
            self.target.commit(&mut self.out)?;
        }
        Ok(())
    }

    /// Declares the namespace of an element name if its prefix is not bound to it in
    /// scope (the element's own declarations start at `element_start` in the scope).
    fn fix_namespace(
        &mut self,
        name: &QName,
        element_start: usize,
        declarations: &mut Vec<(Option<String>, String)>,
    ) {
        let uri = name.namespace.as_deref().unwrap_or("");
        let prefix = name.prefix.as_deref();
        if prefix == Some("xml") || self.bound(prefix).unwrap_or("") == uri {
            return;
        }
        // A conflicting declaration of the element itself is replaced: the name wins.
        if let Some(i) =
            (element_start..self.scope.len()).find(|&i| self.scope[i].0.as_deref() == prefix)
        {
            uri.clone_into(&mut self.scope[i].1);
            if let Some(d) = declarations
                .iter_mut()
                .find(|(p, _)| p.as_deref() == prefix)
            {
                uri.clone_into(&mut d.1);
            }
            return;
        }
        declarations.push((prefix.map(str::to_owned), uri.to_owned()));
        self.scope.push((prefix.map(str::to_owned), uri.to_owned()));
    }

    /// Declares the namespace of an attribute name. When its prefix is already declared
    /// on the element for another namespace, the attribute gets a new prefix
    /// (`prefix_1`, ...), as Saxon does.
    fn fix_attribute_namespace(
        &mut self,
        name: &mut QName,
        element_start: usize,
        declarations: &mut Vec<(Option<String>, String)>,
    ) {
        let Some(namespace) = name.namespace.clone() else {
            return;
        };
        let prefix = name.prefix.clone().unwrap_or_default();
        if prefix == "xml" || self.bound(Some(&prefix)) == Some(namespace.as_str()) {
            return;
        }
        let taken_here = |scope: &[(Option<String>, String)], p: &str| {
            scope[element_start..]
                .iter()
                .any(|(q, _)| q.as_deref() == Some(p))
        };
        let mut chosen = prefix.clone();
        if chosen.is_empty() || taken_here(&self.scope, &chosen) {
            let mut n = 1;
            loop {
                chosen = format!("{prefix}_{n}");
                match self.bound(Some(&chosen)) {
                    Some(uri) if uri == namespace => break,
                    None => break,
                    Some(_) => n += 1,
                }
            }
            name.prefix = Some(chosen.clone());
            if self.bound(Some(&chosen)) == Some(namespace.as_str()) {
                return;
            }
        }
        declarations.push((Some(chosen.clone()), namespace.clone()));
        self.scope.push((Some(chosen), namespace));
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

    /// Before an element or comment: indentation, unless in mixed content.
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

impl<T: Target> Sink for XmlWriter<T> {
    fn start_element(&mut self, name: &QName, namespaces: &Arc<[NamespaceBinding]>) -> Result<()> {
        self.flush_tag()?;
        self.last_atomic = false;
        self.before_markup();
        let mut declarations = Vec::new();
        for ns in namespaces.iter() {
            let already = self.bound(ns.prefix.as_deref()).unwrap_or("") == ns.uri;
            if !already {
                declarations.push((ns.prefix.clone(), ns.uri.clone()));
            }
        }
        self.pending = Some(Pending {
            name: name.clone(),
            attributes: Vec::new(),
            declarations,
        });
        Ok(())
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
        if let Some(existing) = pending
            .attributes
            .iter_mut()
            .find(|(n, _)| n.same_expanded_name(name))
        {
            existing.0 = name.clone();
            value.clone_into(&mut existing.1);
        } else {
            pending.attributes.push((name.clone(), value.to_owned()));
        }
        Ok(())
    }

    fn end_element(&mut self) -> Result<()> {
        self.last_atomic = false;
        if let Some(pending) = self.pending.take() {
            // An empty element: written as `<name .../>`.
            self.pending = Some(pending);
            self.flush();
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

fn escape_text(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '\r' => out.push_str("&#xD;"),
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
            c => out.push(c),
        }
    }
}

/// Builds a tree: a temporary tree (a variable whose value is a sequence constructor,
/// without `as`), a new document node; or the content of an element constructed in a
/// sequence, a parentless element.
#[derive(Debug)]
pub(crate) struct TreeSink {
    builder: TreeBuilder,
    last_atomic: bool,
    /// Whether the element just opened may still receive attributes.
    in_start_tag: bool,
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
            in_start_tag: false,
        }
    }

    pub(crate) fn finish(self) -> Result<Rc<Document>> {
        self.builder
            .finish(None, None)
            .map_err(|e| TransformError::new("INTERNAL", e.message()))
    }
}

fn tree_error(e: &einvoice_xpath_eval::DocumentError) -> TransformError {
    TransformError::new("INTERNAL", e.message())
}

impl Sink for TreeSink {
    fn start_element(&mut self, name: &QName, namespaces: &Arc<[NamespaceBinding]>) -> Result<()> {
        self.last_atomic = false;
        self.in_start_tag = true;
        self.builder
            .start_element_with_namespaces(name.clone(), Arc::clone(namespaces))
            .map_err(|e| tree_error(&e))
    }

    fn attribute(&mut self, name: &QName, value: &str) -> Result<()> {
        self.last_atomic = false;
        if !self.in_start_tag {
            return err(
                "XTDE0410",
                format!(
                    "an attribute node ({name}) cannot be created after the children of the containing element, or at the top of a temporary tree"
                ),
            );
        }
        self.builder
            .attribute(name.clone(), value)
            .map_err(|e| tree_error(&e))
    }

    fn end_element(&mut self) -> Result<()> {
        self.last_atomic = false;
        self.in_start_tag = false;
        self.builder.end_element().map_err(|e| tree_error(&e))
    }

    fn text(&mut self, text: &str) -> Result<()> {
        self.last_atomic = false;
        self.in_start_tag = false;
        self.builder.text(text).map_err(|e| tree_error(&e))
    }

    fn comment(&mut self, text: &str) -> Result<()> {
        self.last_atomic = false;
        self.in_start_tag = false;
        self.builder.comment(text).map_err(|e| tree_error(&e))
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
    /// Depth inside a constructed element, whose string value is one part.
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
}

impl Sink for TextSink {
    fn start_element(
        &mut self,
        _name: &QName,
        _namespaces: &Arc<[NamespaceBinding]>,
    ) -> Result<()> {
        if self.depth == 0 {
            self.parts.push(String::new());
            self.last_text = false;
        }
        self.depth += 1;
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
/// constructed at the top of the sequence is a new parentless node: an element is built
/// with its content, as complex content; each attribute, text (each instruction's, not
/// merged with the next, and kept when of zero length, as Saxon does) and comment is a
/// node of its own. Items of `xsl:sequence` are kept as they are (nodes are not copied).
#[derive(Debug, Default)]
pub(crate) struct SequenceSink {
    pub(crate) items: Vec<Item>,
    /// The element being constructed at the top, and how many elements are open in it
    /// (boxed: a function call holds a sink on the stack of each level of recursion).
    element: Option<Box<(TreeSink, usize)>>,
}

impl SequenceSink {
    /// The tree of the element being constructed, if any.
    fn open(&mut self) -> Option<&mut TreeSink> {
        self.element.as_mut().map(|e| &mut e.0)
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
}

impl Sink for SequenceSink {
    fn start_element(&mut self, name: &QName, namespaces: &Arc<[NamespaceBinding]>) -> Result<()> {
        let element = self
            .element
            .get_or_insert_with(|| Box::new((TreeSink::parentless(), 0)));
        element.1 += 1;
        element.0.start_element(name, namespaces)
    }

    fn attribute(&mut self, name: &QName, value: &str) -> Result<()> {
        if let Some(tree) = self.open() {
            return tree.attribute(name, value);
        }
        self.push_parentless(|b| b.attribute(name.clone(), value))
    }

    fn end_element(&mut self) -> Result<()> {
        let Some(element) = self.element.as_mut() else {
            return err("INTERNAL", "end of an element that is not open");
        };
        element.0.end_element()?;
        element.1 -= 1;
        if element.1 == 0
            && let Some(element) = self.element.take()
        {
            self.items.push(Item::Node(element.0.finish()?.root()));
        }
        Ok(())
    }

    fn text(&mut self, text: &str) -> Result<()> {
        if let Some(tree) = self.open() {
            return tree.text(text);
        }
        self.push_parentless(|b| b.text(text))
    }

    fn comment(&mut self, text: &str) -> Result<()> {
        if let Some(tree) = self.open() {
            return tree.comment(text);
        }
        self.push_parentless(|b| b.comment(text))
    }

    fn item(&mut self, item: &Item) -> Result<()> {
        if let Some(tree) = self.open() {
            return tree.item(item);
        }
        self.items.push(item.clone());
        Ok(())
    }
}

/// The name of a computed attribute (`xsl:attribute/@name`): a lexical `QName` whose
/// prefix is resolved with the namespaces in scope on the instruction.
pub(crate) fn attribute_name(text: &str, namespaces: &[NamespaceBinding]) -> Result<QName> {
    let invalid = || {
        TransformError::new(
            "XTDE0850",
            format!("invalid attribute name {text:?} in xsl:attribute"),
        )
    };
    let (prefix, local) = match text.split_once(':') {
        Some((p, l)) => (Some(p), l),
        None => (None, text),
    };
    if !einvoice_xpath::is_ncname(local) || prefix.is_some_and(|p| !einvoice_xpath::is_ncname(p)) {
        return Err(invalid());
    }
    if prefix.is_none() && local == "xmlns" {
        return err("XTDE0855", "an attribute cannot be named xmlns");
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
