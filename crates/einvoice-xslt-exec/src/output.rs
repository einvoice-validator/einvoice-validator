//! The result of a transformation kept compactly ([`CompactOutput`]), and what reads it
//! as it is written ([`ResultObserver`]).

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt::{self, Write as _};
use std::hash::{BuildHasherDefault, DefaultHasher, Hash as _, Hasher};

use einvoice_xpath::QName;

use crate::error::{Result, err};

/// The serialized result document of
/// [`Executable::transform_compact`](crate::Executable::transform_compact), kept as a
/// sequence of pieces, each distinct piece stored once. A piece is what the serializer
/// writes from one tag to the next written in full: an element with its indentation, a
/// start tag, or text with the end tag that follows.
///
/// A compiled Schematron writes the same `svrl:fired-rule` element, with the same
/// indentation, each time a rule fires, and the same message for each node an
/// assertion fails on: on a large document, its SVRL report is mostly copies of a few
/// hundred pieces, and kept this way takes four bytes per fired rule instead of the
/// length of the element. The distinct pieces are kept in blocks that are never
/// reallocated, so that the store does not grow by copying.
///
/// Two outputs are equal when their texts are; [`fmt::Display`] writes the text and
/// [`fmt::Debug`] shows it as [`String`]'s does.
#[derive(Clone, Default)]
pub struct CompactOutput {
    /// The distinct pieces, one after the other, in blocks of up to [`BLOCK`] bytes (or
    /// of one larger piece); a piece is in one block.
    blocks: Vec<String>,
    /// Each distinct piece, by number: where it is in `blocks`.
    spans: Vec<Span>,
    /// The text: the numbers of its pieces, in order.
    sequence: Vec<u32>,
    /// The length of the text, in bytes.
    len: usize,
}

/// Where a distinct piece is: its block, and its byte range in it.
#[derive(Debug, Clone, Copy)]
struct Span {
    block: u32,
    start: u32,
    end: u32,
}

/// The size of the blocks of distinct pieces, once there are a few.
const BLOCK: usize = 64 << 10;

impl CompactOutput {
    /// The length of the text, in bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the text is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The pieces of the text, in order: their concatenation is the text.
    pub fn pieces(&self) -> impl Iterator<Item = &str> {
        self.sequence.iter().map(|&id| self.piece(id))
    }

    /// The text, in a string of the exact length.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut text = String::with_capacity(self.len);
        for piece in self.pieces() {
            text.push_str(piece);
        }
        text
    }

    /// Releases the room its buffers have beyond what they hold.
    pub fn shrink_to_fit(&mut self) {
        if let Some(last) = self.blocks.last_mut() {
            last.shrink_to_fit();
        }
        self.blocks.shrink_to_fit();
        self.spans.shrink_to_fit();
        self.sequence.shrink_to_fit();
    }

    fn piece(&self, id: u32) -> &str {
        let span = self.spans[id as usize];
        &self.blocks[span.block as usize][span.start as usize..span.end as usize]
    }

    /// Stores a new distinct piece: its number.
    fn add(&mut self, piece: &str) -> Result<u32> {
        let fits = self
            .blocks
            .last()
            .is_some_and(|b| b.capacity() - b.len() >= piece.len());
        if !fits {
            // Blocks grow from 1 KiB to BLOCK, each as large as those before it
            // together: a small result takes little room, a large one few blocks.
            let before: usize = self.blocks.iter().map(String::capacity).sum();
            let size = before.clamp(1 << 10, BLOCK).max(piece.len());
            self.blocks.push(String::with_capacity(size));
        }
        let block = self.blocks.len() - 1;
        let text = &mut self.blocks[block];
        let start = text.len();
        text.push_str(piece);
        let (Ok(id), Ok(block), Ok(start), Ok(end)) = (
            u32::try_from(self.spans.len()),
            u32::try_from(block),
            u32::try_from(start),
            u32::try_from(text.len()),
        ) else {
            return err(
                "INTERNAL",
                "the result has more than 2^32 distinct pieces or a piece of 4 GiB",
            );
        };
        self.spans.push(Span { block, start, end });
        Ok(id)
    }

    /// Whether the text is `text`, compared piece by piece.
    fn text_is(&self, text: &str) -> bool {
        if self.len != text.len() {
            return false;
        }
        let mut rest = text.as_bytes();
        for piece in self.pieces() {
            let (head, tail) = rest.split_at(piece.len());
            if head != piece.as_bytes() {
                return false;
            }
            rest = tail;
        }
        true
    }
}

impl PartialEq for CompactOutput {
    fn eq(&self, other: &Self) -> bool {
        if self.len != other.len {
            return false;
        }
        // Both texts walked piece by piece, the pieces cut at different places.
        let mut left = self.pieces().map(str::as_bytes);
        let mut right = other.pieces().map(str::as_bytes);
        let (mut a, mut b): (&[u8], &[u8]) = (&[], &[]);
        loop {
            if a.is_empty() {
                match left.next() {
                    Some(next) => a = next,
                    None => return b.is_empty() && right.all(<[u8]>::is_empty),
                }
                continue;
            }
            if b.is_empty() {
                match right.next() {
                    Some(next) => b = next,
                    None => return false,
                }
                continue;
            }
            let n = a.len().min(b.len());
            if a[..n] != b[..n] {
                return false;
            }
            a = &a[n..];
            b = &b[n..];
        }
    }
}

impl Eq for CompactOutput {}

impl PartialEq<str> for CompactOutput {
    fn eq(&self, other: &str) -> bool {
        self.text_is(other)
    }
}

impl PartialEq<&str> for CompactOutput {
    fn eq(&self, other: &&str) -> bool {
        self.text_is(other)
    }
}

impl fmt::Display for CompactOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for piece in self.pieces() {
            f.write_str(piece)?;
        }
        Ok(())
    }
}

impl fmt::Debug for CompactOutput {
    /// The text as [`String`]'s `Debug` shows it: quoted, each character escaped as
    /// [`char::escape_debug`] escapes it, except the single quote.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_char('"')?;
        for piece in self.pieces() {
            for c in piece.chars() {
                if c == '\'' {
                    f.write_char(c)?;
                } else {
                    for e in c.escape_debug() {
                        f.write_char(e)?;
                    }
                }
            }
        }
        f.write_char('"')
    }
}

impl From<&str> for CompactOutput {
    /// The text as a single piece (none for a text of 4 GiB or more, which the
    /// store cannot hold).
    fn from(text: &str) -> Self {
        let mut builder = CompactBuilder::default();
        if builder.push(text).is_err() {
            return Self::default();
        }
        builder.finish()
    }
}

/// A hasher for keys that are already hashes.
#[derive(Debug, Default)]
struct HashKey(u64);

impl Hasher for HashKey {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(b);
        }
    }

    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

/// Builds a [`CompactOutput`] piece by piece.
#[derive(Debug, Default)]
pub(crate) struct CompactBuilder {
    output: CompactOutput,
    /// The distinct pieces by the hash of their text. A piece whose hash is that of
    /// another text is stored as a new distinct piece, not indexed.
    index: HashMap<u64, u32, BuildHasherDefault<HashKey>>,
}

impl CompactBuilder {
    /// Appends a piece.
    pub(crate) fn push(&mut self, piece: &str) -> Result<()> {
        if piece.is_empty() {
            return Ok(());
        }
        let mut hasher = DefaultHasher::new();
        piece.hash(&mut hasher);
        let hash = hasher.finish();
        let id = match self.index.entry(hash) {
            Entry::Occupied(entry) if self.output.piece(*entry.get()) == piece => *entry.get(),
            Entry::Occupied(_) => self.output.add(piece)?,
            Entry::Vacant(vacant) => *vacant.insert(self.output.add(piece)?),
        };
        self.output.sequence.push(id);
        self.output.len += piece.len();
        Ok(())
    }

    /// Whether nothing was appended.
    pub(crate) const fn is_empty(&self) -> bool {
        self.output.is_empty()
    }

    pub(crate) fn finish(self) -> CompactOutput {
        self.output
    }
}

/// Reads the result document of
/// [`Executable::transform_compact`](crate::Executable::transform_compact) as it is written, as a parser of its text
/// would read it: the elements with their attributes, the character data (the
/// whitespace the serializer writes to indent included, as text of the element it is
/// in) and the comments, in document order. Nothing outside the document element but
/// comments is reported: the serializer writes no text there but line ends.
///
/// The text it reads is well-formed, except in two cases a parser would refuse and the
/// observer has to: a result with no element or more than one at its top, and a
/// comment that contains `--` or ends with `-`.
pub trait ResultObserver {
    /// The start of an element: its name, and its attributes as written (namespace
    /// declarations excluded; an attribute's prefix may differ from the one it was
    /// constructed with, never its namespace).
    fn start_element(&mut self, name: &QName, attributes: &[(QName, String)]);
    /// The end of the innermost open element.
    fn end_element(&mut self);
    /// Character data: adjacent calls make one text node.
    fn text(&mut self, text: &str);
    /// A comment.
    fn comment(&mut self, text: &str);
}

/// Observes nothing.
impl ResultObserver for () {
    fn start_element(&mut self, _name: &QName, _attributes: &[(QName, String)]) {}
    fn end_element(&mut self) {}
    fn text(&mut self, _text: &str) {}
    fn comment(&mut self, _text: &str) {}
}

/// The result of [`Executable::transform_compact`](crate::Executable::transform_compact).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CompactTransformation {
    /// The result document, serialized as `xsl:output` asks.
    pub output: CompactOutput,
    /// The warnings, as [`Transformation::warnings`](crate::Transformation::warnings).
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(pieces: &[&str]) -> CompactOutput {
        let mut builder = CompactBuilder::default();
        for piece in pieces {
            builder.push(piece).unwrap();
        }
        builder.finish()
    }

    #[test]
    fn keeps_each_distinct_piece_once() {
        let output = build(&["<a>", "\n <b/>", "", "\n <b/>", "x", "\n <b/>", "</a>"]);
        assert_eq!(output.to_text(), "<a>\n <b/>\n <b/>x\n <b/></a>");
        assert_eq!(output.len(), output.to_text().len());
        assert_eq!(output.blocks, ["<a>\n <b/>x</a>"]);
        assert_eq!(output.sequence, [0, 1, 1, 2, 1, 3]);
        assert_eq!(output.to_string(), output.to_text());
    }

    #[test]
    fn pieces_fill_blocks_and_a_larger_piece_has_its_own() {
        let small = "x".repeat(1000);
        let large = "y".repeat(BLOCK + 1);
        let mut pieces: Vec<String> = (0..100).map(|i| format!("{i}{small}")).collect();
        pieces.insert(30, large.clone());
        pieces.push(large);
        let refs: Vec<&str> = pieces.iter().map(String::as_str).collect();
        let mut output = build(&refs);
        assert_eq!(output.to_text(), pieces.concat());
        // 101 distinct pieces: the large one is stored once.
        assert_eq!(output.spans.len(), 101);
        // Blocks of 1, 1, 2, 4, ... KiB, then of BLOCK.
        let capacities: Vec<usize> = output.blocks.iter().map(String::capacity).collect();
        assert_eq!(capacities[..4], [1 << 10, 1 << 10, 2 << 10, 4 << 10]);
        assert!(capacities.contains(&(BLOCK + 1)), "{capacities:?}");
        assert!(capacities.iter().all(|&c| c <= BLOCK + 1));
        let distinct: usize = pieces.iter().take(101).map(String::len).sum();
        assert_eq!(
            output.blocks.iter().map(String::len).sum::<usize>(),
            distinct
        );
        output.shrink_to_fit();
        assert_eq!(output.to_text(), pieces.concat());
    }

    #[test]
    fn equal_when_the_texts_are() {
        let a = build(&["ab", "cd", "e"]);
        let b = build(&["a", "bcde"]);
        let c = build(&["ab", "cd", "f"]);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, build(&["ab", "cd"]));
        assert_eq!(a, "abcde");
        assert_ne!(a, "abcdf");
        assert_eq!(CompactOutput::default(), "");
        assert_eq!(CompactOutput::default(), build(&["", ""]));
        assert_eq!(CompactOutput::from("abcde"), a);
        assert_eq!(CompactOutput::from(""), CompactOutput::default());
    }

    #[test]
    fn debug_as_a_string() {
        let text = "<a b=\"'\">\n\t\\ \u{301}é\u{0}</a>";
        let output = build(&["<a b=\"'\">", "\n\t\\ \u{301}", "é\u{0}</a>"]);
        assert_eq!(format!("{output:?}"), format!("{text:?}"));
    }
}
