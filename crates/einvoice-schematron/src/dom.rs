//! The DOM document of a compiled stylesheet, as ph-schematron holds it: a document of
//! the JDK's DOM (Xerces), built by Saxon's `DOMWriter` from the result tree of the last
//! step.
//!
//! What the XML writer reads of it, and so what is kept here: for an element, its
//! namespace, its qualified name as Saxon names it (the prefix is used for nothing but the
//! order of attributes, see below), its attributes and its children; for an attribute,
//! its namespace, its qualified name and its value; the data of text nodes, CDATA
//! sections and comments; the target and data of processing instructions. The namespace
//! declarations Saxon makes are attributes in the `xmlns` namespace (`xmlns:p`, `xmlns`),
//! as in a DOM.
//!
//! The attributes of an element are kept in the order of Xerces' `AttributeMap`: sorted
//! by qualified name, comparing UTF-16 code units as Java's `String.compareTo` does (an
//! attribute whose qualified name is already there, in another namespace, goes before
//! it), the namespace declarations among the others. [`Element::set_attribute_ns`]
//! inserts as Xerces' `Element.setAttributeNS` does, [`Element::normalize`] merges text
//! nodes as Xerces' `Node.normalize` does.
//!
//! Xerces checks the names it is given (`CoreDocumentImpl.checkQName`, `checkDOMNSErr`,
//! `ElementNSImpl.setName`, `AttrNSImpl.setName`, `isXMLName`), with the name characters
//! of XML 1.0 before its fifth edition and none outside the Basic Multilingual Plane, so
//! it refuses names that Saxon accepts: [`Element::new`],
//! [`Element::set_attribute_ns`] and [`Node::processing_instruction`] refuse them the
//! same way ([`crate::Error::Dom`]), where ph-schematron fails.

use std::cmp::Ordering;

use crate::Error;
use crate::tables::xerces_accepts;

/// The `xmlns` namespace, of namespace declarations.
pub const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

/// The `xml` namespace.
pub const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

/// A document node: its children, in order. A document built by
/// [`Document::from_result`] has one element child, the document element, and may have
/// comments and processing instructions around it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Document {
    /// The children of the document node.
    pub children: Vec<Node>,
}

impl Document {
    /// An empty document.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The document element: the first element child.
    #[must_use]
    pub fn document_element(&self) -> Option<&Element> {
        self.children.iter().find_map(|child| match child {
            Node::Element(element) => Some(element),
            _ => None,
        })
    }
}

/// A node below the document node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// An element.
    Element(Element),
    /// A text node: its data.
    Text(String),
    /// A CDATA section: its data. Saxon's `DOMWriter` makes none; the writer supports
    /// them as ph-commons does.
    CData(String),
    /// A comment: its data.
    Comment(String),
    /// A processing instruction.
    ProcessingInstruction {
        /// Its target.
        target: String,
        /// Its data, empty for none.
        data: String,
    },
}

impl Node {
    /// A processing instruction, as Xerces' `Document.createProcessingInstruction` makes
    /// it.
    ///
    /// # Errors
    ///
    /// The target is not a name for Xerces (`INVALID_CHARACTER_ERR`).
    pub fn processing_instruction(target: &str, data: &str) -> Result<Self, Error> {
        if !xerces_accepts(target, true) {
            return Err(Error::Dom {
                code: "INVALID_CHARACTER_ERR",
                name: target.to_owned(),
            });
        }
        Ok(Self::ProcessingInstruction {
            target: target.to_owned(),
            data: data.to_owned(),
        })
    }

    /// Whether ph-commons' writer takes the node for inline content
    /// (`XMLHelper.isInlineNode`: a text node or an entity reference; a CDATA section is a
    /// text node): no indentation is written next to it.
    #[must_use]
    pub fn is_inline(&self) -> bool {
        matches!(self, Self::Text(_) | Self::CData(_))
    }
}

/// An element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element {
    namespace: Option<String>,
    name: String,
    attributes: Vec<Attribute>,
    /// The children, in order.
    pub children: Vec<Node>,
}

/// An attribute, or a namespace declaration (namespace [`XMLNS_NAMESPACE`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    /// Its namespace, `None` for none.
    pub namespace: Option<String>,
    /// Its qualified name.
    pub name: String,
    /// Its value.
    pub value: String,
}

impl Attribute {
    /// The local part of the name: after the colon, or the whole name.
    #[must_use]
    pub fn local_name(&self) -> &str {
        local_part(&self.name)
    }

    /// The prefix of the name, `None` for none.
    #[must_use]
    pub fn prefix(&self) -> Option<&str> {
        self.name.split_once(':').map(|(prefix, _)| prefix)
    }
}

impl Element {
    /// An element with this namespace (`None` for none) and qualified name, without
    /// attributes or children, as Xerces' `Document.createElementNS` makes it.
    ///
    /// # Errors
    ///
    /// Xerces refuses the name (see [`check_name`]).
    pub fn new(namespace: Option<&str>, name: &str) -> Result<Self, Error> {
        check_name(namespace, name)?;
        Ok(Self {
            namespace: namespace.map(str::to_owned),
            name: name.to_owned(),
            attributes: Vec::new(),
            children: Vec::new(),
        })
    }

    /// Its namespace, `None` for none.
    #[must_use]
    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    /// Its qualified name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The local part of the name: after the colon, or the whole name.
    #[must_use]
    pub fn local_name(&self) -> &str {
        local_part(&self.name)
    }

    /// The attributes, namespace declarations included, in the order of Xerces'
    /// `AttributeMap` (see the module documentation).
    #[must_use]
    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    /// Sets an attribute as Xerces' `Element.setAttributeNS` does: an attribute with the
    /// same namespace and local name gets the new value, and keeps its place unless its
    /// qualified name changes (another prefix), when it is taken out and inserted again;
    /// a new attribute is inserted at its place in the order of qualified names, before
    /// an attribute of the same qualified name if there is one.
    ///
    /// # Errors
    ///
    /// Xerces refuses the name (see [`check_name`]).
    pub fn set_attribute_ns(
        &mut self,
        namespace: Option<&str>,
        name: &str,
        value: &str,
    ) -> Result<(), Error> {
        check_name(namespace, name)?;
        let local = local_part(name);
        if let Some(index) = self
            .attributes
            .iter()
            .position(|a| a.namespace.as_deref() == namespace && a.local_name() == local)
        {
            if self.attributes[index].name == name {
                value.clone_into(&mut self.attributes[index].value);
                return Ok(());
            }
            self.attributes.remove(index);
        }
        let at = insertion_point(&self.attributes, name);
        self.attributes.insert(
            at,
            Attribute {
                namespace: namespace.map(str::to_owned),
                name: name.to_owned(),
                value: value.to_owned(),
            },
        );
        Ok(())
    }

    /// Merges adjacent text nodes and removes empty ones, in the element and in every
    /// element below it, as Xerces' `Node.normalize` does. CDATA sections are left as
    /// they are.
    pub fn normalize(&mut self) {
        let mut normalized: Vec<Node> = Vec::with_capacity(self.children.len());
        for mut child in std::mem::take(&mut self.children) {
            if let Node::Element(element) = &mut child {
                element.normalize();
            }
            if let (Node::Text(data), Some(Node::Text(previous))) = (&child, normalized.last_mut())
            {
                previous.push_str(data);
                continue;
            }
            normalized.push(child);
        }
        normalized.retain(|child| !matches!(child, Node::Text(data) if data.is_empty()));
        self.children = normalized;
    }
}

/// Whether Xerces accepts `name` for an element or an attribute in `namespace`
/// (`ElementNSImpl.setName`, `AttrNSImpl.setName`): at most one colon, neither first nor
/// last; the prefix and the local name XML 1.0 names without colon by Xerces' tables
/// (`INVALID_CHARACTER_ERR` otherwise); a prefix only with a namespace, the prefix `xml`
/// only for the `xml` namespace, and the `xmlns` namespace exactly for the name `xmlns`
/// and the prefix `xmlns` (`NAMESPACE_ERR` otherwise).
///
/// # Errors
///
/// [`Error::Dom`] with the code of Xerces' `DOMException`.
pub fn check_name(namespace: Option<&str>, name: &str) -> Result<(), Error> {
    let error = |code: &'static str| {
        Err(Error::Dom {
            code,
            name: name.to_owned(),
        })
    };
    let colons = name.matches(':').count();
    if colons > 1 || name.starts_with(':') || name.ends_with(':') {
        return error("NAMESPACE_ERR");
    }
    let is_xmlns = namespace == Some(XMLNS_NAMESPACE);
    match name.split_once(':') {
        None => {
            if !xerces_accepts(name, false) {
                return error("INVALID_CHARACTER_ERR");
            }
            if (name == "xmlns") != is_xmlns {
                return error("NAMESPACE_ERR");
            }
        }
        Some((prefix, local)) => {
            if namespace.is_none() || (prefix == "xml" && namespace != Some(XML_NAMESPACE)) {
                return error("NAMESPACE_ERR");
            }
            if !xerces_accepts(prefix, false) || !xerces_accepts(local, false) {
                return error("INVALID_CHARACTER_ERR");
            }
            if (prefix == "xmlns") != is_xmlns {
                return error("NAMESPACE_ERR");
            }
        }
    }
    Ok(())
}

/// The place of an attribute named `name` in `attributes`, as Xerces'
/// `NamedNodeMapImpl.findNamePoint(String, int)` finds it: a binary search on the
/// qualified names; when one is equal, its position.
fn insertion_point(attributes: &[Attribute], name: &str) -> usize {
    let mut first = 0_usize;
    let mut last = attributes.len();
    // The half-open range [first, last) is searched; Xerces' closed range with its
    // midpoint rounding down visits the same positions.
    while first < last {
        let middle = first + (last - first - 1) / 2;
        match java_compare(name, &attributes[middle].name) {
            Ordering::Equal => return middle,
            Ordering::Less => last = middle,
            Ordering::Greater => first = middle + 1,
        }
    }
    first
}

/// Java's `String.compareTo`: the UTF-16 code units in order.
pub(crate) fn java_compare(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

fn local_part(name: &str) -> &str {
    name.split_once(':').map_or(name, |(_, local)| local)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(element: &Element) -> Vec<&str> {
        element
            .attributes()
            .iter()
            .map(|a| a.name.as_str())
            .collect()
    }

    #[test]
    fn attributes_are_sorted_by_qualified_name() {
        let mut e = Element::new(None, "e").unwrap();
        for name in ["version", "xmlns:xsl", "b", "xmlns:a", "a:z", "xmlns", "A"] {
            let namespace = if name.starts_with("xmlns") {
                Some(XMLNS_NAMESPACE)
            } else if name.contains(':') {
                Some("urn:a")
            } else {
                None
            };
            e.set_attribute_ns(namespace, name, "1").unwrap();
        }
        assert_eq!(
            names(&e),
            ["A", "a:z", "b", "version", "xmlns", "xmlns:a", "xmlns:xsl"]
        );
    }

    #[test]
    fn insertion_point_is_xerces_binary_search() {
        // With equal names, Xerces' search stops at the first equal one it meets.
        let attributes: Vec<Attribute> = ["a", "b", "b", "b", "c"]
            .iter()
            .map(|n| Attribute {
                namespace: None,
                name: (*n).to_owned(),
                value: String::new(),
            })
            .collect();
        assert_eq!(insertion_point(&attributes, "b"), 2);
        assert_eq!(insertion_point(&attributes, "a"), 0);
        assert_eq!(insertion_point(&attributes, "bb"), 4);
        assert_eq!(insertion_point(&attributes, "d"), 5);
        assert_eq!(insertion_point(&[], "d"), 0);
    }

    #[test]
    fn names_xerces_refuses() {
        assert!(check_name(None, "a").is_ok());
        assert!(check_name(None, "\u{132}").is_err());
        assert!(check_name(None, "a\u{10000}").is_err());
        assert!(check_name(None, "xmlns").is_err());
        assert!(check_name(None, "p:a").is_err());
        assert!(check_name(Some("urn:x"), "a:b:c").is_err());
        assert!(check_name(Some(XMLNS_NAMESPACE), "xmlns:p").is_ok());
        assert!(check_name(Some(XMLNS_NAMESPACE), "p:a").is_err());
        assert!(Node::processing_instruction("a:b", "").is_ok());
        assert!(Node::processing_instruction("1a", "").is_err());
    }
}
