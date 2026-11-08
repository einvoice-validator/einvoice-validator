//! Canonical, line-oriented form of an SVRL report (the Schematron Validation Report
//! Language of ISO/IEC 19757-3). The format is specified in `corpus/SVRL-CANONICAL.md` of
//! the repository; in short:
//!
//! - the first line is `svrl-canonical 1`, or `svrl-canonical 2` for a report with an
//!   `svrl:metadata` element (SchXslt writes one), which version 2 represents: its
//!   descendants, outside the SVRL namespace, one line each, named `{namespace}local`, the
//!   time of the run written `{run-time}`;
//! - then one line per SVRL element and comment, in document order:
//!   `<indent><kind>( <attribute>="<value>")*( "<text>")?`, the root and its
//!   children at indent 0, their children two spaces further in;
//! - attributes sorted by name; values verbatim (escaped), except the URI of
//!   the validated document in `active-pattern/@documents`, which is written
//!   `{document}`;
//! - text content and comments whitespace normalised (XPath `normalize-space`).
//!
//! Anything the format cannot represent faithfully (an element outside the
//! SVRL namespace, mixed content, a processing instruction, ...) is an error:
//! nothing is dropped silently.
//!
//! Two engines agree on a document when their canonical forms are identical.
//!
//! ```
//! let svrl = r#"<svrl:schematron-output xmlns:svrl="http://purl.oclc.org/dsdl/svrl" title="T">
//!   <svrl:active-pattern documents="file:/tmp/i.xml" id="P"/>
//!   <svrl:failed-assert id="R-1" flag="fatal" location="/" test="x"><svrl:text> m </svrl:text></svrl:failed-assert>
//! </svrl:schematron-output>"#;
//! let canonical = einvoice_svrl::canonicalize(svrl, &|uri| einvoice_svrl::is_document_uri(uri, "/tmp/i.xml"))?;
//! assert_eq!(canonical, "svrl-canonical 1\nschematron-output title=\"T\"\n\
//!     active-pattern documents=\"{document}\" id=\"P\"\n\
//!     failed-assert flag=\"fatal\" id=\"R-1\" location=\"/\" test=\"x\"\n  text \"m\"\n");
//! # Ok::<(), einvoice_svrl::Error>(())
//! ```

use std::fmt::Write as _;

use roxmltree::{Document, Node, NodeType, ParsingOptions};

/// A report the canonical form cannot represent, or a canonical file that does not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// The explanation.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// The result of the functions of this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// XPath `normalize-space`: strips leading and trailing XML whitespace
/// (space, tab, CR, LF) and collapses inner runs to one space. Other Unicode
/// spaces (no-break space, ...) are content and are kept.
#[must_use]
pub fn normalize_space(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text
        .split([' ', '\t', '\r', '\n'])
        .filter(|w| !w.is_empty())
    {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// The SVRL namespace.
const SVRL_NS: &str = "http://purl.oclc.org/dsdl/svrl";
/// The XML namespace (`xml:lang`, ...).
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
/// First line of a canonical file of version 1: a report without `svrl:metadata`.
pub const HEADER: &str = "svrl-canonical 1";
/// First line of a canonical file of version 2: a report with an `svrl:metadata`.
pub const HEADER_V2: &str = "svrl-canonical 2";
/// Stands for the URI of the validated document.
pub const DOCUMENT_PLACEHOLDER: &str = "{document}";
/// Stands for the time of the run in `svrl:metadata` (version 2).
pub const RUN_TIME_PLACEHOLDER: &str = "{run-time}";
/// The Dublin Core terms namespace (`dct:created`, the time of the run in
/// `svrl:metadata`).
const DCT_NS: &str = "http://purl.org/dc/terms/";

/// SVRL elements that hold other elements.
const CONTAINERS: &[&str] = &[
    "schematron-output",
    "active-pattern",
    "fired-rule",
    "failed-assert",
    "successful-report",
    "suppressed-rule",
    "property-reference",
];
/// SVRL elements that hold text only (or nothing).
const TEXT_HOLDERS: &[&str] = &[
    "ns-prefix-in-attribute-values",
    "text",
    "diagnostic-reference",
];

/// Canonicalises an SVRL report. `document_uri` decides which `documents`
/// tokens are the validated document (see [`is_document_uri`]).
///
/// # Errors
///
/// The text is not well-formed XML, or the report holds something the canonical form
/// cannot represent (see the crate documentation).
pub fn canonicalize(svrl: &str, document_uri: &dyn Fn(&str) -> bool) -> Result<String> {
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: u32::MAX,
        ..ParsingOptions::default()
    };
    let doc = Document::parse_with_options(svrl, options)
        .map_err(|e| Error::new(format!("SVRL: {e}")))?;
    let root = doc.root_element();
    if svrl_name(root)? != "schematron-output" {
        return Err(Error::new("SVRL: the root is not svrl:schematron-output"));
    }
    for node in doc.root().children() {
        check_outside_root(node)?;
    }
    let metadata = metadata_elements(root);
    if metadata.len() > 1 {
        return Err(Error::new("SVRL: more than one svrl:metadata"));
    }
    let mut out = String::new();
    out.push_str(if metadata.is_empty() {
        HEADER
    } else {
        HEADER_V2
    });
    out.push('\n');
    element(root, 0, 1, document_uri, &mut out)?;
    Ok(out)
}

/// The `svrl:metadata` children of the root.
fn metadata_elements<'a, 'input>(root: Node<'a, 'input>) -> Vec<Node<'a, 'input>> {
    root.children()
        .filter(|n| {
            n.is_element()
                && n.tag_name().namespace() == Some(SVRL_NS)
                && n.tag_name().name() == "metadata"
        })
        .collect()
}

/// The time of the run that a report's `svrl:metadata` records (its `dct:created`
/// child, which [`canonicalize`] writes [`RUN_TIME_PLACEHOLDER`]), whitespace trimmed;
/// `None` for a report without `svrl:metadata`.
///
/// # Errors
///
/// The text is not well-formed XML, the report has more than one `svrl:metadata`, or
/// its `svrl:metadata` has not exactly one `dct:created`.
pub fn run_time(svrl: &str) -> Result<Option<String>> {
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: u32::MAX,
        ..ParsingOptions::default()
    };
    let doc = Document::parse_with_options(svrl, options)
        .map_err(|e| Error::new(format!("SVRL: {e}")))?;
    let metadata = metadata_elements(doc.root_element());
    match metadata.as_slice() {
        [] => Ok(None),
        [metadata] => run_time_of(*metadata).map(|(_, time)| Some(time)),
        _ => Err(Error::new("SVRL: more than one svrl:metadata")),
    }
}

/// The `dct:created` child of `svrl:metadata` and its text, trimmed.
fn run_time_of<'a, 'input>(metadata: Node<'a, 'input>) -> Result<(Node<'a, 'input>, String)> {
    let created: Vec<Node<'_, '_>> = metadata
        .children()
        .filter(|n| {
            n.is_element()
                && n.tag_name().namespace() == Some(DCT_NS)
                && n.tag_name().name() == "created"
        })
        .collect();
    let [created] = created.as_slice() else {
        return Err(Error::new(
            "SVRL: svrl:metadata without exactly one dct:created (the time of the run)",
        ));
    };
    let text: String = created.children().filter_map(|c| c.text()).collect();
    Ok((
        *created,
        text.trim_matches([' ', '\t', '\r', '\n']).to_owned(),
    ))
}

/// Whether `text` is the lexical form of an `xs:dateTime` with a timezone, as
/// `fn:current-dateTime()` writes it: `YYYY-MM-DDThh:mm:ss`, optional fractional
/// seconds, then `Z` or `+hh:mm`/`-hh:mm`.
fn is_date_time_with_timezone(text: &str) -> bool {
    let b = text.as_bytes();
    let digits = |range: std::ops::Range<usize>| {
        b.get(range)
            .is_some_and(|d| d.iter().all(u8::is_ascii_digit))
    };
    let fixed = b.len() >= 20
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && b[10] == b'T'
        && digits(11..13)
        && b[13] == b':'
        && digits(14..16)
        && b[16] == b':'
        && digits(17..19);
    if !fixed {
        return false;
    }
    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let n = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 {
            return false;
        }
        rest = &fraction[n..];
    }
    rest == "Z"
        || (rest.len() == 6
            && matches!(rest.as_bytes()[0], b'+' | b'-')
            && rest.as_bytes()[1..3].iter().all(u8::is_ascii_digit)
            && rest.as_bytes()[3] == b':'
            && rest.as_bytes()[4..6].iter().all(u8::is_ascii_digit))
}

/// The lines of `svrl:metadata` (version 2): `metadata` with its attributes, then each
/// descendant element one level further in, `{namespace}local`, its attributes sorted
/// (`{namespace}name` for a namespaced one), and its text (normalised) when it has no
/// element child. The time of the run, the `dct:created` child, is written
/// [`RUN_TIME_PLACEHOLDER`]. Comments, processing instructions and mixed content are
/// refused. The descendants are walked with an explicit stack.
fn metadata_lines(metadata: Node<'_, '_>, out: &mut String) -> Result<()> {
    let (run_time_node, run_time) = run_time_of(metadata)?;
    if !is_date_time_with_timezone(&run_time) {
        return Err(Error::new(format!(
            "SVRL: the time of the run in svrl:metadata is not a dateTime with a timezone: {run_time:?}"
        )));
    }
    let mut pending: Vec<(Node<'_, '_>, usize)> = vec![(metadata, 0)];
    while let Some((node, depth)) = pending.pop() {
        if depth >= MAX_LEVEL {
            return Err(Error::new(format!(
                "SVRL: svrl:metadata nested more than {MAX_LEVEL} deep at byte {}",
                node.range().start
            )));
        }
        push_indent(out, depth);
        if node == metadata {
            out.push_str("metadata");
        } else {
            let _ = write!(
                out,
                "{{{}}}{}",
                node.tag_name().namespace().unwrap_or(""),
                node.tag_name().name()
            );
        }
        let mut attributes: Vec<(String, String)> = node
            .attributes()
            .map(|a| {
                let key = match a.namespace() {
                    None => a.name().to_owned(),
                    Some(XML_NS) => format!("xml:{}", a.name()),
                    Some(ns) => format!("{{{ns}}}{}", a.name()),
                };
                (key, a.value().to_owned())
            })
            .collect();
        attributes.sort();
        for (key, value) in &attributes {
            out.push(' ');
            out.push_str(key);
            out.push('=');
            push_quoted(out, value);
        }
        let mut text = String::new();
        let mut elements = Vec::new();
        for child in node.children() {
            match child.node_type() {
                NodeType::Element => elements.push(child),
                NodeType::Text => text.push_str(child.text().unwrap_or("")),
                NodeType::Comment | NodeType::PI | NodeType::Root => {
                    return Err(Error::new(format!(
                        "SVRL: a comment or processing instruction in svrl:metadata at byte {}",
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
        if elements.is_empty() && node != metadata {
            out.push(' ');
            push_quoted(
                out,
                if node == run_time_node {
                    RUN_TIME_PLACEHOLDER
                } else {
                    &text
                },
            );
        }
        out.push('\n');
        pending.extend(elements.into_iter().rev().map(|e| (e, depth + 1)));
    }
    Ok(())
}

fn check_outside_root(node: Node<'_, '_>) -> Result<()> {
    match node.node_type() {
        NodeType::Element | NodeType::Comment => Ok(()),
        NodeType::Text if node.text().is_some_and(|t| t.trim().is_empty()) => Ok(()),
        _ => Err(Error::new(format!(
            "SVRL: unexpected content outside the root at byte {}",
            node.range().start
        ))),
    }
}

/// The deepest nesting of SVRL elements read: an SVRL report nests three deep
/// (`svrl:schematron-output`, `svrl:failed-assert`, `svrl:text`), and anything deeper is
/// refused before the recursion over it could exhaust the stack.
const MAX_LEVEL: usize = 16;

/// The canonical lines of `node`, an element `level` deep (the root is at level 1),
/// indented `depth` steps.
fn element(
    node: Node<'_, '_>,
    depth: usize,
    level: usize,
    document_uri: &dyn Fn(&str) -> bool,
    out: &mut String,
) -> Result<()> {
    if level > MAX_LEVEL {
        return Err(Error::new(format!(
            "SVRL: elements nested more than {MAX_LEVEL} deep at byte {}",
            node.range().start
        )));
    }
    let name = svrl_name(node)?;
    if name == "metadata" && level == 2 {
        return metadata_lines(node, out);
    }
    let is_container = CONTAINERS.contains(&name);
    if !is_container && !TEXT_HOLDERS.contains(&name) {
        return Err(Error::new(format!(
            "SVRL: unknown element svrl:{name} at byte {}",
            node.range().start
        )));
    }

    push_indent(out, depth);
    out.push_str(name);
    let mut attributes: Vec<(String, String)> = Vec::new();
    for attribute in node.attributes() {
        let key = match attribute.namespace() {
            None => attribute.name().to_owned(),
            Some(XML_NS) => format!("xml:{}", attribute.name()),
            Some(ns) => format!("{{{ns}}}{}", attribute.name()),
        };
        let value = if name == "active-pattern" && key == "documents" {
            documents(attribute.value(), document_uri)?
        } else {
            attribute.value().to_owned()
        };
        attributes.push((key, value));
    }
    attributes.sort();
    for (key, value) in &attributes {
        out.push(' ');
        out.push_str(key);
        out.push('=');
        push_quoted(out, value);
    }

    // The root's children share its depth: everything an SVRL report says is
    // a child of the root, and indenting all of it would be noise.
    let child_depth = if name == "schematron-output" {
        depth
    } else {
        depth + 1
    };
    let mut text = String::new();
    let mut has_elements = false;
    let mut lines = String::new();
    for child in node.children() {
        match child.node_type() {
            NodeType::Element => {
                has_elements = true;
                element(child, child_depth, level + 1, document_uri, &mut lines)?;
            }
            NodeType::Comment => {
                push_indent(&mut lines, child_depth);
                lines.push_str("comment ");
                push_quoted(&mut lines, &normalize_space(child.text().unwrap_or("")));
                lines.push('\n');
            }
            NodeType::Text => text.push_str(child.text().unwrap_or("")),
            NodeType::PI | NodeType::Root => {
                return Err(Error::new(format!(
                    "SVRL: processing instruction in svrl:{name} at byte {}",
                    child.range().start
                )));
            }
        }
    }
    let text = normalize_space(&text);
    if is_container && !text.is_empty() {
        return Err(Error::new(format!(
            "SVRL: text directly in svrl:{name} at byte {}",
            node.range().start
        )));
    }
    if !is_container && has_elements {
        return Err(Error::new(format!(
            "SVRL: markup inside svrl:{name} at byte {} (not representable)",
            node.range().start
        )));
    }
    // A text holder always shows its text, even empty, so that an empty
    // message is visible as such.
    if !is_container && name != "ns-prefix-in-attribute-values" {
        out.push(' ');
        push_quoted(out, &text);
    } else if !text.is_empty() {
        return Err(Error::new(format!(
            "SVRL: text in svrl:{name} at byte {}",
            node.range().start
        )));
    }
    out.push('\n');
    out.push_str(&lines);
    Ok(())
}

/// `active-pattern/@documents`: a list of URIs; the validated document's own
/// URI becomes the placeholder, any other URI is an error (it would leak a
/// machine-specific path into the golden).
fn documents(value: &str, document_uri: &dyn Fn(&str) -> bool) -> Result<String> {
    let tokens: Vec<&str> = value.split_ascii_whitespace().collect();
    let mut out = Vec::with_capacity(tokens.len());
    for token in tokens {
        if document_uri(token) {
            out.push(DOCUMENT_PLACEHOLDER);
        } else {
            return Err(Error::new(format!(
                "SVRL: active-pattern/@documents names another document: {token}"
            )));
        }
    }
    Ok(out.join(" "))
}

/// True when `uri` (a `file:` URI, possibly percent-encoded) designates the
/// local file `path` (absolute, `/` or `\` separated).
#[must_use]
pub fn is_document_uri(uri: &str, path: &str) -> bool {
    let Some(rest) = uri.strip_prefix("file:") else {
        return false;
    };
    let Some(decoded) = percent_decode(rest) else {
        return false;
    };
    // file:/a, file:///a and file://localhost/a are the same file.
    let decoded = decoded
        .strip_prefix("//localhost")
        .or_else(|| decoded.strip_prefix("//"))
        .unwrap_or(&decoded);
    let path = path.replace('\\', "/");
    // Windows: file:/C:/x for C:\x.
    decoded == path || decoded.strip_prefix('/') == Some(path.as_str())
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn svrl_name<'a>(node: Node<'a, '_>) -> Result<&'a str> {
    if node.tag_name().namespace() == Some(SVRL_NS) {
        Ok(node.tag_name().name())
    } else {
        Err(Error::new(format!(
            "SVRL: element {{{}}}{} outside the SVRL namespace at byte {}",
            node.tag_name().namespace().unwrap_or(""),
            node.tag_name().name(),
            node.range().start
        )))
    }
}

fn push_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// A double-quoted string: `\` `"` and control characters escaped (`\n`,
/// `\r`, `\t`), and characters that would be invisible or ambiguous in a diff
/// written `\u{hex}` (see [`is_invisible`]); everything else verbatim.
fn push_quoted(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if is_invisible(c) => {
                let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Control characters, whitespace other than the plain space (no-break
/// space, ...), zero-width characters, the byte order mark and the soft
/// hyphen.
fn is_invisible(c: char) -> bool {
    c.is_control()
        || (c.is_whitespace() && c != ' ')
        || matches!(
            c,
            '\u{ad}' | '\u{200b}'..='\u{200f}' | '\u{2060}' | '\u{feff}'
        )
}

/// One line of a canonical file, parsed back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// The nesting level (the indentation divided by two).
    pub depth: usize,
    /// The SVRL element's local name, or `comment`.
    pub kind: String,
    /// The attributes, sorted by name, unescaped.
    pub attributes: Vec<(String, String)>,
    /// The text, unescaped, if the line has one.
    pub text: Option<String>,
}

impl Line {
    /// The value of the attribute `name`, if present.
    #[must_use]
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Parses a canonical file of either version back into lines (the header excluded).
/// The lines of `svrl:metadata` (version 2) are kept: kind `metadata`, then
/// `{namespace}local` for its descendants.
///
/// # Errors
///
/// The first line is not a header, or a line is not in the canonical syntax.
pub fn parse(canonical: &str) -> Result<Vec<Line>> {
    let mut lines = canonical.lines();
    if !matches!(lines.next(), Some(HEADER | HEADER_V2)) {
        return Err(Error::new(format!(
            "canonical SVRL: first line is not `{HEADER}` or `{HEADER_V2}`"
        )));
    }
    lines
        .enumerate()
        .map(|(index, line)| {
            parse_line(line).map_err(|e| Error::new(format!("line {}: {e}", index + 2)))
        })
        .collect()
}

fn parse_line(line: &str) -> Result<Line> {
    let body = line.trim_start_matches(' ');
    let indent = line.len() - body.len();
    if !indent.is_multiple_of(2) {
        return Err(Error::new("odd indentation"));
    }
    let (kind, mut rest) = body.split_once(' ').unwrap_or((body, ""));
    let mut attributes = Vec::new();
    let mut text = None;
    while !rest.is_empty() {
        if text.is_some() {
            return Err(Error::new("content after the text"));
        }
        if rest.starts_with('"') {
            let (value, after) = unquote(rest)?;
            text = Some(value);
            rest = after;
        } else {
            let (key, after) = rest
                .split_once('=')
                .ok_or_else(|| Error::new("expected name=\"value\""))?;
            let (value, after) = unquote(after)?;
            attributes.push((key.to_owned(), value));
            rest = after;
        }
        rest = match rest.strip_prefix(' ') {
            Some(r) if !r.is_empty() => r,
            Some(_) => return Err(Error::new("trailing space")),
            None if rest.is_empty() => rest,
            None => return Err(Error::new("expected a space between items")),
        };
    }
    Ok(Line {
        depth: indent / 2,
        kind: kind.to_owned(),
        attributes,
        text,
    })
}

/// Reads a quoted string at the start of `text`; returns it and the rest.
fn unquote(text: &str) -> Result<(String, &str)> {
    let mut chars = text.char_indices();
    if chars.next().map(|(_, c)| c) != Some('"') {
        return Err(Error::new("expected a quoted string"));
    }
    let mut value = String::new();
    while let Some((index, c)) = chars.next() {
        match c {
            '"' => return Ok((value, &text[index + 1..])),
            '\\' => match chars.next().map(|(_, c)| c) {
                Some('\\') => value.push('\\'),
                Some('"') => value.push('"'),
                Some('n') => value.push('\n'),
                Some('r') => value.push('\r'),
                Some('t') => value.push('\t'),
                Some('u') => {
                    let rest = &text[index + 2..];
                    let close = rest.find('}').ok_or_else(|| Error::new("bad \\u escape"))?;
                    let hex = rest
                        .get(1..close)
                        .filter(|_| rest.starts_with('{'))
                        .ok_or_else(|| Error::new("bad \\u escape"))?;
                    let code =
                        u32::from_str_radix(hex, 16).map_err(|_| Error::new("bad \\u escape"))?;
                    value.push(char::from_u32(code).ok_or_else(|| Error::new("bad \\u escape"))?);
                    // Skip `{hex}`.
                    for _ in 0..=close {
                        chars.next();
                    }
                }
                _ => return Err(Error::new("bad escape")),
            },
            c => value.push(c),
        }
    }
    Err(Error::new("unterminated quoted string"))
}

#[cfg(test)]
mod tests {
    use super::{HEADER, HEADER_V2, canonicalize, is_document_uri, parse, run_time};

    const SVRL: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<svrl:schematron-output xmlns:svrl="http://purl.oclc.org/dsdl/svrl" title="T  t" schemaVersion=""><!--
  c   -->
   <svrl:ns-prefix-in-attribute-values prefix="cbc" uri="urn:cbc"/>
   <svrl:active-pattern documents="file:/tmp/a%20b/c1.xml" id="P" name="P"/>
   <svrl:fired-rule context="/a | /b"/>
   <svrl:failed-assert test="x &lt; 1&#xA;and &quot;y&quot;" id="BR-1" flag="fatal" location="/*:a[1]">
      <svrl:text xml:lang="en">[BR-1]-Two
        lines\ </svrl:text>
   </svrl:failed-assert>
   <svrl:successful-report test="1" id="R" flag="warning" location="/"><svrl:text/></svrl:successful-report>
</svrl:schematron-output>
"#;

    fn uri(u: &str) -> bool {
        is_document_uri(u, "/tmp/a b/c1.xml")
    }

    #[test]
    fn canonical_form() {
        let canonical = canonicalize(SVRL, &uri).unwrap();
        let expected = [
            HEADER,
            r#"schematron-output schemaVersion="" title="T  t""#,
            r#"comment "c""#,
            r#"ns-prefix-in-attribute-values prefix="cbc" uri="urn:cbc""#,
            r#"active-pattern documents="{document}" id="P" name="P""#,
            r#"fired-rule context="/a | /b""#,
            r#"failed-assert flag="fatal" id="BR-1" location="/*:a[1]" test="x < 1\nand \"y\"""#,
            r#"  text xml:lang="en" "[BR-1]-Two lines\\""#,
            r#"successful-report flag="warning" id="R" location="/" test="1""#,
            r#"  text """#,
            "",
        ]
        .join("\n");
        assert_eq!(canonical, expected);
    }

    #[test]
    fn parses_back() {
        let lines = parse(&canonicalize(SVRL, &uri).unwrap()).unwrap();
        assert_eq!(lines.len(), 9);
        let assert = &lines[5];
        assert_eq!(assert.kind, "failed-assert");
        assert_eq!(assert.depth, 0);
        assert_eq!(assert.attribute("test"), Some("x < 1\nand \"y\""));
        assert_eq!(assert.attribute("id"), Some("BR-1"));
        let text = &lines[6];
        assert_eq!(text.depth, 1);
        assert_eq!(text.text.as_deref(), Some("[BR-1]-Two lines\\"));
        assert_eq!(text.attribute("xml:lang"), Some("en"));
    }

    #[test]
    fn control_characters_round_trip() {
        let svrl = r#"<svrl:schematron-output xmlns:svrl="http://purl.oclc.org/dsdl/svrl" t="a&#x7F;b&#x9;&#xA0;&#x200B;é"><svrl:active-pattern><svrl:fired-rule/></svrl:active-pattern><svrl:failed-assert><svrl:text>x&#160; </svrl:text></svrl:failed-assert></svrl:schematron-output>"#;
        let canonical = canonicalize(svrl, &uri).unwrap();
        assert!(
            canonical.contains(r#"t="a\u{7f}b\t\u{a0}\u{200b}é""#),
            "{canonical}"
        );
        // normalize-space keeps the no-break space; it is shown escaped.
        assert!(canonical.ends_with("  text \"x\\u{a0}\"\n"), "{canonical}");
        let lines = parse(&canonical).unwrap();
        assert_eq!(lines[0].attribute("t"), Some("a\u{7f}b\t\u{a0}\u{200b}é"));
        assert_eq!(lines[4].text.as_deref(), Some("x\u{a0}"));
        // Nesting below the root's children is indented.
        assert_eq!(
            lines.iter().map(|l| l.depth).collect::<Vec<_>>(),
            [0, 0, 1, 0, 1]
        );
    }

    #[test]
    fn refuses_what_it_cannot_represent() {
        let other_document = SVRL.replace("file:/tmp/a%20b/c1.xml", "file:/tmp/other.xml");
        assert!(canonicalize(&other_document, &uri).is_err());
        let markup_in_text = SVRL.replace("lines\\ ", "<svrl:emph>x</svrl:emph>");
        assert!(canonicalize(&markup_in_text, &uri).is_err());
        let unknown = SVRL.replace("svrl:fired-rule", "svrl:fired-rules");
        assert!(canonicalize(&unknown, &uri).is_err());
        let foreign = SVRL.replace(
            "<svrl:fired-rule context=\"/a | /b\"/>",
            "<x:y xmlns:x=\"urn:x\"/>",
        );
        assert!(canonicalize(&foreign, &uri).is_err());
    }

    /// SVRL elements nested deeply (a stylesheet copying a deep document into its
    /// report) are refused, not recursed over: on a 2 MiB thread, at 100 000 levels.
    #[test]
    fn deep_nesting_is_refused() {
        let nested = |n: usize| {
            SVRL.replace(
                "<svrl:fired-rule context=\"/a | /b\"/>",
                &format!(
                    "{}{}",
                    "<svrl:fired-rule>".repeat(n),
                    "</svrl:fired-rule>".repeat(n)
                ),
            )
        };
        // The root and 15 levels below it: within the bound.
        assert!(canonicalize(&nested(15), &uri).is_ok());
        let error = canonicalize(&nested(16), &uri).unwrap_err();
        assert!(
            error
                .message()
                .starts_with("SVRL: elements nested more than 16 deep"),
            "{error}"
        );
        let deep = nested(100_000);
        let error = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || canonicalize(&deep, &uri).map(drop))
            .unwrap()
            .join()
            .unwrap()
            .unwrap_err();
        assert!(error.message().contains("nested more than 16"), "{error}");
    }

    fn schxslt(created: &str) -> String {
        SVRL.replace(
            "<svrl:ns-prefix-in-attribute-values prefix=\"cbc\" uri=\"urn:cbc\"/>",
            &format!(
                r#"<svrl:ns-prefix-in-attribute-values prefix="cbc" uri="urn:cbc"/>
   <svrl:metadata xmlns:dct="http://purl.org/dc/terms/" xmlns:skos="http://www.w3.org/2004/02/skos/core#" xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
      <dct:creator><dct:Agent><skos:prefLabel>SAXON/HE 12.10</skos:prefLabel></dct:Agent></dct:creator>
      <dct:created>{created}</dct:created>
      <dct:source><rdf:Description rdf:about="x"><dct:created>2026-09-02T11:54:24.1Z</dct:created></rdf:Description></dct:source>
   </svrl:metadata>"#
            ),
        )
    }

    #[test]
    fn version_2_represents_the_metadata() {
        let canonical = canonicalize(&schxslt(" 2000-01-01T00:00:00Z "), &uri).unwrap();
        let lines: Vec<&str> = canonical.lines().collect();
        assert_eq!(lines[0], HEADER_V2);
        assert_eq!(
            &lines[4..12],
            [
                "metadata",
                "  {http://purl.org/dc/terms/}creator",
                "    {http://purl.org/dc/terms/}Agent",
                "      {http://www.w3.org/2004/02/skos/core#}prefLabel \"SAXON/HE 12.10\"",
                "  {http://purl.org/dc/terms/}created \"{run-time}\"",
                "  {http://purl.org/dc/terms/}source",
                "    {http://www.w3.org/1999/02/22-rdf-syntax-ns#}Description {http://www.w3.org/1999/02/22-rdf-syntax-ns#}about=\"x\"",
                "      {http://purl.org/dc/terms/}created \"2026-09-02T11:54:24.1Z\"",
            ]
        );
        // The rest is version 1.
        assert_eq!(
            canonical.lines().skip(12).collect::<Vec<_>>(),
            canonicalize(SVRL, &uri)
                .unwrap()
                .lines()
                .skip(4)
                .collect::<Vec<_>>()
        );
        // The time of the run does not change the canonical form.
        assert_eq!(
            canonicalize(&schxslt("2026-11-08T10:00:00.123+01:00"), &uri).unwrap(),
            canonical
        );
        assert_eq!(
            run_time(&schxslt(" 2000-01-01T00:00:00Z "))
                .unwrap()
                .as_deref(),
            Some("2000-01-01T00:00:00Z")
        );
        assert_eq!(run_time(SVRL).unwrap(), None);
        let lines = parse(&canonical).unwrap();
        assert_eq!(lines[3].kind, "metadata");
        assert_eq!(lines[7].text.as_deref(), Some("{run-time}"));
    }

    #[test]
    fn version_2_refuses_what_it_cannot_represent() {
        for bad in [
            schxslt("yesterday"),
            schxslt("2000-01-01T00:00:00"),
            schxslt("2000-01-01T00:00:00Z").replace("<dct:source>", "<dct:source>t"),
            schxslt("2000-01-01T00:00:00Z").replace("<dct:source>", "<dct:source><!--c-->"),
            schxslt("2000-01-01T00:00:00Z")
                .replace("<dct:created>2000", "<dct:modified>2000")
                .replace("Z</dct:created>", "Z</dct:modified>"),
            schxslt("2000-01-01T00:00:00Z").replace(
                "</svrl:schematron-output>",
                "<svrl:metadata/></svrl:schematron-output>",
            ),
        ] {
            assert!(canonicalize(&bad, &uri).is_err(), "{bad}");
        }
    }

    #[test]
    fn document_uris() {
        assert!(is_document_uri("file:/tmp/a%20b/c1.xml", "/tmp/a b/c1.xml"));
        assert!(is_document_uri("file:///tmp/c.xml", "/tmp/c.xml"));
        assert!(is_document_uri("file:/C:/x/c.xml", "C:\\x\\c.xml"));
        assert!(!is_document_uri("file:/tmp/c.xml", "/tmp/d.xml"));
        assert!(!is_document_uri("http://x/tmp/c.xml", "/tmp/c.xml"));
    }
}
