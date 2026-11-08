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
//!
//! It also infers the static item type of every subexpression as Saxon-HE 12.10 does
//! (`Expression.getItemType`, with the literals it folds), as far as it decides a
//! result: the constructor `xs:decimal` strips the trailing zeros of an argument that is
//! statically an `xs:integer` (see [`StaticType`]).

use std::sync::Arc;

use einvoice_xdm::{AtomicType, AtomicValue, Decimal, Integer, XPathRegex, ops, parse_double};
use einvoice_xpath::{self as ast, AtomicKind, KindTest, NameOrWildcard, NameTest, QName};

use crate::context::{
    Cardinality, Clock, Context, DynamicContext, FunctionId, NoHost, StaticContext, VariableId,
};
use crate::error::{Error, ErrorCode, Result, err};
use crate::functions::{self, Builtin, Returns};
use crate::item::Item;
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
    /// `operand cast as target` (`?` when `allows_empty`). `upcast` as for
    /// [`CallData::Upcast`]: an `xs:decimal` target and an operand statically typed
    /// `xs:integer`.
    Cast {
        operand: Box<Expr>,
        target: AtomicType,
        allows_empty: bool,
        upcast: bool,
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
    /// `xs:decimal` of an argument whose static type is `xs:integer`: Saxon-HE 12.10
    /// casts it with its up-casting converter, which strips the trailing zeros
    /// (`AtomicValue::upcast`), where the cast of an operand typed at run time keeps
    /// scale 0.
    Upcast,
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

/// The static item type of a value as Saxon-HE 12.10 infers it (`getItemType`), reduced
/// to the distinctions that can change a result. It decides one thing: whether the
/// argument of `xs:decimal` is an `xs:integer`, which Saxon then casts with its
/// up-casting converter (trailing zeros stripped) rather than with the converter chosen
/// at run time (scale 0 kept). The rules are Saxon's:
///
/// * literals have their type, and so have the expressions Saxon-HE folds into literals
///   (its optimizer option `CONSTANT_FOLDING` is off, so only a few): unary minus of a
///   literal and sequences of literals when parsed, arithmetic on two literals and casts
///   of a literal when type-checked, and, when optimized, filters of a literal sequence
///   that depend on the focus only (after which what contains them is type-checked
///   again, see [`Typing`]); function calls and `if` are not folded;
/// * `count`, `string-length`, `position`, `last`, `string-to-codepoints`, `to`,
///   `xs:integer` give integers; `number` doubles; `xs:decimal` decimals;
///   `distinct-values`, host functions and the external variables anything;
/// * arithmetic follows Saxon's calculators: untyped operands count as doubles, any
///   double operand gives a double, any decimal one a decimal, two integers an integer,
///   except that `div` of two integers is a decimal and `idiv` always an integer; an
///   operand of unknown type makes the result unknown;
/// * `sum` gives the common type of its atomized argument (doubles for untyped values)
///   and `xs:integer`, or the argument's type when it is exactly one item; `round`
///   and `abs` the type of their argument; `reverse`, filters, unary minus, `for` and
///   the range variables keep the type of their operand; `if` and sequences the
///   common super type of their branches or items (`()` aside), integer and decimal
///   giving decimal and any other mix giving anything;
/// * a filter `E[. instance of T]` has type `T`, unless the items of `E` are known to be
///   of type `T` (the test is then folded into `true()`);
/// * elements, attributes, text and document nodes atomize to untyped values (Saxon-HE
///   has no schema), comments and processing instructions to strings.
///
/// The context item at the top of an expression is of unknown type (the host knows
/// it); inside a predicate or on the right of `/` it has the type of the items it
/// ranges over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StaticType {
    /// No item: the empty sequence (Saxon's `ErrorType`), neutral in a common type.
    Empty,
    Integer,
    Decimal,
    Double,
    UntypedAtomic,
    /// A non-numeric atomic type: string, boolean, anyURI, date or dateTime.
    OtherAtomic,
    /// Elements, attributes, text or document nodes: their typed value is untyped.
    UntypedNodes,
    /// Comments or processing instructions: their typed value is a string.
    StringNodes,
    /// Nodes of kinds from both groups above, or of any kind.
    Nodes,
    /// Any item, any atomic value, any number.
    Unknown,
}

impl StaticType {
    /// The common super type (`Type.getCommonSuperType`).
    const fn union(self, other: Self) -> Self {
        match (self, other) {
            (Self::Empty, t) | (t, Self::Empty) => t,
            (Self::Integer, Self::Integer) => Self::Integer,
            (Self::Integer | Self::Decimal, Self::Integer | Self::Decimal) => Self::Decimal,
            (Self::Double, Self::Double) => Self::Double,
            (Self::UntypedAtomic, Self::UntypedAtomic) => Self::UntypedAtomic,
            (Self::OtherAtomic, Self::OtherAtomic) => Self::OtherAtomic,
            (Self::UntypedNodes, Self::UntypedNodes) => Self::UntypedNodes,
            (Self::StringNodes, Self::StringNodes) => Self::StringNodes,
            (
                Self::UntypedNodes | Self::StringNodes | Self::Nodes,
                Self::UntypedNodes | Self::StringNodes | Self::Nodes,
            ) => Self::Nodes,
            _ => Self::Unknown,
        }
    }

    /// The type of the atomized items (`Atomizer`, nodes untyped).
    const fn atomized(self) -> Self {
        match self {
            Self::UntypedNodes => Self::UntypedAtomic,
            Self::StringNodes => Self::OtherAtomic,
            Self::Nodes => Self::Unknown,
            t => t,
        }
    }

    /// The type of an operand of arithmetic, `round`, `abs` or `sum`: atomized,
    /// untyped values converted to doubles.
    const fn numeric(self) -> Self {
        match self.atomized() {
            Self::UntypedAtomic => Self::Double,
            t => t,
        }
    }

    /// The type of `a op b`: the result type of Saxon's calculator for the operand
    /// types (`Calculator.getResultType`), unknown when there is no specific one.
    const fn arithmetic(op: ops::ArithmeticOp, a: Self, b: Self) -> Self {
        let (a, b) = (a.numeric(), b.numeric());
        let idiv = matches!(op, ops::ArithmeticOp::IntegerDivide);
        match (a, b) {
            (Self::Empty, _) | (_, Self::Empty) => Self::Empty,
            (Self::Double, Self::Integer | Self::Decimal | Self::Double)
            | (Self::Integer | Self::Decimal, Self::Double) => {
                if idiv {
                    Self::Integer
                } else {
                    Self::Double
                }
            }
            (Self::Decimal, Self::Integer | Self::Decimal) | (Self::Integer, Self::Decimal) => {
                if idiv {
                    Self::Integer
                } else {
                    Self::Decimal
                }
            }
            (Self::Integer, Self::Integer) => {
                if matches!(op, ops::ArithmeticOp::Divide) {
                    Self::Decimal
                } else {
                    Self::Integer
                }
            }
            _ => Self::Unknown,
        }
    }

    /// The type of the items of a value.
    fn of_items(items: &[Item]) -> Self {
        items.iter().fold(Self::Empty, |t, item| {
            t.union(match item {
                Item::Node(node) => match node.kind() {
                    NodeKind::Comment | NodeKind::ProcessingInstruction => Self::StringNodes,
                    _ => Self::UntypedNodes,
                },
                Item::Atomic(value) => Self::of_atomic(value.atomic_type()),
            })
        })
    }

    const fn of_atomic(t: AtomicType) -> Self {
        match t {
            AtomicType::Integer => Self::Integer,
            AtomicType::Decimal => Self::Decimal,
            AtomicType::Double => Self::Double,
            AtomicType::UntypedAtomic => Self::UntypedAtomic,
            _ => Self::OtherAtomic,
        }
    }

    /// The type of the nodes a node test selects.
    const fn of_test(test: &NodeTest) -> Self {
        match test {
            NodeTest::AnyNode => Self::Nodes,
            NodeTest::Kind(NodeKind::Comment | NodeKind::ProcessingInstruction) => {
                Self::StringNodes
            }
            NodeTest::Kind(_) | NodeTest::Named { .. } => Self::UntypedNodes,
        }
    }

    /// Whether every item of this type is of type `t` (for `. instance of t`).
    const fn subsumed_by(self, t: Self) -> bool {
        matches!(
            (self, t),
            (Self::Integer, Self::Integer | Self::Decimal)
                | (Self::Decimal, Self::Decimal)
                | (Self::Double, Self::Double)
                | (Self::OtherAtomic, Self::OtherAtomic)
                | (Self::UntypedNodes, Self::UntypedNodes)
                | (Self::StringNodes, Self::StringNodes)
        )
    }

    /// The type of `-x` (`NegateExpression`: the primitive type of the operand).
    const fn negated(self) -> Self {
        match self.numeric() {
            t @ (Self::Empty | Self::Integer | Self::Decimal | Self::Double) => t,
            _ => Self::Unknown,
        }
    }
}

/// The static item type of a value at the two moments Saxon-HE 12.10 computes it.
/// `checked` is the type after type checking, when a cast chooses its converter: an
/// argument of `xs:decimal` of type `xs:integer` gets the up-casting converter, one of
/// type `xs:decimal` is returned unchanged, one of unknown type gets no converter (it is
/// chosen at run time). `optimized` is the type after optimization, which folds the
/// filters of literal sequences: an expression that contains such a filter is
/// type-checked again, so that a cast without a converter then gets the up-casting
/// one if its argument has become a literal integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Typing {
    checked: StaticType,
    optimized: StaticType,
}

impl Typing {
    const fn of(t: StaticType) -> Self {
        Self {
            checked: t,
            optimized: t,
        }
    }

    const fn union(self, other: Self) -> Self {
        Self {
            checked: self.checked.union(other.checked),
            optimized: self.optimized.union(other.optimized),
        }
    }

    fn map(self, f: impl Fn(StaticType) -> StaticType) -> Self {
        Self {
            checked: f(self.checked),
            optimized: f(self.optimized),
        }
    }

    fn zip(self, other: Self, f: impl Fn(StaticType, StaticType) -> StaticType) -> Self {
        Self {
            checked: f(self.checked, other.checked),
            optimized: f(self.optimized, other.optimized),
        }
    }

    /// Whether Saxon casts a value of this type to `xs:decimal` with its up-casting
    /// converter: an integer when type-checked, or of unknown type then and an integer
    /// once optimized.
    const fn upcasts_to_decimal(self) -> bool {
        matches!(
            (self.checked.atomized(), self.optimized.atomized()),
            (StaticType::Integer, _) | (StaticType::Unknown, StaticType::Integer)
        )
    }
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
    /// The static item type.
    ty: Typing,
    /// Whether it is statically exactly one item.
    count: Count,
    /// Whether Saxon folds it into a literal.
    folding: Folding,
}

/// Whether a value is statically exactly one item (for `sum`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Count {
    ExactlyOne,
    Other,
}

impl Count {
    const fn of(one: bool) -> Self {
        if one { Self::ExactlyOne } else { Self::Other }
    }
}

/// When Saxon-HE folds an expression into a literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Folding {
    Never,
    /// When optimizing.
    Optimized,
    /// When type checking (and so when optimizing too).
    Checked,
}

impl Props {
    const fn atomic(max_one: bool, class: Class) -> Self {
        Self {
            max_one,
            kinds: Kinds::Atomics,
            class,
            ordered: false,
            peer: false,
            ty: Typing::of(StaticType::Unknown),
            count: Count::Other,
            folding: Folding::Never,
        }
    }

    const BOOLEAN: Self = Self::atomic(true, Class::NotNumeric).typed(StaticType::OtherAtomic);

    const fn unknown(max_one: bool) -> Self {
        Self {
            max_one,
            kinds: Kinds::Unknown,
            class: Class::Unknown,
            ordered: max_one,
            peer: max_one,
            ty: Typing::of(StaticType::Unknown),
            count: Count::Other,
            folding: Folding::Never,
        }
    }

    const fn nodes(max_one: bool, ordered: bool, peer: bool) -> Self {
        Self {
            max_one,
            kinds: Kinds::Nodes,
            class: Class::NotNumeric,
            ordered,
            peer,
            ty: Typing::of(StaticType::Nodes),
            count: Count::Other,
            folding: Folding::Never,
        }
    }

    const fn typed(self, ty: StaticType) -> Self {
        Self {
            ty: Typing::of(ty),
            ..self
        }
    }

    const fn typing(self, ty: Typing) -> Self {
        Self { ty, ..self }
    }

    const fn exactly_one(self, one: bool) -> Self {
        Self {
            count: Count::of(one),
            ..self
        }
    }

    const fn one(&self) -> bool {
        matches!(self.count, Count::ExactlyOne)
    }

    /// Whether Saxon folds it into a literal when type-checking, and when optimizing.
    const fn literal(self, checked: bool, optimized: bool) -> Self {
        Self {
            folding: if checked {
                Folding::Checked
            } else if optimized {
                Folding::Optimized
            } else {
                Folding::Never
            },
            ..self
        }
    }

    /// A literal once type-checked.
    const fn is_literal(&self) -> bool {
        matches!(self.folding, Folding::Checked)
    }

    /// A literal once optimized.
    const fn folds(&self) -> bool {
        matches!(self.folding, Folding::Checked | Folding::Optimized)
    }

    /// Either of two values (the branches of `if`).
    const fn either(a: Self, b: Self) -> Self {
        Self {
            max_one: a.max_one && b.max_one,
            kinds: if matches!(
                (a.kinds, b.kinds),
                (Kinds::Nodes, Kinds::Nodes)
                    | (Kinds::Atomics, Kinds::Atomics)
                    | (Kinds::Unknown, Kinds::Unknown)
            ) {
                a.kinds
            } else {
                Kinds::Unknown
            },
            class: if matches!((a.class, b.class), (Class::NotNumeric, Class::NotNumeric)) {
                Class::NotNumeric
            } else {
                Class::Unknown
            },
            ordered: a.ordered && b.ordered,
            peer: a.peer && b.peer,
            ty: a.ty.union(b.ty),
            count: Count::of(a.one() && b.one()),
            folding: Folding::Never,
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
        focus: Typing::of(StaticType::Unknown),
    };
    let (expr, props) = compiler.expr(expr)?;
    Ok(CompiledExpr {
        expr,
        max_one: props.max_one,
    })
}

struct Compiler<'a> {
    context: &'a dyn StaticContext,
    /// The range variables in scope, innermost last, with their static item type.
    locals: Vec<(QName, Typing)>,
    /// The static type of the context item.
    focus: Typing,
}

/// The type of the value of an expression Saxon folds into a literal, evaluated as Saxon
/// does at compile time, or `None` when the evaluation fails (Saxon then keeps the
/// expression, and reports the error only if it is evaluated).
fn folded_type(expr: &Expr) -> Option<StaticType> {
    // Only the type of the value is used: `current-dateTime()` has the same at any time,
    // so any clock does.
    let dc = DynamicContext::new(&NoHost, Clock::from_unix_millis(0, 0).ok()?);
    let compiled = CompiledExpr {
        expr: expr.clone(),
        max_one: false,
    };
    let items = compiled.evaluate(&dc, &Context::new()).ok()?;
    Some(StaticType::of_items(&items))
}

/// Whether an expression refers to a variable or a host function anywhere
/// (`ExpressionTool.refersToVariableOrFunction`).
fn refers_to_variable_or_function(expr: &Expr) -> bool {
    let any = |items: &[Expr]| items.iter().any(refers_to_variable_or_function);
    let preds = |predicates: &[Predicate]| {
        predicates
            .iter()
            .any(|p| refers_to_variable_or_function(&p.expr))
    };
    match expr {
        Expr::Local(_) | Expr::External(_) | Expr::HostCall { .. } => true,
        Expr::Literal(_) | Expr::ContextItem | Expr::Root => false,
        Expr::Call { args, .. } | Expr::Sequence(args) => any(args),
        Expr::Filter { base, predicates } => {
            refers_to_variable_or_function(base) || preds(predicates)
        }
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
        } => refers_to_variable_or_function(lhs) || refers_to_variable_or_function(rhs),
        Expr::Negate(e)
        | Expr::InstanceOf { operand: e, .. }
        | Expr::Castable { operand: e, .. }
        | Expr::Cast { operand: e, .. } => refers_to_variable_or_function(e),
        Expr::If {
            condition,
            then,
            otherwise,
        } => {
            refers_to_variable_or_function(condition)
                || refers_to_variable_or_function(then)
                || refers_to_variable_or_function(otherwise)
        }
    }
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
        | Expr::Castable { operand: e, .. }
        | Expr::Cast { operand: e, .. } => calls_focus(e, position),
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
            ast::Expr::ContextItem => Ok((
                Expr::ContextItem,
                Props::unknown(true).typing(self.focus).exactly_one(true),
            )),
            ast::Expr::FunctionCall { name, args } => self.call(name, args),
            ast::Expr::Sequence(items) => self.sequence(items),
            ast::Expr::Filter {
                base,
                predicates: ast_predicates,
            } => {
                let (base, props) = self.expr(base)?;
                let predicates = self.with_focus(props.ty, |c| c.predicates(ast_predicates))?;
                let positional = predicates
                    .iter()
                    .any(|p| matches!(p.kind, PredicateKind::Position(_)));
                // `E[. instance of T]` has type T, unless the test is always true.
                let ty = ast_predicates.iter().fold(props.ty, |ty, p| match p {
                    ast::Expr::InstanceOf { operand, target }
                        if matches!(**operand, ast::Expr::ContextItem) =>
                    {
                        let tested = match &target.item {
                            ast::ItemType::Atomic(atomic) => {
                                StaticType::of_atomic(atomic_type(atomic.kind))
                            }
                            ast::ItemType::Kind(kind) => StaticType::of_test(&kind_test(kind)),
                        };
                        ty.map(|t| if t.subsumed_by(tested) { t } else { tested })
                    }
                    _ => ty,
                });
                // The optimizer folds the filter of a literal sequence (not a range)
                // when every predicate is `[1]` (`FirstItemExpression`), or when none
                // is positional (numeric, or reading the position or the size) and
                // each depends on the focus only (`tryEarlyEvaluation`). Other
                // positional predicates are rewritten and not folded, and so is `[1]`
                // after another predicate.
                let first_items = predicates
                    .iter()
                    .all(|p| p.kind == PredicateKind::Position(Some(1)));
                let early = predicates.iter().all(|p| {
                    p.kind == PredicateKind::Boolean
                        && !calls_focus(&p.expr, true)
                        && !refers_to_variable_or_function(&p.expr)
                        && !depends_on_more_than_focus(&p.expr)
                });
                let folds =
                    props.folds() && !matches!(base, Expr::Range(..)) && (first_items || early);
                let compiled = Expr::Filter {
                    base: boxed(base),
                    predicates,
                };
                let ty = match folds.then(|| folded_type(&compiled)).flatten() {
                    Some(value) => Typing {
                        optimized: value,
                        ..ty
                    },
                    None => ty,
                };
                let props = Props {
                    max_one: props.max_one || positional,
                    count: Count::Other,
                    ..props
                }
                .typing(ty)
                .literal(false, folds);
                Ok((compiled, props))
            }
            ast::Expr::Path(path) => self.path(path),
            ast::Expr::Negate(operand) => {
                let (operand, p) = self.expr(operand)?;
                Ok((
                    Expr::Negate(boxed(operand)),
                    Props::atomic(true, Class::Unknown)
                        .typing(p.ty.map(StaticType::negated))
                        .exactly_one(p.one())
                        .literal(p.is_literal(), p.folds()),
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
                    Props::BOOLEAN.exactly_one(true),
                ))
            }
            // As the constructor functions, which Saxon-HE 12.10 compiles to the same
            // `CastExpression` with the empty sequence allowed.
            ast::Expr::Cast { operand, target } => {
                let (operand, p) = self.expr(operand)?;
                let atomic = atomic_type(target.atomic.kind);
                let class = if matches!(atomic, AtomicType::Integer | AtomicType::Decimal) {
                    Class::Unknown
                } else {
                    Class::NotNumeric
                };
                Ok((
                    Expr::Cast {
                        operand: boxed(operand),
                        target: atomic,
                        allows_empty: target.allows_empty,
                        upcast: atomic == AtomicType::Decimal && p.ty.upcasts_to_decimal(),
                    },
                    Props::atomic(true, class)
                        .typed(StaticType::of_atomic(atomic))
                        .exactly_one(!target.allows_empty || p.one())
                        .literal(p.is_literal(), p.folds()),
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
                    Props::BOOLEAN.exactly_one(true),
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
                    Props::nodes(false, true, false).typing(lp.ty.union(rp.ty)),
                ))
            }
            ast::Expr::Arithmetic { op, lhs, rhs } => {
                let op = arithmetic_op(*op);
                let (lhs, lp) = self.expr(lhs)?;
                let (rhs, rp) = self.expr(rhs)?;
                let compiled = Expr::Arithmetic {
                    op,
                    lhs: boxed(lhs),
                    rhs: boxed(rhs),
                };
                let mut ty = lp.ty.zip(rp.ty, |a, b| StaticType::arithmetic(op, a, b));
                // Arithmetic on two literals is folded when type-checked (and, after
                // optimization, on two operands folded by then), unless it fails.
                let (mut literal, mut folds) = (false, false);
                if lp.folds()
                    && rp.folds()
                    && let Some(value) = folded_type(&compiled)
                {
                    literal = lp.is_literal() && rp.is_literal();
                    folds = true;
                    ty.optimized = value;
                    if literal {
                        ty.checked = value;
                    }
                }
                Ok((
                    compiled,
                    Props::atomic(true, Class::Unknown)
                        .typing(ty)
                        .exactly_one(lp.one() && rp.one())
                        .literal(literal, folds),
                ))
            }
            ast::Expr::Range(lhs, rhs) => {
                let (lhs, _) = self.expr(lhs)?;
                let (rhs, _) = self.expr(rhs)?;
                Ok((
                    Expr::Range(boxed(lhs), boxed(rhs)),
                    Props::atomic(false, Class::Unknown).typed(StaticType::Integer),
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
                Ok((e, Props::BOOLEAN.exactly_one(true)))
            }
            ast::Expr::If {
                condition,
                then,
                otherwise,
            } => {
                // Not folded, even with a literal condition (the optimizer option
                // `CONSTANT_FOLDING` is off in Saxon-HE).
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
                    Ok((body, (props.kinds, props.ty)))
                })
                .map(|(e, (kinds, ty))| {
                    (
                        e,
                        Props {
                            kinds,
                            ..Props::unknown(false)
                        }
                        .typing(ty),
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
        if let Some((depth, (_, ty))) = self
            .locals
            .iter()
            .rev()
            .enumerate()
            .find(|(_, (local, _))| local.same_expanded_name(name))
        {
            return Ok((
                Expr::Local(depth),
                Props::unknown(true).typing(*ty).exactly_one(true),
            ));
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
        let mut all_literal = true;
        for item in items {
            let (e, p) = self.expr(item)?;
            all_literal &= p.is_literal() && p.kinds == Kinds::Atomics;
            compiled.push(e);
            props = Some(props.map_or(p, |q| Props::either(q, p)));
        }
        let props = match props {
            // `()` and a sequence of literals are literals once parsed.
            None => Props::atomic(true, Class::NotNumeric)
                .typed(StaticType::Empty)
                .literal(true, true),
            Some(p) => Props {
                max_one: false,
                ordered: false,
                peer: false,
                count: Count::Other,
                ..p
            }
            .literal(all_literal, all_literal),
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
        let props = Props::BOOLEAN
            .exactly_one(matches!(op, ast::ComparisonOp::General(_)) || (lp.one() && rp.one()));
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
        Ok((e, props))
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
        let (sequence, p) = self.expr(&first.sequence)?;
        self.locals.push((first.variable.clone(), p.ty));
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
        let (sequence, p) = self.expr(&first.sequence)?;
        self.locals.push((first.variable.clone(), p.ty));
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

    /// Runs `compile` with the context item of that static type.
    fn with_focus<T>(
        &mut self,
        focus: Typing,
        compile: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let outer = std::mem::replace(&mut self.focus, focus);
        let result = compile(self);
        self.focus = outer;
        result
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
                let test = node_test(&child.test, NodeAxis::Child);
                let focus = Typing::of(StaticType::of_test(&test));
                let predicates = self.with_focus(focus, |c| c.predicates(&child.predicates))?;
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
            ast::PathStart::Root => Some((
                Expr::Root,
                Props::nodes(true, true, true)
                    .typed(StaticType::UntypedNodes)
                    .exactly_one(true),
            )),
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
                PathStep::Ast(ast::Step::Expr(e)) => {
                    let focus = acc.as_ref().map_or(self.focus, |(_, p)| p.ty);
                    self.with_focus(focus, |c| c.expr(e))?
                }
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
        let ty = StaticType::of_test(&test);
        let predicates = self.with_focus(Typing::of(ty), |c| c.predicates(&step.predicates))?;
        let props = step_props(axis, &test, &predicates).typed(ty);
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
                Props::atomic(lp.max_one && rp.max_one, rp.class).typing(rp.ty),
            ),
            Kinds::Unknown => (
                PathMode::Dynamic,
                Props::unknown(lp.max_one && rp.max_one).typing(rp.ty),
            ),
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
                    Props::nodes(lp.max_one && rp.max_one, true, peer).typing(rp.ty),
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
        let mut data = self.call_data(builtin, args)?;
        let mut compiled = Vec::with_capacity(args.len());
        let mut arg_props = Vec::with_capacity(args.len());
        for arg in args {
            let (e, p) = self.expr(arg)?;
            compiled.push(e);
            arg_props.push(p);
        }
        let many = builtin.returns_many();
        let props = match builtin.returns() {
            Returns::Boolean => Props::BOOLEAN,
            Returns::NonNumeric => {
                Props::atomic(!many, Class::NotNumeric).typed(StaticType::OtherAtomic)
            }
            Returns::Atomic => Props::atomic(!many, Class::Unknown),
            Returns::Nodes => Props::nodes(!many, !many, !many),
            Returns::Items => Props::unknown(!many),
        };
        let first = arg_props.first().copied();
        let ty = Typing {
            checked: call_type(builtin, first.map(|p| (p.ty.checked, p.one())))
                .unwrap_or(props.ty.checked),
            optimized: call_type(builtin, first.map(|p| (p.ty.optimized, p.one())))
                .unwrap_or(props.ty.optimized),
        };
        let constructor = matches!(
            builtin,
            Builtin::XsDecimal | Builtin::XsDate | Builtin::XsInteger | Builtin::XsString
        );
        let one = if constructor {
            first.is_some_and(|p| p.one())
        } else {
            builtin.exactly_one()
        };
        if builtin == Builtin::XsDecimal && first.is_some_and(|p| p.ty.upcasts_to_decimal()) {
            data = CallData::Upcast;
        }
        // A cast of a literal is folded (other calls are not: no `CONSTANT_FOLDING`).
        let props = props.typing(ty).exactly_one(one).literal(
            constructor && first.is_some_and(|p| p.is_literal()),
            constructor && first.is_some_and(|p| p.folds()),
        );
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
    let ty = StaticType::of_atomic(value.atomic_type());
    Ok((
        Expr::Literal(value),
        Props::atomic(true, class)
            .typed(ty)
            .exactly_one(true)
            .literal(true, true),
    ))
}

/// The static type of a call to a built-in function, when it does not follow from what
/// the function returns: `first` is the type of the first argument and whether it is
/// exactly one item.
fn call_type(builtin: Builtin, first: Option<(StaticType, bool)>) -> Option<StaticType> {
    let arg = first.map_or(StaticType::Unknown, |(t, _)| t);
    Some(match builtin {
        Builtin::Count
        | Builtin::StringLength
        | Builtin::Position
        | Builtin::Last
        | Builtin::StringToCodepoints
        | Builtin::XsInteger => StaticType::Integer,
        Builtin::Number => StaticType::Double,
        Builtin::XsDecimal => StaticType::Decimal,
        // `Sum.getResultItemType`.
        Builtin::Sum => {
            let base = arg.numeric();
            if first.is_some_and(|(_, one)| one) {
                base
            } else {
                base.union(StaticType::Integer)
            }
        }
        // AS_PRIM_ARG0: the atomized type of the argument, untyped as double.
        Builtin::Round | Builtin::Abs => arg.negated(),
        // AS_ARG0.
        Builtin::Reverse => arg,
        Builtin::DistinctValues | Builtin::Current => StaticType::Unknown,
        Builtin::Document => StaticType::UntypedNodes,
        _ => return None,
    })
}

/// Whether a predicate depends on more than the focus (`current()`, the current date
/// and time, `document()`), so that Saxon does not fold the filter.
fn depends_on_more_than_focus(expr: &Expr) -> bool {
    let any = |items: &[Expr]| items.iter().any(depends_on_more_than_focus);
    let preds = |predicates: &[Predicate]| {
        predicates
            .iter()
            .any(|p| depends_on_more_than_focus(&p.expr))
    };
    match expr {
        Expr::Call { function, args, .. } => {
            matches!(
                function,
                Builtin::Current | Builtin::CurrentDateTime | Builtin::Document
            ) || any(args)
        }
        Expr::Literal(_) | Expr::ContextItem | Expr::Root | Expr::Local(_) | Expr::External(_) => {
            false
        }
        Expr::HostCall { args, .. } | Expr::Sequence(args) => any(args),
        Expr::Filter { base, predicates } => depends_on_more_than_focus(base) || preds(predicates),
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
        } => depends_on_more_than_focus(lhs) || depends_on_more_than_focus(rhs),
        Expr::Negate(e)
        | Expr::InstanceOf { operand: e, .. }
        | Expr::Castable { operand: e, .. }
        | Expr::Cast { operand: e, .. } => depends_on_more_than_focus(e),
        Expr::If {
            condition,
            then,
            otherwise,
        } => {
            depends_on_more_than_focus(condition)
                || depends_on_more_than_focus(then)
                || depends_on_more_than_focus(otherwise)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::SimpleStaticContext;
    use einvoice_xpath::{NamespaceMap, XS_NAMESPACE, parse_expr};

    /// The static types of an expression, and whether it is an up-casting `xs:decimal`.
    fn typing(text: &str) -> (Typing, bool) {
        let ns = NamespaceMap::new().bind("xs", XS_NAMESPACE);
        let context = SimpleStaticContext::new();
        let mut compiler = Compiler {
            context: &context,
            locals: Vec::new(),
            focus: Typing::of(StaticType::Unknown),
        };
        let (expr, props) = compiler.expr(&parse_expr(text, &ns).unwrap()).unwrap();
        let upcast = matches!(
            expr,
            Expr::Call {
                function: Builtin::XsDecimal,
                data: CallData::Upcast,
                ..
            }
        );
        (props.ty, upcast)
    }

    #[test]
    fn calculator_result_types() {
        use StaticType::{Decimal, Double, Integer, Unknown, UntypedAtomic};
        let t = StaticType::arithmetic;
        assert_eq!(t(ops::ArithmeticOp::Add, Integer, Integer), Integer);
        assert_eq!(t(ops::ArithmeticOp::Divide, Integer, Integer), Decimal);
        assert_eq!(
            t(ops::ArithmeticOp::IntegerDivide, Decimal, Integer),
            Integer
        );
        assert_eq!(
            t(ops::ArithmeticOp::IntegerDivide, UntypedAtomic, Integer),
            Integer
        );
        assert_eq!(
            t(ops::ArithmeticOp::Multiply, UntypedAtomic, Integer),
            Double
        );
        assert_eq!(t(ops::ArithmeticOp::Subtract, Decimal, Integer), Decimal);
        assert_eq!(t(ops::ArithmeticOp::Add, Unknown, Integer), Unknown);
        assert_eq!(Integer.union(Decimal), Decimal);
        assert_eq!(Integer.union(Double), Unknown);
        assert_eq!(StaticType::Empty.union(Integer), Integer);
    }

    #[test]
    fn xs_decimal_upcasts_static_integers_only() {
        // Statically xs:integer: up-cast.
        assert!(typing("xs:decimal(300)").1);
        assert!(typing("xs:decimal(count(//a) * 100)").1);
        assert!(
            typing("xs:decimal(3000 div 10)").1,
            "literal arithmetic is folded"
        );
        assert!(
            typing("xs:decimal((300, 'a')[1])").1,
            "folded when optimized"
        );
        // Statically decimal, unknown or untyped: the converter of the run time.
        assert!(!typing("xs:decimal(count(//a) * 1000 div 10)").1);
        assert!(!typing("xs:decimal((300, 2.5)[1])").1);
        assert!(!typing("xs:decimal(if (true()) then 300 else 'a')").1);
        assert!(!typing("xs:decimal(sum((150, 150)) div 1)").1);
        assert!(!typing("xs:decimal(//a)").1);
        assert!(!typing("xs:decimal(('a', 300)[2])").1);
        assert_eq!(typing("(300, 'a')[1]").0.checked, StaticType::Unknown);
        assert_eq!(typing("(300, 'a')[1]").0.optimized, StaticType::Integer);
    }
}
