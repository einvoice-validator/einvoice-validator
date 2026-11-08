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

use super::Parser;
use crate::ast::{
    Axis, NodeTest, PathPattern, PathStep, Pattern, PatternAlternative, PatternAnchor, PatternAxis,
    PatternLink, PatternStep,
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
    /// \[1\] `Pattern ::= PathPattern | Pattern '|' PathPattern`
    pub(crate) fn pattern(&mut self) -> Result<Pattern> {
        let mut alternatives = self.pattern_alternative()?;
        loop {
            for word in ["union", "intersect", "except"] {
                if self.peek()?.is_word(word) {
                    return Err(self.error_here(
                        ErrorKind::Unsupported,
                        format!("`{word}` operator in a pattern"),
                    ));
                }
            }
            if *self.peek()? != Tok::Pipe {
                return Ok(Pattern { alternatives });
            }
            self.bump()?;
            alternatives.extend(self.pattern_alternative()?);
        }
    }

    /// \[2\] `PathPattern`, plus the XSLT 3.0 `root()` and parenthesized forms: one
    /// alternative, or several when a parenthesized pattern is distributed.
    fn pattern_alternative(&mut self) -> Result<Vec<PatternAlternative>> {
        let tok = self.peek()?.clone();
        let anchor = match tok {
            Tok::Slash => {
                self.bump()?;
                if !self.starts_pattern_step()? {
                    return Ok(vec![PatternAlternative::DocumentRoot]);
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
                "root" => return self.root_pattern().map(|a| vec![a]),
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
        let first = self.path_step()?;
        let mut rest = Vec::new();
        loop {
            let link = match self.peek()? {
                Tok::Slash => PatternLink::Child,
                Tok::DoubleSlash => PatternLink::Descendant,
                _ => break,
            };
            self.bump()?;
            rest.push((link, self.path_step()?));
        }
        self.normalise(anchor, first, rest, start)
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

    /// A step: `(Pattern) Predicate*` (XSLT 3.0 `PostfixExprP`) or an axis step.
    fn path_step(&mut self) -> Result<PathStep> {
        if *self.peek()? != Tok::LParen {
            return self.pattern_step().map(PathStep::Axis);
        }
        self.bump()?;
        let inner = self.pattern()?;
        self.expect(&Tok::RParen, "`)` or `|`")?;
        let predicates = self.predicates()?;
        Ok(PathStep::Parenthesized {
            inner: Box::new(inner),
            predicates,
        })
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

    /// \[4\] `PatternStep ::= PatternAxis? NodeTest PredicateList`
    fn pattern_step(&mut self) -> Result<PatternStep> {
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
        let predicates = self.predicates()?;
        Ok(PatternStep {
            axis,
            test,
            predicates,
        })
    }
}
