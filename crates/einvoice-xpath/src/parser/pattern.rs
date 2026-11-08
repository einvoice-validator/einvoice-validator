//! XSLT match patterns.
//!
//! XSLT 2.0 section 5.5.2:
//!
//! ```text
//! [1] Pattern             ::= PathPattern | Pattern '|' PathPattern
//! [2] PathPattern         ::= RelativePathPattern | '/' RelativePathPattern?
//!                           | '//' RelativePathPattern | IdKeyPattern (('/' | '//') RelativePathPattern)?
//! [3] RelativePathPattern ::= PatternStep (('/' | '//') RelativePathPattern)?
//! [4] PatternStep         ::= PatternAxis? NodeTest PredicateList
//! [5] PatternAxis         ::= ('child' '::' | 'attribute' '::' | '@')
//! ```
//!
//! `IdKeyPattern` is not supported. Two XSLT 3.0 forms are accepted, as XSLT 3.0 processors
//! accept them in 2.0 stylesheets: `root()` alone, and parenthesized patterns as steps
//! (`PostfixExprP ::= '(' Pattern ')' PredicateList`), normalised as Saxon-HE 12.10 reads
//! them (see [`crate::parse_pattern`]).
//!
//! The depth of a path pattern counts its steps as the depth of a path expression does
//! (a pattern is matched step by step from its last one), and the depth of a
//! parenthesized step is that of its pattern: both are bounded by
//! [`MAX_DEPTH`](crate::MAX_DEPTH).

use super::Parser;
use crate::ast::{
    Axis, Expr, NodeTest, PathPattern, PathStep, Pattern, PatternAlternative, PatternAnchor,
    PatternAxis, PatternLink, PatternStep,
};
use crate::error::{ErrorKind, Result};
use crate::lexer::Tok;

/// Whether an alternative is a relative path of one axis step.
fn is_single_step(alternative: &PatternAlternative) -> bool {
    matches!(
        alternative,
        PatternAlternative::Path(PathPattern {
            anchor: PatternAnchor::Relative,
            first: PathStep::Axis(_),
            rest,
        }) if rest.is_empty()
    )
}

/// Whether a path has a parenthesized step.
fn path_has_parenthesized_step(path: &PathPattern) -> bool {
    std::iter::once(&path.first)
        .chain(path.rest.iter().map(|(_, step)| step))
        .any(|step| matches!(step, PathStep::Parenthesized { .. }))
}

/// Whether an alternative has a parenthesized step.
fn has_parenthesized_step(alternative: &PatternAlternative) -> bool {
    match alternative {
        PatternAlternative::DocumentRoot | PatternAlternative::RootFunction => false,
        PatternAlternative::Path(path) => path_has_parenthesized_step(path),
    }
}

impl Parser<'_> {
    /// \[1\] `Pattern ::= PathPattern | Pattern '|' PathPattern`, and the depth of its
    /// deepest alternative.
    pub(crate) fn pattern(&mut self) -> Result<(Pattern, u32)> {
        let mut pattern = (
            Pattern {
                alternatives: Vec::new(),
            },
            0,
        );
        loop {
            let alternative = self.pattern_alternative()?;
            if !self.pattern_continues(&mut pattern, alternative)? {
                return Ok(pattern);
            }
        }
    }

    /// Adds `alternative` to `pattern`; whether another alternative follows (after `|`).
    fn pattern_continues(
        &mut self,
        pattern: &mut (Pattern, u32),
        (alternatives, depth): (Vec<PatternAlternative>, u32),
    ) -> Result<bool> {
        pattern.0.alternatives.extend(alternatives);
        pattern.1 = pattern.1.max(depth);
        for word in ["union", "intersect", "except"] {
            if self.peek()?.is_word(word) {
                return Err(self.error_here(
                    ErrorKind::Unsupported,
                    format!("`{word}` operator in a pattern"),
                ));
            }
        }
        self.eat(&Tok::Pipe)
    }

    /// \[2\] `PathPattern`, plus the XSLT 3.0 `root()` and parenthesized forms: one
    /// alternative, or several when a parenthesized pattern is distributed; with the
    /// depth of the path.
    fn pattern_alternative(&mut self) -> Result<(Vec<PatternAlternative>, u32)> {
        let mut path = match self.pattern_start()? {
            PatternStart::Whole(alternative) => return Ok((vec![alternative], 1)),
            PatternStart::Path(path) => path,
        };
        loop {
            let step = self.path_step()?;
            if !self.pattern_step_end(&mut path, step)? {
                break;
            }
        }
        self.pattern_end(path)
    }

    /// What precedes the first step of a path pattern, or a whole alternative without
    /// steps (`/`, `root()`).
    fn pattern_start(&mut self) -> Result<PatternStart> {
        let tok = self.peek()?.clone();
        let anchor = match tok {
            Tok::Slash => {
                self.bump()?;
                if !self.starts_pattern_step()? {
                    return Ok(PatternStart::Whole(PatternAlternative::DocumentRoot));
                }
                PatternAnchor::Root
            }
            Tok::DoubleSlash => {
                self.bump()?;
                PatternAnchor::RootDescendant
            }
            Tok::Name {
                prefix: None,
                local,
            } if *self.peek2()? == Tok::LParen => match local.as_str() {
                "root" => return self.root_pattern().map(PatternStart::Whole),
                "id" | "key" | "doc" | "element-with-id" => {
                    return Err(
                        self.error_here(ErrorKind::Unsupported, format!("`{local}()` pattern"))
                    );
                }
                _ => PatternAnchor::Relative,
            },
            Tok::Dollar => {
                return Err(self.error_here(
                    ErrorKind::Unsupported,
                    "variable reference as a pattern (XSLT 3.0)",
                ));
            }
            Tok::Dot => {
                return Err(self.error_here(
                    ErrorKind::Unsupported,
                    "predicate pattern `.[...]` (XSLT 3.0)",
                ));
            }
            _ => PatternAnchor::Relative,
        };
        let start = self.offset()?;
        Ok(PatternStart::Path(Box::new(PatternPath {
            anchor,
            start,
            first: None,
            rest: Vec::new(),
            link: None,
            depth: 0,
        })))
    }

    /// Adds `step` to `path`; whether another step follows (after `/` or `//`).
    fn pattern_step_end(
        &mut self,
        path: &mut PatternPath,
        (step, depth): (Box<PathStep>, u32),
    ) -> Result<bool> {
        match path.link.take() {
            None => {
                path.first = Some(*step);
                path.depth = depth;
            }
            Some((link, offset)) => {
                path.rest.push((link, *step));
                path.depth = self.above([path.depth, depth], offset)?;
            }
        }
        let link = match self.peek()? {
            Tok::Slash => PatternLink::Child,
            Tok::DoubleSlash => PatternLink::Descendant,
            _ => return Ok(false),
        };
        let offset = self.bump()?.start;
        path.link = Some((link, offset));
        Ok(true)
    }

    /// The alternatives of the path read, normalised.
    #[expect(
        clippy::boxed_local,
        reason = "the caller, on the recursion of the parser, holds the box, not the value"
    )]
    fn pattern_end(&self, path: Box<PatternPath>) -> Result<(Vec<PatternAlternative>, u32)> {
        let PatternPath {
            anchor,
            start,
            first,
            rest,
            depth,
            ..
        } = *path;
        let first = first.ok_or_else(|| self.error_at(ErrorKind::Syntax, start, "no step"))?;
        Ok((self.normalise(anchor, first, rest, start)?, depth))
    }

    /// The alternatives of a parsed path, normalised as Saxon reads parenthesized
    /// patterns (see [`crate::parse_pattern`]).
    fn normalise(
        &self,
        anchor: PatternAnchor,
        first: PathStep,
        mut rest: Vec<(PatternLink, PathStep)>,
        start: usize,
    ) -> Result<Vec<PatternAlternative>> {
        let unsupported =
            |message: &str| Err(self.error_at(ErrorKind::Unsupported, start, message));
        // `(P)` alone is `P`.
        if anchor == PatternAnchor::Relative
            && rest.is_empty()
            && let PathStep::Parenthesized { inner, predicates } = &first
            && predicates.is_empty()
        {
            return Ok(inner.alternatives.clone());
        }
        // A path ending with `(A | B)` is the union of the paths ending with A and B.
        let ends_with_union = matches!(
            rest.last(),
            Some((_, PathStep::Parenthesized { predicates, .. })) if predicates.is_empty()
        ) || (rest.is_empty()
            && anchor != PatternAnchor::Relative
            && matches!(&first, PathStep::Parenthesized { predicates, .. } if predicates.is_empty()));
        if ends_with_union {
            let (last, prefix_link) = match rest.pop() {
                Some((link, last)) => (last, Some(link)),
                None => (first.clone(), None),
            };
            let PathStep::Parenthesized { inner, .. } = last else {
                return unsupported("a parenthesized step");
            };
            let mut out = Vec::with_capacity(inner.alternatives.len());
            for alternative in inner.alternatives {
                let PatternAlternative::Path(path) = alternative else {
                    return unsupported("`/` or `root()` in a parenthesized pattern after a step");
                };
                if path.anchor != PatternAnchor::Relative || path_has_parenthesized_step(&path) {
                    return unsupported(
                        "an absolute path or a parenthesized step in a parenthesized pattern after a step",
                    );
                }
                let mut steps = rest.clone();
                let (new_first, mut new_rest) = match prefix_link {
                    Some(link) => {
                        steps.push((link, path.first));
                        (first.clone(), steps)
                    }
                    None => (path.first, steps),
                };
                new_rest.extend(path.rest);
                out.extend(self.normalise_checked(anchor, new_first, new_rest, start)?);
            }
            return Ok(out);
        }
        self.normalise_checked(anchor, first, rest, start)
    }

    /// Checks the parenthesized steps a normalised path keeps.
    fn normalise_checked(
        &self,
        anchor: PatternAnchor,
        first: PathStep,
        rest: Vec<(PatternLink, PathStep)>,
        start: usize,
    ) -> Result<Vec<PatternAlternative>> {
        let unsupported =
            |message: &str| Err(self.error_at(ErrorKind::Unsupported, start, message));
        if let PathStep::Parenthesized { inner, .. } = &first {
            if inner.alternatives.iter().any(has_parenthesized_step) {
                return unsupported("a parenthesized pattern nested in a parenthesized step");
            }
            if anchor != PatternAnchor::Relative && !inner.alternatives.iter().all(is_single_step) {
                return unsupported(
                    "after `/` or `//`, a parenthesized step whose alternatives are not single steps",
                );
            }
        }
        for (_, step) in &rest {
            if let PathStep::Parenthesized { inner, predicates } = step {
                if !predicates.is_empty() {
                    return unsupported("a predicate on a parenthesized step after the first");
                }
                if !inner.alternatives.iter().all(is_single_step) {
                    return unsupported(
                        "a parenthesized step after the first whose alternatives are not single steps",
                    );
                }
            }
        }
        Ok(vec![PatternAlternative::Path(PathPattern {
            anchor,
            first,
            rest,
        })])
    }

    /// A step: `(Pattern) Predicate*` (XSLT 3.0 `PostfixExprP`) or an axis step; with its
    /// depth.
    fn path_step(&mut self) -> Result<(Box<PathStep>, u32)> {
        if !self.group_start()? {
            return self.pattern_step();
        }
        let (inner, depth) = self.pattern()?;
        self.group_end()?;
        let predicates = self.predicates(depth)?;
        self.leave();
        Ok(group(inner, predicates))
    }

    /// `(` starting a parenthesized step, one level deeper; false, and nothing read,
    /// otherwise.
    fn group_start(&mut self) -> Result<bool> {
        if *self.peek()? != Tok::LParen {
            return Ok(false);
        }
        self.enter()?;
        self.bump()?;
        Ok(true)
    }

    /// The `)` of a parenthesized step.
    fn group_end(&mut self) -> Result<()> {
        self.expect(&Tok::RParen, "`)` or `|`")?;
        Ok(())
    }

    /// `root()` alone (XSLT 3.0 `RootedPath` with no predicate and no step).
    fn root_pattern(&mut self) -> Result<PatternAlternative> {
        self.bump()?;
        self.bump()?;
        self.expect(&Tok::RParen, "`)`")?;
        if matches!(self.peek()?, Tok::LBracket | Tok::Slash | Tok::DoubleSlash) {
            return Err(self.error_here(
                ErrorKind::Unsupported,
                "predicates or steps after `root()` in a pattern",
            ));
        }
        Ok(PatternAlternative::RootFunction)
    }

    fn starts_pattern_step(&mut self) -> Result<bool> {
        Ok(matches!(
            self.peek()?,
            Tok::Name { .. }
                | Tok::PrefixWildcard(_)
                | Tok::LocalWildcard(_)
                | Tok::Star
                | Tok::At
                | Tok::LParen
        ))
    }

    /// \[4\] `PatternStep ::= PatternAxis? NodeTest PredicateList`, with its depth.
    fn pattern_step(&mut self) -> Result<(Box<PathStep>, u32)> {
        let offset = self.offset()?;
        let tok = self.peek()?.clone();
        let explicit = match tok {
            Tok::At => {
                self.bump()?;
                Some(PatternAxis::Attribute)
            }
            Tok::Name {
                prefix: None,
                local,
            } if *self.peek2()? == Tok::ColonColon => {
                let axis = match local.as_str() {
                    "child" => PatternAxis::Child,
                    "attribute" => PatternAxis::Attribute,
                    "self" | "descendant" | "descendant-or-self" | "parent" | "ancestor"
                    | "ancestor-or-self" | "following" | "following-sibling" | "preceding"
                    | "preceding-sibling" | "namespace" => {
                        return Err(self.error_at(
                            ErrorKind::Unsupported,
                            offset,
                            format!("axis `{local}::` in a pattern"),
                        ));
                    }
                    _ => {
                        return Err(self.error_at(
                            ErrorKind::Syntax,
                            offset,
                            format!("unknown axis `{local}::`"),
                        ));
                    }
                };
                self.bump()?;
                self.bump()?;
                Some(axis)
            }
            _ => None,
        };
        let lookup_axis = if explicit == Some(PatternAxis::Attribute) {
            Axis::Attribute
        } else {
            Axis::Child
        };
        let test = self.node_test(lookup_axis)?;
        let axis = explicit.unwrap_or(match test {
            NodeTest::Kind(crate::ast::KindTest::Attribute(_)) => PatternAxis::Attribute,
            _ => PatternAxis::Child,
        });
        let (predicates, depth) = self.predicates(1)?;
        let step = PatternStep {
            axis,
            test,
            predicates,
        };
        Ok((Box::new(PathStep::Axis(step)), depth))
    }
}

/// The parenthesized step `(inner)` with its predicates, and its depth.
fn group(inner: Pattern, (predicates, depth): (Vec<Expr>, u32)) -> (Box<PathStep>, u32) {
    let step = PathStep::Parenthesized {
        inner: Box::new(inner),
        predicates,
    };
    (Box::new(step), depth)
}

/// The start of an alternative, as read by [`Parser::pattern_start`].
#[derive(Debug)]
enum PatternStart {
    /// An alternative without steps.
    Whole(PatternAlternative),
    /// A path pattern, its steps not read yet.
    Path(Box<PatternPath>),
}

/// A path pattern being read.
#[derive(Debug)]
struct PatternPath {
    anchor: PatternAnchor,
    /// Where the first step starts.
    start: usize,
    first: Option<PathStep>,
    rest: Vec<(PatternLink, PathStep)>,
    /// The separator read before the next step, and its offset.
    link: Option<(PatternLink, usize)>,
    /// The depth of the steps read.
    depth: u32,
}
