//! The canonical SVRL form (`corpus/SVRL-CANONICAL.md`) is implemented by the
//! `einvoice-svrl` crate, shared with the engine's equivalence tests so that
//! both sides of a comparison use the same code.
//!
//! Reports written by SchXslt (the XRechnung configuration,
//! [`Generator::SchXslt`]) start with an `svrl:metadata` element that canonical
//! form 1 cannot represent: its content is outside the SVRL namespace (Dublin
//! Core, SKOS, RDF and SchXslt elements naming the processor, the compiler, the
//! time of the compilation and the time of the run). [`split`] takes it out of
//! the report before the canonical form is computed and returns it in a form
//! of its own, after checking that it is exactly what SchXslt writes: one
//! `svrl:metadata`, the first child of the root, and the time of the run equal
//! to the clock the oracle fixes ([`FIXED_NOW`]). Anything else is an error, as
//! in the canonical form itself: nothing is trimmed silently.

use std::fmt::Write as _;

use roxmltree::{Document, Node, NodeType, ParsingOptions};

pub(crate) use einvoice_svrl::{canonicalize, is_document_uri, parse};

use crate::ruleset::Generator;
use crate::util::{Error, Result, normalize_space};

/// The `current-dateTime()` of every oracle run (Saxon-HE `-now:`), so that a
/// report that records the time of its run is the same on every run.
pub(crate) const FIXED_NOW: &str = "2000-01-01T00:00:00Z";

const SVRL_NS: &str = "http://purl.oclc.org/dsdl/svrl";
const DCT_NS: &str = "http://purl.org/dc/terms/";

/// The canonical form of a report written by a stylesheet of `generator`:
/// `(canonical report, metadata)`, the metadata in the line form of
/// [`metadata_lines`] for SchXslt, `None` for the ISO skeleton. With
/// `fixed_clock`, the time of the run must be [`FIXED_NOW`] (a report of the
/// oracle); without, it is not checked (a report of an engine that does not
/// fix its clock).
pub(crate) fn split(
    raw: &str,
    generator: Generator,
    document_uri: &dyn Fn(&str) -> bool,
    fixed_clock: bool,
) -> Result<(String, Option<String>)> {
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: u32::MAX,
        ..ParsingOptions::default()
    };
    let doc =
        Document::parse_with_options(raw, options).map_err(|e| Error::new(format!("SVRL: {e}")))?;
    let root = doc.root_element();
    let metadata: Vec<Node<'_, '_>> = root
        .children()
        .filter(|n| n.is_element() && n.tag_name().namespace() == Some(SVRL_NS))
        .filter(|n| n.tag_name().name() == "metadata")
        .collect();
    match (generator, metadata.as_slice()) {
        (Generator::IsoSkeleton, []) => Ok((canonicalize(raw, document_uri)?, None)),
        (Generator::IsoSkeleton, [_, ..]) => Err(Error::new(
            "SVRL: svrl:metadata in a report of the ISO Schematron skeleton",
        )),
        (Generator::SchXslt, [node]) => {
            let first = root
                .children()
                .filter(Node::is_element)
                .find(|n| {
                    n.tag_name().namespace() != Some(SVRL_NS)
                        || n.tag_name().name() != "ns-prefix-in-attribute-values"
                })
                .ok_or_else(|| Error::new("SVRL: an empty report"))?;
            if first != *node {
                return Err(Error::new(
                    "SVRL: svrl:metadata is not the first child after the namespace prefixes",
                ));
            }
            if fixed_clock {
                check_run_time(*node)?;
            }
            let lines = metadata_lines(*node)?;
            let range = node.range();
            let rest = format!("{}{}", &raw[..range.start], &raw[range.end..]);
            Ok((canonicalize(&rest, document_uri)?, Some(lines)))
        }
        (Generator::SchXslt, []) => {
            Err(Error::new("SVRL: no svrl:metadata in a report of SchXslt"))
        }
        (Generator::SchXslt, [_, _, ..]) => Err(Error::new("SVRL: two svrl:metadata")),
    }
}

/// The time of the run (`svrl:metadata/dct:created`) must be [`FIXED_NOW`]; the
/// time of the compilation is under `dct:source`.
fn check_run_time(metadata: Node<'_, '_>) -> Result<()> {
    let created: Vec<Node<'_, '_>> = metadata
        .children()
        .filter(|n| n.is_element() && n.tag_name().namespace() == Some(DCT_NS))
        .filter(|n| n.tag_name().name() == "created")
        .collect();
    let [created] = created.as_slice() else {
        return Err(Error::new(
            "SVRL: svrl:metadata without exactly one dct:created",
        ));
    };
    let text: String = created.children().filter_map(|c| c.text()).collect();
    if text.trim() == FIXED_NOW {
        Ok(())
    } else {
        Err(Error::new(format!(
            "SVRL: svrl:metadata/dct:created is {:?}, not the fixed clock {FIXED_NOW}",
            text.trim()
        )))
    }
}

/// The metadata element as lines: one per element, `<indent>{<namespace>}<local
/// name>`, then its attributes sorted by name (`{<namespace>}<name>="<value>"`
/// for a namespaced one), then its text (whitespace normalised) when it has
/// only text; comments and processing instructions are refused.
pub(crate) fn metadata_lines(node: Node<'_, '_>) -> Result<String> {
    let mut out = String::new();
    element_lines(node, 0, &mut out)?;
    Ok(out)
}

fn element_lines(node: Node<'_, '_>, depth: usize, out: &mut String) -> Result<()> {
    let name = node.tag_name();
    let _ = write!(
        out,
        "{:indent$}{{{}}}{}",
        "",
        name.namespace().unwrap_or(""),
        name.name(),
        indent = depth * 2
    );
    let mut attributes: Vec<(String, String)> = node
        .attributes()
        .map(|a| {
            let key = match a.namespace() {
                None => a.name().to_owned(),
                Some(ns) => format!("{{{ns}}}{}", a.name()),
            };
            (key, a.value().to_owned())
        })
        .collect();
    attributes.sort();
    for (key, value) in attributes {
        let _ = write!(out, " {key}={}", quote(&value));
    }
    let elements: Vec<Node<'_, '_>> = node.children().filter(Node::is_element).collect();
    let mut text = String::new();
    for child in node.children() {
        match child.node_type() {
            NodeType::Element => {}
            NodeType::Text => text.push_str(child.text().unwrap_or("")),
            _ => {
                return Err(Error::new(format!(
                    "SVRL: unexpected content in svrl:metadata at byte {}",
                    child.range().start
                )));
            }
        }
    }
    let text = normalize_space(&text);
    if !elements.is_empty() && !text.is_empty() {
        return Err(Error::new(format!(
            "SVRL: mixed content in svrl:metadata at byte {}",
            node.range().start
        )));
    }
    if elements.is_empty() {
        let _ = write!(out, " {}", quote(&text));
    }
    out.push('\n');
    for child in elements {
        element_lines(child, depth + 1, out)?;
    }
    Ok(())
}

/// `"<text>"`, with `\`, `"` and control characters escaped.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::{FIXED_NOW, split};
    use crate::ruleset::Generator;

    fn report(created: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<svrl:schematron-output xmlns:svrl="http://purl.oclc.org/dsdl/svrl" title="t">
   <svrl:ns-prefix-in-attribute-values prefix="cbc" uri="urn:x"/>
   <svrl:metadata xmlns:dct="http://purl.org/dc/terms/" xmlns:skos="http://www.w3.org/2004/02/skos/core#">
      <dct:creator><dct:Agent><skos:prefLabel>SAXON/HE 12.10</skos:prefLabel></dct:Agent></dct:creator>
      <dct:created>{created}</dct:created>
   </svrl:metadata>
   <svrl:active-pattern documents="file:/w/c1.xml" id="p" name="p"/>
   <svrl:fired-rule context="/"/>
</svrl:schematron-output>
"#
        )
    }

    #[test]
    fn schxslt_metadata_is_kept_apart() {
        let (canonical, metadata) = split(
            &report(FIXED_NOW),
            Generator::SchXslt,
            &|uri| uri == "file:/w/c1.xml",
            true,
        )
        .unwrap();
        assert_eq!(
            canonical,
            "svrl-canonical 1\nschematron-output title=\"t\"\nns-prefix-in-attribute-values prefix=\"cbc\" uri=\"urn:x\"\nactive-pattern documents=\"{document}\" id=\"p\" name=\"p\"\nfired-rule context=\"/\"\n"
        );
        assert_eq!(
            metadata.unwrap(),
            "{http://purl.oclc.org/dsdl/svrl}metadata\n  {http://purl.org/dc/terms/}creator\n    {http://purl.org/dc/terms/}Agent\n      {http://www.w3.org/2004/02/skos/core#}prefLabel \"SAXON/HE 12.10\"\n  {http://purl.org/dc/terms/}created \"2000-01-01T00:00:00Z\"\n"
        );
    }

    #[test]
    fn anything_else_is_refused() {
        let uri = |_: &str| true;
        let error = split(
            &report("2026-11-08T10:00:00Z"),
            Generator::SchXslt,
            &uri,
            true,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("not the fixed clock"), "{error}");
        assert!(
            split(
                &report("2026-11-08T10:00:00Z"),
                Generator::SchXslt,
                &uri,
                false
            )
            .is_ok()
        );
        let error = split(&report(FIXED_NOW), Generator::IsoSkeleton, &uri, true)
            .unwrap_err()
            .to_string();
        assert!(error.contains("ISO Schematron skeleton"), "{error}");
        let without = report(FIXED_NOW).replace("svrl:metadata", "svrl:text");
        assert!(split(&without, Generator::SchXslt, &uri, true).is_err());
    }
}
