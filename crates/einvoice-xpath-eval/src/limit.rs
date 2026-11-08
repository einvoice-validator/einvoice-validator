//! Limits that keep the processing of a document within a bounded call stack.
//!
//! Walking a tree recursively needs stack in proportion to its depth: without a bound, a
//! small document nested deeply enough would overflow the stack of its thread, which
//! aborts the whole process. A parsed document is therefore refused when an element is
//! nested deeper than a maximum depth ([`DEFAULT_MAX_DEPTH`] unless the caller sets
//! another one), with a [`LimitExceeded::Depth`] error.

use std::fmt;

/// The maximum element depth of a parsed document, unless the caller sets another one:
/// the document element is at depth 1, its child elements at depth 2, and so on.
///
/// 256, the value of libxml2's `xmlParserMaxDepth` (without `XML_PARSE_HUGE`). libxml2
/// compares it with the number of open ancestors of an element, so `xmllint` accepts 257
/// nested elements and refuses 258 ("Excessive depth in document: 256").
pub const DEFAULT_MAX_DEPTH: u32 = 256;

/// A limit that a document or a transformation exceeded: the input is refused rather
/// than processed with an unbounded call stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LimitExceeded {
    /// An element is nested deeper than `max` elements (the document element is at
    /// depth 1); `line` and `column` (1-based) are the start of its start tag.
    Depth {
        /// The maximum depth.
        max: u32,
        /// The line of the element's start tag.
        line: u32,
        /// The column of the element's start tag, in characters.
        column: u32,
    },
    /// Template rules, built-in template rules, named templates and stylesheet functions
    /// are invoked one inside another more than `max` deep.
    Nesting {
        /// The maximum nesting.
        max: u32,
    },
    /// The invocations in progress use more than `max` bytes of the call stack, measured
    /// from the start of the transformation: a stylesheet whose templates or functions
    /// need more stack per level than the nesting allows for.
    Stack {
        /// The maximum, in bytes.
        max: u32,
    },
}

impl LimitExceeded {
    /// The limit's value (bytes for [`LimitExceeded::Stack`]).
    #[must_use]
    pub const fn max(self) -> u32 {
        match self {
            Self::Depth { max, .. } | Self::Nesting { max } | Self::Stack { max } => max,
        }
    }
}

impl fmt::Display for LimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Depth { max, line, column } => write!(
                f,
                "{line}:{column}: element nested deeper than the maximum depth of {max}"
            ),
            Self::Nesting { max } => write!(
                f,
                "templates and functions invoked one inside another more than {max} deep"
            ),
            Self::Stack { max } => write!(
                f,
                "templates and functions invoked one inside another use more than {max} bytes of stack"
            ),
        }
    }
}
