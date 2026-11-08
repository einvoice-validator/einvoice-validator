//! Printing syntax trees back to XPath text.
//!
//! The printer writes the canonical form: explicit axes except `child::` (omitted), `@` and
//! `..`; `//` written out as `/descendant-or-self::node()/`; string literals in double
//! quotes; parentheses only where the grammar needs them, plus around `instance of` (whose
//! sequence type would otherwise swallow a following `*` or `+`) and around the lone `/`
//! (which would otherwise take a following `*` or name as its first step) unless they stand
//! alone or before a closing bracket. Parsing the output gives back the same tree.

use std::fmt::{self, Display, Formatter, Write};

use crate::ast::{
    ArithmeticOp, Avt, AvtPart, Axis, AxisStep, Binding, CompareKind, ComparisonOp, Expr, ItemType,
    KindTest, Literal, NameOrWildcard, NameTest, NodeTest, Occurrence, PathExpr, PathStart,
    Pattern, PatternAlternative, PatternAnchor, PatternAxis, PatternLink, PatternStep, Quantifier,
    SequenceType, SingleType, Step,
};

/// Binding strength of each expression form, loosest first.
mod prec {
    pub(super) const SEQUENCE: u8 = 0;
    pub(super) const SINGLE: u8 = 1;
    pub(super) const OR: u8 = 2;
    pub(super) const AND: u8 = 3;
    pub(super) const COMPARISON: u8 = 4;
    pub(super) const RANGE: u8 = 5;
    pub(super) const ADDITIVE: u8 = 6;
    pub(super) const MULTIPLICATIVE: u8 = 7;
    pub(super) const UNION: u8 = 8;
    pub(super) const INSTANCE_OF: u8 = 10;
    pub(super) const CASTABLE: u8 = 12;
    pub(super) const CAST: u8 = 13;
    pub(super) const UNARY: u8 = 14;
    pub(super) const PATH: u8 = 15;
    pub(super) const FILTER: u8 = 16;
    pub(super) const PRIMARY: u8 = 17;
}

fn precedence(expr: &Expr) -> u8 {
    match expr {
        Expr::Sequence(items) if !items.is_empty() => prec::SEQUENCE,
        Expr::If { .. } | Expr::For { .. } | Expr::Quantified { .. } => prec::SINGLE,
        Expr::Or(..) => prec::OR,
        Expr::And(..) => prec::AND,
        Expr::Comparison { .. } => prec::COMPARISON,
        Expr::Range(..) => prec::RANGE,
        Expr::Arithmetic {
            op: ArithmeticOp::Add | ArithmeticOp::Subtract,
            ..
        } => prec::ADDITIVE,
        Expr::Arithmetic { .. } => prec::MULTIPLICATIVE,
        Expr::Union(..) => prec::UNION,
        // Parenthesized everywhere but where a delimiter follows, see the module
        // documentation.
        Expr::InstanceOf { .. } => prec::SEQUENCE,
        Expr::Castable { .. } => prec::CASTABLE,
        Expr::Cast { .. } => prec::CAST,
        Expr::Negate(_) => prec::UNARY,
        Expr::Path(PathExpr { steps, .. }) if steps.is_empty() => prec::SEQUENCE,
        Expr::Path(_) => prec::PATH,
        Expr::Filter { .. } => prec::FILTER,
        Expr::Literal(_)
        | Expr::VarRef(_)
        | Expr::ContextItem
        | Expr::FunctionCall { .. }
        | Expr::Sequence(_) => prec::PRIMARY,
    }
}

struct Printer<'a, 'b> {
    f: &'a mut Formatter<'b>,
}

impl Printer<'_, '_> {
    /// Writes `expr`, in parentheses if it binds more loosely than `min`.
    fn expr(&mut self, expr: &Expr, min: u8) -> fmt::Result {
        if precedence(expr) < min {
            self.f.write_char('(')?;
            self.bare(expr)?;
            self.f.write_char(')')
        } else {
            self.bare(expr)
        }
    }

    fn binary(
        &mut self,
        lhs: &Expr,
        op: &str,
        rhs: &Expr,
        lhs_min: u8,
        rhs_min: u8,
    ) -> fmt::Result {
        self.expr(lhs, lhs_min)?;
        write!(self.f, " {op} ")?;
        self.expr(rhs, rhs_min)
    }

    fn bare(&mut self, expr: &Expr) -> fmt::Result {
        match expr {
            Expr::Literal(literal) => self.literal(literal),
            Expr::VarRef(name) => write!(self.f, "${name}"),
            Expr::ContextItem => self.f.write_char('.'),
            Expr::FunctionCall { name, args } => {
                write!(self.f, "{name}(")?;
                self.list(args, prec::SINGLE)?;
                self.f.write_char(')')
            }
            Expr::Sequence(items) if items.is_empty() => self.f.write_str("()"),
            Expr::Sequence(items) => self.list(items, prec::SINGLE),
            Expr::Filter { base, predicates } => {
                self.expr(base, prec::PRIMARY)?;
                self.predicates(predicates)
            }
            Expr::Path(path) => self.path(path),
            Expr::Negate(operand) => {
                self.f.write_char('-')?;
                self.expr(operand, prec::UNARY)
            }
            Expr::Castable { operand, target } => {
                self.expr(operand, prec::CAST)?;
                write!(self.f, " castable as {target}")
            }
            Expr::Cast { operand, target } => {
                self.expr(operand, prec::UNARY)?;
                write!(self.f, " cast as {target}")
            }
            Expr::InstanceOf { operand, target } => {
                self.expr(operand, prec::CASTABLE)?;
                write!(self.f, " instance of {target}")
            }
            Expr::Union(lhs, rhs) => self.binary(lhs, "|", rhs, prec::UNION, prec::INSTANCE_OF),
            Expr::Arithmetic { op, lhs, rhs } => {
                let (text, level) = match op {
                    ArithmeticOp::Add => ("+", prec::ADDITIVE),
                    ArithmeticOp::Subtract => ("-", prec::ADDITIVE),
                    ArithmeticOp::Multiply => ("*", prec::MULTIPLICATIVE),
                    ArithmeticOp::Divide => ("div", prec::MULTIPLICATIVE),
                    ArithmeticOp::IntegerDivide => ("idiv", prec::MULTIPLICATIVE),
                    ArithmeticOp::Modulo => ("mod", prec::MULTIPLICATIVE),
                };
                self.binary(lhs, text, rhs, level, level + 1)
            }
            Expr::Range(lhs, rhs) => self.binary(lhs, "to", rhs, prec::ADDITIVE, prec::ADDITIVE),
            Expr::Comparison { op, lhs, rhs } => {
                self.binary(lhs, comparison_text(*op), rhs, prec::RANGE, prec::RANGE)
            }
            Expr::And(lhs, rhs) => self.binary(lhs, "and", rhs, prec::AND, prec::COMPARISON),
            Expr::Or(lhs, rhs) => self.binary(lhs, "or", rhs, prec::OR, prec::AND),
            Expr::If {
                condition,
                then,
                otherwise,
            } => {
                self.f.write_str("if (")?;
                self.expr(condition, prec::SEQUENCE)?;
                self.f.write_str(") then ")?;
                self.expr(then, prec::SINGLE)?;
                self.f.write_str(" else ")?;
                self.expr(otherwise, prec::SINGLE)
            }
            Expr::For { bindings, body } => {
                self.f.write_str("for ")?;
                self.bindings(bindings)?;
                self.f.write_str(" return ")?;
                self.expr(body, prec::SINGLE)
            }
            Expr::Quantified {
                quantifier,
                bindings,
                satisfies,
            } => {
                self.f.write_str(match quantifier {
                    Quantifier::Some => "some ",
                    Quantifier::Every => "every ",
                })?;
                self.bindings(bindings)?;
                self.f.write_str(" satisfies ")?;
                self.expr(satisfies, prec::SINGLE)
            }
        }
    }

    fn list(&mut self, items: &[Expr], min: u8) -> fmt::Result {
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                self.f.write_str(", ")?;
            }
            self.expr(item, min)?;
        }
        Ok(())
    }

    fn bindings(&mut self, bindings: &[Binding]) -> fmt::Result {
        for (i, binding) in bindings.iter().enumerate() {
            if i > 0 {
                self.f.write_str(", ")?;
            }
            write!(self.f, "${} in ", binding.variable)?;
            self.expr(&binding.sequence, prec::SINGLE)?;
        }
        Ok(())
    }

    fn literal(&mut self, literal: &Literal) -> fmt::Result {
        match literal {
            Literal::String(s) => {
                self.f.write_char('"')?;
                self.f.write_str(&s.replace('"', "\"\""))?;
                self.f.write_char('"')
            }
            Literal::Integer(n) | Literal::Decimal(n) | Literal::Double(n) => self.f.write_str(n),
        }
    }

    fn predicates(&mut self, predicates: &[Expr]) -> fmt::Result {
        for predicate in predicates {
            self.f.write_char('[')?;
            self.expr(predicate, prec::SEQUENCE)?;
            self.f.write_char(']')?;
        }
        Ok(())
    }

    fn path(&mut self, path: &PathExpr) -> fmt::Result {
        if path.start == PathStart::Root {
            self.f.write_char('/')?;
        }
        for (i, step) in path.steps.iter().enumerate() {
            if i > 0 {
                self.f.write_char('/')?;
            }
            match step {
                Step::Axis(step) => self.axis_step(step)?,
                Step::Expr(expr) => self.expr(expr, prec::FILTER)?,
            }
        }
        Ok(())
    }

    fn axis_step(&mut self, step: &AxisStep) -> fmt::Result {
        let is_attribute_test = matches!(step.test, NodeTest::Kind(KindTest::Attribute(_)));
        match (step.axis, &step.test) {
            (Axis::Parent, NodeTest::Kind(KindTest::AnyKind)) => self.f.write_str("..")?,
            (Axis::Child, test) if !is_attribute_test => write!(self.f, "{test}")?,
            (Axis::Attribute, test) => write!(self.f, "@{test}")?,
            (axis, test) => write!(self.f, "{}::{test}", axis.name())?,
        }
        self.predicates(&step.predicates)
    }
}

fn comparison_text(op: ComparisonOp) -> &'static str {
    match op {
        ComparisonOp::General(kind) => match kind {
            CompareKind::Eq => "=",
            CompareKind::Ne => "!=",
            CompareKind::Lt => "<",
            CompareKind::Le => "<=",
            CompareKind::Gt => ">",
            CompareKind::Ge => ">=",
        },
        ComparisonOp::Value(kind) => match kind {
            CompareKind::Eq => "eq",
            CompareKind::Ne => "ne",
            CompareKind::Lt => "lt",
            CompareKind::Le => "le",
            CompareKind::Gt => "gt",
            CompareKind::Ge => "ge",
        },
    }
}

impl Display for Expr {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Printer { f }.expr(self, prec::SEQUENCE)
    }
}

impl Display for NodeTest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(NameTest::Name(name)) => write!(f, "{name}"),
            Self::Name(NameTest::Any) => f.write_char('*'),
            Self::Name(NameTest::AnyLocal { prefix, .. }) => write!(f, "{prefix}:*"),
            Self::Name(NameTest::AnyNamespace { local }) => write!(f, "*:{local}"),
            Self::Kind(kind) => write!(f, "{kind}"),
        }
    }
}

impl Display for KindTest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let (name, arg) = match self {
            Self::AnyKind => ("node", None),
            Self::Text => ("text", None),
            Self::Comment => ("comment", None),
            Self::ProcessingInstruction => ("processing-instruction", None),
            Self::Element(arg) => ("element", arg.as_ref()),
            Self::Attribute(arg) => ("attribute", arg.as_ref()),
        };
        match arg {
            None => write!(f, "{name}()"),
            Some(NameOrWildcard::Wildcard) => write!(f, "{name}(*)"),
            Some(NameOrWildcard::Name(qname)) => write!(f, "{name}({qname})"),
        }
    }
}

impl Display for SequenceType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match &self.item {
            ItemType::Atomic(atomic) => write!(f, "{}", atomic.name)?,
            ItemType::Kind(kind) => write!(f, "{kind}")?,
        }
        f.write_str(match self.occurrence {
            Occurrence::One => "",
            Occurrence::ZeroOrOne => "?",
            Occurrence::ZeroOrMore => "*",
            Occurrence::OneOrMore => "+",
        })
    }
}

impl Display for SingleType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}",
            self.atomic.name,
            if self.allows_empty { "?" } else { "" }
        )
    }
}

impl Display for Pattern {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for (i, alternative) in self.alternatives.iter().enumerate() {
            if i > 0 {
                f.write_str(" | ")?;
            }
            write!(f, "{alternative}")?;
        }
        Ok(())
    }
}

impl Display for PatternAlternative {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DocumentRoot => f.write_char('/'),
            Self::RootFunction => f.write_str("root()"),
            Self::Path(path) => {
                f.write_str(match path.anchor {
                    PatternAnchor::Relative => "",
                    PatternAnchor::Root => "/",
                    PatternAnchor::RootDescendant => "//",
                })?;
                write!(f, "{}", path.first)?;
                for (link, step) in &path.rest {
                    f.write_str(match link {
                        PatternLink::Child => "/",
                        PatternLink::Descendant => "//",
                    })?;
                    write!(f, "{step}")?;
                }
                Ok(())
            }
            Self::Parenthesized { inner, predicates } => {
                write!(f, "({inner})")?;
                Printer { f }.predicates(predicates)
            }
        }
    }
}

impl Display for PatternStep {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let is_attribute_test = matches!(self.test, NodeTest::Kind(KindTest::Attribute(_)));
        match self.axis {
            PatternAxis::Attribute => write!(f, "@{}", self.test)?,
            PatternAxis::Child if is_attribute_test => write!(f, "child::{}", self.test)?,
            PatternAxis::Child => write!(f, "{}", self.test)?,
        }
        Printer { f }.predicates(&self.predicates)
    }
}

impl Display for Avt {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for part in &self.parts {
            match part {
                AvtPart::Text(text) => f.write_str(&text.replace('{', "{{").replace('}', "}}"))?,
                AvtPart::Expr(expr) => write!(f, "{{{expr}}}")?,
            }
        }
        Ok(())
    }
}
