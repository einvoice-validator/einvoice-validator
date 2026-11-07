//! Static analysis: an expression of the syntax tree becomes an evaluable tree in which
//! functions and variables are resolved, numeric literals are values, and every path step
//! knows how its result is to be ordered.
//!
//! The analysis also computes, for each subexpression, what is statically known of its
//! value: whether it has at most one item, whether its items are nodes or atomic values,
//! and for node sequences whether they come out in document order without duplicates
//! (and whether no node is an ancestor of another). These properties choose the
//! evaluation strategy, never the result: a path whose order is known is evaluated
//! lazily, one whose order is not is sorted; a general comparison reads first the
//! operand known to be a single item, as Saxon does.

use std::sync::Arc;

use einvoice_xdm::{AtomicType, AtomicValue, Decimal, Integer, XPathRegex, ops, parse_double};
use einvoice_xpath::{self as ast, AtomicKind, KindTest, NameOrWildcard, NameTest, QName};

use crate::context::{Cardinality, FunctionId, StaticContext, VariableId};
use crate::error::{Error, ErrorCode, Result, err};
use crate::functions::{self, Builtin, Returns};
use crate::tree::{Document, NodeAxis, NodeKind};

/// A compiled expression.
#[derive(Debug, Clone)]
pub(crate) enum Expr {
    Literal(AtomicValue),
    ContextItem,
    /// `/`: the document node of the context node's tree.
    Root,
    /// A range variable of `for`, `some`, `every`: its distance from the innermost one.
    Local(usize),
    External(VariableId),
    Sequence(Vec<Expr>),
    Filter {
        base: Box<Expr>,
        predicates: Vec<Predicate>,
    },
    Step(Box<Step>),
    Path {
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        mode: PathMode,
        /// Whether `rhs` reads the context size, so that `lhs` must be counted first.
        rhs_uses_last: bool,
    },
    /// `lhs | rhs`; each flag says whether that operand is known to deliver document
    /// order without duplicates (it is read lazily) or not (it is sorted first).
    Union {
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        lhs_ordered: bool,
        rhs_ordered: bool,
    },
    Negate(Box<Expr>),
    Arithmetic {
        op: ops::ArithmeticOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Range(Box<Expr>, Box<Expr>),
    GeneralComparison {
        op: ops::ComparisonOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        strategy: CompareStrategy,
    },
    ValueComparison {
        op: ops::ComparisonOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    If {
        condition: Box<Expr>,
        then: Box<Expr>,
        otherwise: Box<Expr>,
    },
    For {
        sequence: Box<Expr>,
        body: Box<Expr>,
    },
    Quantified {
        every: bool,
        sequence: Box<Expr>,
        satisfies: Box<Expr>,
    },
    InstanceOf {
        operand: Box<Expr>,
        item: ItemTest,
        occurrence: ast::Occurrence,
    },
    Castable {
        operand: Box<Expr>,
        target: AtomicType,
        allows_empty: bool,
    },
    Call {
        function: Builtin,
        args: Vec<Expr>,
        data: CallData,
    },
    HostCall {
        id: FunctionId,
        args: Vec<Expr>,
    },
}

/// What a call to a built-in function needs beyond its arguments.
#[derive(Debug, Clone)]
pub(crate) enum CallData {
    None,
    /// The regular expression of `matches`, `replace` or `tokenize` when pattern and
    /// flags are literals, compiled once (an invalid one is reported when called).
    Regex(Arc<std::result::Result<XPathRegex, Error>>),
    /// The property named by the literal argument of `system-property`.
    Property(QName),
}

/// An axis step.
#[derive(Debug, Clone)]
pub(crate) struct Step {
    pub(crate) axis: NodeAxis,
    pub(crate) test: NodeTest,
    pub(crate) predicates: Vec<Predicate>,
}

/// A predicate, with how its value is to be read.
#[derive(Debug, Clone)]
pub(crate) struct Predicate {
    pub(crate) expr: Expr,
    pub(crate) kind: PredicateKind,
    /// Whether the predicate reads the context size (`last()`), so that the sequence it
    /// filters must be counted first.
    pub(crate) uses_last: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PredicateKind {
    /// An integer literal: keeps the item at that position (`None`: at no position).
    Position(Option<usize>),
    /// A value that is never numeric: its effective boolean value decides.
    Boolean,
    /// Numeric (a position) or not (an effective boolean value), known at run time.
    Dynamic,
}

/// How the result of `lhs/rhs` is assembled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathMode {
    /// Nodes, already in document order without duplicates: concatenated lazily.
    Ordered,
    /// Nodes in an unknown order: sorted, duplicates removed.
    Sort,
    /// Atomic values: concatenated lazily.
    Atomic,
    /// Unknown until run time: nodes are sorted, atomic values kept, a mix is an error.
    Dynamic,
}

/// Which operand of a general comparison is read first (Saxon's comparison
/// cardinalities).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompareStrategy {
    /// Both at most one item: the left one first; an empty one gives false.
    OneToOne,
    /// The right one is at most one item: it is evaluated first, then the left one read.
    ManyToOne,
    /// The left one is at most one item: it is evaluated first, then the right one read.
    OneToMany,
    /// Items read from each side in turn, each compared with those read from the other.
    ManyToMany,
}

/// A node test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NodeTest {
    AnyNode,
    Kind(NodeKind),
    Named { kind: NodeKind, name: NameMatch },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NameMatch {
    Any,
    Exact {
        namespace: Option<String>,
        local: String,
    },
    Namespace(String),
    Local(String),
}

impl NodeTest {
    pub(crate) fn matches(&self, doc: &Document, index: u32) -> bool {
        let kind = doc.kind(index);
        match self {
            Self::AnyNode => true,
            Self::Kind(k) => kind == *k,
            Self::Named { kind: k, name } => {
                if kind != *k {
                    return false;
                }
                let Some(node_name) = doc.name(index) else {
                    return false;
                };
                match name {
                    NameMatch::Any => true,
                    NameMatch::Exact { namespace, local } => {
                        node_name.local == *local && node_name.namespace == *namespace
                    }
                    NameMatch::Namespace(namespace) => {
                        node_name.namespace.as_deref() == Some(namespace.as_str())
                    }
                    NameMatch::Local(local) => node_name.local == *local,
                }
            }
        }
    }
}

/// The item type of `instance of`.
#[derive(Debug, Clone)]
pub(crate) enum ItemTest {
    Atomic(AtomicType),
    Node(NodeTest),
}

/// What is known of the items of a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kinds {
    Nodes,
    Atomics,
    Unknown,
}

/// What is known of a value, for the predicates it may appear in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// Never a single numeric value: its effective boolean value decides.
    NotNumeric,
    /// May be numeric.
    Unknown,
}

/// Statically known properties of a value.
#[derive(Debug, Clone, Copy)]
struct Props {
    max_one: bool,
    kinds: Kinds,
    class: Class,
    /// Nodes in document order without duplicates.
    ordered: bool,
    /// No node is an ancestor of another (only meaningful with `ordered`).
    peer: bool,
}

impl Props {
    const fn atomic(max_one: bool, class: Class) -> Self {
        Self {
            max_one,
            kinds: Kinds::Atomics,
            class,
            ordered: false,
            peer: false,
        }
    }

    const BOOLEAN: Self = Self::atomic(true, Class::NotNumeric);

    const fn unknown(max_one: bool) -> Self {
        Self {
            max_one,
            kinds: Kinds::Unknown,
            class: Class::Unknown,
            ordered: max_one,
            peer: max_one,
        }
    }

    const fn nodes(max_one: bool, ordered: bool, peer: bool) -> Self {
        Self {
            max_one,
            kinds: Kinds::Nodes,
            class: Class::NotNumeric,
            ordered,
            peer,
        }
    }

    /// Either of two values (the branches of `if`).
    fn either(a: Self, b: Self) -> Self {
        Self {
            max_one: a.max_one && b.max_one,
            kinds: if a.kinds == b.kinds {
                a.kinds
            } else {
                Kinds::Unknown
            },
            class: if a.class == Class::NotNumeric && b.class == Class::NotNumeric {
                Class::NotNumeric
            } else {
                Class::Unknown
            },
            ordered: a.ordered && b.ordered,
            peer: a.peer && b.peer,
        }
    }
}

/// An expression ready to be evaluated: functions and variables resolved against a
/// static context. It holds no node and can be shared between threads.
#[derive(Debug, Clone)]
pub struct CompiledExpr {
    pub(crate) expr: Expr,
    max_one: bool,
}

impl CompiledExpr {
    /// Whether the value is statically known to have at most one item.
    #[must_use]
    pub const fn is_at_most_one(&self) -> bool {
        self.max_one
    }
}

/// Compiles an expression in a static context.
///
/// # Errors
///
/// `XPST0008` for a variable not in scope, `XPST0017` for an unknown function or arity,
/// `XTDE1390` for an invalid `system-property` name, [`ErrorCode::Unsupported`] for a
/// function of the specifications that is not implemented or a literal out of range.
pub fn compile(expr: &ast::Expr, context: &dyn StaticContext) -> Result<CompiledExpr> {
    let mut compiler = Compiler {
        context,
        locals: Vec::new(),
    };
    let (expr, props) = compiler.expr(expr)?;
    Ok(CompiledExpr {
        expr,
        max_one: props.max_one,
    })
}

struct Compiler<'a> {
    context: &'a dyn StaticContext,
    /// The range variables in scope, innermost last.
    locals: Vec<QName>,
}

fn boxed(e: Expr) -> Box<Expr> {
    Box::new(e)
}

fn comparison_op(kind: ast::CompareKind) -> ops::ComparisonOp {
    match kind {
        ast::CompareKind::Eq => ops::ComparisonOp::Equal,
        ast::CompareKind::Ne => ops::ComparisonOp::NotEqual,
        ast::CompareKind::Lt => ops::ComparisonOp::Less,
        ast::CompareKind::Le => ops::ComparisonOp::LessOrEqual,
        ast::CompareKind::Gt => ops::ComparisonOp::Greater,
        ast::CompareKind::Ge => ops::ComparisonOp::GreaterOrEqual,
    }
}

fn arithmetic_op(op: ast::ArithmeticOp) -> ops::ArithmeticOp {
    match op {
        ast::ArithmeticOp::Add => ops::ArithmeticOp::Add,
        ast::ArithmeticOp::Subtract => ops::ArithmeticOp::Subtract,
        ast::ArithmeticOp::Multiply => ops::ArithmeticOp::Multiply,
        ast::ArithmeticOp::Divide => ops::ArithmeticOp::Divide,
        ast::ArithmeticOp::IntegerDivide => ops::ArithmeticOp::IntegerDivide,
        ast::ArithmeticOp::Modulo => ops::ArithmeticOp::Modulo,
    }
}

pub(crate) const fn atomic_type(kind: AtomicKind) -> AtomicType {
    match kind {
        AtomicKind::String => AtomicType::String,
        AtomicKind::AnyUri => AtomicType::AnyUri,
        AtomicKind::Boolean => AtomicType::Boolean,
        AtomicKind::Integer => AtomicType::Integer,
        AtomicKind::Decimal => AtomicType::Decimal,
        AtomicKind::Date => AtomicType::Date,
    }
}

fn node_axis(axis: ast::Axis) -> NodeAxis {
    match axis {
        ast::Axis::Child => NodeAxis::Child,
        ast::Axis::Attribute => NodeAxis::Attribute,
        ast::Axis::SelfAxis => NodeAxis::SelfAxis,
        ast::Axis::DescendantOrSelf => NodeAxis::DescendantOrSelf,
        ast::Axis::FollowingSibling => NodeAxis::FollowingSibling,
        ast::Axis::Parent => NodeAxis::Parent,
        ast::Axis::Ancestor => NodeAxis::Ancestor,
        ast::Axis::AncestorOrSelf => NodeAxis::AncestorOrSelf,
        ast::Axis::PrecedingSibling => NodeAxis::PrecedingSibling,
        ast::Axis::Preceding => NodeAxis::Preceding,
    }
}

fn name_match(name: Option<&NameOrWildcard>) -> NameMatch {
    match name {
        None | Some(NameOrWildcard::Wildcard) => NameMatch::Any,
        Some(NameOrWildcard::Name(q)) => NameMatch::Exact {
            namespace: q.namespace.clone(),
            local: q.local.clone(),
        },
    }
}

/// The node test of a kind test.
pub(crate) fn kind_test(test: &KindTest) -> NodeTest {
    match test {
        KindTest::AnyKind => NodeTest::AnyNode,
        KindTest::Text => NodeTest::Kind(NodeKind::Text),
        KindTest::Comment => NodeTest::Kind(NodeKind::Comment),
        KindTest::ProcessingInstruction => NodeTest::Kind(NodeKind::ProcessingInstruction),
        KindTest::Element(name) => NodeTest::Named {
            kind: NodeKind::Element,
            name: name_match(name.as_ref()),
        },
        KindTest::Attribute(name) => NodeTest::Named {
            kind: NodeKind::Attribute,
            name: name_match(name.as_ref()),
        },
    }
}

fn node_test(test: &ast::NodeTest, axis: NodeAxis) -> NodeTest {
    let principal = if axis == NodeAxis::Attribute {
        NodeKind::Attribute
    } else {
        NodeKind::Element
    };
    match test {
        ast::NodeTest::Kind(kind) => kind_test(kind),
        ast::NodeTest::Name(name) => NodeTest::Named {
            kind: principal,
            name: match name {
                NameTest::Name(q) => NameMatch::Exact {
                    namespace: q.namespace.clone(),
                    local: q.local.clone(),
                },
                NameTest::Any => NameMatch::Any,
                NameTest::AnyLocal { namespace, .. } => NameMatch::Namespace(namespace.clone()),
                NameTest::AnyNamespace { local } => NameMatch::Local(local.clone()),
            },
        },
    }
}

/// Whether an expression calls `last()` (or, with `position`, `position()`) anywhere.
/// Conservative: a call inside a nested focus counts too.
fn calls_focus(expr: &Expr, position: bool) -> bool {
    let any = |items: &[Expr]| items.iter().any(|e| calls_focus(e, position));
    let preds =
        |predicates: &[Predicate]| predicates.iter().any(|p| calls_focus(&p.expr, position));
    match expr {
        Expr::Literal(_) | Expr::ContextItem | Expr::Root | Expr::Local(_) | Expr::External(_) => {
            false
        }
        Expr::Call { function, args, .. } => {
            *function == Builtin::Last || (position && *function == Builtin::Position) || any(args)
        }
        Expr::HostCall { args, .. } | Expr::Sequence(args) => any(args),
        Expr::Filter { base, predicates } => calls_focus(base, position) || preds(predicates),
        Expr::Step(step) => preds(&step.predicates),
        Expr::Path { lhs, rhs, .. }
        | Expr::Union { lhs, rhs, .. }
        | Expr::Arithmetic { lhs, rhs, .. }
        | Expr::Range(lhs, rhs)
        | Expr::GeneralComparison { lhs, rhs, .. }
        | Expr::ValueComparison { lhs, rhs, .. }
        | Expr::And(lhs, rhs)
        | Expr::Or(lhs, rhs)
        | Expr::For {
            sequence: lhs,
            body: rhs,
        }
        | Expr::Quantified {
            sequence: lhs,
            satisfies: rhs,
            ..
        } => calls_focus(lhs, position) || calls_focus(rhs, position),
        Expr::Negate(e)
        | Expr::InstanceOf { operand: e, .. }
        | Expr::Castable { operand: e, .. } => calls_focus(e, position),
        Expr::If {
            condition,
            then,
            otherwise,
        } => {
            calls_focus(condition, position)
                || calls_focus(then, position)
                || calls_focus(otherwise, position)
        }
    }
}

/// The axis of an expression used as a path step, if it is an axis step (`.` counts as
/// `self::node()`).
fn step_axis(expr: &Expr) -> Option<NodeAxis> {
    match expr {
        Expr::Step(step) => Some(step.axis),
        Expr::ContextItem => Some(NodeAxis::SelfAxis),
        _ => None,
    }
}

/// Whether `lhs/rhs` yields nodes in document order without duplicates when `lhs` does.
fn path_is_ordered(lhs: Props, axis: NodeAxis) -> bool {
    if lhs.max_one {
        // A step from one node: axis results are delivered in document order.
        return true;
    }
    if !lhs.ordered {
        return false;
    }
    match axis {
        NodeAxis::Attribute | NodeAxis::SelfAxis => true,
        NodeAxis::Child | NodeAxis::Descendant | NodeAxis::DescendantOrSelf => lhs.peer,
        _ => false,
    }
}

/// Properties of an axis step from a single node.
fn step_props(axis: NodeAxis, test: &NodeTest, predicates: &[Predicate]) -> Props {
    let positional = predicates
        .iter()
        .any(|p| matches!(p.kind, PredicateKind::Position(_)));
    let one_attribute = axis == NodeAxis::Attribute
        && matches!(
            test,
            NodeTest::Named {
                name: NameMatch::Exact { .. },
                ..
            }
        );
    let max_one =
        positional || one_attribute || matches!(axis, NodeAxis::SelfAxis | NodeAxis::Parent);
    let peer = max_one
        || matches!(
            axis,
            NodeAxis::Child
                | NodeAxis::Attribute
                | NodeAxis::FollowingSibling
                | NodeAxis::PrecedingSibling
        );
    Props::nodes(max_one, true, peer)
}

/// A path step as the compiler sees it: a step of the syntax tree, or `//name` merged
/// into one `descendant::name` step.
enum PathStep<'a> {
    Ast(&'a ast::Step),
    Descendant(&'a ast::AxisStep),
}

impl Compiler<'_> {
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per expression form of the syntax tree"
    )]
    fn expr(&mut self, expr: &ast::Expr) -> Result<(Expr, Props)> {
        match expr {
            ast::Expr::Literal(literal) => literal_value(literal),
            ast::Expr::VarRef(name) => self.variable(name),
            ast::Expr::ContextItem => Ok((Expr::ContextItem, Props::unknown(true))),
            ast::Expr::FunctionCall { name, args } => self.call(name, args),
            ast::Expr::Sequence(items) => self.sequence(items),
            ast::Expr::Filter { base, predicates } => {
                let (base, props) = self.expr(base)?;
                let predicates = self.predicates(predicates)?;
                let positional = predicates
                    .iter()
                    .any(|p| matches!(p.kind, PredicateKind::Position(_)));
                let props = Props {
                    max_one: props.max_one || positional,
                    ..props
                };
                Ok((
                    Expr::Filter {
                        base: boxed(base),
                        predicates,
                    },
                    props,
                ))
            }
            ast::Expr::Path(path) => self.path(path),
            ast::Expr::Negate(operand) => {
                let (operand, _) = self.expr(operand)?;
                Ok((
                    Expr::Negate(boxed(operand)),
                    Props::atomic(true, Class::Unknown),
                ))
            }
            ast::Expr::Castable { operand, target } => {
                let (operand, _) = self.expr(operand)?;
                Ok((
                    Expr::Castable {
                        operand: boxed(operand),
                        target: atomic_type(target.atomic.kind),
                        allows_empty: target.allows_empty,
                    },
                    Props::BOOLEAN,
                ))
            }
            ast::Expr::InstanceOf { operand, target } => {
                let (operand, _) = self.expr(operand)?;
                let item = match &target.item {
                    ast::ItemType::Atomic(atomic) => ItemTest::Atomic(atomic_type(atomic.kind)),
                    ast::ItemType::Kind(kind) => ItemTest::Node(kind_test(kind)),
                };
                Ok((
                    Expr::InstanceOf {
                        operand: boxed(operand),
                        item,
                        occurrence: target.occurrence,
                    },
                    Props::BOOLEAN,
                ))
            }
            ast::Expr::Union(lhs, rhs) => {
                let (lhs, lp) = self.expr(lhs)?;
                let (rhs, rp) = self.expr(rhs)?;
                Ok((
                    Expr::Union {
                        lhs: boxed(lhs),
                        rhs: boxed(rhs),
                        lhs_ordered: lp.ordered && lp.kinds == Kinds::Nodes,
                        rhs_ordered: rp.ordered && rp.kinds == Kinds::Nodes,
                    },
                    Props::nodes(false, true, false),
                ))
            }
            ast::Expr::Arithmetic { op, lhs, rhs } => {
                let (lhs, _) = self.expr(lhs)?;
                let (rhs, _) = self.expr(rhs)?;
                Ok((
                    Expr::Arithmetic {
                        op: arithmetic_op(*op),
                        lhs: boxed(lhs),
                        rhs: boxed(rhs),
                    },
                    Props::atomic(true, Class::Unknown),
                ))
            }
            ast::Expr::Range(lhs, rhs) => {
                let (lhs, _) = self.expr(lhs)?;
                let (rhs, _) = self.expr(rhs)?;
                Ok((
                    Expr::Range(boxed(lhs), boxed(rhs)),
                    Props::atomic(false, Class::Unknown),
                ))
            }
            ast::Expr::Comparison { op, lhs, rhs } => self.comparison(*op, lhs, rhs),
            ast::Expr::And(lhs, rhs) | ast::Expr::Or(lhs, rhs) => {
                let (l, _) = self.expr(lhs)?;
                let (r, _) = self.expr(rhs)?;
                let e = if matches!(expr, ast::Expr::And(..)) {
                    Expr::And(boxed(l), boxed(r))
                } else {
                    Expr::Or(boxed(l), boxed(r))
                };
                Ok((e, Props::BOOLEAN))
            }
            ast::Expr::If {
                condition,
                then,
                otherwise,
            } => {
                let (condition, _) = self.expr(condition)?;
                let (then, a) = self.expr(then)?;
                let (otherwise, b) = self.expr(otherwise)?;
                Ok((
                    Expr::If {
                        condition: boxed(condition),
                        then: boxed(then),
                        otherwise: boxed(otherwise),
                    },
                    Props::either(a, b),
                ))
            }
            ast::Expr::For { bindings, body } => self
                .bindings(bindings, &mut |c| {
                    let (body, props) = c.expr(body)?;
                    Ok((body, props.kinds))
                })
                .map(|(e, kinds)| {
                    (
                        e,
                        Props {
                            kinds,
                            ..Props::unknown(false)
                        },
                    )
                }),
            ast::Expr::Quantified {
                quantifier,
                bindings,
                satisfies,
            } => self.quantified(*quantifier == ast::Quantifier::Every, bindings, satisfies),
        }
    }

    fn variable(&self, name: &QName) -> Result<(Expr, Props)> {
        if let Some(depth) = self
            .locals
            .iter()
            .rev()
            .position(|local| local.same_expanded_name(name))
        {
            return Ok((Expr::Local(depth), Props::unknown(true)));
        }
        match self.context.variable(name) {
            Some(info) => Ok((
                Expr::External(info.id),
                Props::unknown(info.cardinality == Cardinality::AtMostOne),
            )),
            None => err(
                ErrorCode::Xpst0008,
                format!("Variable ${name} has not been declared"),
            ),
        }
    }

    fn sequence(&mut self, items: &[ast::Expr]) -> Result<(Expr, Props)> {
        let mut compiled = Vec::with_capacity(items.len());
        let mut props: Option<Props> = None;
        for item in items {
            let (e, p) = self.expr(item)?;
            compiled.push(e);
            props = Some(props.map_or(p, |q| Props::either(p, q)));
        }
        let props = match props {
            None => Props::atomic(true, Class::NotNumeric),
            Some(p) => Props {
                max_one: false,
                ordered: false,
                peer: false,
                ..p
            },
        };
        Ok((Expr::Sequence(compiled), props))
    }

    fn comparison(
        &mut self,
        op: ast::ComparisonOp,
        lhs: &ast::Expr,
        rhs: &ast::Expr,
    ) -> Result<(Expr, Props)> {
        let (l, lp) = self.expr(lhs)?;
        let (r, rp) = self.expr(rhs)?;
        let e = match op {
            ast::ComparisonOp::General(kind) => Expr::GeneralComparison {
                op: comparison_op(kind),
                lhs: boxed(l),
                rhs: boxed(r),
                strategy: match (lp.max_one, rp.max_one) {
                    (true, true) => CompareStrategy::OneToOne,
                    (_, true) => CompareStrategy::ManyToOne,
                    (true, false) => CompareStrategy::OneToMany,
                    (false, false) => CompareStrategy::ManyToMany,
                },
            },
            ast::ComparisonOp::Value(kind) => Expr::ValueComparison {
                op: comparison_op(kind),
                lhs: boxed(l),
                rhs: boxed(r),
            },
        };
        Ok((e, Props::BOOLEAN))
    }

    /// Compiles nested bindings (`for $a in A, $b in B return E` is
    /// `for $a in A return for $b in B return E`), with `inner` compiling the innermost
    /// body.
    fn bindings<T>(
        &mut self,
        bindings: &[ast::Binding],
        inner: &mut dyn FnMut(&mut Self) -> Result<(Expr, T)>,
    ) -> Result<(Expr, T)> {
        let Some((first, rest)) = bindings.split_first() else {
            return inner(self);
        };
        let (sequence, _) = self.expr(&first.sequence)?;
        self.locals.push(first.variable.clone());
        let body = self.bindings(rest, inner);
        self.locals.pop();
        let (body, extra) = body?;
        Ok((
            Expr::For {
                sequence: boxed(sequence),
                body: boxed(body),
            },
            extra,
        ))
    }

    fn quantified(
        &mut self,
        every: bool,
        bindings: &[ast::Binding],
        satisfies: &ast::Expr,
    ) -> Result<(Expr, Props)> {
        let Some((first, rest)) = bindings.split_first() else {
            return err(ErrorCode::Internal, "quantified expression without binding");
        };
        let (sequence, _) = self.expr(&first.sequence)?;
        self.locals.push(first.variable.clone());
        let body = if rest.is_empty() {
            self.expr(satisfies).map(|(e, _)| e)
        } else {
            self.quantified(every, rest, satisfies).map(|(e, _)| e)
        };
        self.locals.pop();
        Ok((
            Expr::Quantified {
                every,
                sequence: boxed(sequence),
                satisfies: boxed(body?),
            },
            Props::BOOLEAN,
        ))
    }

    fn predicates(&mut self, predicates: &[ast::Expr]) -> Result<Vec<Predicate>> {
        predicates
            .iter()
            .map(|p| {
                let (expr, props) = self.expr(p)?;
                let kind = match &expr {
                    Expr::Literal(AtomicValue::Integer(n)) => PredicateKind::Position(
                        n.to_i64()
                            .and_then(|n| usize::try_from(n).ok())
                            .filter(|&n| n > 0),
                    ),
                    _ if props.class == Class::NotNumeric => PredicateKind::Boolean,
                    _ => PredicateKind::Dynamic,
                };
                let uses_last = calls_focus(&expr, false);
                Ok(Predicate {
                    expr,
                    kind,
                    uses_last,
                })
            })
            .collect()
    }

    /// Whether the predicates of a `child::` step can be moved to a `descendant::` step
    /// (`//name[p]`): none of them may depend on the position.
    fn positional(predicates: &[Predicate]) -> bool {
        predicates
            .iter()
            .any(|p| p.kind != PredicateKind::Boolean || calls_focus(&p.expr, true))
    }

    fn path(&mut self, path: &ast::PathExpr) -> Result<(Expr, Props)> {
        // `descendant-or-self::node()/child::name` without positional predicate is
        // `descendant::name`, which keeps document order from a single node.
        let mut steps = Vec::with_capacity(path.steps.len());
        let mut i = 0;
        while i < path.steps.len() {
            let step = &path.steps[i];
            if let (ast::Step::Axis(dos), Some(ast::Step::Axis(child))) =
                (step, path.steps.get(i + 1))
                && dos.axis == ast::Axis::DescendantOrSelf
                && dos.test == ast::NodeTest::Kind(KindTest::AnyKind)
                && dos.predicates.is_empty()
                && child.axis == ast::Axis::Child
            {
                let predicates = self.predicates(&child.predicates)?;
                if !Self::positional(&predicates) {
                    steps.push(PathStep::Descendant(child));
                    i += 2;
                    continue;
                }
            }
            steps.push(PathStep::Ast(step));
            i += 1;
        }
        let mut acc = match path.start {
            ast::PathStart::Root => Some((Expr::Root, Props::nodes(true, true, true))),
            ast::PathStart::Relative => None,
        };
        for step in steps {
            let (rhs, rhs_props) = match step {
                PathStep::Ast(ast::Step::Axis(axis_step)) => {
                    self.axis_step(node_axis(axis_step.axis), axis_step)?
                }
                PathStep::Descendant(axis_step) => {
                    self.axis_step(NodeAxis::Descendant, axis_step)?
                }
                PathStep::Ast(ast::Step::Expr(e)) => self.expr(e)?,
            };
            acc = Some(match acc {
                None => (rhs, rhs_props),
                Some((lhs, lhs_props)) => Self::slash(lhs, lhs_props, rhs, rhs_props),
            });
        }
        acc.ok_or_else(|| Error::new(ErrorCode::Internal, "empty path"))
    }

    fn axis_step(&mut self, axis: NodeAxis, step: &ast::AxisStep) -> Result<(Expr, Props)> {
        let test = node_test(&step.test, axis);
        let predicates = self.predicates(&step.predicates)?;
        let props = step_props(axis, &test, &predicates);
        Ok((
            Expr::Step(Box::new(Step {
                axis,
                test,
                predicates,
            })),
            props,
        ))
    }

    fn slash(lhs: Expr, lp: Props, rhs: Expr, rp: Props) -> (Expr, Props) {
        let axis = step_axis(&rhs);
        let rhs_kinds = if axis.is_some() {
            Kinds::Nodes
        } else {
            rp.kinds
        };
        let (mode, props) = match rhs_kinds {
            Kinds::Atomics => (
                PathMode::Atomic,
                Props::atomic(lp.max_one && rp.max_one, rp.class),
            ),
            Kinds::Unknown => (PathMode::Dynamic, Props::unknown(lp.max_one && rp.max_one)),
            Kinds::Nodes => {
                let ordered = axis.is_some_and(|a| path_is_ordered(lp, a));
                let peer = if lp.max_one {
                    rp.peer
                } else {
                    lp.peer
                        && rp.peer
                        && matches!(
                            axis,
                            Some(NodeAxis::Child | NodeAxis::Attribute | NodeAxis::SelfAxis)
                        )
                };
                (
                    if ordered {
                        PathMode::Ordered
                    } else {
                        PathMode::Sort
                    },
                    Props::nodes(lp.max_one && rp.max_one, true, peer),
                )
            }
        };
        let rhs_uses_last = calls_focus(&rhs, false);
        (
            Expr::Path {
                lhs: boxed(lhs),
                rhs: boxed(rhs),
                mode,
                rhs_uses_last,
            },
            props,
        )
    }

    fn call(&mut self, name: &QName, args: &[ast::Expr]) -> Result<(Expr, Props)> {
        let builtin = functions::resolve(name, args.len(), self.context.xslt_functions())?;
        let Some(builtin) = builtin else {
            return match self.context.function(name, args.len()) {
                Some(id) => {
                    let args = args
                        .iter()
                        .map(|a| self.expr(a).map(|(e, _)| e))
                        .collect::<Result<_>>()?;
                    Ok((Expr::HostCall { id, args }, Props::unknown(false)))
                }
                None => err(
                    ErrorCode::Xpst0017,
                    format!(
                        "Cannot find a {}-argument function named {}{}",
                        args.len(),
                        name.namespace
                            .as_deref()
                            .map(|ns| format!("Q{{{ns}}}"))
                            .unwrap_or_default(),
                        name.local
                    ),
                ),
            };
        };
        let data = self.call_data(builtin, args)?;
        let compiled = args
            .iter()
            .map(|a| self.expr(a).map(|(e, _)| e))
            .collect::<Result<Vec<_>>>()?;
        let many = builtin.returns_many();
        let props = match builtin.returns() {
            Returns::Boolean => Props::BOOLEAN,
            Returns::NonNumeric => Props::atomic(!many, Class::NotNumeric),
            Returns::Atomic => Props::atomic(!many, Class::Unknown),
            Returns::Nodes => Props::nodes(!many, !many, !many),
            Returns::Items => Props::unknown(!many),
        };
        Ok((
            Expr::Call {
                function: builtin,
                args: compiled,
                data,
            },
            props,
        ))
    }

    fn call_data(&self, builtin: Builtin, args: &[ast::Expr]) -> Result<CallData> {
        let literal = |i: usize| match args.get(i) {
            Some(ast::Expr::Literal(ast::Literal::String(s))) => Some(s.as_str()),
            _ => None,
        };
        match builtin {
            Builtin::Matches | Builtin::Replace | Builtin::Tokenize => {
                let flags_at = if builtin == Builtin::Replace { 3 } else { 2 };
                let flags = if args.len() > flags_at {
                    literal(flags_at)
                } else {
                    Some("")
                };
                Ok(match (literal(1), flags) {
                    (Some(pattern), Some(flags)) => CallData::Regex(Arc::new(
                        XPathRegex::new(pattern, flags).map_err(Error::from),
                    )),
                    _ => CallData::None,
                })
            }
            Builtin::SystemProperty => {
                let Some(text) = literal(0) else {
                    return err(
                        ErrorCode::Unsupported,
                        "system-property() with an argument that is not a string literal",
                    );
                };
                self.property_name(text.trim()).map(CallData::Property)
            }
            _ => Ok(CallData::None),
        }
    }

    fn property_name(&self, text: &str) -> Result<QName> {
        let invalid = || {
            Error::new(
                ErrorCode::Xtde1390,
                format!("Invalid QName {text:?} in system-property()"),
            )
        };
        match text.split_once(':') {
            None if einvoice_xpath::is_ncname(text) => Ok(QName::local(text)),
            Some((prefix, local))
                if einvoice_xpath::is_ncname(prefix) && einvoice_xpath::is_ncname(local) =>
            {
                let namespace = self.context.namespace_uri(prefix).ok_or_else(invalid)?;
                Ok(QName {
                    prefix: Some(prefix.to_owned()),
                    namespace: Some(namespace),
                    local: local.to_owned(),
                })
            }
            _ => Err(invalid()),
        }
    }
}

fn literal_value(literal: &ast::Literal) -> Result<(Expr, Props)> {
    let (value, class) = match literal {
        ast::Literal::String(s) => (AtomicValue::String(s.clone()), Class::NotNumeric),
        ast::Literal::Integer(text) => {
            (AtomicValue::Integer(Integer::parse(text)?), Class::Unknown)
        }
        ast::Literal::Decimal(text) => {
            (AtomicValue::Decimal(Decimal::parse(text)?), Class::Unknown)
        }
        ast::Literal::Double(text) => (AtomicValue::Double(parse_double(text)?), Class::Unknown),
    };
    Ok((Expr::Literal(value), Props::atomic(true, class)))
}
