//! The DOM document that Saxon's `DOMWriter` (Saxon-HE 12.10, `net.sf.saxon.dom`) builds
//! from a result tree, read here from the result tree written as XML.
//!
//! What `DOMWriter` does with each event of the tree, and so what is done here:
//!
//! - an element: `Document.createElementNS(uri, displayName)`, `None` for no namespace;
//!   then, for each namespace binding of the element that its parent's bindings do not
//!   have (`NamespaceMap.getDifferences(parent, false)`: a prefix bound to another URI, a
//!   new prefix, and the default namespace undeclared when the parent binds one, but only
//!   if the element has some binding, a quirk of the merge), never the `xml` namespace,
//!   an attribute `xmlns:p` or `xmlns` in the `xmlns` namespace; then
//!   each attribute, with `Element.setAttributeNS(uri, displayName, value)`; at the end
//!   of the element, `Node.normalize` on it;
//! - text: a text node, except text of whitespace only (spaces, tabs, line feeds,
//!   carriage returns) outside the document element, which is dropped (other text
//!   there, or a second element, cannot be a child of a DOM document:
//!   `HIERARCHY_REQUEST_ERR`; neither is well-formed XML);
//! - comments and processing instructions: nodes, wherever they are.
//!
//! The names Xerces refuses (see [`dom`](crate::dom)) are refused here, where `DOMWriter`
//! fails.
//!
//! The XML of a result tree has its namespace bindings as declarations: the bindings of
//! an element are those of its parent with its declarations, `xmlns=""` taking the
//! default namespace away. Its prefixes and its text are those of the tree. XML cannot
//! say that an element of the tree lacks a prefix its parent binds (XSLT can make one,
//! with `inherit-namespaces="no"`): read from XML, it has it. Only the namespace
//! declarations of the DOM below the document element can then differ, which the XML
//! writer does not read.

use einvoice_xml::{Event, Limits, Reader};

use crate::dom::{Document, Element, Node, XML_NAMESPACE, XMLNS_NAMESPACE, java_compare};
use crate::{Error, MAX_DEPTH};

/// Builds the document from the result tree written as XML.
pub(crate) fn from_result(xml: &str) -> Result<Document, Error> {
    let limits = Limits::default().with_depth(u32::try_from(MAX_DEPTH).unwrap_or(u32::MAX));
    let mut reader = Reader::with_limits(xml, limits);
    let mut document = Document::new();
    // The open elements, innermost last.
    let mut open: Vec<Element> = Vec::new();
    // The namespace maps of the open elements, innermost last, the map of the document
    // node (empty) first.
    let mut maps: Vec<NamespaceMap> = vec![NamespaceMap::default()];
    while let Some(event) = reader.next_event()? {
        match event {
            Event::Start(start) => {
                let name = start.name();
                let mut element =
                    Element::new(name.namespace, &display_name(name.prefix, name.local))?;
                let parent = maps.last().cloned().unwrap_or_default();
                let mut map = parent.clone();
                for (prefix, uri) in start.namespace_declarations() {
                    map.bind(prefix, uri);
                }
                for (prefix, uri) in map.differences(&parent) {
                    let qname = if prefix.is_empty() {
                        "xmlns".to_owned()
                    } else {
                        format!("xmlns:{prefix}")
                    };
                    element.set_attribute_ns(Some(XMLNS_NAMESPACE), &qname, uri)?;
                }
                maps.push(map);
                for attribute in start.attributes() {
                    let name = attribute.name;
                    element.set_attribute_ns(
                        name.namespace,
                        &display_name(name.prefix, name.local),
                        attribute.value,
                    )?;
                }
                open.push(element);
            }
            Event::End(_) => {
                let Some(mut element) = open.pop() else {
                    continue;
                };
                maps.pop();
                element.normalize();
                append(&mut document, &mut open, Node::Element(element));
            }
            Event::Text(text) => {
                if open.is_empty() {
                    // Whitespace: well-formed XML has no other text there.
                    continue;
                }
                append(
                    &mut document,
                    &mut open,
                    Node::Text(text.value.into_owned()),
                );
            }
            Event::Comment(comment) => {
                append(
                    &mut document,
                    &mut open,
                    Node::Comment(comment.value.into_owned()),
                );
            }
            Event::ProcessingInstruction(pi) => {
                let node = Node::processing_instruction(
                    pi.target,
                    pi.value.as_deref().unwrap_or_default(),
                )?;
                append(&mut document, &mut open, node);
            }
        }
    }
    Ok(document)
}

fn append(document: &mut Document, open: &mut [Element], node: Node) {
    match open.last_mut() {
        Some(parent) => parent.children.push(node),
        None => document.children.push(node),
    }
}

/// Saxon's `NamespaceMap`: the bindings of an element, sorted by prefix (`""` for the
/// default namespace), never one for the `xml` namespace.
#[derive(Clone, Debug, Default)]
struct NamespaceMap {
    bindings: Vec<(String, String)>,
}

impl NamespaceMap {
    /// A declaration: `uri` empty takes the prefix away (`NamespaceMap.bind` with
    /// `NamespaceUri.NULL`); the `xml` namespace is never bound.
    fn bind(&mut self, prefix: &str, uri: &str) {
        if uri == XML_NAMESPACE {
            return;
        }
        let at = self
            .bindings
            .binary_search_by(|(p, _)| java_compare(p, prefix));
        match (at, uri.is_empty()) {
            (Ok(i), true) => {
                self.bindings.remove(i);
            }
            (Ok(i), false) => uri.clone_into(&mut self.bindings[i].1),
            (Err(_), true) => {}
            (Err(i), false) => self.bindings.insert(i, (prefix.to_owned(), uri.to_owned())),
        }
    }

    /// `NamespaceMap.getDifferences(parent, false)`, as Saxon-HE 12.10 merges the two
    /// sorted lists: each binding of this map that `parent` does not have, and the
    /// default namespace undeclared (an empty URI) where `parent` binds it and this map
    /// does not, but only when this map still has a binding to compare with at that
    /// point of the merge (when this map is not empty, since `""` sorts first).
    fn differences<'m>(&'m self, parent: &'m Self) -> Vec<(&'m str, &'m str)> {
        let (mine, theirs) = (&self.bindings, &parent.bindings);
        let mut result = Vec::new();
        let (mut at_mine, mut at_theirs) = (0, 0);
        loop {
            match (mine.get(at_mine), theirs.get(at_theirs)) {
                (Some((prefix, uri)), Some((their_prefix, their_uri))) => {
                    match java_compare(prefix, their_prefix) {
                        std::cmp::Ordering::Less => {
                            result.push((prefix.as_str(), uri.as_str()));
                            at_mine += 1;
                        }
                        std::cmp::Ordering::Equal => {
                            if uri != their_uri {
                                result.push((prefix.as_str(), uri.as_str()));
                            }
                            at_mine += 1;
                            at_theirs += 1;
                        }
                        std::cmp::Ordering::Greater => {
                            if their_prefix.is_empty() {
                                result.push(("", ""));
                            }
                            at_theirs += 1;
                        }
                    }
                }
                (Some((prefix, uri)), None) => {
                    result.push((prefix.as_str(), uri.as_str()));
                    at_mine += 1;
                }
                (None, Some(_)) => at_theirs += 1,
                (None, None) => return result,
            }
        }
    }
}

fn display_name(prefix: &str, local: &str) -> String {
    if prefix.is_empty() {
        local.to_owned()
    } else {
        format!("{prefix}:{local}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attribute_names(element: &Element) -> Vec<(&str, &str)> {
        element
            .attributes()
            .iter()
            .map(|a| (a.name.as_str(), a.value.as_str()))
            .collect()
    }

    #[test]
    fn declarations_are_the_differences_with_the_parent() {
        let document = from_result(
            "<a:r xmlns:a='urn:a' xmlns='urn:d' x='1'>\
               <c xmlns:a='urn:a' xmlns:b='urn:b'/>\
               <d xmlns=''/>\
               <a:e xmlns:a='urn:other'><f xmlns=''><g xmlns=''/></f></a:e>\
             </a:r>",
        )
        .unwrap();
        let root = document.document_element().unwrap();
        assert_eq!(
            attribute_names(root),
            [("x", "1"), ("xmlns", "urn:d"), ("xmlns:a", "urn:a")]
        );
        let children: Vec<&Element> = root
            .children
            .iter()
            .filter_map(|n| match n {
                Node::Element(e) => Some(e),
                _ => None,
            })
            .collect();
        assert_eq!(attribute_names(children[0]), [("xmlns:b", "urn:b")]);
        assert_eq!(attribute_names(children[1]), [("xmlns", "")]);
        assert_eq!(children[1].namespace(), None);
        assert_eq!(attribute_names(children[2]), [("xmlns:a", "urn:other")]);
        let Node::Element(f) = &children[2].children[0] else {
            panic!("no f")
        };
        assert_eq!(attribute_names(f), [("xmlns", "")]);
        let Node::Element(g) = &f.children[0] else {
            panic!("no g")
        };
        assert_eq!(attribute_names(g), []);
    }

    #[test]
    fn the_default_namespace_is_undeclared_only_beside_another_binding() {
        // getDifferences reaches the parent's default binding with nothing left in an
        // empty map, and adds no undeclaration then.
        let document = from_result("<r xmlns='urn:d'><c xmlns=''/></r>").unwrap();
        let root = document.document_element().unwrap();
        let Node::Element(c) = &root.children[0] else {
            panic!("no c")
        };
        assert_eq!(attribute_names(c), []);
        let document = from_result("<r xmlns='urn:d'><c xmlns='' xmlns:p='urn:p'/></r>").unwrap();
        let root = document.document_element().unwrap();
        let Node::Element(c) = &root.children[0] else {
            panic!("no c")
        };
        assert_eq!(attribute_names(c), [("xmlns", ""), ("xmlns:p", "urn:p")]);
    }

    #[test]
    fn whitespace_outside_the_root_is_dropped_other_text_is_refused() {
        let document = from_result("<?xml version='1.0'?>\n<!--c-->\n<r/>\n<?p d?>\n").unwrap();
        assert_eq!(document.children.len(), 3);
        // Text and a second element outside the document element, which a DOM document
        // cannot have, are not well-formed XML either.
        assert!(matches!(from_result("<r/><s/>"), Err(Error::Xml(_))));
        assert!(matches!(from_result("<r/>t"), Err(Error::Xml(_))));
    }

    #[test]
    fn text_is_merged() {
        let document = from_result("<r>a<![CDATA[b]]>&amp;c<!--x-->d</r>").unwrap();
        let root = document.document_element().unwrap();
        assert_eq!(
            root.children,
            [
                Node::Text("ab&c".into()),
                Node::Comment("x".into()),
                Node::Text("d".into())
            ]
        );
    }
}
