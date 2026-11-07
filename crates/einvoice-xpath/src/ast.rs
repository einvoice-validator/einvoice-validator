//! The syntax tree of the supported XPath 2.0 subset, of XSLT patterns, sequence types and
//! attribute value templates.
//!
//! The tree is canonical: parentheses are not kept (the nesting of the tree carries the
//! grouping), abbreviations are expanded where the XPath 2.0 specification defines them as
//! abbreviations (`//` is `/descendant-or-self::node()/`, `..` is `parent::node()`, `@n` is
//! `attribute::n`). Printing a tree with [`std::fmt::Display`] and parsing the text again
//! gives the same tree.

use crate::names::QName;

/// An XPath expression (XPath 2.0 production \[2\] `Expr` and below).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// A string or numeric literal (\[42\] `Literal`).
    Literal(Literal),
    /// `$name` (\[44\] `VarRef`). Unprefixed variable names are in no namespace.
    VarRef(QName),
    /// `.` (\[47\] `ContextItemExpr`).
    ContextItem,
    /// `name(args)` (\[48\] `FunctionCall`). Unprefixed function names are in the
    /// [`FN_NAMESPACE`](crate::FN_NAMESPACE); which function the name and arity designate
    /// is decided when the expression is compiled, not by the parser.
    FunctionCall {
        /// The function name.
        name: QName,
        /// The arguments, each an `ExprSingle`.
        args: Vec<Expr>,
    },
    /// The comma operator (\[2\] `Expr`) with two or more operands, or `()` (\[46\]
    /// `ParenthesizedExpr` with no content) with none. A single parenthesized expression is
    /// that expression itself.
    Sequence(Vec<Expr>),
    /// A primary expression followed by one or more predicates (\[38\] `FilterExpr`).
    Filter {
        /// The filtered expression.
        base: Box<Expr>,
        /// The predicates, applied left to right.
        predicates: Vec<Expr>,
    },
    /// A path expression with at least one `/` or `//`, or a single axis step (\[25\]
    /// `PathExpr`). A single filter expression without slash is not a path.
    Path(PathExpr),
    /// Unary minus (\[20\] `UnaryExpr`); `--x` is two nested negations.
    Negate(Box<Expr>),
    /// `operand castable as T?` (\[18\] `CastableExpr`).
    Castable {
        /// The tested expression.
        operand: Box<Expr>,
        /// The target type.
        target: SingleType,
    },
    /// `operand instance of T` (\[16\] `InstanceofExpr`).
    InstanceOf {
        /// The tested expression.
        operand: Box<Expr>,
        /// The tested type.
        target: SequenceType,
    },
    /// `lhs | rhs` (\[14\] `UnionExpr`), left associative.
    Union(Box<Expr>, Box<Expr>),
    /// `+ - * div idiv mod` (\[12\] `AdditiveExpr`, \[13\] `MultiplicativeExpr`), left
    /// associative.
    Arithmetic {
        /// The operator.
        op: ArithmeticOp,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
    },
    /// `lhs to rhs` (\[11\] `RangeExpr`), not associative.
    Range(Box<Expr>, Box<Expr>),
    /// A general or value comparison (\[10\] `ComparisonExpr`), not associative.
    Comparison {
        /// The operator.
        op: ComparisonOp,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
    },
    /// `lhs and rhs` (\[9\] `AndExpr`), left associative.
    And(Box<Expr>, Box<Expr>),
    /// `lhs or rhs` (\[8\] `OrExpr`), left associative.
    Or(Box<Expr>, Box<Expr>),
    /// `if (condition) then a else b` (\[7\] `IfExpr`).
    If {
        /// The condition (an `Expr`, so possibly a sequence).
        condition: Box<Expr>,
        /// The `then` branch.
        then: Box<Expr>,
        /// The `else` branch.
        otherwise: Box<Expr>,
    },
    /// `for $v in e, ... return body` (\[4\] `ForExpr`).
    For {
        /// The bindings, in order (one or more).
        bindings: Vec<Binding>,
        /// The `return` expression.
        body: Box<Expr>,
    },
    /// `some|every $v in e, ... satisfies test` (\[6\] `QuantifiedExpr`).
    Quantified {
        /// `some` or `every`.
        quantifier: Quantifier,
        /// The bindings, in order (one or more).
        bindings: Vec<Binding>,
        /// The `satisfies` expression.
        satisfies: Box<Expr>,
    },
}

/// `$variable in sequence` in a `for`, `some` or `every` expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// The range variable.
    pub variable: QName,
    /// The binding sequence.
    pub sequence: Expr,
}

/// `some` or `every`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Quantifier {
    /// `some`
    Some,
    /// `every`
    Every,
}

/// A literal (\[42\] `Literal`). Numeric literals keep their kind and their exact lexical
/// form: the value is computed when the expression is compiled.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Literal {
    /// A string literal, with its doubled quotes already reduced.
    String(String),
    /// \[71\] `IntegerLiteral`: digits only, of any length.
    Integer(String),
    /// \[72\] `DecimalLiteral`, as written (`1.50`, `.5`, `1.`).
    Decimal(String),
    /// \[73\] `DoubleLiteral`, as written (`1e3`, `1.5E-2`).
    Double(String),
}

/// An arithmetic operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArithmeticOp {
    /// `+`
    Add,
    /// `-`
    Subtract,
    /// `*`
    Multiply,
    /// `div`
    Divide,
    /// `idiv`
    IntegerDivide,
    /// `mod`
    Modulo,
}

/// A comparison operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComparisonOp {
    /// A general comparison (\[22\] `GeneralComp`): existentially quantified over both
    /// operand sequences.
    General(CompareKind),
    /// A value comparison (\[23\] `ValueComp`): on single atomic values.
    Value(CompareKind),
}

/// Which comparison: `=`/`eq`, `!=`/`ne`, `<`/`lt`, `<=`/`le`, `>`/`gt`, `>=`/`ge`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompareKind {
    /// `=` or `eq`
    Eq,
    /// `!=` or `ne`
    Ne,
    /// `<` or `lt`
    Lt,
    /// `<=` or `le`
    Le,
    /// `>` or `gt`
    Gt,
    /// `>=` or `ge`
    Ge,
}

/// A path expression: an optional leading `/` and the steps between slashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathExpr {
    /// Whether the path starts at the root of the context node's tree.
    pub start: PathStart,
    /// The steps; empty only for the lone `/`. `//` appears as a
    /// `descendant-or-self::node()` step.
    pub steps: Vec<Step>,
}

/// Where a path expression starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathStart {
    /// At the context item.
    Relative,
    /// At the root of the tree that contains the context node (`/`, `//`).
    Root,
}

/// One step of a path (\[27\] `StepExpr`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// An axis step (\[28\] `AxisStep`).
    Axis(AxisStep),
    /// A filter expression used as a step (\[38\] `FilterExpr`): a function call, a
    /// variable, `.`, a parenthesized expression or a literal, with any predicates.
    Expr(Expr),
}

/// `axis::test[predicate]...`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AxisStep {
    /// The axis.
    pub axis: Axis,
    /// The node test.
    pub test: NodeTest,
    /// The predicates, applied left to right.
    pub predicates: Vec<Expr>,
}

/// The supported axes. `descendant`, `following` and `namespace` are refused by the parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    /// `child::` (also the default axis)
    Child,
    /// `attribute::` or `@`
    Attribute,
    /// `self::`
    SelfAxis,
    /// `descendant-or-self::` (from `//`)
    DescendantOrSelf,
    /// `following-sibling::`
    FollowingSibling,
    /// `parent::` or `..`
    Parent,
    /// `ancestor::`
    Ancestor,
    /// `ancestor-or-self::`
    AncestorOrSelf,
    /// `preceding-sibling::`
    PrecedingSibling,
    /// `preceding::`
    Preceding,
}

impl Axis {
    /// The axis name as written before `::`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Child => "child",
            Self::Attribute => "attribute",
            Self::SelfAxis => "self",
            Self::DescendantOrSelf => "descendant-or-self",
            Self::FollowingSibling => "following-sibling",
            Self::Parent => "parent",
            Self::Ancestor => "ancestor",
            Self::AncestorOrSelf => "ancestor-or-self",
            Self::PrecedingSibling => "preceding-sibling",
            Self::Preceding => "preceding",
        }
    }

    /// Whether the axis is a reverse axis (positions count backwards in a predicate).
    #[must_use]
    pub fn is_reverse(self) -> bool {
        matches!(
            self,
            Self::Parent
                | Self::Ancestor
                | Self::AncestorOrSelf
                | Self::PrecedingSibling
                | Self::Preceding
        )
    }
}

/// A node test (\[35\] `NodeTest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeTest {
    /// A name test (\[36\] `NameTest`).
    Name(NameTest),
    /// A kind test (\[54\] `KindTest`).
    Kind(KindTest),
}

/// A name test: a `QName` or a wildcard (\[36\] `NameTest`, \[37\] `Wildcard`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameTest {
    /// A name. Unprefixed element names take the default element namespace, unprefixed
    /// attribute names are in no namespace.
    Name(QName),
    /// `*`
    Any,
    /// `prefix:*`: any local name in one namespace.
    AnyLocal {
        /// The prefix as written.
        prefix: String,
        /// The namespace it is bound to.
        namespace: String,
    },
    /// `*:local`: one local name in any namespace (or none).
    AnyNamespace {
        /// The local name.
        local: String,
    },
}

/// The supported kind tests (\[54\] `KindTest`). `document-node()`, `schema-element()`,
/// `schema-attribute()`, `processing-instruction(name)` and type names in `element()` and
/// `attribute()` are refused by the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KindTest {
    /// `node()`
    AnyKind,
    /// `text()`
    Text,
    /// `comment()`
    Comment,
    /// `processing-instruction()`
    ProcessingInstruction,
    /// `element()`, `element(*)` or `element(name)`.
    Element(Option<NameOrWildcard>),
    /// `attribute()`, `attribute(*)` or `attribute(name)`.
    Attribute(Option<NameOrWildcard>),
}

/// The argument of `element(...)` or `attribute(...)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameOrWildcard {
    /// A name.
    Name(QName),
    /// `*`
    Wildcard,
}

/// A sequence type (\[50\] `SequenceType`), as in `instance of` and XSLT `as` attributes.
/// `empty-sequence()` and `item()` are refused by the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceType {
    /// The item type.
    pub item: ItemType,
    /// How many items.
    pub occurrence: Occurrence,
}

/// An item type (\[52\] `ItemType`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemType {
    /// An atomic type.
    Atomic(AtomicType),
    /// A node kind.
    Kind(KindTest),
}

/// An occurrence indicator (\[51\] `OccurrenceIndicator`), or its absence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Occurrence {
    /// Exactly one (no indicator).
    One,
    /// `?`
    ZeroOrOne,
    /// `*`
    ZeroOrMore,
    /// `+`
    OneOrMore,
}

/// A single type (\[49\] `SingleType`), the target of `castable as`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingleType {
    /// The atomic type.
    pub atomic: AtomicType,
    /// Whether `?` follows it (the empty sequence is then castable).
    pub allows_empty: bool,
}

/// An atomic type name (\[53\] `AtomicType`), limited to the supported types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomicType {
    /// The name as written and resolved.
    pub name: QName,
    /// Which type it is.
    pub kind: AtomicKind,
}

/// The atomic types accepted in sequence types and `castable as`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AtomicKind {
    /// `xs:string`
    String,
    /// `xs:anyURI`
    AnyUri,
    /// `xs:boolean`
    Boolean,
    /// `xs:integer`
    Integer,
    /// `xs:decimal`
    Decimal,
    /// `xs:date`
    Date,
}

impl AtomicKind {
    /// The kind whose local name in the XML Schema namespace is `local`, if supported.
    #[must_use]
    pub fn from_local_name(local: &str) -> Option<Self> {
        Some(match local {
            "string" => Self::String,
            "anyURI" => Self::AnyUri,
            "boolean" => Self::Boolean,
            "integer" => Self::Integer,
            "decimal" => Self::Decimal,
            "date" => Self::Date,
            _ => return None,
        })
    }
}

/// An XSLT match pattern: one or more alternatives separated by `|` (XSLT 2.0 section 5.5.2
/// production \[1\] `Pattern`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    /// The alternatives, in source order (at least one).
    pub alternatives: Vec<PatternAlternative>,
}

/// One alternative of a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternAlternative {
    /// `/`: matches a document node.
    DocumentRoot,
    /// `root()` (an XSLT 3.0 `RootedPath`, accepted by XSLT 3.0 processors running a 2.0
    /// stylesheet): matches the root of any tree.
    RootFunction,
    /// A path pattern (XSLT 2.0 production \[2\] `PathPattern`, without `IdKeyPattern`).
    Path(PathPattern),
    /// `(pattern)[predicate]...` (XSLT 3.0 `PostfixExprP`): at least one predicate, and
    /// nothing after it.
    Parenthesized {
        /// The pattern inside the parentheses.
        inner: Box<Pattern>,
        /// The predicates (at least one).
        predicates: Vec<Expr>,
    },
}

/// A path pattern: `step`, `/step`, `//step`, then steps separated by `/` or `//`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathPattern {
    /// What precedes the first step.
    pub anchor: PatternAnchor,
    /// The first step.
    pub first: PatternStep,
    /// The following steps, each with the separator before it.
    pub rest: Vec<(PatternLink, PatternStep)>,
}

/// What precedes the first step of a path pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatternAnchor {
    /// Nothing: the first step may be at any depth.
    Relative,
    /// `/`: the first step is a child of a document node.
    Root,
    /// `//`: the first step is a descendant of a document node.
    RootDescendant,
}

/// The separator between two pattern steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatternLink {
    /// `/`: the previous step matches the parent.
    Child,
    /// `//`: the previous step matches an ancestor.
    Descendant,
}

/// A pattern step (XSLT 2.0 production \[4\] `PatternStep`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternStep {
    /// The axis: patterns allow only `child` and `attribute`.
    pub axis: PatternAxis,
    /// The node test.
    pub test: NodeTest,
    /// The predicates.
    pub predicates: Vec<Expr>,
}

/// The axes allowed in a pattern step (XSLT 2.0 production \[5\] `PatternAxis`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatternAxis {
    /// `child::` or no axis.
    Child,
    /// `attribute::` or `@`.
    Attribute,
}

/// An attribute value template: literal text and `{expression}` parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Avt {
    /// The parts, in order; adjacent text parts never occur.
    pub parts: Vec<AvtPart>,
}

impl Avt {
    /// The value, when the template has no expression part.
    #[must_use]
    pub fn as_literal(&self) -> Option<String> {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                AvtPart::Text(text) => out.push_str(text),
                AvtPart::Expr(_) => return None,
            }
        }
        Some(out)
    }
}

/// One part of an attribute value template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvtPart {
    /// Literal text, with `{{` and `}}` already reduced.
    Text(String),
    /// An `{expression}`.
    Expr(Expr),
}
