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
//! accept them in 2.0 stylesheets: `root()` alone, and a parenthesized pattern followed by
//! predicates, `(A | B)[p]`, with nothing after it.

use super::Parser;
use crate::ast::{
    Axis, NodeTest, PathPattern, Pattern, PatternAlternative, PatternAnchor, PatternAxis,
    PatternLink, PatternStep,
};
use crate::error::{ErrorKind, Result};
use crate::lexer::Tok;

impl Parser<'_> {
    /// \[1\] `Pattern ::= PathPattern | Pattern '|' PathPattern`
    pub(crate) fn pattern(&mut self) -> Result<Pattern> {
        let mut alternatives = vec![self.pattern_alternative()?];
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
            alternatives.push(self.pattern_alternative()?);
        }
    }

    /// \[2\] `PathPattern`, plus the XSLT 3.0 `root()` and `(pattern)[predicate]` forms.
    fn pattern_alternative(&mut self) -> Result<PatternAlternative> {
        let tok = self.peek()?.clone();
        match tok {
            Tok::Slash => {
                self.bump()?;
                if self.starts_pattern_step()? {
                    Ok(PatternAlternative::Path(
                        self.path_pattern(PatternAnchor::Root)?,
                    ))
                } else {
                    Ok(PatternAlternative::DocumentRoot)
                }
            }
            Tok::DoubleSlash => {
                self.bump()?;
                Ok(PatternAlternative::Path(
                    self.path_pattern(PatternAnchor::RootDescendant)?,
                ))
            }
            Tok::LParen => self.parenthesized_pattern(),
            Tok::Name {
                prefix: None,
                local,
            } if *self.peek2()? == Tok::LParen => match local.as_str() {
                "root" => self.root_pattern(),
                "id" | "key" | "doc" | "element-with-id" => {
                    Err(self.error_here(ErrorKind::Unsupported, format!("`{local}()` pattern")))
                }
                _ => Ok(PatternAlternative::Path(
                    self.path_pattern(PatternAnchor::Relative)?,
                )),
            },
            Tok::Dollar => Err(self.error_here(
                ErrorKind::Unsupported,
                "variable reference as a pattern (XSLT 3.0)",
            )),
            Tok::Dot => Err(self.error_here(
                ErrorKind::Unsupported,
                "predicate pattern `.[...]` (XSLT 3.0)",
            )),
            _ => Ok(PatternAlternative::Path(
                self.path_pattern(PatternAnchor::Relative)?,
            )),
        }
    }

    /// `(Pattern) Predicate+` (XSLT 3.0 `PostfixExprP`), nothing after it.
    fn parenthesized_pattern(&mut self) -> Result<PatternAlternative> {
        let open = self.bump()?.start;
        let inner = self.pattern()?;
        self.expect(&Tok::RParen, "`)` or `|`")?;
        let predicates = self.predicates()?;
        if predicates.is_empty() {
            return Err(self.error_at(
                ErrorKind::Unsupported,
                open,
                "parenthesized pattern without a predicate",
            ));
        }
        if matches!(self.peek()?, Tok::Slash | Tok::DoubleSlash) {
            return Err(self.error_here(
                ErrorKind::Unsupported,
                "path steps after a parenthesized pattern",
            ));
        }
        Ok(PatternAlternative::Parenthesized {
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
            Tok::Name { .. } | Tok::PrefixWildcard(_) | Tok::LocalWildcard(_) | Tok::Star | Tok::At
        ))
    }

    /// \[3\] `RelativePathPattern`, after the anchor.
    fn path_pattern(&mut self, anchor: PatternAnchor) -> Result<PathPattern> {
        let first = self.pattern_step()?;
        let mut rest = Vec::new();
        loop {
            let link = match self.peek()? {
                Tok::Slash => PatternLink::Child,
                Tok::DoubleSlash => PatternLink::Descendant,
                _ => {
                    return Ok(PathPattern {
                        anchor,
                        first,
                        rest,
                    });
                }
            };
            self.bump()?;
            rest.push((link, self.pattern_step()?));
        }
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
