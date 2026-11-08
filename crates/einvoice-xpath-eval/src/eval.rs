//! Evaluation of compiled expressions.
//!
//! Sequences are produced lazily, as iterators of fallible items, so that a consumer that
//! needs only part of a sequence (an effective boolean value, `exists`, a general
//! comparison that finds a match, a positional predicate, `some` and `every`) stops
//! reading it there, as Saxon does. This decides which dynamic errors are raised: an item
//! that is never read never raises its error. Sequences whose order is not known
//! statically (see `compile`) are materialised and sorted.

use std::rc::Rc;

use einvoice_xdm::{AtomicValue, Integer, ops};

use crate::compile::{
    CompareStrategy, CompiledExpr, Expr, ItemTest, PathMode, Predicate, PredicateKind, Step,
};
use crate::context::{Context, DynamicContext};
use crate::error::{Error, ErrorCode, Result, err};
use crate::functions::Builtin;
use crate::item::Item;
use crate::tree::{Node, NodeAxis};

/// A lazily produced sequence.
pub(crate) type Iter<'s> = Box<dyn Iterator<Item = Result<Item>> + 's>;

/// The range variables in scope, innermost first.
#[derive(Debug)]
pub(crate) struct Env {
    item: Item,
    parent: Option<Rc<Env>>,
}

/// The focus and range variables of a subexpression.
#[derive(Debug, Clone)]
pub(crate) struct Ctx {
    pub(crate) item: Option<Item>,
    pub(crate) position: usize,
    /// `None` when the size was not counted: the compiler counts it wherever `last()`
    /// is called.
    pub(crate) size: Option<usize>,
    env: Option<Rc<Env>>,
}

impl Ctx {
    fn with_focus(&self, item: Item, position: usize, size: Option<usize>) -> Self {
        Self {
            item: Some(item),
            position,
            size,
            env: self.env.clone(),
        }
    }

    fn with_local(&self, item: Item) -> Self {
        Self {
            env: Some(Rc::new(Env {
                item,
                parent: self.env.clone(),
            })),
            ..self.clone()
        }
    }
}

/// One evaluation: the dynamic context and the current item.
pub(crate) struct Run<'a> {
    pub(crate) dc: &'a DynamicContext<'a>,
    pub(crate) current: Option<Item>,
}

pub(crate) fn once<'s>(item: Item) -> Iter<'s> {
    Box::new(std::iter::once(Ok(item)))
}

pub(crate) fn failed<'s>(error: Error) -> Iter<'s> {
    Box::new(std::iter::once(Err(error)))
}

pub(crate) fn from_vec<'s>(items: Vec<Item>) -> Iter<'s> {
    Box::new(items.into_iter().map(Ok))
}

pub(crate) fn sort_nodes(mut nodes: Vec<Node>) -> Vec<Item> {
    nodes.sort_by(Node::cmp_document_order);
    nodes.dedup();
    nodes.into_iter().map(Item::Node).collect()
}

/// The effective boolean value of a sequence, reading at most two items.
fn ebv_of(mut items: Iter<'_>) -> Result<bool> {
    match items.next() {
        None => Ok(false),
        Some(first) => match first? {
            Item::Node(_) => Ok(true),
            Item::Atomic(value) => {
                if let Some(second) = items.next() {
                    second?;
                    return err(
                        ErrorCode::Forg0006,
                        format!(
                            "Effective boolean value is not defined for a sequence of two or more items starting with a {} value",
                            value.atomic_type()
                        ),
                    );
                }
                Ok(ops::effective_boolean_value(&value)?)
            }
        },
    }
}

impl CompiledExpr {
    /// Evaluates the expression: the whole sequence.
    ///
    /// # Errors
    ///
    /// The dynamic error the evaluation raises.
    pub fn evaluate(&self, dc: &DynamicContext<'_>, context: &Context) -> Result<Vec<Item>> {
        let run = Run {
            dc,
            current: context.current.clone(),
        };
        let ctx = initial(context);
        run.seq(&self.expr, &ctx)?.collect()
    }

    /// Evaluates the effective boolean value of the expression, reading no more of the
    /// sequence than needed (as `xsl:if` and `xsl:when` do).
    ///
    /// # Errors
    ///
    /// The dynamic error the evaluation raises.
    pub fn effective_boolean_value(
        &self,
        dc: &DynamicContext<'_>,
        context: &Context,
    ) -> Result<bool> {
        let run = Run {
            dc,
            current: context.current.clone(),
        };
        run.ebv(&self.expr, &initial(context))
    }
}

fn initial(context: &Context) -> Ctx {
    match &context.focus {
        Some(focus) => Ctx {
            item: Some(focus.item.clone()),
            position: focus.position,
            size: Some(focus.size),
            env: None,
        },
        None => Ctx {
            item: None,
            position: 0,
            size: None,
            env: None,
        },
    }
}

fn boolean<'s>(value: bool) -> Iter<'s> {
    once(Item::Atomic(AtomicValue::Boolean(value)))
}

impl<'a> Run<'a> {
    /// The sequence of `expr`, lazily.
    pub(crate) fn seq<'s>(&'s self, expr: &'s Expr, ctx: &Ctx) -> Result<Iter<'s>>
    where
        'a: 's,
    {
        Ok(match expr {
            Expr::Literal(value) => once(Item::Atomic(value.clone())),
            Expr::ContextItem => once(context_item(ctx)?),
            Expr::Root => {
                let node = context_node(ctx, "the root expression /")?;
                once(Item::Node(node.root()))
            }
            Expr::Local(depth) => once(local(ctx, *depth)?),
            Expr::External(id) => {
                let value = self.dc.host.variable(*id)?;
                Box::new((0..value.len()).map(move |i| Ok(value[i].clone())))
            }
            Expr::Sequence(items) => {
                let ctx = ctx.clone();
                Box::new(
                    items
                        .iter()
                        .flat_map(move |e| self.seq(e, &ctx).unwrap_or_else(failed)),
                )
            }
            Expr::Filter { base, predicates } => {
                let input = self.seq(base, ctx)?;
                self.filter(input, predicates, ctx)?
            }
            Expr::Step(step) => self.step(step, ctx)?,
            Expr::Path {
                lhs,
                rhs,
                mode,
                rhs_uses_last,
            } => self.path(lhs, rhs, *mode, *rhs_uses_last, ctx)?,
            Expr::Union {
                lhs,
                rhs,
                lhs_ordered,
                rhs_ordered,
            } => {
                let left = self.union_operand(lhs, *lhs_ordered, ctx, "first")?;
                let right = self.union_operand(rhs, *rhs_ordered, ctx, "second")?;
                Box::new(Merge {
                    left,
                    right,
                    heads: None,
                })
            }
            Expr::Range(lhs, rhs) => self.range(lhs, rhs, ctx)?,
            Expr::If {
                condition,
                then,
                otherwise,
            } => {
                let branch = if self.ebv(condition, ctx)? {
                    then
                } else {
                    otherwise
                };
                self.seq(branch, ctx)?
            }
            Expr::For { sequence, body } => {
                let input = self.seq(sequence, ctx)?;
                let ctx = ctx.clone();
                Box::new(input.flat_map(move |item| match item {
                    Ok(item) => self.seq(body, &ctx.with_local(item)).unwrap_or_else(failed),
                    Err(e) => failed(e),
                }))
            }
            Expr::Call {
                function,
                args,
                data,
            } => self.call(*function, args, data, ctx)?,
            Expr::HostCall { id, args } => {
                let values = args
                    .iter()
                    .map(|a| self.seq(a, ctx)?.collect::<Result<Vec<_>>>())
                    .collect::<Result<Vec<_>>>()?;
                from_vec(self.dc.host.call_function(*id, values)?)
            }
            Expr::Cast { .. } => match self.cast(expr, ctx)? {
                Some(value) => once(Item::Atomic(value)),
                None => Box::new(std::iter::empty()),
            },
            Expr::Negate(..) | Expr::Arithmetic { .. } | Expr::ValueComparison { .. } => {
                match self.atomic_operation(expr, ctx)? {
                    Some(value) => once(Item::Atomic(value)),
                    None => Box::new(std::iter::empty()),
                }
            }
            Expr::GeneralComparison { .. }
            | Expr::And(..)
            | Expr::Or(..)
            | Expr::Quantified { .. }
            | Expr::InstanceOf { .. }
            | Expr::Castable { .. } => boolean(self.ebv(expr, ctx)?),
        })
    }

    /// `operand cast as target`, `?` when `allows_empty`: the atomized operand, at most
    /// one item, cast to the target (up-cast for an operand statically typed
    /// `xs:integer` and the target `xs:decimal`).
    fn cast(&self, expr: &Expr, ctx: &Ctx) -> Result<Option<AtomicValue>> {
        let Expr::Cast {
            operand,
            target,
            allows_empty,
            upcast,
        } = expr
        else {
            return err(ErrorCode::Internal, "not a cast");
        };
        match self.atomic_opt(operand, ctx, "value in 'cast as' expression")? {
            None if *allows_empty => Ok(None),
            None => err(
                ErrorCode::Xpty0004,
                "An empty sequence is not allowed as the value in 'cast as' expression",
            ),
            Some(value) if *upcast => Ok(Some(value.upcast(*target)?)),
            Some(value) => Ok(Some(value.cast(*target)?)),
        }
    }

    /// Unary minus, arithmetic and value comparisons: the operands are atomized and read
    /// left to right, and an empty one makes the result empty without reading the next.
    fn atomic_operation(&self, expr: &Expr, ctx: &Ctx) -> Result<Option<AtomicValue>> {
        let operands = |lhs: &Expr,
                        rhs: &Expr,
                        what: &str|
         -> Result<Option<(AtomicValue, AtomicValue)>> {
            let Some(a) = self.atomic_opt(lhs, ctx, &format!("first operand of {what}"))? else {
                return Ok(None);
            };
            let Some(b) = self.atomic_opt(rhs, ctx, &format!("second operand of {what}"))? else {
                return Ok(None);
            };
            Ok(Some((a, b)))
        };
        Ok(match expr {
            Expr::Negate(operand) => match self.atomic_opt(operand, ctx, "operand of '-'")? {
                Some(value) => Some(ops::negate(&value)?),
                None => None,
            },
            Expr::Arithmetic { op, lhs, rhs } => {
                match operands(lhs, rhs, &format!("'{}'", arithmetic_name(*op)))? {
                    Some((a, b)) => Some(ops::arithmetic(*op, &a, &b)?),
                    None => None,
                }
            }
            Expr::ValueComparison { op, lhs, rhs } => {
                match operands(lhs, rhs, "a value comparison")? {
                    Some((a, b)) => Some(AtomicValue::Boolean(ops::value_compare(
                        &a,
                        *op,
                        &b,
                        self.dc.implicit_timezone,
                    )?)),
                    None => None,
                }
            }
            _ => return err(ErrorCode::Internal, "not an atomic operation"),
        })
    }

    /// `lhs and rhs` (`decisive` false) or `lhs or rhs` (`decisive` true), as Saxon-HE
    /// 12.10 evaluates them (`AndElaborator`, `OrElaborator`): the first operand first,
    /// and its value decides when it is `decisive`; when it raises a dynamic error, the
    /// second operand is evaluated all the same, and the error is raised only if the
    /// second does not decide the result (an error of the second wins). A refusal or a
    /// bug of this crate is never set aside.
    fn and_or(&self, lhs: &Expr, rhs: &Expr, decisive: bool, ctx: &Ctx) -> Result<bool> {
        match self.ebv(lhs, ctx) {
            Ok(value) if value == decisive => Ok(decisive),
            Ok(_) => self.ebv(rhs, ctx),
            Err(e) if matches!(e.code(), ErrorCode::Unsupported | ErrorCode::Internal) => Err(e),
            Err(e) => {
                if self.ebv(rhs, ctx)? == decisive {
                    Ok(decisive)
                } else {
                    Err(e)
                }
            }
        }
    }

    /// The effective boolean value of `expr`.
    pub(crate) fn ebv(&self, expr: &Expr, ctx: &Ctx) -> Result<bool> {
        match expr {
            Expr::GeneralComparison {
                op,
                lhs,
                rhs,
                strategy,
            } => self.general_comparison(*op, lhs, rhs, *strategy, ctx),
            Expr::And(lhs, rhs) => self.and_or(lhs, rhs, false, ctx),
            // As Saxon: the union is not empty when one operand is not, read in turn.
            Expr::Union { lhs, rhs, .. } => {
                Ok(self.has_node(lhs, ctx, "first")? || self.has_node(rhs, ctx, "second")?)
            }
            Expr::Or(lhs, rhs) => self.and_or(lhs, rhs, true, ctx),
            Expr::Quantified {
                every,
                sequence,
                satisfies,
            } => {
                for item in self.seq(sequence, ctx)? {
                    let holds = self.ebv(satisfies, &ctx.with_local(item?))?;
                    if holds != *every {
                        return Ok(holds);
                    }
                }
                Ok(*every)
            }
            Expr::InstanceOf {
                operand,
                item,
                occurrence,
            } => self.instance_of(operand, item, *occurrence, ctx),
            Expr::Castable {
                operand,
                target,
                allows_empty,
            } => {
                let mut values = self.seq(operand, ctx)?;
                let Some(first) = values.next() else {
                    return Ok(*allows_empty);
                };
                let first = first?.atomize();
                if let Some(second) = values.next() {
                    second?;
                    return Ok(false);
                }
                match first.cast(*target) {
                    Ok(_) => Ok(true),
                    Err(e) if e.code() == einvoice_xdm::ErrorCode::Unsupported => Err(e.into()),
                    Err(_) => Ok(false),
                }
            }
            // As Saxon: `count(E)` as a boolean is `exists(E)`.
            Expr::Call {
                function: Builtin::Count,
                args,
                ..
            } => self.non_empty(&args[0], ctx),
            Expr::Call {
                function: Builtin::Boolean,
                args,
                ..
            } => self.ebv(&args[0], ctx),
            _ => ebv_of(self.seq(expr, ctx)?),
        }
    }

    /// Whether `expr` has an item (`fn:exists`), reading at most one: the operands of a
    /// union are tried in turn, as Saxon does.
    pub(crate) fn non_empty(&self, expr: &Expr, ctx: &Ctx) -> Result<bool> {
        if let Expr::Union { lhs, rhs, .. } = expr {
            return Ok(self.has_node(lhs, ctx, "first")? || self.has_node(rhs, ctx, "second")?);
        }
        Ok(self.seq(expr, ctx)?.next().transpose()?.is_some())
    }

    /// Whether an operand of `|` has an item (which must be a node).
    fn has_node(&self, expr: &Expr, ctx: &Ctx, side: &str) -> Result<bool> {
        match self.seq(expr, ctx)?.next().transpose()? {
            None => Ok(false),
            Some(Item::Node(_)) => Ok(true),
            Some(Item::Atomic(value)) => err(
                ErrorCode::Xpty0004,
                format!(
                    "Required item type of {side} operand of '|' is node(); supplied value has item type {}",
                    value.atomic_type()
                ),
            ),
        }
    }

    fn instance_of(
        &self,
        operand: &Expr,
        test: &ItemTest,
        occurrence: einvoice_xpath::Occurrence,
        ctx: &Ctx,
    ) -> Result<bool> {
        use einvoice_xpath::Occurrence;
        let mut count = 0_usize;
        for item in self.seq(operand, ctx)? {
            let item = item?;
            count += 1;
            let matches = match (test, &item) {
                (ItemTest::Atomic(t), Item::Atomic(v)) => v.is_instance_of(*t),
                (ItemTest::Node(t), Item::Node(n)) => t.matches(n.document(), n.index()),
                _ => false,
            };
            if !matches {
                return Ok(false);
            }
            if count > 1 && matches!(occurrence, Occurrence::One | Occurrence::ZeroOrOne) {
                return Ok(false);
            }
        }
        Ok(match occurrence {
            Occurrence::One | Occurrence::OneOrMore => count >= 1,
            Occurrence::ZeroOrOne | Occurrence::ZeroOrMore => true,
        })
    }

    /// The atomized items of `expr`, lazily.
    pub(crate) fn atomized<'s>(
        &'s self,
        expr: &'s Expr,
        ctx: &Ctx,
    ) -> Result<impl Iterator<Item = Result<AtomicValue>> + 's>
    where
        'a: 's,
    {
        Ok(self.seq(expr, ctx)?.map(|item| item.map(|i| i.atomize())))
    }

    /// At most one item of `expr`, atomized; `XPTY0004` when there are more.
    pub(crate) fn atomic_opt(
        &self,
        expr: &Expr,
        ctx: &Ctx,
        what: &str,
    ) -> Result<Option<AtomicValue>> {
        Ok(self.item_opt(expr, ctx, what)?.map(|i| i.atomize()))
    }

    /// At most one item of `expr`; `XPTY0004` when there are more.
    pub(crate) fn item_opt(&self, expr: &Expr, ctx: &Ctx, what: &str) -> Result<Option<Item>> {
        let mut items = self.seq(expr, ctx)?;
        let Some(first) = items.next() else {
            return Ok(None);
        };
        let first = first?;
        if let Some(second) = items.next() {
            second?;
            return err(
                ErrorCode::Xpty0004,
                format!("A sequence of more than one item is not allowed as the {what}"),
            );
        }
        Ok(Some(first))
    }

    fn general_comparison(
        &self,
        op: ops::ComparisonOp,
        lhs: &Expr,
        rhs: &Expr,
        strategy: CompareStrategy,
        ctx: &Ctx,
    ) -> Result<bool> {
        let tz = self.dc.implicit_timezone;
        let compare = |a: &AtomicValue, b: &AtomicValue| -> Result<bool> {
            Ok(ops::general_compare(a, op, b, tz)?)
        };
        let what = "operand of a general comparison";
        match strategy {
            CompareStrategy::OneToOne => {
                let Some(a) = self.atomic_opt(lhs, ctx, what)? else {
                    return Ok(false);
                };
                let Some(b) = self.atomic_opt(rhs, ctx, what)? else {
                    return Ok(false);
                };
                compare(&a, &b)
            }
            CompareStrategy::ManyToOne => {
                let Some(b) = self.atomic_opt(rhs, ctx, what)? else {
                    return Ok(false);
                };
                for a in self.atomized(lhs, ctx)? {
                    if compare(&a?, &b)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            CompareStrategy::OneToMany => {
                let Some(a) = self.atomic_opt(lhs, ctx, what)? else {
                    return Ok(false);
                };
                for b in self.atomized(rhs, ctx)? {
                    if compare(&a, &b?)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            CompareStrategy::ManyToMany => {
                let left = self.atomized(lhs, ctx)?;
                let right = self.atomized(rhs, ctx)?;
                many_to_many(left, right, &compare)
            }
        }
    }

    fn filter<'s>(
        &'s self,
        input: Iter<'s>,
        predicates: &'s [Predicate],
        ctx: &Ctx,
    ) -> Result<Iter<'s>>
    where
        'a: 's,
    {
        let mut current = input;
        for predicate in predicates {
            current = self.apply_predicate(current, predicate, ctx)?;
        }
        Ok(current)
    }

    fn apply_predicate<'s>(
        &'s self,
        input: Iter<'s>,
        predicate: &'s Predicate,
        ctx: &Ctx,
    ) -> Result<Iter<'s>>
    where
        'a: 's,
    {
        if let PredicateKind::Position(position) = predicate.kind {
            let Some(n) = position else {
                return Ok(Box::new(std::iter::empty()));
            };
            let mut input = input;
            let mut done = false;
            return Ok(Box::new(std::iter::from_fn(move || {
                if done {
                    return None;
                }
                done = true;
                for _ in 1..n {
                    match input.next()? {
                        Ok(_) => {}
                        Err(e) => return Some(Err(e)),
                    }
                }
                input.next()
            })));
        }
        let (input, size): (Iter<'s>, Option<usize>) = if predicate.uses_last {
            let items = input.collect::<Result<Vec<_>>>()?;
            let size = items.len();
            (from_vec(items), Some(size))
        } else {
            (input, None)
        };
        let ctx = ctx.clone();
        let mut position = 0;
        Ok(Box::new(input.filter_map(move |item| {
            let item = match item {
                Ok(item) => item,
                Err(e) => return Some(Err(e)),
            };
            position += 1;
            let focus = ctx.with_focus(item.clone(), position, size);
            match self.predicate_holds(predicate, &focus) {
                Ok(true) => Some(Ok(item)),
                Ok(false) => None,
                Err(e) => Some(Err(e)),
            }
        })))
    }

    fn predicate_holds(&self, predicate: &Predicate, ctx: &Ctx) -> Result<bool> {
        if predicate.kind == PredicateKind::Boolean {
            return self.ebv(&predicate.expr, ctx);
        }
        let mut values = self.seq(&predicate.expr, ctx)?;
        let Some(first) = values.next() else {
            return Ok(false);
        };
        match first? {
            Item::Node(_) => Ok(true),
            Item::Atomic(value) => {
                if let Some(second) = values.next() {
                    second?;
                    return err(
                        ErrorCode::Forg0006,
                        format!(
                            "Effective boolean value is not defined for a sequence of two or more items starting with a {} value",
                            value.atomic_type()
                        ),
                    );
                }
                if value.is_numeric() {
                    let position = AtomicValue::Integer(Integer::from(
                        i64::try_from(ctx.position).unwrap_or(i64::MAX),
                    ));
                    return Ok(ops::value_compare(
                        &value,
                        ops::ComparisonOp::Equal,
                        &position,
                        self.dc.implicit_timezone,
                    )?);
                }
                Ok(ops::effective_boolean_value(&value)?)
            }
        }
    }

    fn step<'s>(&'s self, step: &'s Step, ctx: &Ctx) -> Result<Iter<'s>>
    where
        'a: 's,
    {
        let node = context_node(ctx, &format!("the axis step {}::", axis_name(step.axis)))?;
        let doc = Rc::clone(node.document());
        let mut cursor = doc.cursor(node.index(), step.axis);
        let test = &step.test;
        let nodes: Iter<'s> = Box::new(std::iter::from_fn(move || {
            loop {
                let index = cursor.next(&doc)?;
                if test.matches(&doc, index) {
                    return Some(Ok(Item::Node(Node::new(Rc::clone(&doc), index))));
                }
            }
        }));
        let filtered = self.filter(nodes, &step.predicates, ctx)?;
        if step.axis.is_reverse() && step.axis != NodeAxis::Parent {
            // Predicates count in axis order; the step delivers document order.
            let mut items = filtered.collect::<Result<Vec<_>>>()?;
            items.reverse();
            return Ok(from_vec(items));
        }
        Ok(filtered)
    }

    fn path<'s>(
        &'s self,
        lhs: &'s Expr,
        rhs: &'s Expr,
        mode: PathMode,
        rhs_uses_last: bool,
        ctx: &Ctx,
    ) -> Result<Iter<'s>>
    where
        'a: 's,
    {
        let left = self.seq(lhs, ctx)?;
        let (left, size): (Iter<'s>, Option<usize>) = if rhs_uses_last {
            let items = left.collect::<Result<Vec<_>>>()?;
            let size = items.len();
            (from_vec(items), Some(size))
        } else {
            (left, None)
        };
        let ctx = ctx.clone();
        let mapped = left.enumerate().flat_map(move |(i, item)| match item {
            Err(e) => failed(e),
            Ok(Item::Atomic(value)) => failed(Error::new(
                ErrorCode::Xpty0019,
                format!(
                    "Required item type of first operand of '/' is node(); supplied value has item type {}",
                    value.atomic_type()
                ),
            )),
            Ok(node) => self
                .seq(rhs, &ctx.with_focus(node, i + 1, size))
                .unwrap_or_else(failed),
        });
        match mode {
            PathMode::Ordered | PathMode::Atomic => Ok(Box::new(mapped)),
            PathMode::Sort | PathMode::Dynamic => {
                let mut nodes = Vec::new();
                let mut atomics = Vec::new();
                for item in mapped {
                    match item? {
                        Item::Node(node) => nodes.push(node),
                        Item::Atomic(value) => atomics.push(Item::Atomic(value)),
                    }
                }
                if atomics.is_empty() {
                    Ok(from_vec(sort_nodes(nodes)))
                } else if nodes.is_empty() {
                    Ok(from_vec(atomics))
                } else {
                    err(
                        ErrorCode::Xpty0018,
                        "Cannot mix nodes and atomic values in the result of a path expression",
                    )
                }
            }
        }
    }

    /// An operand of `|`, as nodes in document order: read lazily when it is known to be
    /// in that order, sorted first otherwise.
    fn union_operand<'s>(
        &'s self,
        expr: &'s Expr,
        ordered: bool,
        ctx: &Ctx,
        side: &'static str,
    ) -> Result<Box<dyn Iterator<Item = Result<Node>> + 's>>
    where
        'a: 's,
    {
        let nodes = self.seq(expr, ctx)?.map(move |item| match item? {
            Item::Node(node) => Ok(node),
            Item::Atomic(value) => err(
                ErrorCode::Xpty0004,
                format!(
                    "Required item type of {side} operand of '|' is node(); supplied value has item type {}",
                    value.atomic_type()
                ),
            ),
        });
        if ordered {
            return Ok(Box::new(nodes));
        }
        let sorted = sort_nodes(nodes.collect::<Result<Vec<_>>>()?);
        Ok(Box::new(sorted.into_iter().filter_map(|item| match item {
            Item::Node(node) => Some(Ok(node)),
            Item::Atomic(_) => None,
        })))
    }

    fn range<'s>(&'s self, lhs: &'s Expr, rhs: &'s Expr, ctx: &Ctx) -> Result<Iter<'s>>
    where
        'a: 's,
    {
        let bound = |expr: &Expr, what: &str| -> Result<Option<i64>> {
            let Some(value) = self.atomic_opt(expr, ctx, what)? else {
                return Ok(None);
            };
            let value = match value {
                AtomicValue::UntypedAtomic(_) => value.cast(einvoice_xdm::AtomicType::Integer)?,
                AtomicValue::Integer(_) => value,
                other => {
                    return err(
                        ErrorCode::Xpty0004,
                        format!(
                            "Required item type of the {what} is xs:integer; supplied value has item type {}",
                            other.atomic_type()
                        ),
                    );
                }
            };
            match value {
                AtomicValue::Integer(i) => i.to_i64().map(Some).ok_or_else(|| {
                    Error::new(ErrorCode::Unsupported, "range bound beyond 64 bits")
                }),
                _ => err(ErrorCode::Internal, "range bound"),
            }
        };
        let Some(from) = bound(lhs, "first operand of 'to'")? else {
            return Ok(Box::new(std::iter::empty()));
        };
        let Some(to) = bound(rhs, "second operand of 'to'")? else {
            return Ok(Box::new(std::iter::empty()));
        };
        Ok(Box::new((from..=to).map(|i| {
            Ok(Item::Atomic(AtomicValue::Integer(Integer::from(i))))
        })))
    }
}

type NodeIter<'s> = Box<dyn Iterator<Item = Result<Node>> + 's>;

/// The union of two node sequences in document order, read lazily (Saxon's
/// `VennIterator`): the first node of each operand is read on the first call.
struct Merge<'s> {
    left: NodeIter<'s>,
    right: NodeIter<'s>,
    /// The next node of each side, once started (`None` inside: that side is done).
    heads: Option<(Option<Node>, Option<Node>)>,
}

impl Merge<'_> {
    /// The next node of each side that needs one.
    fn pull(
        &mut self,
        heads: (Option<Node>, Option<Node>),
        left: bool,
        right: bool,
    ) -> Result<(Option<Node>, Option<Node>)> {
        let (mut l, mut r) = heads;
        if left {
            l = self.left.next().transpose()?;
        }
        if right {
            r = self.right.next().transpose()?;
        }
        Ok((l, r))
    }

    fn step(&mut self) -> Result<Option<Node>> {
        let heads = if let Some(heads) = self.heads.take() {
            heads
        } else {
            self.pull((None, None), true, true)?
        };
        let (output, left, right) = match &heads {
            (None, None) => {
                self.heads = Some(heads);
                return Ok(None);
            }
            (Some(l), None) => (l.clone(), true, false),
            (None, Some(r)) => (r.clone(), false, true),
            (Some(l), Some(r)) => match l.cmp_document_order(r) {
                std::cmp::Ordering::Less => (l.clone(), true, false),
                std::cmp::Ordering::Greater => (r.clone(), false, true),
                std::cmp::Ordering::Equal => (l.clone(), true, true),
            },
        };
        // As Saxon's union iterator: the side just read from is read again before the
        // node is returned.
        self.heads = Some(self.pull(heads, left, right)?);
        Ok(Some(output))
    }
}

impl Iterator for Merge<'_> {
    type Item = Result<Item>;

    fn next(&mut self) -> Option<Result<Item>> {
        self.step().map(|n| n.map(Item::Node)).transpose()
    }
}

fn context_item(ctx: &Ctx) -> Result<Item> {
    ctx.item
        .clone()
        .ok_or_else(|| Error::new(ErrorCode::Xpdy0002, "The context item is absent"))
}

/// The context item of an axis step or of `/`.
fn context_node(ctx: &Ctx, what: &str) -> Result<Node> {
    match &ctx.item {
        None => err(
            ErrorCode::Xpdy0002,
            format!("The context item for {what} is absent"),
        ),
        Some(Item::Atomic(v)) => err(
            ErrorCode::Xpty0020,
            format!(
                "The context item for {what} is not a node (it is a {})",
                v.atomic_type()
            ),
        ),
        Some(Item::Node(node)) => Ok(node.clone()),
    }
}

fn local(ctx: &Ctx, depth: usize) -> Result<Item> {
    let mut env = ctx.env.as_ref();
    for _ in 0..depth {
        env = env.and_then(|e| e.parent.as_ref());
    }
    env.map(|e| e.item.clone())
        .ok_or_else(|| Error::new(ErrorCode::Internal, "range variable out of scope"))
}

/// Saxon's many-to-many general comparison: items are read from the two sides in turn,
/// each compared with the items already read from the other side.
fn many_to_many(
    mut left: impl Iterator<Item = Result<AtomicValue>>,
    mut right: impl Iterator<Item = Result<AtomicValue>>,
    compare: &dyn Fn(&AtomicValue, &AtomicValue) -> Result<bool>,
) -> Result<bool> {
    let (mut left_done, mut right_done) = (false, false);
    let (mut left_seen, mut right_seen) = (Vec::new(), Vec::new());
    loop {
        if !left_done {
            match left.next() {
                None => {
                    if right_done {
                        return Ok(false);
                    }
                    left_done = true;
                }
                Some(a) => {
                    let a = a?;
                    for b in &right_seen {
                        if compare(&a, b)? {
                            return Ok(true);
                        }
                    }
                    if !right_done {
                        left_seen.push(a);
                    }
                }
            }
        }
        if !right_done {
            match right.next() {
                None => {
                    if left_done {
                        return Ok(false);
                    }
                    right_done = true;
                }
                Some(b) => {
                    let b = b?;
                    for a in &left_seen {
                        if compare(a, &b)? {
                            return Ok(true);
                        }
                    }
                    if !left_done {
                        right_seen.push(b);
                    }
                }
            }
        }
    }
}

const fn arithmetic_name(op: ops::ArithmeticOp) -> &'static str {
    match op {
        ops::ArithmeticOp::Add => "+",
        ops::ArithmeticOp::Subtract => "-",
        ops::ArithmeticOp::Multiply => "*",
        ops::ArithmeticOp::Divide => "div",
        ops::ArithmeticOp::IntegerDivide => "idiv",
        ops::ArithmeticOp::Modulo => "mod",
    }
}

const fn axis_name(axis: NodeAxis) -> &'static str {
    match axis {
        NodeAxis::Child => "child",
        NodeAxis::Attribute => "attribute",
        NodeAxis::SelfAxis => "self",
        NodeAxis::Descendant => "descendant",
        NodeAxis::DescendantOrSelf => "descendant-or-self",
        NodeAxis::FollowingSibling => "following-sibling",
        NodeAxis::Parent => "parent",
        NodeAxis::Ancestor => "ancestor",
        NodeAxis::AncestorOrSelf => "ancestor-or-self",
        NodeAxis::PrecedingSibling => "preceding-sibling",
        NodeAxis::Preceding => "preceding",
    }
}
