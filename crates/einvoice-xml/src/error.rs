//! Well-formedness errors and their positions.

use std::fmt;

/// A position in the text: 1-based row (lines are counted by line feeds) and 1-based
/// column (counted in characters from the last line feed).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TextPos {
    /// The line, from 1.
    pub row: u32,
    /// The character in the line, from 1.
    pub col: u32,
}

impl TextPos {
    /// A position from its row and column.
    #[must_use]
    pub const fn new(row: u32, col: u32) -> Self {
        Self { row, col }
    }

    /// The position of the byte `offset` of `text` (an offset past the end is taken as
    /// the end). Linear in `offset`: meant for errors, not for every event.
    #[must_use]
    pub fn at(text: &str, offset: usize) -> Self {
        let before = text.as_bytes().get(..offset).unwrap_or(text.as_bytes());
        let line_start = memchr::memrchr(b'\n', before).map_or(0, |i| i + 1);
        let rows = memchr::memchr_iter(b'\n', before).count();
        let cols = before[line_start..]
            .iter()
            .filter(|&&b| b & 0xC0 != 0x80)
            .count();
        Self {
            row: saturating_u32(rows.saturating_add(1)),
            col: saturating_u32(cols.saturating_add(1)),
        }
    }
}

fn saturating_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

impl fmt::Display for TextPos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.row, self.col)
    }
}

/// Why a text is not a document the reader accepts.
///
/// The variants before [`Error::InvalidCharacterReference`] are reported for the same
/// documents, at the same positions and with the same messages as the `roxmltree` crate
/// (version 0.21, with document type declarations refused) reports them. The variants
/// from [`Error::InvalidCharacterReference`] on are for documents that `roxmltree`
/// accepts although they are not well-formed (XML 1.0) or not namespace-well-formed
/// (Namespaces in XML 1.0), and for the limits that it does not have; the crate
/// documentation lists them.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub enum Error {
    /// `xmlns:xml` declares a namespace other than
    /// `http://www.w3.org/XML/1998/namespace`.
    InvalidXmlPrefixUri(TextPos),
    /// The `http://www.w3.org/XML/1998/namespace` namespace is declared for a prefix
    /// other than `xml`, or as the default namespace.
    UnexpectedXmlUri(TextPos),
    /// The `http://www.w3.org/2000/xmlns/` namespace is declared.
    UnexpectedXmlnsUri(TextPos),
    /// An element name with the `xmlns` prefix.
    InvalidElementNamePrefix(TextPos),
    /// The same prefix is declared twice on an element.
    DuplicatedNamespace(String, TextPos),
    /// An element or attribute name whose prefix is not declared (the prefix).
    UnknownNamespace(String, TextPos),
    /// An end tag that does not match the open element.
    UnexpectedCloseTag {
        /// The name of the open element, as written.
        expected: String,
        /// The name in the end tag.
        actual: String,
        /// Where the end tag starts.
        pos: TextPos,
    },
    /// A reference to an entity other than the five predefined ones (its name).
    UnknownEntityReference(String, TextPos),
    /// An `&` that does not start a well-formed reference to an XML character.
    MalformedEntityReference(TextPos),
    /// Two attributes of an element with the same expanded name (the local name).
    DuplicatedAttribute(String, TextPos),
    /// No element.
    NoRootNode,
    /// The text ends before the document element is closed.
    UnclosedRootNode,
    /// An XML declaration elsewhere than at the start of the text.
    UnexpectedDeclaration(TextPos),
    /// A document type declaration: refused, because its entities and attribute
    /// defaults could change the document.
    DtdDetected,
    /// More nodes than [`crate::Limits::nodes`].
    NodesLimitReached,
    /// More attributes than [`crate::Limits::attributes`].
    AttributesLimitReached,
    /// More distinct namespace declarations than [`crate::Limits::namespaces`].
    NamespacesLimitReached,
    /// A name that is not a name.
    InvalidName(TextPos),
    /// A character that XML does not allow.
    NonXmlChar(char, TextPos),
    /// A byte other than the one expected.
    InvalidChar {
        /// The byte expected.
        expected: u8,
        /// The byte found.
        actual: u8,
        /// Where.
        pos: TextPos,
    },
    /// A byte other than one of a class expected (described in words).
    ExpectedChar {
        /// What was expected.
        expected: &'static str,
        /// The byte found.
        actual: u8,
        /// Where.
        pos: TextPos,
    },
    /// The string expected is missing.
    InvalidString(&'static str, TextPos),
    /// A comment that contains `--` or ends with `-` (the position of the comment).
    InvalidComment(TextPos),
    /// `]]>` in character data (the position of the end of the text).
    InvalidCharacterData(TextPos),
    /// Something that is not markup where markup is expected.
    UnknownToken(TextPos),
    /// The text ends in the middle of markup.
    UnexpectedEndOfStream,
    /// A character reference to a code point that is not a Unicode scalar value (a
    /// surrogate or a number above `0x10FFFF`).
    InvalidCharacterReference(TextPos),
    /// A processing instruction target that is `xml` in any case or contains `:`.
    InvalidPiTarget(TextPos),
    /// A processing instruction without whitespace between its target and its data.
    PiWithoutWhitespace(TextPos),
    /// An XML declaration with a pseudo-attribute that is not `version`, `encoding` or
    /// `standalone` as written, or with an invalid value (the reason).
    InvalidXmlDeclaration(&'static str, TextPos),
    /// A name that starts with `:`, which is not a qualified name.
    InvalidQName(TextPos),
    /// A prefix declared with an empty namespace name (the prefix).
    EmptyPrefixedNamespace(String, TextPos),
    /// A declaration of the `xmlns` prefix.
    XmlnsPrefixDeclared(TextPos),
    /// The default namespace (`xmlns`) or the `xml` prefix (`xmlns:xml`) declared twice
    /// on an element (the attribute name).
    DuplicatedNamespaceDeclaration(String, TextPos),
    /// An attribute whose local name is `xmlns` and which has a prefix (its name).
    PrefixedXmlnsAttribute(String, TextPos),
    /// Elements nested deeper than [`crate::Limits::depth`].
    DepthLimitReached,
}

impl Error {
    /// The position of the error; `1:1` for the errors that have none.
    #[must_use]
    pub fn pos(&self) -> TextPos {
        match self {
            Self::InvalidXmlPrefixUri(pos)
            | Self::UnexpectedXmlUri(pos)
            | Self::UnexpectedXmlnsUri(pos)
            | Self::InvalidElementNamePrefix(pos)
            | Self::DuplicatedNamespace(_, pos)
            | Self::UnknownNamespace(_, pos)
            | Self::UnexpectedCloseTag { pos, .. }
            | Self::UnknownEntityReference(_, pos)
            | Self::MalformedEntityReference(pos)
            | Self::DuplicatedAttribute(_, pos)
            | Self::UnexpectedDeclaration(pos)
            | Self::InvalidName(pos)
            | Self::NonXmlChar(_, pos)
            | Self::InvalidChar { pos, .. }
            | Self::ExpectedChar { pos, .. }
            | Self::InvalidString(_, pos)
            | Self::InvalidComment(pos)
            | Self::InvalidCharacterData(pos)
            | Self::UnknownToken(pos)
            | Self::InvalidCharacterReference(pos)
            | Self::InvalidPiTarget(pos)
            | Self::PiWithoutWhitespace(pos)
            | Self::InvalidXmlDeclaration(_, pos)
            | Self::InvalidQName(pos)
            | Self::EmptyPrefixedNamespace(_, pos)
            | Self::XmlnsPrefixDeclared(pos)
            | Self::DuplicatedNamespaceDeclaration(_, pos)
            | Self::PrefixedXmlnsAttribute(_, pos) => *pos,
            Self::NoRootNode
            | Self::UnclosedRootNode
            | Self::DtdDetected
            | Self::NodesLimitReached
            | Self::AttributesLimitReached
            | Self::NamespacesLimitReached
            | Self::UnexpectedEndOfStream
            | Self::DepthLimitReached => TextPos::new(1, 1),
        }
    }
}

impl fmt::Display for Error {
    #[expect(clippy::too_many_lines, reason = "one message per error, in one place")]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidXmlPrefixUri(pos) => {
                write!(f, "'xml' namespace prefix mapped to wrong URI at {pos}")
            }
            Self::UnexpectedXmlUri(pos) => write!(
                f,
                "the 'xml' namespace URI is used for not 'xml' prefix at {pos}"
            ),
            Self::UnexpectedXmlnsUri(pos) => write!(
                f,
                "the 'xmlns' URI is used at {pos}, but it must not be declared"
            ),
            Self::InvalidElementNamePrefix(pos) => {
                write!(f, "the 'xmlns' prefix is used at {pos}, but it must not be")
            }
            Self::DuplicatedNamespace(name, pos) => {
                write!(f, "namespace '{name}' at {pos} is already defined")
            }
            Self::UnknownNamespace(name, pos) => {
                write!(f, "an unknown namespace prefix '{name}' at {pos}")
            }
            Self::UnexpectedCloseTag {
                expected,
                actual,
                pos,
            } => write!(f, "expected '{expected}' tag, not '{actual}' at {pos}"),
            Self::UnknownEntityReference(name, pos) => {
                write!(f, "unknown entity reference '{name}' at {pos}")
            }
            Self::MalformedEntityReference(pos) => {
                write!(f, "malformed entity reference at {pos}")
            }
            Self::DuplicatedAttribute(name, pos) => {
                write!(f, "attribute '{name}' at {pos} is already defined")
            }
            Self::NoRootNode => f.write_str("the document does not have a root node"),
            Self::UnclosedRootNode => f.write_str("the root node was opened but never closed"),
            Self::UnexpectedDeclaration(pos) => write!(f, "unexpected XML declaration at {pos}"),
            Self::DtdDetected => f.write_str("XML with DTD detected"),
            Self::NodesLimitReached => f.write_str("nodes limit reached"),
            Self::AttributesLimitReached => f.write_str("more than 2^32 attributes were parsed"),
            Self::NamespacesLimitReached => {
                f.write_str("more than 2^16 unique namespaces were parsed")
            }
            Self::InvalidName(pos) => write!(f, "invalid name token at {pos}"),
            Self::NonXmlChar(c, pos) => write!(f, "a non-XML character {c:?} found at {pos}"),
            Self::InvalidChar {
                expected,
                actual,
                pos,
            } => write!(
                f,
                "expected '{}' not '{}' at {pos}",
                char::from(*expected),
                char::from(*actual)
            ),
            Self::ExpectedChar {
                expected,
                actual,
                pos,
            } => write!(
                f,
                "expected {expected} not '{}' at {pos}",
                char::from(*actual)
            ),
            Self::InvalidString(expected, pos) => write!(f, "expected '{expected}' at {pos}"),
            Self::InvalidComment(pos) => write!(f, "comment at {pos} contains '--'"),
            Self::InvalidCharacterData(pos) => {
                write!(f, "']]>' at {pos} is not allowed inside a character data")
            }
            Self::UnknownToken(pos) => write!(f, "unknown token at {pos}"),
            Self::UnexpectedEndOfStream => f.write_str("unexpected end of stream"),
            Self::InvalidCharacterReference(pos) => write!(
                f,
                "the character reference at {pos} is not a Unicode scalar value"
            ),
            Self::InvalidPiTarget(pos) => write!(
                f,
                "the processing instruction target at {pos} is reserved or contains ':'"
            ),
            Self::PiWithoutWhitespace(pos) => write!(
                f,
                "expected a whitespace between the processing instruction target and its data at {pos}"
            ),
            Self::InvalidXmlDeclaration(reason, pos) => {
                write!(f, "invalid XML declaration at {pos}: {reason}")
            }
            Self::InvalidQName(pos) => write!(f, "a name with an empty prefix at {pos}"),
            Self::EmptyPrefixedNamespace(prefix, pos) => write!(
                f,
                "the prefix '{prefix}' at {pos} is bound to an empty namespace name"
            ),
            Self::XmlnsPrefixDeclared(pos) => {
                write!(
                    f,
                    "the 'xmlns' prefix is declared at {pos}, but it must not be"
                )
            }
            Self::DuplicatedNamespaceDeclaration(name, pos) => {
                write!(
                    f,
                    "namespace declaration '{name}' at {pos} is already defined"
                )
            }
            Self::PrefixedXmlnsAttribute(name, pos) => write!(
                f,
                "attribute '{name}' at {pos}: a prefixed attribute named 'xmlns' is not supported"
            ),
            Self::DepthLimitReached => f.write_str("elements nested deeper than the limit"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_count_lines_and_characters() {
        let text = "ab\n\u{e9}cd\r\nx";
        assert_eq!(TextPos::at(text, 0), TextPos::new(1, 1));
        assert_eq!(TextPos::at(text, 2), TextPos::new(1, 3));
        assert_eq!(TextPos::at(text, 3), TextPos::new(2, 1));
        assert_eq!(TextPos::at(text, 5), TextPos::new(2, 2));
        // A carriage return is a character of the line; only line feeds end lines.
        assert_eq!(TextPos::at(text, 8), TextPos::new(2, 5));
        assert_eq!(TextPos::at(text, 9), TextPos::new(3, 1));
        assert_eq!(TextPos::at(text, 1000), TextPos::new(3, 2));
        assert_eq!(TextPos::new(3, 7).to_string(), "3:7");
    }
}
