//! Property tests against simple reference models.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

use std::fmt::Write as _;

mod common;

use proptest::prelude::*;

fn run(xslt: &str, input: &str) -> String {
    let sheet = einvoice_xslt::load(xslt).unwrap();
    let executable = einvoice_xslt_exec::Executable::new(&sheet).unwrap();
    executable.transform(input, None).unwrap().output
}

/// Characters allowed in XML 1.0 text (a mix of plain, special and invisible ones).
fn xml_char() -> impl Strategy<Value = char> {
    prop_oneof![
        4 => proptest::char::range('a', 'e'),
        2 => prop::sample::select(vec!['<', '>', '&', '"', '\'', '-', ' ', '\t', '\n', '\r', '{', '}']),
        1 => prop::sample::select(vec!['\u{a0}', '\u{200b}', 'é', '€', '\u{10348}', '\u{fffd}']),
    ]
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Any string goes through attributes, text and comments of the result unchanged
    /// (comments with a space after each `-` that XML forbids there).
    #[test]
    fn values_survive_serialization(value in proptest::collection::vec(xml_char(), 0..40)) {
        let value: String = value.into_iter().collect();
        let input = format!("<r a=\"{}\">{}</r>", escape(&value), escape(&value));
        let xslt = r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
          <xsl:template match="/"><out v="{r/@a}"><xsl:attribute name="w" select="string(r)"/><xsl:value-of select="r/@a"/><xsl:comment><xsl:value-of select="r"/></xsl:comment></out></xsl:template>
        </xsl:stylesheet>"#;
        let output = run(xslt, &input);
        let doc = roxmltree::Document::parse(&output).unwrap();
        let out = doc.root_element();
        prop_assert_eq!(out.attribute("v"), Some(value.as_str()));
        prop_assert_eq!(out.attribute("w"), Some(value.as_str()));
        prop_assert_eq!(out.text().unwrap_or(""), value.as_str());
        let comment = out.children().find(roxmltree::Node::is_comment).unwrap();
        let mut expected = String::new();
        let mut previous_dash = false;
        for c in value.chars() {
            if c == '-' && previous_dash {
                expected.push(' ');
            }
            previous_dash = c == '-';
            expected.push(c);
        }
        if previous_dash {
            expected.push(' ');
        }
        prop_assert_eq!(comment.text().unwrap_or(""), expected.as_str());
    }

    /// With explicit priorities, the templates that run for a node, first one and then
    /// one per `xsl:next-match`, are the matching ones by priority, the last declared
    /// first on a tie: whichever rule chain (named, kind, generic) holds them.
    #[test]
    fn next_match_visits_the_matching_rules_in_rank_order(
        templates in proptest::collection::vec((0_usize..6, -2_i32..3), 1..10)
    ) {
        // Patterns that match the `x` element (named, kind and generic rule chains),
        // and one that does not.
        const PATTERNS: [&str; 6] = ["x", "*", "node()", "node()[self::x]", "r/x", "y"];
        let mut xslt = String::from(r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
          <xsl:template match="/"><out><xsl:apply-templates select="r/x" mode="m"/></out></xsl:template>"#);
        for (i, (pattern, priority)) in templates.iter().enumerate() {
            write!(
                xslt,
                r#"<xsl:template match="{}" mode="m" priority="{priority}">{i};<xsl:next-match/></xsl:template>"#,
                PATTERNS[*pattern]
            )
            .unwrap();
        }
        xslt.push_str("</xsl:stylesheet>");
        let mut expected: Vec<(i32, usize)> = templates
            .iter()
            .enumerate()
            .filter(|(_, (pattern, _))| *pattern != 5)
            .map(|(i, (_, priority))| (*priority, i))
            .collect();
        expected.sort_by(|a, b| b.cmp(a));
        // The built-in rule for the (empty) element ends the chain, without output.
        let mut expected_text = String::new();
        for (_, i) in &expected {
            write!(expected_text, "{i};").unwrap();
        }
        let output = run(&xslt, "<r><x/></r>");
        let doc = roxmltree::Document::parse(&output).unwrap();
        prop_assert_eq!(doc.root_element().text().unwrap_or(""), expected_text.as_str());
    }

    /// `xsl:number level="multiple" count="*"` is the position of each ancestor-or-self
    /// element among its element siblings.
    #[test]
    fn number_multiple_counts_ancestors(shape in proptest::collection::vec(0_usize..4, 1..6)) {
        // A tree where level k has shape[k] + 1 children named `e`, the last one leading on.
        let mut input = String::new();
        let mut expected = Vec::new();
        for &n in &shape {
            for _ in 0..n {
                input.push_str("<e/>");
            }
            input.push_str("<e>");
            expected.push((n + 1).to_string());
        }
        input.push_str("<leaf/>");
        expected.push("1".to_owned());
        for _ in &shape {
            input.push_str("</e>");
        }
        let input = format!("<r>{input}</r>");
        let xslt = r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
          <xsl:template match="/"><out><xsl:apply-templates select="//leaf"/></out></xsl:template>
          <xsl:template match="leaf"><xsl:number level="multiple" count="*"/></xsl:template>
        </xsl:stylesheet>"#;
        let output = run(xslt, &input);
        let doc = roxmltree::Document::parse(&output).unwrap();
        prop_assert_eq!(doc.root_element().text().unwrap_or(""), format!("1.{}", expected.join(".")));
    }
}

/// A node of a generated document.
#[derive(Debug, Clone)]
enum Generated {
    Element(usize, Vec<(usize, String)>, Vec<Generated>),
    Text(String),
    Comment(String),
}

/// Element names (`p` and `q` are declared on the document element), attribute names.
const ELEMENT_NAMES: [&str; 4] = ["a", "b", "p:c", "q:d"];
const ATTRIBUTE_NAMES: [&str; 3] = ["x", "y", "p:z"];

fn short_text() -> impl Strategy<Value = String> {
    proptest::collection::vec(xml_char(), 0..6).prop_map(|c| c.into_iter().collect())
}

fn generated() -> impl Strategy<Value = Generated> {
    let leaf = prop_oneof![
        3 => short_text().prop_map(Generated::Text),
        1 => proptest::collection::vec(proptest::char::range('a', 'c'), 0..4)
            .prop_map(|c| Generated::Comment(c.into_iter().collect())),
        2 => (0..ELEMENT_NAMES.len()).prop_map(|n| Generated::Element(n, Vec::new(), Vec::new())),
    ];
    leaf.prop_recursive(4, 40, 5, |inner| {
        (
            0..ELEMENT_NAMES.len(),
            proptest::collection::vec((0..ATTRIBUTE_NAMES.len(), short_text()), 0..3),
            proptest::collection::vec(inner, 0..5),
        )
            .prop_map(|(name, attributes, children)| Generated::Element(name, attributes, children))
    })
}

fn write_generated(node: &Generated, out: &mut String) {
    match node {
        Generated::Element(name, attributes, children) => {
            let name = ELEMENT_NAMES[*name];
            write!(out, "<{name}").unwrap();
            let mut seen = Vec::new();
            for (attribute, value) in attributes {
                if !seen.contains(attribute) {
                    seen.push(*attribute);
                    write!(
                        out,
                        " {}=\"{}\"",
                        ATTRIBUTE_NAMES[*attribute],
                        escape(value)
                    )
                    .unwrap();
                }
            }
            out.push('>');
            for child in children {
                write_generated(child, out);
            }
            write!(out, "</{name}>").unwrap();
        }
        Generated::Text(text) => out.push_str(&escape(text)),
        Generated::Comment(text) => write!(out, "<!--{text}-->").unwrap(),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// A result kept compactly has the text of the result written as a string, and its
    /// observer is given what a parser reads from that text: elements, attributes,
    /// text (the whitespace of indentation in mixed and element content included) and
    /// comments, with namespaces declared, inherited and fixed up.
    #[test]
    fn a_compact_result_is_the_text_a_parser_reads(
        children in proptest::collection::vec(generated(), 0..6),
        indent in any::<bool>(),
        wrap in any::<bool>(),
    ) {
        let mut input = String::from(r#"<r xmlns:p="urn:p" xmlns:q="urn:q">"#);
        for child in &children {
            write_generated(child, &mut input);
        }
        input.push_str("</r>");
        // The document copied, inside a literal result element whose prefix is that of
        // another namespace, or as it is.
        let body = if wrap {
            r#"<p:out xmlns:p="urn:other" p:n="1"><xsl:sequence select="r/node()"/><xsl:comment>end</xsl:comment></p:out>"#
        } else {
            r#"<xsl:sequence select="r"/>"#
        };
        let xslt = format!(
            r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
              <xsl:output indent="{}"/>
              <xsl:template match="/">{body}</xsl:template>
            </xsl:stylesheet>"#,
            if indent { "yes" } else { "no" }
        );
        let sheet = einvoice_xslt::load(&xslt).unwrap();
        let executable = einvoice_xslt_exec::Executable::new(&sheet).unwrap();
        let whole = executable.transform(input.as_str(), None).unwrap();
        let mut recorder = common::Recorder::default();
        let compact = executable
            .transform_compact(input.as_str(), None, &einvoice_xslt_exec::TransformOptions::default(), &mut recorder)
            .unwrap();
        prop_assert_eq!(compact.output.to_text(), whole.output.clone());
        prop_assert_eq!(compact.warnings, whole.warnings);
        prop_assert_eq!(recorder.events, common::parsed_events(&whole.output).unwrap());
    }
}
