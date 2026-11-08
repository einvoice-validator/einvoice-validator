//! Path expressions, steps, node tests and primary expressions (\[25\]–\[48\]).

use super::{NameRole, Parser};
use crate::ast::{
    Axis, AxisStep, Expr, KindTest, Literal, NameOrWildcard, NameTest, NodeTest, PathExpr,
    PathStart, Step,
};
use crate::error::{ErrorKind, Result};
use crate::lexer::Tok;
use crate::names::{QName, is_ncname, is_xml_whitespace};

/// The names that cannot be function names (XPath 2.0 appendix A.3), because a name
/// followed by `(` is a kind test or an expression keyword.
const RESERVED_FUNCTION_NAMES: [&str; 13] = [
    "attribute",
    "comment",
    "document-node",
    "element",
    "empty-sequence",
    "if",
    "item",
    "node",
    "processing-instruction",
    "schema-attribute",
    "schema-element",
    "text",
    "typeswitch",
];

/// The names that start a kind test (\[54\] `KindTest`) when followed by `(`.
const KIND_TEST_NAMES: [&str; 9] = [
    "attribute",
    "comment",
    "document-node",
    "element",
    "node",
    "processing-instruction",
    "schema-attribute",
    "schema-element",
    "text",
];

/// `descendant-or-self::node()`, the step `//` stands for.
pub(crate) fn descendant_or_self_step() -> Step {
    Step::Axis(AxisStep {
        axis: Axis::DescendantOrSelf,
        test: NodeTest::Kind(KindTest::AnyKind),
        predicates: Vec::new(),
    })
}

impl Parser<'_> {
    /// \[25\] `PathExpr ::= ("/" RelativePathExpr?) | ("//" RelativePathExpr) | RelativePathExpr`.
    /// A lone `/` is a path only when no relative path can follow it (the
    /// "leading-lone-slash" constraint of appendix A.2.1.1).
    pub(super) fn path_expr(&mut self) -> Result<Expr> {
        match self.peek()? {
            Tok::Slash => {
                self.bump()?;
                let steps = if self.starts_step()? {
                    self.relative_path()?
                } else {
                    Vec::new()
                };
                Ok(Expr::Path(PathExpr {
                    start: PathStart::Root,
                    steps,
                }))
            }
            Tok::DoubleSlash => {
                self.bump()?;
                let mut steps = vec![descendant_or_self_step()];
                steps.extend(self.relative_path()?);
                Ok(Expr::Path(PathExpr {
                    start: PathStart::Root,
                    steps,
                }))
            }
            _ => {
                let mut steps = self.relative_path()?;
                if steps.len() == 1
                    && let Some(Step::Expr(_)) = steps.first()
                    && let Some(Step::Expr(expr)) = steps.pop()
                {
                    return Ok(expr);
                }
                Ok(Expr::Path(PathExpr {
                    start: PathStart::Relative,
                    steps,
                }))
            }
        }
    }

    /// Whether the current token can start a step (\[27\] `StepExpr`).
    pub(super) fn starts_step(&mut self) -> Result<bool> {
        Ok(matches!(
            self.peek()?,
            Tok::Name { .. }
                | Tok::PrefixWildcard(_)
                | Tok::LocalWildcard(_)
                | Tok::Star
                | Tok::At
                | Tok::Dot
                | Tok::DotDot
                | Tok::Dollar
                | Tok::LParen
                | Tok::String(_)
                | Tok::Integer(_)
                | Tok::Decimal(_)
                | Tok::Double(_)
        ))
    }

    /// \[26\] `RelativePathExpr ::= StepExpr (("/" | "//") StepExpr)*`
    fn relative_path(&mut self) -> Result<Vec<Step>> {
        let mut steps = vec![self.step()?];
        loop {
            match self.peek()? {
                Tok::Slash => {
                    self.bump()?;
                }
                Tok::DoubleSlash => {
                    self.bump()?;
                    steps.push(descendant_or_self_step());
                }
                _ => return Ok(steps),
            }
            steps.push(self.step()?);
        }
    }

    /// \[27\] `StepExpr ::= FilterExpr | AxisStep`
    fn step(&mut self) -> Result<Step> {
        let offset = self.offset()?;
        let tok = self.peek()?.clone();
        let axis_step = |parser: &mut Self, axis: Axis| -> Result<Step> {
            let test = parser.node_test(axis)?;
            let predicates = parser.predicates()?;
            Ok(Step::Axis(AxisStep {
                axis,
                test,
                predicates,
            }))
        };
        match tok {
            Tok::DotDot => {
                self.bump()?;
                let predicates = self.predicates()?;
                Ok(Step::Axis(AxisStep {
                    axis: Axis::Parent,
                    test: NodeTest::Kind(KindTest::AnyKind),
                    predicates,
                }))
            }
            Tok::At => {
                self.bump()?;
                axis_step(self, Axis::Attribute)
            }
            Tok::Name {
                prefix: None,
                local,
            } if *self.peek2()? == Tok::ColonColon => {
                let axis = self.axis(&local, offset)?;
                self.bump()?;
                self.bump()?;
                axis_step(self, axis)
            }
            Tok::Name {
                prefix: None,
                local,
            } if *self.peek2()? == Tok::LParen && KIND_TEST_NAMES.contains(&local.as_str()) => {
                // \[31\]: the default axis is `attribute` for an attribute test, else `child`.
                let axis = if local == "attribute" {
                    Axis::Attribute
                } else {
                    Axis::Child
                };
                axis_step(self, axis)
            }
            Tok::Name { .. } if *self.peek2()? == Tok::LParen => {
                Ok(Step::Expr(self.filter_expr()?))
            }
            Tok::Name { .. } | Tok::Star | Tok::PrefixWildcard(_) | Tok::LocalWildcard(_) => {
                axis_step(self, Axis::Child)
            }
            Tok::Dollar
            | Tok::LParen
            | Tok::Dot
            | Tok::String(_)
            | Tok::Integer(_)
            | Tok::Decimal(_)
            | Tok::Double(_) => Ok(Step::Expr(self.filter_expr()?)),
            _ => Err(self.unexpected("an expression")),
        }
    }

    /// \[30\] `ForwardAxis`, \[33\] `ReverseAxis`, limited to the supported axes.
    fn axis(&self, name: &str, offset: usize) -> Result<Axis> {
        Ok(match name {
            "child" => Axis::Child,
            "attribute" => Axis::Attribute,
            "self" => Axis::SelfAxis,
            "descendant-or-self" => Axis::DescendantOrSelf,
            "following-sibling" => Axis::FollowingSibling,
            "parent" => Axis::Parent,
            "ancestor" => Axis::Ancestor,
            "ancestor-or-self" => Axis::AncestorOrSelf,
            "preceding-sibling" => Axis::PrecedingSibling,
            "preceding" => Axis::Preceding,
            "descendant" | "following" | "namespace" => {
                return Err(self.error_at(
                    ErrorKind::Unsupported,
                    offset,
                    format!("axis `{name}::`"),
                ));
            }
            _ => {
                return Err(self.error_at(
                    ErrorKind::Syntax,
                    offset,
                    format!("unknown axis `{name}::`"),
                ));
            }
        })
    }

    /// \[35\] `NodeTest ::= KindTest | NameTest`. Unprefixed names take the default element
    /// namespace, except on the attribute axis.
    pub(super) fn node_test(&mut self, axis: Axis) -> Result<NodeTest> {
        let offset = self.offset()?;
        let tok = self.peek()?.clone();
        let role = if axis == Axis::Attribute {
            NameRole::NoNamespace
        } else {
            NameRole::Element
        };
        match tok {
            Tok::Name {
                prefix: None,
                local,
            } if *self.peek2()? == Tok::LParen && KIND_TEST_NAMES.contains(&local.as_str()) => {
                Ok(NodeTest::Kind(self.kind_test()?))
            }
            Tok::Name { prefix, local } => {
                self.bump()?;
                Ok(NodeTest::Name(NameTest::Name(
                    self.resolve(prefix, local, role, offset)?,
                )))
            }
            Tok::Star => {
                self.bump()?;
                Ok(NodeTest::Name(NameTest::Any))
            }
            Tok::PrefixWildcard(prefix) => {
                self.bump()?;
                let namespace = self.resolve_prefix(&prefix, offset)?;
                Ok(NodeTest::Name(NameTest::AnyLocal { prefix, namespace }))
            }
            Tok::LocalWildcard(local) => {
                self.bump()?;
                Ok(NodeTest::Name(NameTest::AnyNamespace { local }))
            }
            _ => Err(self.unexpected("a node test")),
        }
    }

    /// \[54\] `KindTest`, limited to `node()`, `text()`, `comment()`,
    /// `processing-instruction()`, `element(name?)` and `attribute(name?)`.
    pub(super) fn kind_test(&mut self) -> Result<KindTest> {
        let offset = self.offset()?;
        let Tok::Name { local, .. } = self.bump()?.tok else {
            return Err(self.error_at(ErrorKind::Syntax, offset, "expected a kind test"));
        };
        self.expect(&Tok::LParen, "`(`")?;
        let test = match local.as_str() {
            "node" => KindTest::AnyKind,
            "text" => KindTest::Text,
            "comment" => KindTest::Comment,
            "processing-instruction" => {
                if *self.peek()? != Tok::RParen {
                    return Err(self.error_here(
                        ErrorKind::Unsupported,
                        "`processing-instruction()` with a name argument",
                    ));
                }
                KindTest::ProcessingInstruction
            }
            "element" => KindTest::Element(self.kind_test_name(NameRole::Element, "element")?),
            "attribute" => {
                KindTest::Attribute(self.kind_test_name(NameRole::NoNamespace, "attribute")?)
            }
            other => {
                return Err(self.error_at(
                    ErrorKind::Unsupported,
                    offset,
                    format!("kind test `{other}()`"),
                ));
            }
        };
        self.expect(&Tok::RParen, "`)`")?;
        Ok(test)
    }

    /// The optional name or `*` of `element(...)` (\[64\]) or `attribute(...)` (\[60\]); a
    /// type name after it is refused.
    fn kind_test_name(&mut self, role: NameRole, kind: &str) -> Result<Option<NameOrWildcard>> {
        let offset = self.offset()?;
        let name = match self.peek()?.clone() {
            Tok::RParen => return Ok(None),
            Tok::Star => {
                self.bump()?;
                NameOrWildcard::Wildcard
            }
            Tok::Name { prefix, local } => {
                self.bump()?;
                NameOrWildcard::Name(self.resolve(prefix, local, role, offset)?)
            }
            _ => return Err(self.unexpected("a name, `*` or `)`")),
        };
        if *self.peek()? == Tok::Comma {
            return Err(self.error_here(ErrorKind::Unsupported, format!("type name in `{kind}()`")));
        }
        Ok(Some(name))
    }

    /// \[39\] `PredicateList ::= Predicate*`, \[40\] `Predicate ::= "[" Expr "]"`
    pub(super) fn predicates(&mut self) -> Result<Vec<Expr>> {
        let mut predicates = Vec::new();
        while *self.peek()? == Tok::LBracket {
            self.bump()?;
            predicates.push(self.expr()?);
            self.expect(&Tok::RBracket, "`]`")?;
        }
        Ok(predicates)
    }

    /// \[38\] `FilterExpr ::= PrimaryExpr PredicateList`
    fn filter_expr(&mut self) -> Result<Expr> {
        let base = self.primary_expr()?;
        let predicates = self.predicates()?;
        if predicates.is_empty() {
            Ok(base)
        } else {
            Ok(Expr::Filter {
                base: Box::new(base),
                predicates,
            })
        }
    }

    /// \[41\] `PrimaryExpr ::= Literal | VarRef | ParenthesizedExpr | ContextItemExpr | FunctionCall`
    fn primary_expr(&mut self) -> Result<Expr> {
        let token = self.bump()?;
        Ok(match token.tok {
            Tok::String(s) => Expr::Literal(Literal::String(s)),
            Tok::Integer(n) => Expr::Literal(Literal::Integer(n)),
            Tok::Decimal(n) => Expr::Literal(Literal::Decimal(n)),
            Tok::Double(n) => Expr::Literal(Literal::Double(n)),
            Tok::Dot => Expr::ContextItem,
            Tok::Dollar => Expr::VarRef(self.qname(NameRole::NoNamespace, "a variable name")?),
            Tok::LParen => {
                if *self.peek()? == Tok::RParen {
                    self.bump()?;
                    return Ok(Expr::Sequence(Vec::new()));
                }
                let inner = self.expr()?;
                self.expect(&Tok::RParen, "`)`")?;
                inner
            }
            Tok::Name { prefix, local } => self.function_call(prefix, local, token.start)?,
            other => {
                return Err(self.error_at(
                    ErrorKind::Syntax,
                    token.start,
                    format!("expected an expression, found {}", other.describe()),
                ));
            }
        })
    }

    /// \[48\] `FunctionCall ::= QName "(" (ExprSingle ("," ExprSingle)*)? ")"`; the name has
    /// been read.
    fn function_call(
        &mut self,
        prefix: Option<String>,
        local: String,
        offset: usize,
    ) -> Result<Expr> {
        if prefix.is_none() {
            if local == "function" {
                return Err(self.error_at(
                    ErrorKind::Unsupported,
                    offset,
                    "inline function (XPath 3.0)",
                ));
            }
            if RESERVED_FUNCTION_NAMES.contains(&local.as_str()) {
                return Err(self.error_at(
                    ErrorKind::Syntax,
                    offset,
                    format!("`{local}(` is not allowed here (reserved function name)"),
                ));
            }
        }
        let name = self.resolve(prefix, local, NameRole::Function, offset)?;
        self.expect(&Tok::LParen, "`(`")?;
        let mut args = Vec::new();
        if *self.peek()? == Tok::RParen {
            self.bump()?;
        } else {
            loop {
                args.push(self.expr_single()?);
                if self.bump_if_comma()? {
                    continue;
                }
                self.expect(&Tok::RParen, "`,` or `)`")?;
                break;
            }
        }
        self.resolve_property_name(&name, &mut args);
        Ok(Expr::FunctionCall { name, args })
    }

    /// The argument of `system-property("prefix:local")`, a lexical `QName` resolved with the
    /// in-scope namespaces of the expression (XSLT 2.0 section 16.6.5), is rewritten as the
    /// equivalent `Q{uri}local` (an `EQName`, which XSLT 3.0 accepts there), so that the name
    /// no longer depends on namespaces the compiler does not see. A prefix that is not
    /// bound is left for the compiler to report.
    fn resolve_property_name(&self, name: &QName, args: &mut [Expr]) {
        if name.namespace.as_deref() != Some(crate::FN_NAMESPACE) || name.local != "system-property"
        {
            return;
        }
        let [Expr::Literal(Literal::String(text))] = args else {
            return;
        };
        let Some((prefix, local)) = text.trim_matches(is_xml_whitespace).split_once(':') else {
            return;
        };
        if !is_ncname(prefix) || !is_ncname(local) {
            return;
        }
        if let Some(uri) = self.namespaces.namespace_uri(prefix) {
            *text = format!("Q{{{uri}}}{local}");
        }
    }

    fn bump_if_comma(&mut self) -> Result<bool> {
        if *self.peek()? == Tok::Comma {
            self.bump()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
