//! The comparison with `roxmltree`: the event stream of the reader and a stream
//! derived from `roxmltree`'s tree, as lines of text, or the error of each.

#![allow(dead_code, reason = "each test uses part of this module")]

pub(crate) mod cases;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use einvoice_xml::{Error, Event, Reader};

/// What a parser makes of a text: its stream, or its error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    Stream(Vec<String>),
    Error(Failure),
}

/// An error: message, row, column, and whether it is one that only this reader
/// reports (a construct that `roxmltree` accepts against the XML specification).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) message: String,
    pub(crate) row: u32,
    pub(crate) col: u32,
    pub(crate) strict: bool,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} [{}:{}]", self.message, self.row, self.col)
    }
}

/// Whether an error is one of those that `roxmltree` never reports.
pub(crate) fn is_strict(error: &Error) -> bool {
    matches!(
        error,
        Error::InvalidCharacterReference(_)
            | Error::InvalidPiTarget(_)
            | Error::PiWithoutWhitespace(_)
            | Error::InvalidXmlDeclaration(..)
            | Error::InvalidQName(_)
            | Error::EmptyPrefixedNamespace(..)
            | Error::XmlnsPrefixDeclared(_)
            | Error::DuplicatedNamespaceDeclaration(..)
            | Error::PrefixedXmlnsAttribute(..)
            | Error::DepthLimitReached
    )
}

fn ns(namespace: Option<&str>) -> &str {
    namespace.unwrap_or("")
}

pub(crate) fn element_line(
    namespace: Option<&str>,
    prefix: &str,
    local: &str,
    attributes: &[(String, String, String, String)],
    in_scope: &[(String, String)],
) -> String {
    let mut line = format!("<{{{}}}{local} prefix={prefix:?}", ns(namespace));
    for (namespace, prefix, local, value) in attributes {
        let _ = write!(
            line,
            " @{{{namespace}}}{local} prefix={prefix:?} ={value:?}"
        );
    }
    for (prefix, uri) in in_scope {
        let _ = write!(line, " xmlns[{prefix}]={uri:?}");
    }
    line
}

/// The reader's stream: adjacent text events merged, empty text dropped, the
/// namespaces in scope of each element computed from the declarations.
pub(crate) fn ours(text: &str) -> Outcome {
    ours_with(&mut Reader::new(text))
}

pub(crate) fn ours_with(reader: &mut Reader<'_>) -> Outcome {
    let mut lines = Vec::new();
    let mut pending = String::new();
    let mut scopes: Vec<Vec<(String, String)>> = vec![Vec::new()];
    let flush = |pending: &mut String, lines: &mut Vec<String>| {
        if !pending.is_empty() {
            lines.push(format!("T{pending:?}"));
            pending.clear();
        }
    };
    loop {
        let event = match reader.next_event() {
            Ok(Some(event)) => event,
            Ok(None) => break,
            Err(e) => return Outcome::Error(failure(&e)),
        };
        match event {
            Event::Text(text) => pending.push_str(&text.value),
            Event::Start(start) => {
                flush(&mut pending, &mut lines);
                let mut scope = scopes.last().cloned().unwrap_or_default();
                for (prefix, uri) in start.namespace_declarations() {
                    scope.retain(|(p, _)| p != prefix);
                    scope.push((prefix.to_owned(), uri.to_owned()));
                }
                scope.sort();
                let attributes: Vec<_> = start
                    .attributes()
                    .map(|a| {
                        (
                            ns(a.name.namespace).to_owned(),
                            a.name.prefix.to_owned(),
                            a.name.local.to_owned(),
                            a.value.to_string(),
                        )
                    })
                    .collect();
                let name = start.name();
                lines.push(element_line(
                    name.namespace,
                    name.prefix,
                    name.local,
                    &attributes,
                    &scope,
                ));
                scopes.push(scope);
            }
            Event::End(end) => {
                flush(&mut pending, &mut lines);
                let name = end.name();
                lines.push(format!("</{{{}}}{}>", ns(name.namespace), name.local));
                scopes.pop();
            }
            Event::Comment(comment) => {
                flush(&mut pending, &mut lines);
                lines.push(format!("C{:?}", comment.value));
            }
            Event::ProcessingInstruction(pi) => {
                flush(&mut pending, &mut lines);
                lines.push(format!("P{} {:?}", pi.target, pi.value.as_deref()));
            }
        }
    }
    Outcome::Stream(lines)
}

pub(crate) fn failure(e: &Error) -> Failure {
    Failure {
        message: e.to_string(),
        row: e.pos().row,
        col: e.pos().col,
        strict: is_strict(e),
    }
}

/// `\r\n` and lone `\r` as `\n`.
pub(crate) fn line_ends(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// Whether the text starts (after a byte order mark) with `<?xml` and a whitespace
/// other than a space: an XML declaration for the reader, a processing instruction
/// for `roxmltree`.
pub(crate) fn declaration_without_space(text: &str) -> bool {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    text.strip_prefix("<?xml")
        .is_some_and(|rest| rest.starts_with(['\t', '\n', '\r']))
}

pub(crate) fn roxmltree_options<'a>() -> roxmltree::ParsingOptions<'a> {
    roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: u32::MAX,
        ..roxmltree::ParsingOptions::default()
    }
}

/// `roxmltree`'s stream, with the differences that the crate documents taken out:
/// comments and processing instructions get their line ends normalised, an empty
/// namespace name is no namespace, and the processing instruction that `<?xml` and a
/// tab or line end starts is dropped.
pub(crate) fn theirs(text: &str) -> Outcome {
    theirs_with(text, roxmltree_options())
}

pub(crate) fn theirs_with<'a>(text: &'a str, options: roxmltree::ParsingOptions<'a>) -> Outcome {
    let doc = match roxmltree::Document::parse_with_options(text, options) {
        Ok(doc) => doc,
        Err(e) => {
            return Outcome::Error(Failure {
                message: e.to_string(),
                row: e.pos().row,
                col: e.pos().col,
                strict: false,
            });
        }
    };
    let input = doc.input_text();
    let mut lines = Vec::new();
    let mut skip_declaration = declaration_without_space(text);
    let close = |node: roxmltree::Node<'_, '_>, lines: &mut Vec<String>| {
        lines.push(format!(
            "</{{{}}}{}>",
            ns(node.tag_name().namespace().filter(|n| !n.is_empty())),
            node.tag_name().name()
        ));
    };
    // The elements open, innermost last: closed when the walk in document order
    // reaches a node outside them.
    let mut open: Vec<roxmltree::Node<'_, '_>> = Vec::new();
    for node in doc.root().descendants().skip(1) {
        while let Some(&top) = open.last() {
            if node.parent() == Some(top) {
                break;
            }
            open.pop();
            close(top, &mut lines);
        }
        match node.node_type() {
            roxmltree::NodeType::Root => {}
            roxmltree::NodeType::Element => {
                let prefix = qname_prefix(&input[node.range().start + 1..]);
                let attributes: Vec<_> = node
                    .attributes()
                    .map(|a| {
                        (
                            ns(a.namespace().filter(|n| !n.is_empty())).to_owned(),
                            qname_prefix(&input[a.range_qname()]).to_owned(),
                            a.name().to_owned(),
                            a.value().to_owned(),
                        )
                    })
                    .collect();
                let mut scope: Vec<(String, String)> = node
                    .namespaces()
                    .map(|n| (n.name().unwrap_or("").to_owned(), n.uri().to_owned()))
                    .collect();
                scope.sort();
                lines.push(element_line(
                    node.tag_name().namespace().filter(|n| !n.is_empty()),
                    prefix,
                    node.tag_name().name(),
                    &attributes,
                    &scope,
                ));
                open.push(node);
            }
            roxmltree::NodeType::Text => {
                let value = node.text().unwrap_or_default();
                if !value.is_empty() {
                    lines.push(format!("T{value:?}"));
                }
            }
            roxmltree::NodeType::Comment => {
                lines.push(format!("C{:?}", line_ends(node.text().unwrap_or_default())));
            }
            roxmltree::NodeType::PI => {
                let Some(pi) = node.pi() else { continue };
                if skip_declaration && pi.target == "xml" {
                    skip_declaration = false;
                    continue;
                }
                lines.push(format!("P{} {:?}", pi.target, pi.value.map(line_ends)));
            }
        }
    }
    while let Some(top) = open.pop() {
        close(top, &mut lines);
    }
    Outcome::Stream(lines)
}

/// The prefix of the qualified name at the start of `s`, `""` for none.
fn qname_prefix(s: &str) -> &str {
    let end = s
        .find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>' || c == '=')
        .unwrap_or(s.len());
    s[..end].split_once(':').map_or("", |(prefix, _)| prefix)
}

/// The first line where two streams differ, for a message.
pub(crate) fn first_difference(a: &[String], b: &[String]) -> String {
    let i = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    format!(
        "line {i}: ours {:?}, roxmltree {:?}",
        a.get(i).map(|s| truncate(s)),
        b.get(i).map(|s| truncate(s))
    )
}

fn truncate(s: &str) -> String {
    if s.chars().count() > 300 {
        format!("{}...", s.chars().take(300).collect::<String>())
    } else {
        s.to_owned()
    }
}

/// The exact comparison: the reader and `roxmltree` must give the same stream, or
/// the same error.
pub(crate) fn exact(text: &str) -> Result<(), String> {
    match (ours(text), theirs(text)) {
        (Outcome::Stream(a), Outcome::Stream(b)) if a == b => Ok(()),
        (Outcome::Stream(a), Outcome::Stream(b)) => Err(first_difference(&a, &b)),
        (Outcome::Error(a), Outcome::Error(b))
            if a.message == b.message && (a.row, a.col) == (b.row, b.col) =>
        {
            Ok(())
        }
        (a, b) => Err(format!("ours {}, roxmltree {}", describe(&a), describe(&b))),
    }
}

pub(crate) fn describe(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Stream(lines) => format!("accepts ({} lines)", lines.len()),
        Outcome::Error(failure) => format!("refuses: {failure}"),
    }
}

/// The comparison on any text, which may contain the constructs where the reader
/// differs from `roxmltree` by design:
///
/// * whatever `roxmltree` refuses, the reader refuses;
/// * when both refuse, the same error, unless the reader's is one that `roxmltree`
///   never reports (raised for a construct it accepts) or the text starts with an XML
///   declaration that `roxmltree` reads as a processing instruction;
/// * when `roxmltree` accepts and the reader refuses, the reader's error is one that
///   `roxmltree` never reports (or the same declaration case);
/// * when both accept, the same stream, except text where a carriage return is
///   written next to a reference.
pub(crate) fn invariants(text: &str) -> Result<(), String> {
    invariants_of(text, &ours(text), &theirs(text))
}

/// [`invariants`] on outcomes already computed.
pub(crate) fn invariants_of(text: &str, ours: &Outcome, theirs: &Outcome) -> Result<(), String> {
    let declaration = declaration_without_space(text);
    match (ours, theirs) {
        (Outcome::Stream(_), Outcome::Error(b)) => {
            Err(format!("roxmltree refuses ({b}), the reader accepts"))
        }
        (Outcome::Error(a), Outcome::Error(b)) => {
            if a.strict
                || declaration
                || (a.message == b.message && (a.row, a.col) == (b.row, b.col))
            {
                Ok(())
            } else {
                Err(format!("ours {a}, roxmltree {b}"))
            }
        }
        (Outcome::Error(a), Outcome::Stream(_)) => {
            if a.strict || declaration {
                Ok(())
            } else {
                Err(format!("roxmltree accepts, ours {a}"))
            }
        }
        (Outcome::Stream(a), Outcome::Stream(b)) => {
            if a == b {
                return Ok(());
            }
            let carriage_return_by_reference = text.contains("\r&") || text.contains(";\r");
            if a.len() == b.len()
                && carriage_return_by_reference
                && a.iter()
                    .zip(b)
                    .all(|(x, y)| x == y || (x.starts_with('T') && y.starts_with('T')))
            {
                return Ok(());
            }
            Err(first_difference(a, b))
        }
    }
}

/// The workspace root.
pub(crate) fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every XML file under `dir` (by extension), sorted.
pub(crate) fn xml_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                ["xml", "xsl", "xslt", "xsd", "sch", "svrl"]
                    .contains(&e.to_ascii_lowercase().as_str())
            }) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}
