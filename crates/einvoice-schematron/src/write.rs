//! The XML writer of ph-commons (ph-xml 12.5.3: `XMLWriter.writeToStream`,
//! `XMLSerializer`, `AbstractXMLSerializer`, `XMLEmitter`, `XMLMaskHelper`,
//! `XMLCharHelper`), with the settings `Schematron2XSLTMojo` of
//! ph-schematron-maven-plugin 10.3.1 gives it, on a DOM document.
//!
//! The settings: the defaults of `XMLWriterSettings` (XML 1.0, UTF-8, the XML
//! declaration with `standalone`, indentation and alignment with two spaces, double
//! quotes, a space before `/>`, namespaces emitted, attributes in their order, the
//! namespace declarations of the DOM not used, characters invalid in XML 1.0 dropped),
//! a namespace context and `putNamespaceContextPrefixesInRoot`. The namespace context:
//! the prefix `svrl` for the SVRL namespace unless the document element declares `svrl`,
//! then each `xmlns:p` attribute of the document element, in its order. The line
//! separator is the platform's in ph-commons (`ENewLineMode.DEFAULT`); this writer uses a
//! line feed, as ph-commons on Linux and macOS.
//!
//! How the bytes follow from the DOM:
//!
//! - `<?xml version="1.0" encoding="UTF-8" standalone="no"?>` and a line end (the DOM
//!   document's `xmlStandalone` is false), then the children of the document;
//! - an element is written with the prefix the writer finds for its namespace, not its
//!   own: the innermost prefix bound to that namespace on the stack of the writer
//!   (where the document element binds the whole namespace context, a later prefix of
//!   the context for a namespace replacing an earlier one); if none, no prefix for the
//!   document element, else `ns0`, `ns1`, ..., the first that no level of the stack
//!   binds, declared on the element. An element in
//!   no namespace undeclares the default namespace if one is bound. The same for an
//!   attribute in a namespace (never with the default namespace), the `xml` namespace
//!   having the prefix `xml`, never declared;
//! - the attributes of an element: first the namespace context (document element only),
//!   then the declaration of the element's prefix if it needs one, then the DOM's
//!   attributes in their order (namespace declarations left out), each after the
//!   declaration of its prefix if it needs one;
//! - indentation: before an element, its indentation (two spaces per level, none at the
//!   first level) unless its previous sibling is text; after its start tag, a line end
//!   when its first child is not text; before its end tag, the indentation when its first
//!   child is not text (none at the first level); after the element, a line end unless
//!   its next sibling is text. Before a comment, the indentation unless its previous
//!   sibling is text; after it, a line end unless a sibling next to it is text. After a
//!   processing instruction, always a line end. Text is written as it is;
//! - an element without children is written `<name ... />`;
//! - in text, `&` `<` `>` are written as entity references, a carriage return and the
//!   characters U+007F to U+009F as `&#xD;`, `&#x7F;`...; in an attribute value, `&` `<`
//!   `"` as entity references, a tab, a line feed and a carriage return as `&#x9;`,
//!   `&#xA;`, `&#xD;` (`>` as it is). Comments and processing instructions are written as
//!   they are; a comment containing `<!--` or `-->`, a processing instruction containing
//!   `?>`, are errors. A CDATA section is written as one, split around `]]>`;
//! - characters invalid in XML 1.0 (U+0000 to U+0008, U+000B, U+000C, U+000E to U+001F,
//!   U+FFFE, U+FFFF) are dropped from text, CDATA sections and attribute values; in a
//!   name, an element prefix or an attribute's local name, every occurrence of a
//!   character that is invalid where it first appears invalid (as the first character, or
//!   later in the name) is dropped, by ph-commons' tables of the name characters of XML
//!   1.0 before its fifth edition; an attribute prefix is written as it is. An empty
//!   comment is not written.

use std::fmt::Write as _;

use crate::dom::{Document, Element, Node, XML_NAMESPACE, XMLNS_NAMESPACE};
use crate::tables::{INVALID_NAME, INVALID_NAME_START, INVALID_VALUE, contains};
use crate::{Error, MAX_DEPTH};

/// The SVRL namespace, whose prefix `svrl` heads the namespace context.
pub(crate) const SVRL_NAMESPACE: &str = "http://purl.oclc.org/dsdl/svrl";

const INDENT: &str = "  ";
const NEW_LINE: &str = "\n";

/// Writes `document` as `Schematron2XSLTMojo` writes the compiled stylesheet.
pub(crate) fn write(document: &Document) -> Result<String, Error> {
    let mut writer = Writer {
        out: String::new(),
        indent: String::new(),
        stack: Vec::new(),
        context: namespace_context(document),
    };
    writer
        .out
        .push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>");
    writer.out.push_str(NEW_LINE);
    let root = document
        .children
        .iter()
        .position(|n| matches!(n, Node::Element(_)));
    writer.write_children(&document.children, root, 0)?;
    Ok(writer.out)
}

/// The namespace context of `Schematron2XSLTMojo`, (prefix, URI) in order.
fn namespace_context(document: &Document) -> Vec<(String, String)> {
    let mut from_root: Vec<(String, String)> = Vec::new();
    if let Some(root) = document.document_element() {
        for attribute in root.attributes() {
            // XMLHelper.forAllAttributes with the qualified name: "xmlns:" prefixed.
            if let Some(prefix) = attribute.name.strip_prefix("xmlns:") {
                // A LinkedHashMap: a repeated key keeps its place, takes the value.
                match from_root.iter_mut().find(|(p, _)| p == prefix) {
                    Some(entry) => attribute.value.clone_into(&mut entry.1),
                    None => from_root.push((prefix.to_owned(), attribute.value.clone())),
                }
            }
        }
    }
    let mut context = Vec::with_capacity(from_root.len() + 1);
    if !from_root.iter().any(|(p, _)| p == "svrl") {
        context.push(("svrl".to_owned(), SVRL_NAMESPACE.to_owned()));
    }
    context.extend(from_root);
    context
}

/// One level of ph-commons' `NamespaceStack`: the default namespace bound there, and
/// namespace URI to prefix.
#[derive(Default)]
struct Level {
    default: Option<String>,
    prefixes: Vec<(String, String)>,
}

impl Level {
    fn namespace_of_prefix(&self, prefix: &str) -> Option<&str> {
        if prefix.is_empty() {
            return self.default.as_deref();
        }
        self.prefixes
            .iter()
            .find(|(_, p)| p == prefix)
            .map(|(uri, _)| uri.as_str())
    }

    fn prefix_of_namespace(&self, uri: &str) -> Option<&str> {
        if self.default.as_deref() == Some(uri) {
            return None;
        }
        self.prefixes
            .iter()
            .find(|(u, _)| u == uri)
            .map(|(_, p)| p.as_str())
    }

    fn has_any_namespace(&self) -> bool {
        self.default.is_some() || !self.prefixes.is_empty()
    }

    fn add(&mut self, prefix: Option<&str>, uri: &str) {
        match prefix {
            None | Some("") => self.default = Some(uri.to_owned()),
            Some(prefix) => match self.prefixes.iter_mut().find(|(u, _)| u == uri) {
                Some(entry) => prefix.clone_into(&mut entry.1),
                None => self.prefixes.push((uri.to_owned(), prefix.to_owned())),
            },
        }
    }
}

/// The attributes of a start tag, in order: `javax.xml.namespace.QName` keys (namespace
/// and local part; the prefix is not part of the key) in a `LinkedHashMap`.
#[derive(Default)]
struct Attributes {
    entries: Vec<Entry>,
}

struct Entry {
    namespace: String,
    local: String,
    prefix: String,
    value: String,
}

impl Attributes {
    fn put(&mut self, namespace: &str, local: &str, prefix: &str, value: &str) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.namespace == namespace && e.local == local)
        {
            value.clone_into(&mut entry.value);
            return;
        }
        self.entries.push(Entry {
            namespace: namespace.to_owned(),
            local: local.to_owned(),
            prefix: prefix.to_owned(),
            value: value.to_owned(),
        });
    }

    /// `XMLHelper.getXMLNSAttrQName`: `xmlns:p`, or `xmlns` for no prefix.
    fn put_declaration(&mut self, prefix: Option<&str>, uri: &str) {
        match prefix {
            None | Some("") => self.put(XMLNS_NAMESPACE, "xmlns", "", uri),
            Some(prefix) => self.put(XMLNS_NAMESPACE, prefix, "xmlns", uri),
        }
    }
}

struct Writer {
    out: String,
    indent: String,
    /// The levels of the namespace stack, innermost last.
    stack: Vec<Level>,
    context: Vec<(String, String)>,
}

impl Writer {
    fn write_children(
        &mut self,
        children: &[Node],
        root: Option<usize>,
        depth: usize,
    ) -> Result<(), Error> {
        for (index, node) in children.iter().enumerate() {
            let previous = index.checked_sub(1).and_then(|i| children.get(i));
            let next = children.get(index + 1);
            match node {
                Node::Element(element) => {
                    self.write_element(element, previous, next, root == Some(index), depth)?;
                }
                Node::Text(data) => mask_into(&mut self.out, data, Mode::Text),
                Node::CData(data) => self.write_cdata(data),
                Node::Comment(data) => self.write_comment(data, previous, next)?,
                Node::ProcessingInstruction { target, data } => {
                    if target.contains("?>") || data.contains("?>") {
                        return Err(Error::PiEndMarker(target.clone()));
                    }
                    self.out.push_str("<?");
                    self.out.push_str(target);
                    if !data.is_empty() {
                        self.out.push(' ');
                        self.out.push_str(data);
                    }
                    self.out.push_str("?>");
                    self.out.push_str(NEW_LINE);
                }
            }
        }
        Ok(())
    }

    fn write_comment(
        &mut self,
        data: &str,
        previous: Option<&Node>,
        next: Option<&Node>,
    ) -> Result<(), Error> {
        let indent_previous = previous.is_none_or(|n| !n.is_inline());
        let indent_next = next.is_none_or(|n| !n.is_inline());
        if !self.indent.is_empty() && indent_previous {
            self.out.push_str(&self.indent);
        }
        if !data.is_empty() {
            if data.contains("<!--") || data.contains("-->") {
                return Err(Error::NestedComment(data.to_owned()));
            }
            self.out.push_str("<!--");
            self.out.push_str(data);
            self.out.push_str("-->");
        }
        if indent_previous && indent_next {
            self.out.push_str(NEW_LINE);
        }
        Ok(())
    }

    fn write_cdata(&mut self, data: &str) {
        if data.is_empty() {
            return;
        }
        let parts: Vec<&str> = data.split("]]>").collect();
        let last = parts.len() - 1;
        for (i, part) in parts.iter().enumerate() {
            self.out.push_str("<![CDATA[");
            if i > 0 {
                self.out.push('>');
            }
            mask_into(&mut self.out, part, Mode::CData);
            if i < last {
                self.out.push_str("]]");
            }
            self.out.push_str("]]>");
        }
    }

    fn write_element(
        &mut self,
        element: &Element,
        previous: Option<&Node>,
        next: Option<&Node>,
        is_root: bool,
        depth: usize,
    ) -> Result<(), Error> {
        if depth >= MAX_DEPTH {
            return Err(Error::TooDeep);
        }
        let tag = element.local_name();
        let has_children = !element.children.is_empty();
        let indent_previous = previous.is_none_or(|n| !n.is_inline()) || is_root;
        let indent_next = next.is_none_or(|n| !n.is_inline());
        let first_child_element = element.children.first().is_some_and(|n| !n.is_inline());

        let mut attributes = Attributes::default();
        self.stack.push(Level::default());
        // handlePutNamespaceContextPrefixInRoot: the root's level is the only one.
        if self.stack.len() == 1
            && let Some(level) = self.stack.last_mut()
        {
            for (prefix, uri) in &self.context {
                attributes.put_declaration(Some(prefix), uri);
                level.add(Some(prefix), uri);
            }
        }
        let prefix =
            self.element_prefix(element.namespace().unwrap_or(""), is_root, &mut attributes);
        for attribute in element.attributes() {
            let namespace = attribute.namespace.as_deref().unwrap_or("");
            if namespace == XMLNS_NAMESPACE {
                continue;
            }
            let local = attribute.local_name();
            let prefix = self.attribute_prefix(namespace, local, &attribute.value, &mut attributes);
            attributes.put(
                namespace,
                local,
                prefix.as_deref().unwrap_or(""),
                &attribute.value,
            );
        }

        if !self.indent.is_empty() && indent_previous {
            self.out.push_str(&self.indent);
        }
        self.out.push('<');
        if let Some(prefix) = prefix.as_deref().filter(|p| !p.is_empty()) {
            mask_into(&mut self.out, prefix, Mode::Name);
            self.out.push(':');
        }
        mask_into(&mut self.out, tag, Mode::Name);
        for entry in &attributes.entries {
            self.out.push(' ');
            if !entry.prefix.is_empty() {
                self.out.push_str(&entry.prefix);
                self.out.push(':');
            }
            mask_into(&mut self.out, &entry.local, Mode::Name);
            self.out.push_str("=\"");
            mask_into(&mut self.out, &entry.value, Mode::AttributeValue);
            self.out.push('"');
        }
        if has_children {
            self.out.push('>');
            if first_child_element {
                self.out.push_str(NEW_LINE);
            }
            self.indent.push_str(INDENT);
            let result = self.write_children(&element.children, None, depth + 1);
            self.indent.truncate(self.indent.len() - INDENT.len());
            result?;
            if !self.indent.is_empty() && first_child_element {
                self.out.push_str(&self.indent);
            }
            self.out.push_str("</");
            if let Some(prefix) = prefix.as_deref().filter(|p| !p.is_empty()) {
                mask_into(&mut self.out, prefix, Mode::Name);
                self.out.push(':');
            }
            mask_into(&mut self.out, tag, Mode::Name);
            self.out.push('>');
        } else {
            self.out.push_str(" />");
        }
        if indent_next {
            self.out.push_str(NEW_LINE);
        }
        self.stack.pop();
        Ok(())
    }

    fn add_mapping(&mut self, prefix: Option<&str>, uri: &str) {
        if let Some(level) = self.stack.last_mut() {
            level.add(prefix, uri);
        }
    }

    fn default_namespace(&self) -> Option<&str> {
        self.stack.iter().rev().find_map(|l| l.default.as_deref())
    }

    fn used_prefix(&self, uri: &str) -> Option<String> {
        self.stack
            .iter()
            .rev()
            .find_map(|l| l.prefix_of_namespace(uri))
            .map(str::to_owned)
    }

    /// `MapBasedNamespaceContext.getPrefix` where the writer asks it: only for a namespace
    /// that no level of the stack binds to a prefix. Every namespace of the context is
    /// bound on the document element's level (no default namespace is ever bound to one
    /// of them, since an element in such a namespace finds its prefix first), so the
    /// context answers only for the `xml` and `xmlns` namespaces, which it binds
    /// implicitly.
    fn mapped_prefix(uri: &str) -> Option<&'static str> {
        match uri {
            XML_NAMESPACE => Some("xml"),
            XMLNS_NAMESPACE => Some("xmlns"),
            _ => None,
        }
    }

    fn unique_prefix(&self) -> String {
        let mut count: u64 = 0;
        loop {
            let prefix = format!("ns{count}");
            if self
                .stack
                .iter()
                .all(|l| l.namespace_of_prefix(&prefix).is_none())
            {
                return prefix;
            }
            count += 1;
        }
    }

    /// `NamespaceStack.getElementNamespacePrefixToUse`.
    fn element_prefix(
        &mut self,
        uri: &str,
        is_root: bool,
        attributes: &mut Attributes,
    ) -> Option<String> {
        if uri == self.default_namespace().unwrap_or("") {
            return None;
        }
        if uri.is_empty() {
            attributes.put_declaration(None, "");
            self.add_mapping(None, "");
            return None;
        }
        if let Some(prefix) = self.used_prefix(uri) {
            return Some(prefix);
        }
        let mut prefix = Self::mapped_prefix(uri).map(str::to_owned);
        if prefix.is_none() && !is_root {
            prefix = if self.stack.iter().any(Level::has_any_namespace) {
                Some(self.unique_prefix())
            } else {
                None
            };
        }
        attributes.put_declaration(prefix.as_deref(), uri);
        self.add_mapping(prefix.as_deref(), uri);
        prefix
    }

    /// `NamespaceStack.getAttributeNamespacePrefixToUse`.
    fn attribute_prefix(
        &mut self,
        uri: &str,
        name: &str,
        value: &str,
        attributes: &mut Attributes,
    ) -> Option<String> {
        if uri.is_empty() {
            return None;
        }
        if let Some(prefix) = self.used_prefix(uri) {
            return Some(prefix);
        }
        if uri == XMLNS_NAMESPACE {
            if name != "xmlns" {
                attributes.put_declaration(Some(name), value);
            }
            self.add_mapping(Some(name), value);
            return Some(name.to_owned());
        }
        let prefix = match Self::mapped_prefix(uri) {
            Some(prefix) if !prefix.is_empty() => prefix.to_owned(),
            _ => self.unique_prefix(),
        };
        if prefix != "xml" {
            attributes.put_declaration(Some(&prefix), uri);
        }
        self.add_mapping(Some(&prefix), uri);
        Some(prefix)
    }
}

/// What a string is written as.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// An element name, an element prefix or an attribute's local name.
    Name,
    /// An attribute value, between double quotes.
    AttributeValue,
    /// Text.
    Text,
    /// The content of a CDATA section.
    CData,
}

/// `XMLMaskHelper.maskXMLTextTo`: the characters invalid in this mode dropped, then the
/// replacements of the mode.
fn mask_into(out: &mut String, text: &str, mode: Mode) {
    if text.is_empty() {
        return;
    }
    // The invalid characters: by position for names (the first character, the others).
    let mut invalid: Vec<char> = Vec::new();
    for (index, c) in text.chars().enumerate() {
        let bad = match mode {
            Mode::Name if index == 0 => contains(INVALID_NAME_START, c),
            Mode::Name => contains(INVALID_NAME, c),
            Mode::AttributeValue | Mode::Text | Mode::CData => contains(INVALID_VALUE, c),
        };
        if bad && !invalid.contains(&c) {
            invalid.push(c);
        }
    }
    for c in text.chars() {
        if invalid.contains(&c) {
            continue;
        }
        // The masks of XML 1.0: MASK_TEXT_XML10 and MASK_ATTRIBUTE_VALUE_XML10_DQ; the
        // CDATA mode has none. A character without an entity is written as a hexadecimal
        // character reference (getXMLNumericReference: upper case, no leading zero).
        match (mode, c) {
            (Mode::Text | Mode::AttributeValue, '&') => out.push_str("&amp;"),
            (Mode::Text | Mode::AttributeValue, '<') => out.push_str("&lt;"),
            (Mode::Text, '>') => out.push_str("&gt;"),
            (Mode::AttributeValue, '"') => out.push_str("&quot;"),
            (Mode::AttributeValue, '\t' | '\n')
            | (Mode::Text | Mode::AttributeValue, '\r')
            | (Mode::Text, '\u{7F}'..='\u{9F}') => {
                let _ = write!(out, "&#x{:X};", u32::from(c));
            }
            _ => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masked(text: &str, mode: Mode) -> String {
        let mut out = String::new();
        mask_into(&mut out, text, mode);
        out
    }

    #[test]
    fn text_and_attribute_masks() {
        assert_eq!(
            masked(
                "a<b>&\"'\t\n\r\u{7F}\u{85}\u{9F}\u{A0}\u{1}\u{FFFE}é😀",
                Mode::Text
            ),
            "a&lt;b&gt;&amp;\"'\t\n&#xD;&#x7F;&#x85;&#x9F;\u{A0}é😀"
        );
        assert_eq!(
            masked("a<b>&\"'\t\n\r\u{7F}\u{1}", Mode::AttributeValue),
            "a&lt;b>&amp;&quot;'&#x9;&#xA;&#xD;\u{7F}"
        );
        assert_eq!(masked("a]]>\u{85}\u{2}", Mode::CData), "a]]>\u{85}");
    }

    #[test]
    fn invalid_name_characters_are_dropped_everywhere() {
        // '1' is invalid first, valid later: every '1' goes.
        assert_eq!(masked("1a1", Mode::Name), "a");
        // '-' is valid later: kept.
        assert_eq!(masked("a-b", Mode::Name), "a-b");
        // U+0132 (Latin IJ) is not a name character before XML 1.0 fifth edition.
        assert_eq!(masked("a\u{132}b", Mode::Name), "ab");
        assert_eq!(masked("𝄞x", Mode::Name), "𝄞x");
    }
}
