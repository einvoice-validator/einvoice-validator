//! Path expressions, steps, node tests and primary expressions (\[25\]–\[48\]).

use super::{NameRole, Parsed, Parser};
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
    ///
    /// The depth of a path: that of its first step (of the root, for `/` and `//`), then
    /// one more level for each step after it, or the depth of that step plus one if
    /// deeper (`a/b/c` is evaluated as `(a/b)/c`).
    pub(super) fn path_expr(&mut self) -> Result<Parsed> {
        let mut path = self.path_start()?;
        if path.open {
            loop {
                self.step(&mut path)?;
                if !self.path_continues(&mut path)? {
                    break;
                }
            }
        }
        self.path_end(path)
    }

    /// The start of a path: the root (`/`, `//`) or nothing.
    fn path_start(&mut self) -> Result<PathBeingRead> {
        let offset = self.offset()?;
        let mut path = PathBeingRead {
            offset,
            start: PathStart::Relative,
            steps: Vec::new(),
            depth: 0,
            open: true,
        };
        match self.peek()? {
            Tok::Slash => {
                self.bump()?;
                path.start = PathStart::Root;
                path.depth = 1;
                path.open = self.starts_step()?;
            }
            Tok::DoubleSlash => {
                self.bump()?;
                path.start = PathStart::Root;
                path.steps.push(descendant_or_self_step());
                path.depth = 2;
            }
            _ => {}
        }
        Ok(path)
    }

    /// Adds a step of depth `depth`, read at `offset`, to `path`.
    fn add_step(
        &self,
        path: &mut PathBeingRead,
        step: Step,
        depth: u32,
        offset: usize,
    ) -> Result<()> {
        path.depth = if path.steps.is_empty() && path.start == PathStart::Relative {
            depth
        } else {
            self.above([path.depth, depth], offset)?
        };
        path.steps.push(step);
        Ok(())
    }

    /// After a step: whether another step follows (after `/` or `//`).
    fn path_continues(&mut self, path: &mut PathBeingRead) -> Result<bool> {
        let offset = self.offset()?;
        match self.peek()? {
            Tok::Slash => {
                self.bump()?;
            }
            Tok::DoubleSlash => {
                self.bump()?;
                path.steps.push(descendant_or_self_step());
                path.depth = self.above([path.depth], offset)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// The path read: a single filter expression without slash is that expression.
    fn path_end(&self, mut path: PathBeingRead) -> Result<Parsed> {
        if path.start == PathStart::Relative
            && path.steps.len() == 1
            && let Some(Step::Expr(_)) = path.steps.first()
            && let Some(Step::Expr(expr)) = path.steps.pop()
        {
            return Ok(Parsed::new(expr, path.depth));
        }
        let depth = self.checked(path.depth, path.offset)?;
        let expr = Expr::Path(PathExpr {
            start: path.start,
            steps: path.steps,
        });
        Ok(Parsed::new(expr, depth))
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

    /// \[27\] `StepExpr ::= FilterExpr | AxisStep`, added to `path`.
    fn step(&mut self, path: &mut PathBeingRead) -> Result<()> {
        let offset = self.offset()?;
        if let Some(step) = self.axis_step_start()? {
            let predicates = self.predicates(1)?;
            return self.axis_step_end(path, step, predicates, offset);
        }
        let filter = self.filter_expr()?;
        self.filter_step_end(path, filter, offset)
    }

    /// The end of a filter expression used as a step.
    fn filter_step_end(
        &self,
        path: &mut PathBeingRead,
        filter: Parsed,
        offset: usize,
    ) -> Result<()> {
        self.add_step(path, Step::Expr(*filter.expr), filter.depth, offset)
    }

    /// The axis step at the current token, read up to its predicates; `None` for a
    /// filter expression, not read yet.
    fn axis_step_start(&mut self) -> Result<Option<Box<AxisStep>>> {
        let axis_step = |axis: Axis, test: NodeTest| {
            Some(Box::new(AxisStep {
                axis,
                test,
                predicates: Vec::new(),
            }))
        };
        Ok(match self.step_start()? {
            StepStart::Axis(axis, test) => axis_step(axis, test),
            StepStart::Filter => None,
        })
    }

    /// The end of an axis step: its predicates and their depth.
    #[expect(
        clippy::boxed_local,
        reason = "the caller, on the recursion of the parser, holds the box, not the value"
    )]
    fn axis_step_end(
        &self,
        path: &mut PathBeingRead,
        mut step: Box<AxisStep>,
        (predicates, depth): (Vec<Expr>, u32),
        offset: usize,
    ) -> Result<()> {
        step.predicates = predicates;
        self.add_step(path, Step::Axis(*step), depth, offset)
    }

    /// What the step at the current token is: an axis step, read up to its predicates, or
    /// a filter expression, not read yet.
    fn step_start(&mut self) -> Result<StepStart> {
        let offset = self.offset()?;
        let tok = self.peek()?.clone();
        let axis_step = |parser: &mut Self, axis: Axis| -> Result<StepStart> {
            let test = parser.node_test(axis)?;
            Ok(StepStart::Axis(axis, test))
        };
        match tok {
            Tok::DotDot => {
                self.bump()?;
                Ok(StepStart::Axis(
                    Axis::Parent,
                    NodeTest::Kind(KindTest::AnyKind),
                ))
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
            Tok::Name { .. } if *self.peek2()? == Tok::LParen => Ok(StepStart::Filter),
            Tok::Name { .. } | Tok::Star | Tok::PrefixWildcard(_) | Tok::LocalWildcard(_) => {
                axis_step(self, Axis::Child)
            }
            Tok::Dollar
            | Tok::LParen
            | Tok::Dot
            | Tok::String(_)
            | Tok::Integer(_)
            | Tok::Decimal(_)
            | Tok::Double(_) => Ok(StepStart::Filter),
            _ => Err(self.unexpected("an expression")),
        }
    }

    /// \[30\] `ForwardAxis`, \[33\] `ReverseAxis`, limited to the supported axes.
    fn axis(&self, name: &str, offset: usize) -> Result<Axis> {
        Ok(match name {
            "child" => Axis::Child,
            "attribute" => Axis::Attribute,
            "self" => Axis::SelfAxis,
            "descendant" => Axis::Descendant,
            "descendant-or-self" => Axis::DescendantOrSelf,
            "following-sibling" => Axis::FollowingSibling,
            "parent" => Axis::Parent,
            "ancestor" => Axis::Ancestor,
            "ancestor-or-self" => Axis::AncestorOrSelf,
            "preceding-sibling" => Axis::PrecedingSibling,
            "preceding" => Axis::Preceding,
            "following" | "namespace" => {
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

    /// \[39\] `PredicateList ::= Predicate*`, \[40\] `Predicate ::= "[" Expr "]"`, and the
    /// depth of what they filter: `base` (the depth of the filtered expression), then one
    /// more level for each predicate, or the depth of that predicate plus one if deeper
    /// (each predicate filters the result of the previous ones).
    pub(super) fn predicates(&mut self, base: u32) -> Result<(Vec<Expr>, u32)> {
        let mut list = (Vec::new(), base);
        while let Some(offset) = self.predicate_start()? {
            let predicate = self.expr()?;
            self.predicate_end(&mut list, predicate, offset)?;
        }
        Ok(list)
    }

    /// `[`, if it is the current token: its offset.
    fn predicate_start(&mut self) -> Result<Option<usize>> {
        if *self.peek()? == Tok::LBracket {
            Ok(Some(self.bump()?.start))
        } else {
            Ok(None)
        }
    }

    /// `]`, after `predicate`, which is added to `list`.
    fn predicate_end(
        &mut self,
        list: &mut (Vec<Expr>, u32),
        predicate: Parsed,
        offset: usize,
    ) -> Result<()> {
        list.1 = self.above([list.1, predicate.depth], offset)?;
        list.0.push(*predicate.expr);
        self.expect(&Tok::RBracket, "`]`")?;
        Ok(())
    }

    /// \[38\] `FilterExpr ::= PrimaryExpr PredicateList`
    fn filter_expr(&mut self) -> Result<Parsed> {
        let base = self.primary_expr()?;
        let (predicates, depth) = self.predicates(base.depth)?;
        Ok(filtered(base, predicates, depth))
    }

    /// \[41\] `PrimaryExpr ::= Literal | VarRef | ParenthesizedExpr | ContextItemExpr | FunctionCall`.
    /// A parenthesized expression has the depth of its content: the tree does not keep
    /// the parentheses.
    fn primary_expr(&mut self) -> Result<Parsed> {
        match self.primary_start()? {
            Primary::Leaf(leaf) => Ok(leaf),
            Primary::Parenthesized => {
                let inner = self.expr()?;
                self.close_parenthesis()?;
                Ok(inner)
            }
            Primary::Call(call) => self.function_call(call),
        }
    }

    /// The `)` of a parenthesized expression.
    fn close_parenthesis(&mut self) -> Result<()> {
        self.expect(&Tok::RParen, "`)`")?;
        Ok(())
    }

    /// The primary expression at the current token: a leaf, read; a parenthesized
    /// expression, read up to `(`; a function call, read up to its name.
    fn primary_start(&mut self) -> Result<Primary> {
        let token = self.bump()?;
        Ok(Primary::Leaf(Parsed::leaf(match token.tok {
            Tok::String(s) => Expr::Literal(Literal::String(s)),
            Tok::Integer(n) => Expr::Literal(Literal::Integer(n)),
            Tok::Decimal(n) => Expr::Literal(Literal::Decimal(n)),
            Tok::Double(n) => Expr::Literal(Literal::Double(n)),
            Tok::Dot => Expr::ContextItem,
            Tok::Dollar => Expr::VarRef(self.qname(NameRole::NoNamespace, "a variable name")?),
            Tok::LParen => {
                if *self.peek()? == Tok::RParen {
                    self.bump()?;
                    Expr::Sequence(Vec::new())
                } else {
                    return Ok(Primary::Parenthesized);
                }
            }
            Tok::Name { prefix, local } => {
                let name = self.function_name(prefix, local, token.start)?;
                return Ok(Primary::Call(Box::new((name, token.start))));
            }
            other => {
                return Err(self.error_at(
                    ErrorKind::Syntax,
                    token.start,
                    format!("expected an expression, found {}", other.describe()),
                ));
            }
        })))
    }

    /// The name of a function call, read at `offset`: refused if it is reserved.
    fn function_name(&self, prefix: Option<String>, local: String, offset: usize) -> Result<QName> {
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
        self.resolve(prefix, local, NameRole::Function, offset)
    }

    /// \[48\] `FunctionCall ::= QName "(" (ExprSingle ("," ExprSingle)*)? ")"`; the name,
    /// and the offset where it was found, have been read. One level over its deepest
    /// argument.
    fn function_call(&mut self, call: Box<(QName, usize)>) -> Result<Parsed> {
        let mut args = (Vec::new(), 0);
        if self.arguments_start()? {
            loop {
                let arg = self.expr_single()?;
                if !self.argument_end(&mut args, arg)? {
                    break;
                }
            }
        }
        self.call_end(call, args)
    }

    /// `(`; whether an argument follows (else `)` has been read too).
    fn arguments_start(&mut self) -> Result<bool> {
        self.expect(&Tok::LParen, "`(`")?;
        Ok(!self.eat(&Tok::RParen)?)
    }

    /// `,` or `)` after `arg`, which is added to `args`; whether another argument follows.
    fn argument_end(&mut self, args: &mut (Vec<Expr>, u32), arg: Parsed) -> Result<bool> {
        args.1 = args.1.max(arg.depth);
        args.0.push(*arg.expr);
        if self.eat(&Tok::Comma)? {
            return Ok(true);
        }
        self.expect(&Tok::RParen, "`,` or `)`")?;
        Ok(false)
    }

    /// The end of a function call: its arguments and the depth of the deepest.
    #[expect(
        clippy::boxed_local,
        reason = "the caller, on the recursion of the parser, holds the box, not the value"
    )]
    fn call_end(&self, call: Box<(QName, usize)>, args: (Vec<Expr>, u32)) -> Result<Parsed> {
        let (name, offset) = *call;
        let (mut args, deepest) = args;
        self.resolve_property_name(&name, &mut args);
        let depth = self.above([deepest], offset)?;
        Ok(Parsed::new(Expr::FunctionCall { name, args }, depth))
    }

    /// The first argument of `system-property("prefix:local")`, `element-available` and
    /// `function-available`, a lexical `QName` resolved with the in-scope namespaces of
    /// the expression (XSLT 2.0 sections 16.6.5, 18.1.1 and 18.1.2), is rewritten as the
    /// equivalent `Q{uri}local` (an `EQName`, which XSLT 3.0 accepts there), so that the
    /// name no longer depends on namespaces the compiler does not see. A prefix that is
    /// not bound is left for the compiler to report.
    fn resolve_property_name(&self, name: &QName, args: &mut [Expr]) {
        if name.namespace.as_deref() != Some(crate::FN_NAMESPACE)
            || !matches!(
                name.local.as_str(),
                "system-property" | "element-available" | "function-available"
            )
        {
            return;
        }
        let Some(Expr::Literal(Literal::String(text))) = args.first_mut() else {
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
}

/// A path being read.
#[derive(Debug)]
struct PathBeingRead {
    /// Where it starts.
    offset: usize,
    start: PathStart,
    steps: Vec<Step>,
    /// The depth of the steps read so far (and of the root).
    depth: u32,
    /// Whether steps follow.
    open: bool,
}

/// A step, as read by [`Parser::step_start`].
#[derive(Debug)]
enum StepStart {
    /// An axis step, its predicates not read yet.
    Axis(Axis, NodeTest),
    /// A filter expression, not read yet.
    Filter,
}

/// A primary expression, as read by [`Parser::primary_start`].
#[derive(Debug)]
enum Primary {
    Leaf(Parsed),
    /// After `(`, not followed by `)`.
    Parenthesized,
    /// A function call, read up to its name, found at the offset.
    Call(Box<(QName, usize)>),
}

/// `base` with `predicates`, the whole `depth` deep.
fn filtered(base: Parsed, predicates: Vec<Expr>, depth: u32) -> Parsed {
    if predicates.is_empty() {
        base
    } else {
        let expr = Expr::Filter {
            base: base.expr,
            predicates,
        };
        Parsed::new(expr, depth)
    }
}
