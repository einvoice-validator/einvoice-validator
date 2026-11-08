//! Evaluation of compiled expressions.
//!
//! Sequences are produced lazily, as iterators of fallible items, so that a consumer that
//! needs only part of a sequence (an effective boolean value, `exists`, a general
//! comparison that finds a match, a positional predicate, `some` and `every`) stops
//! reading it there, as Saxon does. This decides which dynamic errors are raised: an item
//! that is never read never raises its error. Sequences whose order is not known
//! statically (see `compile`) are materialised and sorted.

use std::borrow::Cow;
use std::rc::Rc;
use std::sync::Arc;

use einvoice_xdm::{AtomicValue, Integer, ops};

use crate::compile::{
    Calculator, CompareStrategy, CompiledExpr, Expr, HostArg, ItemTest, NameMatch, NodeTest,
    PathMode, Predicate, PredicateKind, Step,
};
use crate::context::{Context, DynamicContext, FunctionId};
use crate::error::{Error, ErrorCode, Result, err};
use crate::functions::Builtin;
use crate::item::Item;
use crate::library::ordinal;
use crate::tree::{Cursor, Document, Node, NodeAxis, NodeKind};

/// A lazily produced sequence.
pub(crate) type Iter<'s> = Box<dyn Iterator<Item = Result<Item>> + 's>;

/// What an operand or an argument is, as an error names it: written out only when an
/// error is raised.
#[derive(Debug, Clone, Copy)]
pub(crate) enum What<'w> {
    /// As written.
    Text(&'w str),
    /// The first (`true`) or second operand of an operator, as named.
    Operand(bool, &'w str),
    /// The argument at an index of a function, by name.
    Argument(usize, &'w str),
}

impl std::fmt::Display for What<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(text) => f.write_str(text),
            Self::Operand(first, of) => {
                let nth = if *first { "first" } else { "second" };
                write!(f, "{nth} operand of {of}")
            }
            Self::Argument(index, function) => {
                write!(f, "{} argument of {function}()", ordinal(*index))
            }
        }
    }
}

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
/// An argument of a call to a host function ([`Host::call_function`](crate::Host::call_function)):
/// its value, or, for an argument Saxon evaluates lazily, what computes it when the host
/// asks for it ([`Argument::evaluate`]), any number of times later during the call.
#[derive(Debug, Clone)]
pub struct Argument(ArgumentValue);

#[derive(Debug, Clone)]
enum ArgumentValue {
    Value(Rc<[Item]>),
    Lazy {
        expr: Arc<Expr>,
        ctx: Ctx,
        current: Option<Item>,
    },
}

impl Argument {
    pub(crate) fn value(items: Vec<Item>) -> Self {
        Self(ArgumentValue::Value(items.into()))
    }

    pub(crate) const fn lazy(expr: Arc<Expr>, ctx: Ctx, current: Option<Item>) -> Self {
        Self(ArgumentValue::Lazy { expr, ctx, current })
    }

    /// An argument already evaluated, for a host's own calls.
    #[must_use]
    pub fn of(items: Vec<Item>) -> Self {
        Self::value(items)
    }

    /// The call site of a lazy argument: the same number for every evaluation of this
    /// argument of this call in the compiled expression, which stays valid as long as the
    /// expression lives (the host may keep state per call site, as Saxon's
    /// `LearningEvaluator`). `None` for an argument evaluated before the call (one that
    /// depends on the current item or on the position or size of the focus, or calls a
    /// host function).
    #[must_use]
    pub fn site(&self) -> Option<usize> {
        match &self.0 {
            ArgumentValue::Value(_) => None,
            ArgumentValue::Lazy { expr, .. } => Some(Arc::as_ptr(expr).addr()),
        }
    }

    /// The value of the argument, computed now for a lazy one (in the dynamic context of
    /// the call, which must be `dc`).
    ///
    /// # Errors
    ///
    /// Whatever evaluating the argument raises.
    pub fn evaluate(&self, dc: &DynamicContext<'_>) -> Result<Vec<Item>> {
        match &self.0 {
            ArgumentValue::Value(items) => Ok(items.to_vec()),
            ArgumentValue::Lazy { expr, ctx, current } => {
                let run = Run {
                    dc,
                    current: current.clone(),
                };
                run.seq(expr, ctx)?.collect()
            }
        }
    }
}

pub(crate) struct Run<'a> {
    pub(crate) dc: &'a DynamicContext<'a>,
    pub(crate) current: Option<Item>,
}

/// How an operand of a general comparison is named in errors.
const GENERAL_OPERAND: &str = "operand of a general comparison";

/// The value, if any, as a sequence.
fn optional<'s>(value: Option<AtomicValue>) -> Iter<'s> {
    match value {
        Some(value) => once(Item::Atomic(value)),
        None => Box::new(std::iter::empty()),
    }
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
        if let Some(nodes) = run.nodes_in_place(&self.expr, &ctx)? {
            return Ok(nodes);
        }
        if walkable(&self.expr, Order::Kept) {
            let mut nodes = Vec::new();
            run.walk(&self.expr, &ctx, Order::Kept, &mut |node| {
                nodes.push(Item::Node(node));
                Ok(false)
            })?;
            return Ok(nodes);
        }
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
    /// A call to a host function: the arguments Saxon evaluates lazily are given as
    /// what computes them, the others evaluated now.
    fn host_call(&self, id: FunctionId, args: &[HostArg], ctx: &Ctx) -> Result<Vec<Item>> {
        let arguments = args
            .iter()
            .map(|a| {
                Ok(if a.lazy {
                    Argument::lazy(Arc::clone(&a.expr), ctx.clone(), self.current.clone())
                } else {
                    Argument::value(self.seq(&a.expr, ctx)?.collect::<Result<Vec<_>>>()?)
                })
            })
            .collect::<Result<Vec<_>>>()?;
        self.dc.host.call_function(id, arguments, self.dc)
    }

    /// The sequence of `expr`, lazily. The forms that evaluate operands are each
    /// evaluated by a function of their own, so that the recursion over a deep
    /// expression holds, at each level, only what that level's form needs (the call stack
    /// of a debug build keeps every local of a function).
    pub(crate) fn seq<'s>(&'s self, expr: &'s Expr, ctx: &Ctx) -> Result<Iter<'s>>
    where
        'a: 's,
    {
        self.dc.check_stack()?;
        match expr {
            Expr::Literal(_)
            | Expr::ContextItem
            | Expr::Root
            | Expr::Local(_)
            | Expr::External(_) => self.leaf(expr, ctx),
            Expr::Sequence(items) => Ok(self.sequence(items, ctx)),
            Expr::Filter { base, predicates } => self.filtered(base, predicates, ctx),
            Expr::Step(step) => self.step(step, ctx),
            Expr::Path {
                lhs,
                rhs,
                mode,
                rhs_uses_last,
            } => self.path(lhs, rhs, *mode, *rhs_uses_last, ctx),
            Expr::Union {
                lhs,
                rhs,
                lhs_ordered,
                rhs_ordered,
            } => self.union(lhs, *lhs_ordered, rhs, *rhs_ordered, ctx),
            Expr::Range(lhs, rhs) => self.range(lhs, rhs, ctx),
            Expr::If {
                condition,
                then,
                otherwise,
            } => self.conditional(condition, then, otherwise, ctx),
            Expr::For { sequence, body } => self.for_each(sequence, body, ctx),
            Expr::Call {
                function,
                args,
                data,
            } => self.call(*function, args, data, ctx),
            Expr::HostCall { id, args } => self.host_call(*id, args, ctx).map(from_vec),
            Expr::Cast { .. } => self.cast(expr, ctx).map(optional),
            Expr::Negate(..) | Expr::Arithmetic { .. } | Expr::ValueComparison { .. } => {
                self.atomic_operation(expr, ctx).map(optional)
            }
            Expr::GeneralComparison { .. }
            | Expr::And(..)
            | Expr::Or(..)
            | Expr::Quantified { .. }
            | Expr::InstanceOf { .. }
            | Expr::Castable { .. } => self.ebv(expr, ctx).map(boolean),
        }
    }

    /// The expressions that evaluate no operand: literals, `.`, `/`, variables.
    #[inline]
    fn leaf<'s>(&'s self, expr: &'s Expr, ctx: &Ctx) -> Result<Iter<'s>>
    where
        'a: 's,
    {
        Ok(match expr {
            Expr::Literal(value) => once(Item::Atomic(value.clone())),
            Expr::ContextItem => once(context_item(ctx)?),
            Expr::Root => {
                let root = context_node(ctx, None)?.root();
                if root.kind() != NodeKind::Document {
                    return err(
                        ErrorCode::Xpdy0050,
                        "The root of the tree containing the context item is not a document node",
                    );
                }
                once(Item::Node(root))
            }
            Expr::Local(depth) => once(local(ctx, *depth)?),
            Expr::External(id) => {
                let value = self.dc.host.variable(*id)?;
                Box::new((0..value.len()).map(move |i| Ok(value[i].clone())))
            }
            _ => return err(ErrorCode::Internal, "not a leaf"),
        })
    }

    /// `(a, b, ...)`
    #[inline]
    fn sequence<'s>(&'s self, items: &'s [Expr], ctx: &Ctx) -> Iter<'s>
    where
        'a: 's,
    {
        let ctx = ctx.clone();
        Box::new(
            items
                .iter()
                .flat_map(move |e| self.seq(e, &ctx).unwrap_or_else(failed)),
        )
    }

    /// `base[predicates]...`
    #[inline]
    fn filtered<'s>(
        &'s self,
        base: &'s Expr,
        predicates: &'s [Predicate],
        ctx: &Ctx,
    ) -> Result<Iter<'s>>
    where
        'a: 's,
    {
        let input = self.seq(base, ctx)?;
        self.filter(input, predicates, ctx)
    }

    /// `lhs | rhs`
    #[inline]
    fn union<'s>(
        &'s self,
        lhs: &'s Expr,
        lhs_ordered: bool,
        rhs: &'s Expr,
        rhs_ordered: bool,
        ctx: &Ctx,
    ) -> Result<Iter<'s>>
    where
        'a: 's,
    {
        let left = self.union_operand(lhs, lhs_ordered, ctx, "first")?;
        let right = self.union_operand(rhs, rhs_ordered, ctx, "second")?;
        Ok(Box::new(Merge {
            left,
            right,
            heads: None,
        }))
    }

    /// `if (condition) then then else otherwise`
    #[inline]
    fn conditional<'s>(
        &'s self,
        condition: &'s Expr,
        then: &'s Expr,
        otherwise: &'s Expr,
        ctx: &Ctx,
    ) -> Result<Iter<'s>>
    where
        'a: 's,
    {
        let branch = if self.ebv(condition, ctx)? {
            then
        } else {
            otherwise
        };
        self.seq(branch, ctx)
    }

    /// `for $v in sequence return body`
    #[inline]
    fn for_each<'s>(&'s self, sequence: &'s Expr, body: &'s Expr, ctx: &Ctx) -> Result<Iter<'s>>
    where
        'a: 's,
    {
        let input = self.seq(sequence, ctx)?;
        let ctx = ctx.clone();
        Ok(Box::new(input.flat_map(move |item| match item {
            Ok(item) => self.seq(body, &ctx.with_local(item)).unwrap_or_else(failed),
            Err(e) => failed(e),
        })))
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
        match self.atomic_opt(operand, ctx, What::Text("value in 'cast as' expression"))? {
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
        match expr {
            Expr::Negate(operand) => self.negated(operand, ctx),
            Expr::Arithmetic {
                op,
                lhs,
                rhs,
                calculator,
            } => self.arithmetic_value(*op, *calculator, lhs, rhs, ctx),
            Expr::ValueComparison { op, lhs, rhs } => self.value_comparison(*op, lhs, rhs, ctx),
            _ => err(ErrorCode::Internal, "not an atomic operation"),
        }
    }

    /// The atomized operands of a binary atomic operation, `None` as soon as one is
    /// empty.
    #[inline]
    fn operands(
        &self,
        lhs: &Expr,
        rhs: &Expr,
        what: &str,
        ctx: &Ctx,
    ) -> Result<Option<(AtomicValue, AtomicValue)>> {
        let Some(a) = self.atomic_opt(lhs, ctx, What::Operand(true, what))? else {
            return Ok(None);
        };
        let Some(b) = self.atomic_opt(rhs, ctx, What::Operand(false, what))? else {
            return Ok(None);
        };
        Ok(Some((a, b)))
    }

    /// `-operand`
    #[inline]
    fn negated(&self, operand: &Expr, ctx: &Ctx) -> Result<Option<AtomicValue>> {
        match self.atomic_opt(operand, ctx, What::Text("operand of '-'"))? {
            Some(value) => Ok(Some(ops::negate(&value)?)),
            None => Ok(None),
        }
    }

    /// `lhs op rhs` for `+ - * div idiv mod`
    #[inline]
    fn arithmetic_value(
        &self,
        op: ops::ArithmeticOp,
        calculator: Calculator,
        lhs: &Expr,
        rhs: &Expr,
        ctx: &Ctx,
    ) -> Result<Option<AtomicValue>> {
        match self.operands(lhs, rhs, arithmetic_name(op), ctx)? {
            Some((a, b)) => Ok(Some(arithmetic(op, calculator, &a, &b)?)),
            None => Ok(None),
        }
    }

    /// A value comparison.
    #[inline]
    fn value_comparison(
        &self,
        op: ops::ComparisonOp,
        lhs: &Expr,
        rhs: &Expr,
        ctx: &Ctx,
    ) -> Result<Option<AtomicValue>> {
        match self.operands(lhs, rhs, "a value comparison", ctx)? {
            Some((a, b)) => Ok(Some(AtomicValue::Boolean(ops::value_compare(
                &a,
                op,
                &b,
                self.dc.implicit_timezone,
            )?))),
            None => Ok(None),
        }
    }

    /// `lhs and rhs` (`decisive` false) or `lhs or rhs` (`decisive` true), as Saxon-HE
    /// 12.10 evaluates them (`AndElaborator`, `OrElaborator`): the first operand first,
    /// and its value decides when it is `decisive`; when it raises a dynamic error, the
    /// second operand is evaluated all the same, and the error is raised only if the
    /// second does not decide the result (an error of the second wins). A refusal, a
    /// limit or a bug of this crate is never set aside.
    fn and_or(&self, lhs: &Expr, rhs: &Expr, decisive: bool, ctx: &Ctx) -> Result<bool> {
        match self.ebv(lhs, ctx) {
            Ok(value) if value == decisive => Ok(decisive),
            Ok(_) => self.ebv(rhs, ctx),
            Err(e)
                if matches!(
                    e.code(),
                    ErrorCode::Unsupported | ErrorCode::Internal | ErrorCode::Limit
                ) =>
            {
                Err(e)
            }
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
        self.dc.check_stack()?;
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
            } => self.quantified(*every, sequence, satisfies, ctx),
            Expr::InstanceOf {
                operand,
                item,
                occurrence,
            } => self.instance_of(operand, item, *occurrence, ctx),
            Expr::Castable {
                operand,
                target,
                allows_empty,
            } => self.castable(operand, *target, *allows_empty, ctx),
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
            // A sequence of nodes is true when it has one.
            Expr::Step(step) if simple(step) => Ok(Self::first_of_step(step, ctx)?.is_some()),
            _ if walkable(expr, Order::Free) => {
                self.walk(expr, ctx, Order::Free, &mut |_| Ok(true))
            }
            _ => ebv_of(self.seq(expr, ctx)?),
        }
    }

    /// Calls `f` on each node of `expr`, which must be [`walkable`], in the order and
    /// with the errors of its sequence ([`Run::seq`]), until `f` returns `true` (then
    /// `true`). The steps are walked on the tree and their predicates evaluated in
    /// turn for each node, without the lazy sequences of the general evaluation: a
    /// node reaches a predicate when it passed the ones before, which count the nodes
    /// they see; a predicate `[n]` ends the walk once its `n`th node is through.
    pub(crate) fn walk(
        &self,
        expr: &Expr,
        ctx: &Ctx,
        order: Order,
        f: &mut dyn FnMut(Node) -> Result<bool>,
    ) -> Result<bool> {
        self.dc.check_stack()?;
        match expr {
            Expr::Step(step) => self.walk_step(step, ctx, f),
            Expr::Path { lhs, rhs, .. } => {
                let mut position = 0;
                let mut each = |item: Item| -> Result<bool> {
                    position += 1;
                    match item {
                        Item::Node(node) => {
                            let focus = ctx.with_focus(Item::Node(node), position, None);
                            self.walk(rhs, &focus, order, f)
                        }
                        Item::Atomic(value) => err(
                            ErrorCode::Xpty0019,
                            format!(
                                "Required item type of first operand of '/' is node(); supplied value has item type {}",
                                value.atomic_type()
                            ),
                        ),
                    }
                };
                if walkable(lhs, order) {
                    return self.walk(lhs, ctx, order, &mut |node| each(Item::Node(node)));
                }
                for item in self.seq(lhs, ctx)? {
                    if each(item?)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => err(ErrorCode::Internal, "not a walkable expression"),
        }
    }

    /// [`Run::walk`] for an axis step.
    fn walk_step(
        &self,
        step: &Step,
        ctx: &Ctx,
        f: &mut dyn FnMut(Node) -> Result<bool>,
    ) -> Result<bool> {
        let node = focus_node(ctx, Some(step.axis))?;
        let doc = node.document();
        let predicates = &step.predicates;
        if predicates
            .iter()
            .any(|p| p.kind == PredicateKind::Position(None))
        {
            return Ok(false);
        }
        // The nodes each predicate has seen ([`MAX_WALKED_PREDICATES`] at most).
        let mut seen = [0_usize; MAX_WALKED_PREDICATES];
        for index in step_nodes(doc, node.index(), step) {
            let candidate = Node::new(Rc::clone(doc), index);
            let mut passes = true;
            let mut last = false;
            for (predicate, seen) in predicates.iter().zip(seen.iter_mut()) {
                *seen += 1;
                let holds = match predicate.kind {
                    PredicateKind::Position(Some(n)) => {
                        last = *seen == n;
                        last
                    }
                    _ => self.predicate_holds(
                        predicate,
                        &ctx.with_focus(Item::Node(candidate.clone()), *seen, None),
                    )?,
                };
                if !holds {
                    passes = false;
                    break;
                }
            }
            if passes && f(candidate)? {
                return Ok(true);
            }
            if last {
                return Ok(false);
            }
        }
        Ok(false)
    }

    /// The value of a [`simple`] step, or of the union of two (`@*|*`), read from the
    /// tree in place of the sequences [`Run::seq`] would build: the same nodes in
    /// document order, and the same error for a context item that is not a node (that
    /// of the first operand). `None` for other expressions.
    fn nodes_in_place(&self, expr: &Expr, ctx: &Ctx) -> Result<Option<Vec<Item>>> {
        // The nodes are in the document of the context node (any other context item is
        // an error, or not read in place).
        let doc = match &ctx.item {
            Some(Item::Node(node)) => Some(node.document()),
            _ => None,
        };
        let mut items = Vec::new();
        let read = select_indices(self.dc, expr, ctx.item.as_ref(), &mut |index| {
            if let Some(doc) = doc {
                items.push(Item::Node(Node::new(Rc::clone(doc), index)));
            }
        })?;
        Ok(read.map(|_| items))
    }

    /// The index of the first node of a [`simple`] step from the context node: the step
    /// evaluated in place, as [`Run::step`] would deliver it.
    #[inline]
    fn first_of_step(step: &Step, ctx: &Ctx) -> Result<Option<u32>> {
        let node = focus_node(ctx, Some(step.axis))?;
        Ok(step_nodes(node.document(), node.index(), step).next())
    }

    /// The first two nodes of a [`simple`] step from the context node.
    #[inline]
    fn two_of_step(step: &Step, ctx: &Ctx) -> Result<(Option<Node>, bool)> {
        let node = focus_node(ctx, Some(step.axis))?;
        let doc = node.document();
        let mut matching = step_nodes(doc, node.index(), step);
        let Some(first) = matching.next() else {
            return Ok((None, false));
        };
        let more = matching.next().is_some();
        Ok((Some(Node::new(Rc::clone(doc), first)), more))
    }

    /// `some` (`every` false) or `every` `$v in sequence satisfies satisfies`.
    #[inline]
    fn quantified(
        &self,
        every: bool,
        sequence: &Expr,
        satisfies: &Expr,
        ctx: &Ctx,
    ) -> Result<bool> {
        for item in self.seq(sequence, ctx)? {
            let holds = self.ebv(satisfies, &ctx.with_local(item?))?;
            if holds != every {
                return Ok(holds);
            }
        }
        Ok(every)
    }

    /// `operand castable as target` (`?` when `allows_empty`).
    #[inline]
    fn castable(
        &self,
        operand: &Expr,
        target: einvoice_xdm::AtomicType,
        allows_empty: bool,
        ctx: &Ctx,
    ) -> Result<bool> {
        let mut values = self.seq(operand, ctx)?;
        let Some(first) = values.next() else {
            return Ok(allows_empty);
        };
        let first = first?.atomize();
        if let Some(second) = values.next() {
            second?;
            return Ok(false);
        }
        match first.cast(target) {
            Ok(_) => Ok(true),
            Err(e) if e.code() == einvoice_xdm::ErrorCode::Unsupported => Err(e.into()),
            Err(_) => Ok(false),
        }
    }

    /// Whether `expr` has an item (`fn:exists`), reading at most one: the operands of a
    /// union are tried in turn, as Saxon does.
    pub(crate) fn non_empty(&self, expr: &Expr, ctx: &Ctx) -> Result<bool> {
        match expr {
            Expr::Union { lhs, rhs, .. } => {
                return Ok(self.has_node(lhs, ctx, "first")? || self.has_node(rhs, ctx, "second")?);
            }
            Expr::Step(step) if simple(step) => {
                self.dc.check_stack()?;
                return Ok(Self::first_of_step(step, ctx)?.is_some());
            }
            _ if walkable(expr, Order::Free) => {
                return self.walk(expr, ctx, Order::Free, &mut |_| Ok(true));
            }
            _ => {}
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
        what: What<'_>,
    ) -> Result<Option<AtomicValue>> {
        Ok(self.item_opt(expr, ctx, what)?.map(|i| i.atomize()))
    }

    /// At most one item of `expr`; `XPTY0004` when there are more.
    pub(crate) fn item_opt(&self, expr: &Expr, ctx: &Ctx, what: What<'_>) -> Result<Option<Item>> {
        // The forms that need no sequence to be built.
        match expr {
            Expr::Literal(value) => {
                self.dc.check_stack()?;
                return Ok(Some(Item::Atomic(value.clone())));
            }
            Expr::ContextItem => {
                self.dc.check_stack()?;
                return context_item(ctx).map(Some);
            }
            Expr::Local(depth) => {
                self.dc.check_stack()?;
                return local(ctx, *depth).map(Some);
            }
            Expr::Step(step) if simple(step) => {
                self.dc.check_stack()?;
                let (first, more) = Self::two_of_step(step, ctx)?;
                if more {
                    return err(
                        ErrorCode::Xpty0004,
                        format!("A sequence of more than one item is not allowed as the {what}"),
                    );
                }
                return Ok(first.map(Item::Node));
            }
            _ if walkable(expr, Order::Kept) => {
                let mut first = None;
                let more = self.walk(expr, ctx, Order::Kept, &mut |node| {
                    if first.is_some() {
                        return Ok(true);
                    }
                    first = Some(node);
                    Ok(false)
                })?;
                if more {
                    return err(
                        ErrorCode::Xpty0004,
                        format!("A sequence of more than one item is not allowed as the {what}"),
                    );
                }
                return Ok(first.map(Item::Node));
            }
            _ => {}
        }
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
        match strategy {
            CompareStrategy::OneToOne => self.one_to_one(op, lhs, rhs, ctx),
            CompareStrategy::ManyToOne => self.many_to_one(op, lhs, rhs, ctx),
            CompareStrategy::OneToMany => self.one_to_many(op, lhs, rhs, ctx),
            CompareStrategy::ManyToMany => {
                let tz = self.dc.implicit_timezone;
                let compare = |a: &AtomicValue, b: &AtomicValue| -> Result<bool> {
                    Ok(ops::general_compare(a, op, b, tz)?)
                };
                let left = self.atomized(lhs, ctx)?;
                let right = self.atomized(rhs, ctx)?;
                many_to_many(left, right, &compare)
            }
        }
    }

    /// A general comparison of two operands of at most one item each.
    #[inline]
    fn one_to_one(&self, op: ops::ComparisonOp, lhs: &Expr, rhs: &Expr, ctx: &Ctx) -> Result<bool> {
        let Some(a) = self.item_opt(lhs, ctx, What::Text(GENERAL_OPERAND))? else {
            return Ok(false);
        };
        let Some(b) = self.item_opt(rhs, ctx, What::Text(GENERAL_OPERAND))? else {
            return Ok(false);
        };
        compare_items(&a, op, &b, self.dc.implicit_timezone)
    }

    /// A general comparison whose second operand has at most one item.
    #[inline]
    fn many_to_one(
        &self,
        op: ops::ComparisonOp,
        lhs: &Expr,
        rhs: &Expr,
        ctx: &Ctx,
    ) -> Result<bool> {
        let Some(b) = self.atomic_opt(rhs, ctx, What::Text(GENERAL_OPERAND))? else {
            return Ok(false);
        };
        if walkable(lhs, Order::Free) {
            let tz = self.dc.implicit_timezone;
            return self.walk(lhs, ctx, Order::Free, &mut |node| {
                compare_node(&node, op, &b, tz, false)
            });
        }
        for a in self.atomized(lhs, ctx)? {
            if ops::general_compare(&a?, op, &b, self.dc.implicit_timezone)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// A general comparison whose first operand has at most one item.
    #[inline]
    fn one_to_many(
        &self,
        op: ops::ComparisonOp,
        lhs: &Expr,
        rhs: &Expr,
        ctx: &Ctx,
    ) -> Result<bool> {
        let Some(a) = self.atomic_opt(lhs, ctx, What::Text(GENERAL_OPERAND))? else {
            return Ok(false);
        };
        if walkable(rhs, Order::Free) {
            let tz = self.dc.implicit_timezone;
            return self.walk(rhs, ctx, Order::Free, &mut |node| {
                compare_node(&node, op, &a, tz, true)
            });
        }
        for b in self.atomized(rhs, ctx)? {
            if ops::general_compare(&a, op, &b?, self.dc.implicit_timezone)? {
                return Ok(true);
            }
        }
        Ok(false)
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
        let node = context_node(ctx, Some(step.axis))?;
        let doc = Rc::clone(node.document());
        let nodes: Iter<'s> = if let Some(listed) = indexed(&doc, node.index(), step) {
            let listed = listed.to_vec();
            Box::new(
                listed
                    .into_iter()
                    .map(move |index| Ok(Item::Node(Node::new(Rc::clone(&doc), index)))),
            )
        } else {
            let mut cursor = doc.cursor(node.index(), step.axis);
            let test = &step.test;
            Box::new(std::iter::from_fn(move || {
                loop {
                    let index = cursor.next(&doc)?;
                    if test.matches(&doc, index) {
                        return Some(Ok(Item::Node(Node::new(Rc::clone(&doc), index))));
                    }
                }
            }))
        };
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
            let Some(value) = self.atomic_opt(expr, ctx, What::Text(what))? else {
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

/// The indices of the nodes a step selects from the node at `from` by its axis and node
/// test, in axis order (its predicates aside): the elements of a name among the
/// descendants are read from the document's index of its elements when it has one.
fn step_nodes<'d>(doc: &'d Document, from: u32, step: &'d Step) -> StepNodes<'d> {
    if let Some(listed) = indexed(doc, from, step) {
        return StepNodes::Listed(listed.iter());
    }
    StepNodes::Walked {
        cursor: doc.cursor(from, step.axis),
        doc,
        test: &step.test,
    }
}

/// The elements of the name a `descendant` or `descendant-or-self` step tests for, from
/// the index of the document's elements ([`Document::elements_named`]).
fn indexed<'d>(doc: &'d Document, from: u32, step: &Step) -> Option<&'d [u32]> {
    let NodeTest::Named {
        kind: NodeKind::Element,
        name: NameMatch::Exact { namespace, local },
    } = &step.test
    else {
        return None;
    };
    let (first_child, end) = doc.descendant_range(from);
    let start = match step.axis {
        NodeAxis::Descendant => first_child,
        NodeAxis::DescendantOrSelf => from,
        _ => return None,
    };
    doc.elements_named(namespace.as_deref(), local, start, end)
}

/// The nodes of [`step_nodes`].
enum StepNodes<'d> {
    /// Read from a list of the document's elements.
    Listed(std::slice::Iter<'d, u32>),
    /// Walked on the axis, those that pass the test.
    Walked {
        cursor: Cursor,
        doc: &'d Document,
        test: &'d NodeTest,
    },
}

impl Iterator for StepNodes<'_> {
    type Item = u32;

    #[inline]
    fn next(&mut self) -> Option<u32> {
        match self {
            Self::Listed(listed) => listed.next().copied(),
            Self::Walked { cursor, doc, test } => loop {
                let index = cursor.next(doc)?;
                if test.matches(doc, index) {
                    return Some(index);
                }
            },
        }
    }
}

/// The nodes of an expression read in place ([`CompiledExpr::select_in_place`]): a
/// document and node indices in document order.
#[derive(Debug, Clone)]
pub struct NodeSelection {
    doc: Rc<Document>,
    indices: Vec<u32>,
}

impl NodeSelection {
    /// The number of nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.indices.len()
    }

    /// Whether there is no node.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// The node at `position` (from 0) in document order.
    #[must_use]
    pub fn get(&self, position: usize) -> Option<Node> {
        let index = *self.indices.get(position)?;
        Some(Node::new(Rc::clone(&self.doc), index))
    }

    /// The nodes, in document order.
    pub fn iter(&self) -> impl Iterator<Item = Node> + '_ {
        self.indices
            .iter()
            .map(|&index| Node::new(Rc::clone(&self.doc), index))
    }

    /// The storage of the selection, emptied, to be given to a later selection
    /// ([`CompiledExpr::select_in_place`]).
    #[must_use]
    pub fn into_storage(self) -> SelectionStorage {
        let mut indices = self.indices;
        indices.clear();
        SelectionStorage(indices)
    }
}

/// Storage for a [`NodeSelection`], reused from one selection to the next.
#[derive(Debug, Clone, Default)]
pub struct SelectionStorage(Vec<u32>);

impl CompiledExpr {
    /// The value of the expression when it is an axis step without predicates on an
    /// axis read in document order, or a union of up to four (`@*|*`,
    /// `*|comment()|processing-instruction()`), or `.` when the context item is a node:
    /// its nodes read from the tree, without the sequences of
    /// [`CompiledExpr::evaluate`] (which gives the same nodes, and raises the same error
    /// for a context item that is not a node). `None` for any other expression, which is
    /// not evaluated.
    ///
    /// # Errors
    ///
    /// The context item is absent or is not a node.
    /// `storage` holds the nodes when they are read; given back by
    /// [`NodeSelection::into_storage`], its memory serves many selections.
    pub fn select_in_place(
        &self,
        dc: &DynamicContext<'_>,
        context: &Context,
        storage: SelectionStorage,
    ) -> Result<Option<NodeSelection>> {
        select_in_place(
            dc,
            &self.expr,
            context.focus.as_ref().map(|f| &f.item),
            storage,
        )
    }
}

/// [`CompiledExpr::select_in_place`] of `expr` with the context item `item`.
fn select_in_place(
    dc: &DynamicContext<'_>,
    expr: &Expr,
    item: Option<&Item>,
    storage: SelectionStorage,
) -> Result<Option<NodeSelection>> {
    let mut indices = storage.0;
    indices.clear();
    let doc = select_indices(dc, expr, item, &mut |index| indices.push(index))?;
    Ok(doc.map(|doc| NodeSelection { doc, indices }))
}

/// The nodes of `expr` with the context item `item` read in place from the tree, as
/// [`CompiledExpr::select_in_place`] reads them: their indices given to `out` in document
/// order, and their document returned. `None` (and nothing given) for another
/// expression.
fn select_indices(
    dc: &DynamicContext<'_>,
    expr: &Expr,
    item: Option<&Item>,
    out: &mut dyn FnMut(u32),
) -> Result<Option<Rc<Document>>> {
    if let Expr::ContextItem = expr {
        // `.`: the context node; an atomic or absent context item is left to the
        // evaluation.
        return Ok(match item {
            Some(Item::Node(node)) => {
                dc.check_stack()?;
                out(node.index());
                Some(Rc::clone(node.document()))
            }
            _ => None,
        });
    }
    let mut steps: [Option<&Step>; MAX_SELECTED_STEPS] = [None; MAX_SELECTED_STEPS];
    let mut count = 0;
    if !union_of_steps(expr, &mut steps, &mut count) {
        return Ok(None);
    }
    let [Some(first), second, ..] = steps else {
        return Ok(None);
    };
    dc.check_stack()?;
    let node = focus_node_of(item, Some(first.axis))?;
    let doc = node.document();
    let from = node.index();
    match (second, count) {
        (None, _) => step_nodes(doc, from, first).for_each(out),
        (Some(second), 2) => merge_two(
            step_nodes(doc, from, first),
            step_nodes(doc, from, second),
            out,
        ),
        _ => {
            // The nodes of each step, in document order, merged.
            let mut walks = steps.map(|step| step.map(|step| step_nodes(doc, from, step)));
            let mut heads = [None; MAX_SELECTED_STEPS];
            for (head, walk) in heads.iter_mut().zip(walks.iter_mut()) {
                *head = walk.as_mut().and_then(Iterator::next);
            }
            while let Some(next) = heads.iter().flatten().min().copied() {
                out(next);
                for (head, walk) in heads.iter_mut().zip(walks.iter_mut()) {
                    if *head == Some(next) {
                        *head = walk.as_mut().and_then(Iterator::next);
                    }
                }
            }
        }
    }
    Ok(Some(Rc::clone(doc)))
}

/// The indices of two walks in document order, merged without duplicates.
fn merge_two(
    mut left: impl Iterator<Item = u32>,
    mut right: impl Iterator<Item = u32>,
    out: &mut dyn FnMut(u32),
) {
    let (mut l, mut r) = (left.next(), right.next());
    loop {
        let next = match (l, r) {
            (None, None) => break,
            (Some(a), None) => {
                l = left.next();
                a
            }
            (None, Some(b)) => {
                r = right.next();
                b
            }
            (Some(a), Some(b)) => {
                if a <= b {
                    l = left.next();
                }
                if b <= a {
                    r = right.next();
                }
                a.min(b)
            }
        };
        out(next);
    }
}

/// The most steps of a union [`select_in_place`] reads.
const MAX_SELECTED_STEPS: usize = 4;

/// Whether `expr` is a [`simple`] step or a union of them (at most
/// [`MAX_SELECTED_STEPS`]), whose steps are put in `steps` from `count` on, left to right.
fn union_of_steps<'e>(
    expr: &'e Expr,
    steps: &mut [Option<&'e Step>; MAX_SELECTED_STEPS],
    count: &mut usize,
) -> bool {
    match expr {
        Expr::Step(step) if simple(step) => match steps.get_mut(*count) {
            Some(slot) => {
                *slot = Some(step);
                *count += 1;
                true
            }
            None => false,
        },
        Expr::Union { lhs, rhs, .. } => {
            union_of_steps(lhs, steps, count) && union_of_steps(rhs, steps, count)
        }
        _ => false,
    }
}

/// The most predicates of a step [`Run::walk`] walks.
const MAX_WALKED_PREDICATES: usize = 4;

/// What a consumer of a node sequence needs of its order, as Saxon-HE 12.10 decides
/// whether to sort a path (`Expression.unordered`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Order {
    /// Document order without duplicates: the sequence as it is.
    Kept,
    /// Neither order nor duplicates matter (an effective boolean value, `exists`,
    /// `empty`, an operand of a general comparison): a path whose nodes would be sorted
    /// is read unsorted, as Saxon reads it there, lazily, its left operand in its own
    /// order, so that the nodes after the one that decides are never evaluated.
    Free,
}

/// Whether [`Run::walk`] can walk `expr` for a consumer needing `order`: an axis step on
/// an axis read in document order whose predicates do not read the context size (at
/// most [`MAX_WALKED_PREDICATES`]), or a path whose right operand is such a step and
/// does not read the context size: of nodes in document order, or with [`Order::Free`]
/// one whose nodes would be sorted, if its left operand is walkable or is not an axis
/// step or a path (its left operand is read as a sequence when it is not walkable). Its
/// items are nodes.
pub(crate) fn walkable(expr: &Expr, order: Order) -> bool {
    match expr {
        Expr::Step(step) => {
            (!step.axis.is_reverse() || step.axis == NodeAxis::Parent)
                && step.predicates.len() <= MAX_WALKED_PREDICATES
                && step.predicates.iter().all(|p| !p.uses_last)
        }
        Expr::Path {
            lhs,
            rhs,
            mode,
            rhs_uses_last: false,
        } => {
            let step = matches!(&**rhs, Expr::Step(_)) && walkable(rhs, order);
            match (mode, order) {
                (PathMode::Ordered, _) => step,
                (PathMode::Sort, Order::Free) => {
                    step && (walkable(lhs, order)
                        || !matches!(&**lhs, Expr::Step(_) | Expr::Path { .. }))
                }
                _ => false,
            }
        }
        _ => false,
    }
}

/// A step without predicates on an axis read in document order: its nodes can be read
/// in place, without building a sequence.
fn simple(step: &Step) -> bool {
    step.predicates.is_empty() && (!step.axis.is_reverse() || step.axis == NodeAxis::Parent)
}

/// The context item of an axis step on `axis`, or of `/` (`None`).
fn context_node(ctx: &Ctx, axis: Option<NodeAxis>) -> Result<Node> {
    focus_node(ctx, axis).cloned()
}

/// The context item of an axis step on `axis`, or of `/` (`None`), borrowed.
fn focus_node(ctx: &Ctx, axis: Option<NodeAxis>) -> Result<&Node> {
    focus_node_of(ctx.item.as_ref(), axis)
}

/// [`focus_node`] of the context item `item`.
fn focus_node_of(item: Option<&Item>, axis: Option<NodeAxis>) -> Result<&Node> {
    let what = || match axis {
        Some(axis) => format!("the axis step {}::", axis_name(axis)),
        None => "the root expression /".to_owned(),
    };
    match item {
        None => err(
            ErrorCode::Xpdy0002,
            format!("The context item for {} is absent", what()),
        ),
        Some(Item::Atomic(v)) => err(
            ErrorCode::Xpty0020,
            format!(
                "The context item for {} is not a node (it is a {})",
                what(),
                v.atomic_type()
            ),
        ),
        Some(Item::Node(node)) => Ok(node),
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

/// The typed value of an item when it is `xs:untypedAtomic` (that of an element,
/// attribute, text or document node, or an untyped value), as text: a node's string
/// value, borrowed from the tree where it can be.
fn untyped_text(item: &Item) -> Option<Cow<'_, str>> {
    match item {
        Item::Node(node) => match node.kind() {
            NodeKind::Comment | NodeKind::ProcessingInstruction => None,
            _ => Some(node.string_value()),
        },
        Item::Atomic(AtomicValue::UntypedAtomic(text)) => Some(Cow::Borrowed(text)),
        Item::Atomic(_) => None,
    }
}

/// The atomized value of an item, borrowed when it is an atomic value.
fn atomized_item(item: &Item) -> Cow<'_, AtomicValue> {
    match item {
        Item::Atomic(value) => Cow::Borrowed(value),
        Item::Node(node) => Cow::Owned(node.typed_value()),
    }
}

/// `a op b`, a general comparison of two items, atomized ([`ops::general_compare`]),
/// an untyped value compared as its text, read in place from the tree for a node.
fn compare_items(a: &Item, op: ops::ComparisonOp, b: &Item, tz: i16) -> Result<bool> {
    Ok(match (untyped_text(a), untyped_text(b)) {
        (Some(x), Some(y)) => ops::general_compare_untyped_pair(&x, op, &y),
        (Some(x), None) => ops::general_compare_untyped(&x, op, &atomized_item(b), tz, false)?,
        (None, Some(y)) => ops::general_compare_untyped(&y, op, &atomized_item(a), tz, true)?,
        (None, None) => ops::general_compare(&atomized_item(a), op, &atomized_item(b), tz)?,
    })
}

/// A general comparison of a node, atomized, and `other` (the right operand, or with
/// `node_is_right` the left one), as [`compare_items`] compares them.
fn compare_node(
    node: &Node,
    op: ops::ComparisonOp,
    other: &AtomicValue,
    tz: i16,
    node_is_right: bool,
) -> Result<bool> {
    Ok(match node.kind() {
        NodeKind::Comment | NodeKind::ProcessingInstruction => {
            let value = node.typed_value();
            if node_is_right {
                ops::general_compare(other, op, &value, tz)?
            } else {
                ops::general_compare(&value, op, other, tz)?
            }
        }
        _ => ops::general_compare_untyped(&node.string_value(), op, other, tz, node_is_right)?,
    })
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

/// `a op b` with the calculator chosen at compile time. Saxon's decimal division
/// (`DecimalDivDecimal`) divides two integer values as decimals of scale 0; any other pair
/// of values is divided as [`ops::arithmetic`] divides it, which is what that calculator
/// does with them.
fn arithmetic(
    op: ops::ArithmeticOp,
    calculator: Calculator,
    a: &AtomicValue,
    b: &AtomicValue,
) -> Result<AtomicValue> {
    match (calculator, a, b) {
        (Calculator::DecimalDivide, AtomicValue::Integer(i), AtomicValue::Integer(j)) => {
            Ok(AtomicValue::Decimal(
                i.as_decimal_operand()
                    .checked_div(&j.as_decimal_operand())?,
            ))
        }
        _ => Ok(ops::arithmetic(op, a, b)?),
    }
}

/// The operator, quoted, as errors name it.
const fn arithmetic_name(op: ops::ArithmeticOp) -> &'static str {
    match op {
        ops::ArithmeticOp::Add => "'+'",
        ops::ArithmeticOp::Subtract => "'-'",
        ops::ArithmeticOp::Multiply => "'*'",
        ops::ArithmeticOp::Divide => "'div'",
        ops::ArithmeticOp::IntegerDivide => "'idiv'",
        ops::ArithmeticOp::Modulo => "'mod'",
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
