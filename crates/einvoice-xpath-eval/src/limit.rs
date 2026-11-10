//! Limits that keep the processing of a document within a bounded call stack.
//!
//! Walking a tree recursively needs stack in proportion to its depth: without a bound, a
//! small document nested deeply enough would overflow the stack of its thread, which
//! aborts the whole process. A parsed document is therefore refused when an element is
//! nested deeper than a maximum depth ([`DEFAULT_MAX_DEPTH`] unless the caller sets
//! another one), with a [`LimitExceeded::Depth`] error.
//!
//! Evaluating an expression recurses over its tree, whose depth is bounded by
//! [`einvoice_xpath::MAX_DEPTH`]. A host that evaluates expressions inside a recursion of
//! its own (the templates and functions of a stylesheet) bounds the stack of the whole
//! with a [`StackLimit`] ([`DynamicContext::with_stack_limit`](crate::DynamicContext::with_stack_limit)):
//! the evaluation then ends with a [`LimitExceeded::Stack`] error when it would use more.

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
    /// The invocations in progress, and the expressions they evaluate, use more than `max`
    /// bytes of the call stack, measured from the start of the transformation (a
    /// [`StackLimit`]): a stylesheet whose templates or functions need more stack per
    /// level than the nesting allows for.
    Stack {
        /// The maximum, in bytes.
        max: u32,
    },
    /// A template ends with an `xsl:call-template`, which ends with another, and so on,
    /// more than `max` times in a row: tail calls, which need no stack (Saxon runs them in
    /// place, as the executor does), but would run for ever when the recursion does not
    /// end.
    TailCalls {
        /// The maximum number of tail calls in a row.
        max: u32,
    },
}

/// A bound on the call stack: at most `max` bytes deeper than the position of the stack
/// where it was taken ([`StackLimit::from_here`]). Stacks grow downwards on every
/// supported target, the WebAssembly shadow stack included; the distance is taken
/// either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackLimit {
    base: usize,
    max: u32,
}

impl StackLimit {
    /// A bound of `max` bytes below the current position of the stack.
    #[must_use]
    #[inline(never)]
    pub fn from_here(max: u32) -> Self {
        Self {
            base: stack_position(),
            max,
        }
    }

    /// The bound, in bytes.
    #[must_use]
    pub const fn max(self) -> u32 {
        self.max
    }

    /// The stack used between the position where the bound was taken and here.
    #[must_use]
    pub fn used(self) -> usize {
        self.base.abs_diff(stack_position())
    }

    /// [`LimitExceeded::Stack`] when the stack used is beyond the bound.
    ///
    /// # Errors
    ///
    /// The stack used is beyond the bound.
    pub fn check(self) -> Result<(), LimitExceeded> {
        if u32::try_from(self.used()).map_or(true, |used| used > self.max) {
            Err(LimitExceeded::Stack { max: self.max })
        } else {
            Ok(())
        }
    }
}

/// The position of the call stack: the address of a local of this function.
#[inline(never)]
fn stack_position() -> usize {
    let marker = 0_u8;
    std::hint::black_box(std::ptr::addr_of!(marker)).addr()
}

impl LimitExceeded {
    /// The limit's value (bytes for [`LimitExceeded::Stack`]).
    #[must_use]
    pub const fn max(self) -> u32 {
        match self {
            Self::Depth { max, .. }
            | Self::Nesting { max }
            | Self::Stack { max }
            | Self::TailCalls { max } => max,
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
            Self::TailCalls { max } => write!(
                f,
                "templates ending with a call of another template more than {max} times in a row"
            ),
        }
    }
}
