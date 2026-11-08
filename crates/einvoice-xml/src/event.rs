//! The events of a document.

use std::borrow::Cow;
use std::ops::Range;

use crate::Reader;

/// One event of a document, in document order. `'r` is the borrow of the reader
/// (names' namespaces and attributes live in it until the next event); `'a` is the
/// text read.
#[derive(Debug)]
pub enum Event<'r, 'a> {
    /// A start tag or an empty-element tag. An empty-element tag is followed at once by
    /// its [`Event::End`].
    Start(StartElement<'r, 'a>),
    /// An end tag, or the end of an empty-element tag.
    End(EndElement<'r, 'a>),
    /// Character data or a CDATA section. Adjacent text events (character data,
    /// references, CDATA sections not separated by markup) make one text node of the
    /// data model.
    Text(Text<'a>),
    /// A comment.
    Comment(Comment<'a>),
    /// A processing instruction.
    ProcessingInstruction(ProcessingInstruction<'a>),
}

/// A name with its namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Name<'r, 'a> {
    /// The prefix as written, `""` for none.
    pub prefix: &'a str,
    /// The local part.
    pub local: &'a str,
    /// The namespace name, `None` for no namespace.
    pub namespace: Option<&'r str>,
}

/// An attribute (namespace declarations are not attributes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute<'r, 'a> {
    /// Its name.
    pub name: Name<'r, 'a>,
    /// The normalised value (references replaced, whitespace characters written as
    /// spaces): a slice of the text unless that changed it.
    pub value: &'r Cow<'a, str>,
    /// The value as written, between the quotes.
    pub raw: &'a str,
    /// The byte range of the attribute, from its name to its closing quote.
    pub range: Range<usize>,
}

/// The start of an element.
#[derive(Clone, Copy)]
pub struct StartElement<'r, 'a> {
    pub(crate) reader: &'r Reader<'a>,
}

impl<'r, 'a> StartElement<'r, 'a> {
    /// The element's name.
    #[must_use]
    pub fn name(&self) -> Name<'r, 'a> {
        current_name(self.reader)
    }

    /// The attributes, in document order (namespace declarations excluded).
    pub fn attributes(&self) -> impl ExactSizeIterator<Item = Attribute<'r, 'a>> + use<'r, 'a> {
        let reader = self.reader;
        reader
            .attributes
            .iter()
            .map(move |data| reader.attribute_view(data))
    }

    /// The value of the attribute with this namespace (`None` for none) and local name.
    #[must_use]
    pub fn attribute(&self, namespace: Option<&str>, local: &str) -> Option<&'r Cow<'a, str>> {
        let reader = self.reader;
        reader
            .attributes
            .iter()
            .find(|data| data.local == local && reader.namespace(data.ns) == namespace)
            .map(|data| &data.value)
    }

    /// The namespace declarations of the element, in document order: the prefix (`""`
    /// for the default namespace) and the namespace name (empty when the default
    /// namespace is undeclared). A declaration of the `xml` prefix is not reported.
    pub fn namespace_declarations(
        &self,
    ) -> impl ExactSizeIterator<Item = (&'a str, &'r str)> + use<'r, 'a> {
        let scope = self.reader.stack.last().map_or(0, |open| open.scope);
        self.reader.scope.since(scope)
    }

    /// The namespace bound to `prefix` in the scope of the element (see
    /// [`Reader::lookup_namespace`]).
    #[must_use]
    pub fn lookup_namespace(&self, prefix: &str) -> Option<&'r str> {
        self.reader.lookup_namespace(prefix)
    }

    /// Whether it is an empty-element tag (`<a/>`).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.reader.empty
    }

    /// The byte range of the start tag.
    #[must_use]
    pub fn range(&self) -> Range<usize> {
        self.reader.tag.clone()
    }
}

impl std::fmt::Debug for StartElement<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartElement")
            .field("name", &self.name())
            .field("attributes", &self.attributes().collect::<Vec<_>>())
            .field("empty", &self.is_empty())
            .finish()
    }
}

/// The end of an element.
#[derive(Clone, Copy)]
pub struct EndElement<'r, 'a> {
    pub(crate) reader: &'r Reader<'a>,
}

impl<'r, 'a> EndElement<'r, 'a> {
    /// The element's name.
    #[must_use]
    pub fn name(&self) -> Name<'r, 'a> {
        current_name(self.reader)
    }

    /// Whether the element was an empty-element tag.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.reader.empty
    }

    /// The byte range of the end tag, or of the empty-element tag.
    #[must_use]
    pub fn range(&self) -> Range<usize> {
        self.reader.tag.clone()
    }
}

impl std::fmt::Debug for EndElement<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EndElement")
            .field("name", &self.name())
            .finish()
    }
}

fn current_name<'r, 'a>(reader: &'r Reader<'a>) -> Name<'r, 'a> {
    match reader.stack.last() {
        Some(open) => reader.name_of(open.prefix, open.local, open.ns),
        None => Name {
            prefix: "",
            local: "",
            namespace: None,
        },
    }
}

/// Character data or a CDATA section.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Text<'a> {
    /// The text: references replaced, line ends normalised; a slice of the input
    /// unless that changed it.
    pub value: Cow<'a, str>,
    /// The byte range as written (a CDATA section's markup included).
    pub range: Range<usize>,
    /// Whether it is a CDATA section.
    pub cdata: bool,
}

/// A comment.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Comment<'a> {
    /// The text between `<!--` and `-->`, line ends normalised.
    pub value: Cow<'a, str>,
    /// The byte range as written.
    pub range: Range<usize>,
}

/// A processing instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProcessingInstruction<'a> {
    /// The target.
    pub target: &'a str,
    /// The data after the whitespace that follows the target, line ends normalised;
    /// `None` when empty.
    pub value: Option<Cow<'a, str>>,
    /// The byte range as written.
    pub range: Range<usize>,
}
