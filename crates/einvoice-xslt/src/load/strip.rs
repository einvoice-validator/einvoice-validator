//! The children of a stylesheet element after whitespace stripping (XSLT 2.0 section 4.2):
//!
//! 1. comments and processing instructions are removed;
//! 2. text nodes that became adjacent are merged;
//! 3. a whitespace-only text node is removed unless its parent is `xsl:text` or the nearest
//!    `xml:space` attribute on an ancestor-or-self says `preserve`;
//! 4. it is removed regardless of `xml:space` when its parent is one of `xsl:analyze-string`,
//!    `xsl:apply-imports`, `xsl:apply-templates`, `xsl:attribute-set`, `xsl:call-template`,
//!    `xsl:character-map`, `xsl:choose`, `xsl:next-match`, `xsl:stylesheet`,
//!    `xsl:transform`;
//! 5. and when its following sibling is `xsl:param` or `xsl:sort`.

use roxmltree::Node;

use super::{XSL, is_xsl};
use einvoice_xpath::{XML_NAMESPACE, is_xml_whitespace};

/// A child that survived stripping.
#[derive(Debug, Clone)]
pub(crate) enum Child<'a, 'input> {
    /// An element.
    Element(Node<'a, 'input>),
    /// Merged text, with the byte offset where it starts.
    Text { value: String, offset: usize },
}

const STRIP_ALWAYS: [&str; 10] = [
    "analyze-string",
    "apply-imports",
    "apply-templates",
    "attribute-set",
    "call-template",
    "character-map",
    "choose",
    "next-match",
    "stylesheet",
    "transform",
];

/// Whether `xml:space="preserve"` is in force on `node` (nearest declaration wins).
fn space_preserved(node: Node<'_, '_>) -> bool {
    node.ancestors()
        .filter(Node::is_element)
        .find_map(|n| n.attribute((XML_NAMESPACE, "space")))
        .is_some_and(|value| value == "preserve")
}

/// The children of `node` after stripping.
pub(crate) fn children<'a, 'input>(node: Node<'a, 'input>) -> Vec<Child<'a, 'input>> {
    let mut merged: Vec<Child<'a, 'input>> = Vec::new();
    for child in node.children() {
        if child.is_element() {
            merged.push(Child::Element(child));
        } else if child.is_text() {
            let text = child.text().unwrap_or("");
            if let Some(Child::Text { value, .. }) = merged.last_mut() {
                value.push_str(text);
            } else {
                merged.push(Child::Text {
                    value: text.to_owned(),
                    offset: child.range().start,
                });
            }
        }
    }
    let parent_is_text = node.has_tag_name((XSL, "text"));
    let strip_always = is_xsl(node) && STRIP_ALWAYS.contains(&node.tag_name().name());
    let preserved = space_preserved(node);
    let mut out = Vec::with_capacity(merged.len());
    let mut iter = merged.into_iter().peekable();
    while let Some(child) = iter.next() {
        if let Child::Text { value, .. } = &child
            && value.chars().all(is_xml_whitespace)
        {
            let before_param_or_sort = matches!(iter.peek(), Some(Child::Element(next))
                if next.has_tag_name((XSL, "param")) || next.has_tag_name((XSL, "sort")));
            if strip_always || before_param_or_sort || (!parent_is_text && !preserved) {
                continue;
            }
        }
        out.push(child);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(xml: &str, path: &[usize]) -> Vec<String> {
        let doc = roxmltree::Document::parse(xml).unwrap();
        let mut node = doc.root_element();
        for &i in path {
            node = node.children().filter(Node::is_element).nth(i).unwrap();
        }
        children(node)
            .into_iter()
            .map(|c| match c {
                Child::Element(e) => format!("<{}>", e.tag_name().name()),
                Child::Text { value, .. } => value,
            })
            .collect()
    }

    const NS: &str = r#"xmlns:xsl="http://www.w3.org/1999/XSL/Transform""#;

    #[test]
    fn strips_whitespace_but_keeps_text() {
        let xml = format!(
            "<xsl:stylesheet {NS}>\n <a>\n  @<xsl:value-of/>\n  <!-- c -->x<!-- d -->y <?pi?>z\n</a></xsl:stylesheet>"
        );
        assert_eq!(texts(&xml, &[]), ["<a>"]);
        assert_eq!(texts(&xml, &[0]), ["\n  @", "<value-of>", "\n  xy z\n"]);
    }

    #[test]
    fn xsl_text_and_xml_space_preserve() {
        let xml = format!(
            "<xsl:stylesheet {NS}><xsl:text> </xsl:text><a xml:space='preserve'> <b xml:space='default'> </b><c> </c></a></xsl:stylesheet>"
        );
        assert_eq!(texts(&xml, &[0]), [" "]);
        assert_eq!(texts(&xml, &[1]), [" ", "<b>", "<c>"]);
        assert_eq!(texts(&xml, &[1, 0]), Vec::<String>::new());
        assert_eq!(texts(&xml, &[1, 1]), [" "]);
    }

    #[test]
    fn rules_four_and_five_ignore_xml_space() {
        let xml = format!(
            "<xsl:stylesheet {NS}><a xml:space='preserve'><xsl:choose> <xsl:when/> </xsl:choose><t> <xsl:param/> <x/></t></a></xsl:stylesheet>"
        );
        assert_eq!(texts(&xml, &[0, 0]), ["<when>"]);
        assert_eq!(texts(&xml, &[0, 1]), ["<param>", " ", "<x>"]);
    }
}
