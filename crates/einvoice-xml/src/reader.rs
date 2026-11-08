//! The pull parser.

use std::borrow::Cow;
use std::collections::HashSet;
use std::ops::Range;

use crate::chars::{self, find_non_xml_char, is_ascii_name, is_space, starts_with_name_start};
use crate::decode::{self, Problem};
use crate::event::{
    Attribute, Comment, EndElement, Event, Name, ProcessingInstruction, StartElement, Text,
};
use crate::namespaces::{NONE, Scope};
use crate::{Error, TextPos};

/// The namespace of the `xml` prefix.
pub const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
/// The namespace of the `xmlns` prefix.
pub const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

/// Bounds on what a document may contain, against hostile input. The defaults are those
/// of `roxmltree` 0.21, so that a document is refused by a limit exactly when it
/// refuses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Nodes in the document, the document node, elements, text nodes (adjacent
    /// character data and CDATA sections being one node), comments and processing
    /// instructions counted: [`Error::NodesLimitReached`] when a node would be the
    /// `nodes`-th. Default `u32::MAX`.
    pub nodes: u32,
    /// Depth of element nesting, the document element at depth 1:
    /// [`Error::DepthLimitReached`] beyond. Default `u32::MAX`.
    pub depth: u32,
    /// Attributes in the document, namespace declarations not counted:
    /// [`Error::AttributesLimitReached`] when the attributes of an element would bring
    /// the count to `attributes`. Default `u32::MAX`.
    pub attributes: u32,
    /// Distinct (prefix, namespace name) declarations in the document, the predefined
    /// `xml` prefix counted: [`Error::NamespacesLimitReached`] when a new one would
    /// bring the count to `namespaces`. Default `65536`.
    pub namespaces: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            nodes: u32::MAX,
            depth: u32::MAX,
            attributes: u32::MAX,
            namespaces: 65536,
        }
    }
}

impl Limits {
    /// The defaults with another node limit.
    #[must_use]
    pub const fn with_nodes(mut self, nodes: u32) -> Self {
        self.nodes = nodes;
        self
    }

    /// The defaults with another depth limit.
    #[must_use]
    pub const fn with_depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }

    /// The defaults with another attribute limit.
    #[must_use]
    pub const fn with_attributes(mut self, attributes: u32) -> Self {
        self.attributes = attributes;
        self
    }

    /// The defaults with another namespace limit.
    #[must_use]
    pub const fn with_namespaces(mut self, namespaces: u32) -> Self {
        self.namespaces = namespaces;
        self
    }
}

/// The namespace of a name: none, the `xml` namespace, or a binding of the scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ns {
    None,
    Xml,
    Binding(u32),
}

/// A qualified name as written.
#[derive(Clone, Copy)]
struct QName<'a> {
    prefix: &'a str,
    local: &'a str,
    /// Written with a colon and an empty prefix (`:a`).
    empty_prefix: bool,
}

/// An attribute of the current start tag (namespace declarations excluded).
#[derive(Debug)]
pub(crate) struct AttributeData<'a> {
    pub(crate) prefix: &'a str,
    pub(crate) local: &'a str,
    pub(crate) value: Cow<'a, str>,
    pub(crate) raw: &'a str,
    pub(crate) ns: Ns,
    pub(crate) range: Range<usize>,
}

/// An open element.
#[derive(Debug)]
pub(crate) struct Open<'a> {
    pub(crate) prefix: &'a str,
    pub(crate) local: &'a str,
    pub(crate) ns: Ns,
    /// The number of bindings in scope before its declarations.
    pub(crate) scope: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// Before the XML declaration.
    Start,
    /// Before the document element.
    Prolog,
    /// Inside the document element.
    Content,
    /// After the document element (or where it should have been).
    Epilog,
    /// Finished.
    Done,
}

/// What to do before reading on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pending {
    Nothing,
    /// Report the end of the empty element just started.
    EmptyEnd,
    /// Close the element just ended.
    Pop,
}

/// An event before it is given a view of the reader.
enum Raw<'a> {
    Start,
    End,
    Text(Text<'a>),
    Comment(Comment<'a>),
    Pi(ProcessingInstruction<'a>),
}

/// A streaming parser of an XML 1.0 document with namespaces, read from a string.
///
/// [`Reader::next_event`] returns the events one by one; the reader keeps only the
/// open elements, the namespace declarations in scope and the attributes of the
/// current start tag. Names, and text without references or carriage returns, are
/// slices of the input. A document is well-formed when every event up to the final
/// `None` is read without error; events read before an error belong to a document
/// that is not.
#[derive(Debug)]
pub struct Reader<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
    limits: Limits,
    phase: Phase,
    pending: Pending,
    pub(crate) stack: Vec<Open<'a>>,
    pub(crate) scope: Scope<'a>,
    pub(crate) attributes: Vec<AttributeData<'a>>,
    /// The byte range of the last start or end tag.
    pub(crate) tag: Range<usize>,
    /// Whether the last start tag was an empty-element tag.
    pub(crate) empty: bool,
    nodes: u64,
    attribute_count: u64,
    /// Whether the last node is a text node (that the next character data joins).
    in_text: bool,
    root_seen: bool,
    failed: Option<Error>,
    /// The first error of the current token that only this reader reports, raised once
    /// the checks that `roxmltree` makes on the token have passed.
    deferred: Option<Error>,
    /// The number of bindings in scope before the current start tag's declarations.
    tag_scope: usize,
    /// What the current start tag declares that `roxmltree` does not track.
    declared: Declared,
}

/// Whether the current start tag declares the default namespace, the `xml` prefix.
#[derive(Clone, Copy, Debug, Default)]
struct Declared {
    default: bool,
    xml: bool,
}

impl std::fmt::Debug for Scope<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scope")
            .field("bindings", &self.len())
            .finish()
    }
}

impl<'a> Reader<'a> {
    /// A reader of `text` with the default [`Limits`]. A byte order mark at the start
    /// is skipped.
    #[must_use]
    pub fn new(text: &'a str) -> Self {
        Self::with_limits(text, Limits::default())
    }

    /// A reader of `text` with the given limits.
    #[must_use]
    pub fn with_limits(text: &'a str, limits: Limits) -> Self {
        Self {
            text,
            bytes: text.as_bytes(),
            pos: 0,
            limits,
            phase: Phase::Start,
            pending: Pending::Nothing,
            stack: Vec::new(),
            scope: Scope::new(),
            attributes: Vec::new(),
            tag: 0..0,
            empty: false,
            nodes: 1,
            attribute_count: 0,
            in_text: false,
            root_seen: false,
            failed: None,
            deferred: None,
            tag_scope: 0,
            declared: Declared::default(),
        }
    }

    /// The text being read.
    #[must_use]
    pub const fn text(&self) -> &'a str {
        self.text
    }

    /// The byte offset reached.
    #[must_use]
    pub const fn offset(&self) -> usize {
        self.pos
    }

    /// The number of open elements: after a [`Event::Start`], its element included;
    /// after an [`Event::End`], its element still included.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// The position of a byte offset of the text (see [`TextPos::at`]).
    #[must_use]
    pub fn text_pos(&self, offset: usize) -> TextPos {
        TextPos::at(self.text, offset)
    }

    /// The namespace bound to `prefix` (`""` for the default namespace) in the scope of
    /// the innermost open element (the element of the last [`Event::Start`] or
    /// [`Event::End`] included): `None` if there is none or if the default namespace is
    /// undeclared. `xml` is always bound to [`XML_NAMESPACE`] and `xmlns` to
    /// [`XMLNS_NAMESPACE`].
    #[must_use]
    pub fn lookup_namespace(&self, prefix: &str) -> Option<&str> {
        match prefix {
            "xml" => Some(XML_NAMESPACE),
            "xmlns" => Some(XMLNS_NAMESPACE),
            _ => {
                let index = self.scope.lookup(prefix);
                (index != NONE)
                    .then(|| self.scope.uri(index))
                    .filter(|uri| !uri.is_empty())
            }
        }
    }

    pub(crate) fn namespace(&self, ns: Ns) -> Option<&str> {
        match ns {
            Ns::None => None,
            Ns::Xml => Some(XML_NAMESPACE),
            Ns::Binding(index) => Some(self.scope.uri(index)).filter(|uri| !uri.is_empty()),
        }
    }

    pub(crate) fn name_of(&self, prefix: &'a str, local: &'a str, ns: Ns) -> Name<'_, 'a> {
        Name {
            prefix,
            local,
            namespace: self.namespace(ns),
        }
    }

    pub(crate) fn attribute_view<'r>(&'r self, data: &'r AttributeData<'a>) -> Attribute<'r, 'a> {
        Attribute {
            name: self.name_of(data.prefix, data.local, data.ns),
            value: &data.value,
            raw: data.raw,
            range: data.range.clone(),
        }
    }

    /// The next event, `Ok(None)` at the end of a well-formed document. After an error,
    /// the same error again.
    ///
    /// # Errors
    ///
    /// The first well-formedness error from where the reader is (see [`Error`]).
    pub fn next_event(&mut self) -> Result<Option<Event<'_, 'a>>, Error> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        let raw = match self.advance() {
            Ok(raw) => raw,
            Err(error) => {
                self.phase = Phase::Done;
                self.failed = Some(error.clone());
                return Err(error);
            }
        };
        Ok(raw.map(|raw| match raw {
            Raw::Start => Event::Start(StartElement { reader: self }),
            Raw::End => Event::End(EndElement { reader: self }),
            Raw::Text(text) => Event::Text(text),
            Raw::Comment(comment) => Event::Comment(comment),
            Raw::Pi(pi) => Event::ProcessingInstruction(pi),
        }))
    }

    /// Reads to the end of the document.
    ///
    /// # Errors
    ///
    /// The first well-formedness error from where the reader is.
    pub fn read_to_end(&mut self) -> Result<(), Error> {
        while self.next_event()?.is_some() {}
        Ok(())
    }

    fn advance(&mut self) -> Result<Option<Raw<'a>>, Error> {
        match std::mem::replace(&mut self.pending, Pending::Nothing) {
            Pending::Nothing => {}
            Pending::EmptyEnd => {
                self.pending = Pending::Pop;
                return Ok(Some(Raw::End));
            }
            Pending::Pop => {
                if let Some(open) = self.stack.pop() {
                    self.scope.truncate(open.scope);
                }
                if self.stack.is_empty() {
                    self.phase = Phase::Epilog;
                }
            }
        }
        loop {
            match self.phase {
                Phase::Start => {
                    self.declaration()?;
                    self.phase = Phase::Prolog;
                }
                Phase::Prolog => {
                    self.skip_spaces();
                    if self.starts_with(b"<!--") {
                        return self.comment().map(Some);
                    }
                    if self.starts_with(b"<?") {
                        return self.pi().map(Some);
                    }
                    if self.starts_with(b"<!DOCTYPE") {
                        return Err(Error::DtdDetected);
                    }
                    if self.bytes.get(self.pos) == Some(&b'<') && self.start_tag()? {
                        return Ok(Some(Raw::Start));
                    }
                    // No document element, or the text ends in its start tag.
                    self.phase = Phase::Epilog;
                }
                Phase::Content => {
                    let Some(&b) = self.bytes.get(self.pos) else {
                        self.phase = Phase::Epilog;
                        continue;
                    };
                    if b != b'<' {
                        return self.text_token().map(Some);
                    }
                    match self.bytes.get(self.pos + 1) {
                        None => return Err(Error::UnknownToken(self.text_pos(self.pos))),
                        Some(b'!') => {
                            if self.starts_with(b"<!--") {
                                return self.comment().map(Some);
                            }
                            if self.starts_with(b"<![CDATA[") {
                                return self.cdata().map(Some);
                            }
                            return Err(Error::UnknownToken(self.text_pos(self.pos)));
                        }
                        Some(b'?') => return self.pi().map(Some),
                        Some(b'/') => return self.end_tag().map(Some),
                        Some(_) => {
                            if self.start_tag()? {
                                return Ok(Some(Raw::Start));
                            }
                            // The text ends in the start tag: the next turn ends it.
                        }
                    }
                }
                Phase::Epilog => {
                    self.skip_spaces();
                    if self.starts_with(b"<!--") {
                        return self.comment().map(Some);
                    }
                    if self.starts_with(b"<?") {
                        return self.pi().map(Some);
                    }
                    if self.pos < self.bytes.len() {
                        return Err(Error::UnknownToken(self.text_pos(self.pos)));
                    }
                    self.phase = Phase::Done;
                    if !self.root_seen {
                        return Err(Error::NoRootNode);
                    }
                    if !self.stack.is_empty() {
                        return Err(Error::UnclosedRootNode);
                    }
                    return Ok(None);
                }
                Phase::Done => return Ok(None),
            }
        }
    }

    // --- The cursor.

    fn starts_with(&self, prefix: &[u8]) -> bool {
        self.bytes
            .get(self.pos..)
            .is_some_and(|rest| rest.starts_with(prefix))
    }

    fn skip_spaces(&mut self) {
        while self.bytes.get(self.pos).is_some_and(|&b| is_space(b)) {
            self.pos += 1;
        }
    }

    fn at_space(&self) -> bool {
        self.bytes.get(self.pos).is_some_and(|&b| is_space(b))
    }

    fn current(&self) -> Result<u8, Error> {
        self.bytes
            .get(self.pos)
            .copied()
            .ok_or(Error::UnexpectedEndOfStream)
    }

    fn consume_byte(&mut self, expected: u8) -> Result<(), Error> {
        let actual = self.current()?;
        if actual != expected {
            return Err(Error::InvalidChar {
                expected,
                actual,
                pos: self.text_pos(self.pos),
            });
        }
        self.pos += 1;
        Ok(())
    }

    fn skip_string(&mut self, expected: &'static str) -> Result<(), Error> {
        if !self.starts_with(expected.as_bytes()) {
            return Err(Error::InvalidString(expected, self.text_pos(self.pos)));
        }
        self.pos += expected.len();
        Ok(())
    }

    /// `S? '=' S?`
    fn consume_eq(&mut self) -> Result<(), Error> {
        self.skip_spaces();
        self.consume_byte(b'=')?;
        self.skip_spaces();
        Ok(())
    }

    fn consume_quote(&mut self) -> Result<u8, Error> {
        let c = self.current()?;
        if c == b'\'' || c == b'"' {
            self.pos += 1;
            Ok(c)
        } else {
            Err(Error::ExpectedChar {
                expected: "a quote",
                actual: c,
                pos: self.text_pos(self.pos),
            })
        }
    }

    /// Checks that `[start, end)` holds only XML characters.
    fn check_chars(&self, start: usize, end: usize) -> Result<(), Error> {
        match find_non_xml_char(&self.text[start..end]) {
            None => Ok(()),
            Some((offset, c)) => Err(Error::NonXmlChar(c, self.text_pos(start + offset))),
        }
    }

    /// The offset of the first `needle` at or after `from`, or the end of the text.
    fn find(&self, from: usize, needle: &[u8]) -> Option<usize> {
        memchr::memmem::find(&self.bytes[from..], needle).map(|n| from + n)
    }

    /// A qualified name: `Name` with at most one colon, the prefix (if any) and the
    /// local part each starting with a `NameStartChar`.
    fn qname(&mut self) -> Result<QName<'a>, Error> {
        let start = self.pos;
        let mut colon = None;
        while let Some(&b) = self.bytes.get(self.pos) {
            if b < 0x80 {
                if b == b':' {
                    if colon.is_some() {
                        return Err(Error::InvalidName(self.text_pos(start)));
                    }
                    colon = Some(self.pos);
                    self.pos += 1;
                } else if is_ascii_name(b) {
                    self.pos += 1;
                } else {
                    break;
                }
            } else {
                match self.text[self.pos..].chars().next() {
                    Some(c) if chars::is_name(c) => self.pos += c.len_utf8(),
                    _ => break,
                }
            }
        }
        let text = self.text;
        let (prefix, local) = match colon {
            Some(colon) => (&text[start..colon], &text[colon + 1..self.pos]),
            None => ("", &text[start..self.pos]),
        };
        if (!prefix.is_empty() && !starts_with_name_start(prefix)) || !starts_with_name_start(local)
        {
            return Err(Error::InvalidName(self.text_pos(start)));
        }
        Ok(QName {
            prefix,
            local,
            empty_prefix: colon == Some(start),
        })
    }

    /// A `Name` (colons allowed).
    fn name(&mut self) -> Result<&'a str, Error> {
        let start = self.pos;
        match self.text[self.pos..].chars().next() {
            Some(c) if chars::is_name_start(c) => self.pos += c.len_utf8(),
            _ => return Err(Error::InvalidName(self.text_pos(start))),
        }
        while let Some(c) = self.text[self.pos..].chars().next() {
            if !chars::is_name(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
        Ok(&self.text[start..self.pos])
    }

    fn defer(&mut self, error: Error) {
        if self.deferred.is_none() {
            self.deferred = Some(error);
        }
    }

    fn raise_deferred(&mut self) -> Result<(), Error> {
        match self.deferred.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn count_node(&mut self) -> Result<(), Error> {
        if self.nodes >= u64::from(self.limits.nodes) {
            return Err(Error::NodesLimitReached);
        }
        self.nodes += 1;
        Ok(())
    }

    fn decode_error(&self, problem: &Problem<'_>, base: usize) -> Error {
        match problem {
            Problem::UnknownEntity(name, at) => {
                Error::UnknownEntityReference((*name).to_owned(), self.text_pos(base + at))
            }
            Problem::Malformed(at) => Error::MalformedEntityReference(self.text_pos(base + at)),
        }
    }

    // --- The prolog.

    /// The XML declaration, if the text starts with one (after a byte order mark).
    fn declaration(&mut self) -> Result<(), Error> {
        if self.starts_with("\u{FEFF}".as_bytes()) {
            self.pos += 3;
        }
        if !(self.starts_with(b"<?xml")
            && self.bytes.get(self.pos + 5).is_some_and(|&b| is_space(b)))
        {
            return Ok(());
        }
        self.pos += 5;
        self.declaration_spaces()?;
        if !self.starts_with(b"version") {
            return Err(Error::InvalidString("version", self.text_pos(self.pos)));
        }
        let version = self.pseudo_attribute()?;
        self.declaration_spaces()?;
        let encoding = if self.starts_with(b"encoding") {
            let encoding = self.pseudo_attribute()?;
            self.declaration_spaces()?;
            Some(encoding)
        } else {
            None
        };
        let standalone = if self.starts_with(b"standalone") {
            Some(self.pseudo_attribute()?)
        } else {
            None
        };
        self.skip_spaces();
        self.skip_string("?>")?;

        // What `roxmltree` does not check.
        let check = |(name_start, name, value_start, value): (usize, &str, usize, &str),
                     expected: &str,
                     valid: fn(&str) -> bool,
                     reason: &'static str| {
            if name != expected {
                return Err(Error::InvalidXmlDeclaration(
                    "unknown pseudo-attribute",
                    TextPos::at(self.text, name_start),
                ));
            }
            if !valid(value) {
                return Err(Error::InvalidXmlDeclaration(
                    reason,
                    TextPos::at(self.text, value_start),
                ));
            }
            Ok(())
        };
        check(
            version,
            "version",
            |v| {
                v.strip_prefix("1.")
                    .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
            },
            "the version is not 1. followed by digits",
        )?;
        if let Some(encoding) = encoding {
            check(
                encoding,
                "encoding",
                |e| {
                    e.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
                        && e.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
                },
                "the encoding is not an encoding name",
            )?;
        }
        if let Some(standalone) = standalone {
            check(
                standalone,
                "standalone",
                |s| s == "yes" || s == "no",
                "standalone is neither yes nor no",
            )?;
        }
        Ok(())
    }

    /// The whitespace required before a pseudo-attribute, unless the declaration ends.
    fn declaration_spaces(&mut self) -> Result<(), Error> {
        if self.at_space() {
            self.skip_spaces();
        } else if let Some(&actual) = self.bytes.get(self.pos)
            && !self.starts_with(b"?>")
        {
            return Err(Error::ExpectedChar {
                expected: "a whitespace",
                actual,
                pos: self.text_pos(self.pos),
            });
        }
        Ok(())
    }

    /// A pseudo-attribute of the XML declaration: its name as written, the offsets of
    /// the name and of the value, and the value.
    fn pseudo_attribute(&mut self) -> Result<(usize, &'a str, usize, &'a str), Error> {
        let name_start = self.pos;
        self.qname()?;
        let name = &self.text[name_start..self.pos];
        self.consume_eq()?;
        let quote = self.consume_quote()?;
        let value_start = self.pos;
        let end = self.bytes[value_start..]
            .iter()
            .position(|&b| b == quote || b == b'<')
            .map_or(self.bytes.len(), |n| value_start + n);
        self.check_chars(value_start, end)?;
        self.pos = end;
        let value = &self.text[value_start..end];
        self.consume_byte(quote)?;
        Ok((name_start, name, value_start, value))
    }

    // --- Comments, processing instructions, character data.

    fn comment(&mut self) -> Result<Raw<'a>, Error> {
        let start = self.pos;
        let body = start + 4;
        let end = self.find(body, b"-->");
        self.check_chars(body, end.unwrap_or(self.bytes.len()))?;
        let Some(end) = end else {
            self.pos = self.bytes.len();
            return Err(Error::InvalidString("-->", self.text_pos(self.pos)));
        };
        self.pos = end + 3;
        let value = &self.text[body..end];
        if value.contains("--") || value.ends_with('-') {
            return Err(Error::InvalidComment(self.text_pos(start)));
        }
        self.in_text = false;
        self.count_node()?;
        Ok(Raw::Comment(Comment {
            value: decode::line_ends(value),
            range: start..self.pos,
        }))
    }

    fn pi(&mut self) -> Result<Raw<'a>, Error> {
        if self.starts_with(b"<?xml ") {
            return Err(Error::UnexpectedDeclaration(self.text_pos(self.pos)));
        }
        let start = self.pos;
        self.pos += 2;
        let target_start = self.pos;
        let target = self.name()?;
        let target_end = self.pos;
        self.skip_spaces();
        let data = self.pos;
        let end = self.find(data, b"?>");
        self.check_chars(data, end.unwrap_or(self.bytes.len()))?;
        let Some(end) = end else {
            self.pos = self.bytes.len();
            return Err(Error::InvalidString("?>", self.text_pos(self.pos)));
        };
        self.pos = end + 2;
        // What `roxmltree` does not check.
        if target.contains(':') || target.eq_ignore_ascii_case("xml") {
            return Err(Error::InvalidPiTarget(self.text_pos(target_start)));
        }
        if data == target_end && data != end {
            return Err(Error::PiWithoutWhitespace(self.text_pos(target_end)));
        }
        self.in_text = false;
        self.count_node()?;
        let value = &self.text[data..end];
        Ok(Raw::Pi(ProcessingInstruction {
            target,
            value: (!value.is_empty()).then(|| decode::line_ends(value)),
            range: start..self.pos,
        }))
    }

    fn cdata(&mut self) -> Result<Raw<'a>, Error> {
        let start = self.pos;
        let body = start + 9;
        let end = self.find(body, b"]]>");
        self.check_chars(body, end.unwrap_or(self.bytes.len()))?;
        let Some(end) = end else {
            self.pos = self.bytes.len();
            return Err(Error::InvalidString("]]>", self.text_pos(self.pos)));
        };
        self.pos = end + 3;
        self.text_node()?;
        Ok(Raw::Text(Text {
            value: decode::line_ends(&self.text[body..end]),
            range: start..self.pos,
            cdata: true,
        }))
    }

    fn text_token(&mut self) -> Result<Raw<'a>, Error> {
        let start = self.pos;
        let end =
            memchr::memchr(b'<', &self.bytes[start..]).map_or(self.bytes.len(), |n| start + n);
        self.check_chars(start, end)?;
        self.pos = end;
        let raw = &self.text[start..end];
        if memchr::memchr(b'>', raw.as_bytes()).is_some() && raw.contains("]]>") {
            return Err(Error::InvalidCharacterData(self.text_pos(end)));
        }
        let (value, not_scalar) = decode::text(raw).map_err(|p| self.decode_error(&p, start))?;
        if let Some(at) = not_scalar {
            return Err(Error::InvalidCharacterReference(self.text_pos(start + at)));
        }
        self.text_node()?;
        Ok(Raw::Text(Text {
            value,
            range: start..end,
            cdata: false,
        }))
    }

    /// Character data: a new text node unless it joins the last one.
    fn text_node(&mut self) -> Result<(), Error> {
        if !self.in_text {
            self.count_node()?;
            self.in_text = true;
        }
        Ok(())
    }

    // --- Tags.

    /// A start tag, from its `<`: `true` once it is read, `false` if the text ends in
    /// it.
    fn start_tag(&mut self) -> Result<bool, Error> {
        let start = self.pos;
        self.pos += 1;
        let name = self.qname()?;
        self.in_text = false;
        if name.prefix == "xmlns" {
            return Err(Error::InvalidElementNamePrefix(self.text_pos(start + 1)));
        }
        self.deferred = None;
        if name.empty_prefix {
            self.defer(Error::InvalidQName(self.text_pos(start + 1)));
        }
        self.attributes.clear();
        self.declared = Declared::default();
        let scope = self.scope.len();
        self.tag_scope = scope;
        while self.pos < self.bytes.len() {
            let has_space = self.at_space();
            self.skip_spaces();
            let attribute_start = self.pos;
            match self.current()? {
                b'/' => {
                    self.pos += 1;
                    self.consume_byte(b'>')?;
                    self.open(start, name, scope, true)?;
                    return Ok(true);
                }
                b'>' => {
                    self.pos += 1;
                    self.open(start, name, scope, false)?;
                    return Ok(true);
                }
                actual => {
                    if !has_space {
                        return Err(Error::ExpectedChar {
                            expected: "a whitespace",
                            actual,
                            pos: self.text_pos(self.pos),
                        });
                    }
                    let attribute = self.qname()?;
                    self.consume_eq()?;
                    let quote = self.consume_quote()?;
                    let value_start = self.pos;
                    let Some(n) = memchr::memchr2(quote, b'<', &self.bytes[value_start..]) else {
                        return Err(Error::UnexpectedEndOfStream);
                    };
                    let value_end = value_start + n;
                    self.check_chars(value_start, value_end)?;
                    self.pos = value_end;
                    self.consume_byte(quote)?;
                    let raw = &self.text[value_start..value_end];
                    self.attribute(attribute_start..self.pos, attribute, raw, value_start)?;
                }
            }
        }
        self.scope.truncate(scope);
        Ok(false)
    }

    /// An attribute or namespace declaration of the current start tag.
    fn attribute(
        &mut self,
        range: Range<usize>,
        name: QName<'a>,
        raw: &'a str,
        value_start: usize,
    ) -> Result<(), Error> {
        let (text, at) = (self.text, range.start);
        let pos = move || TextPos::at(text, at);
        if name.empty_prefix {
            self.defer(Error::InvalidQName(pos()));
        }
        let (value, not_scalar) =
            decode::attribute(raw).map_err(|p| self.decode_error(&p, value_start))?;
        if let Some(at) = not_scalar {
            self.defer(Error::InvalidCharacterReference(
                self.text_pos(value_start + at),
            ));
        }
        if name.prefix == "xmlns" {
            if value == XMLNS_NAMESPACE {
                return Err(Error::UnexpectedXmlnsUri(pos()));
            }
            let xml = value == XML_NAMESPACE;
            if name.local == "xml" {
                if !xml {
                    return Err(Error::InvalidXmlPrefixUri(pos()));
                }
            } else if xml {
                return Err(Error::UnexpectedXmlUri(pos()));
            }
            let scope_start = self.tag_scope;
            if self.scope.declared_since(scope_start, name.local) {
                return Err(Error::DuplicatedNamespace(name.local.to_owned(), pos()));
            }
            if xml {
                if self.declared.xml {
                    self.defer(Error::DuplicatedNamespaceDeclaration(
                        "xmlns:xml".to_owned(),
                        pos(),
                    ));
                }
                self.declared.xml = true;
                return Ok(());
            }
            if name.local == "xmlns" {
                self.defer(Error::XmlnsPrefixDeclared(pos()));
            }
            if value.is_empty() {
                self.defer(Error::EmptyPrefixedNamespace(name.local.to_owned(), pos()));
            }
            self.scope.push(name.local, value, self.limits.namespaces)?;
        } else if name.local == "xmlns" {
            if value == XML_NAMESPACE {
                return Err(Error::UnexpectedXmlUri(pos()));
            }
            if value == XMLNS_NAMESPACE {
                return Err(Error::UnexpectedXmlnsUri(pos()));
            }
            if !name.prefix.is_empty() {
                self.defer(Error::PrefixedXmlnsAttribute(
                    format!("{}:xmlns", name.prefix),
                    pos(),
                ));
            } else if self.declared.default {
                self.defer(Error::DuplicatedNamespaceDeclaration(
                    "xmlns".to_owned(),
                    pos(),
                ));
            }
            self.declared.default = true;
            self.scope.push("", value, self.limits.namespaces)?;
        } else {
            self.attributes.push(AttributeData {
                prefix: name.prefix,
                local: name.local,
                value,
                raw,
                ns: Ns::None,
                range,
            });
        }
        Ok(())
    }

    /// The end of a start tag: names resolved, duplicates refused, the element opened.
    fn open(
        &mut self,
        start: usize,
        name: QName<'a>,
        scope: usize,
        empty: bool,
    ) -> Result<(), Error> {
        self.in_text = false;
        if !self.attributes.is_empty() {
            let count = self.attributes.len() as u64;
            if self.attribute_count + count >= u64::from(self.limits.attributes) {
                return Err(Error::AttributesLimitReached);
            }
            self.resolve_attributes()?;
            self.attribute_count += count;
        }
        let ns = if name.prefix.is_empty() {
            match self.scope.lookup("") {
                NONE => Ns::None,
                index => Ns::Binding(index),
            }
        } else {
            match self.scope.lookup(name.prefix) {
                NONE => {
                    return Err(Error::UnknownNamespace(
                        name.prefix.to_owned(),
                        self.text_pos(start + 1),
                    ));
                }
                index => Ns::Binding(index),
            }
        };
        self.raise_deferred()?;
        self.count_node()?;
        if self.stack.len() >= self.limits.depth as usize {
            return Err(Error::DepthLimitReached);
        }
        self.stack.push(Open {
            prefix: name.prefix,
            local: name.local,
            ns,
            scope,
        });
        self.tag = start..self.pos;
        self.empty = empty;
        self.root_seen = true;
        self.phase = Phase::Content;
        if empty {
            self.pending = Pending::EmptyEnd;
        }
        Ok(())
    }

    /// Resolves the prefixes of the attributes of the current start tag and refuses
    /// two with the same expanded name, in document order.
    fn resolve_attributes(&mut self) -> Result<(), Error> {
        let Self {
            attributes,
            scope,
            text,
            ..
        } = self;
        let uri = |ns: Ns| match ns {
            Ns::None => None,
            Ns::Xml => Some(XML_NAMESPACE),
            Ns::Binding(index) => Some(scope.uri(index)),
        };
        let mut seen: HashSet<(Option<&str>, &str)> = HashSet::new();
        let hashed = attributes.len() > 8;
        for i in 0..attributes.len() {
            let attribute = &attributes[i];
            let ns = if attribute.prefix == "xml" {
                Ns::Xml
            } else if attribute.prefix.is_empty() {
                Ns::None
            } else {
                match scope.lookup(attribute.prefix) {
                    NONE => {
                        return Err(Error::UnknownNamespace(
                            attribute.prefix.to_owned(),
                            TextPos::at(text, attribute.range.start),
                        ));
                    }
                    index => Ns::Binding(index),
                }
            };
            let key = (uri(ns), attribute.local);
            let duplicate = if hashed {
                !seen.insert(key)
            } else {
                attributes[..i].iter().any(|a| (uri(a.ns), a.local) == key)
            };
            if duplicate {
                return Err(Error::DuplicatedAttribute(
                    attribute.local.to_owned(),
                    TextPos::at(text, attribute.range.start),
                ));
            }
            attributes[i].ns = ns;
        }
        Ok(())
    }

    /// An end tag, from its `</`.
    fn end_tag(&mut self) -> Result<Raw<'a>, Error> {
        let start = self.pos;
        self.pos += 2;
        let name = self.qname()?;
        self.skip_spaces();
        self.consume_byte(b'>')?;
        self.in_text = false;
        if let Some(open) = self.stack.last()
            && (open.prefix != name.prefix || open.local != name.local)
        {
            return Err(Error::UnexpectedCloseTag {
                expected: qualified(open.prefix, open.local),
                actual: qualified(name.prefix, name.local),
                pos: self.text_pos(start),
            });
        }
        if name.empty_prefix {
            return Err(Error::InvalidQName(self.text_pos(start + 2)));
        }
        self.tag = start..self.pos;
        self.empty = false;
        self.pending = Pending::Pop;
        Ok(Raw::End)
    }
}

fn qualified(prefix: &str, local: &str) -> String {
    if prefix.is_empty() {
        local.to_owned()
    } else {
        format!("{prefix}:{local}")
    }
}
