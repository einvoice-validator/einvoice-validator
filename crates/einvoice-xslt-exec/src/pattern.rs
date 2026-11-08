//! Match patterns (XSLT 2.0 section 5.5), compiled for matching from the node upwards.
//!
//! A node matches a path pattern when it matches the last step (node test and
//! predicates), its parent the step before (after `/`) or one of its ancestors (after
//! `//`), and so on to the anchor. A predicate whose value can depend on the position of
//! the node among its siblings (it may be numeric, by its form and its static type, or
//! calls `position()` or `last()`) is
//! evaluated as XSLT defines it: the step is evaluated as an axis step from the parent
//! and the node must be in the result. Any other predicate is the effective boolean value
//! of the expression with the node as context item, which is the same thing, evaluated
//! once. In every predicate `current()` is the node being matched.

use einvoice_xpath::{
    self as ast, Axis, AxisStep, KindTest, NameOrWildcard, NameTest, PathExpr, PathStart,
    PatternAlternative, PatternAnchor, PatternAxis, PatternLink, PatternStep, Step,
};
use einvoice_xpath_eval::{
    CompiledExpr, Context, DynamicContext, Focus, Item, Node, NodeKind, Result, StaticContext,
    compile,
};

/// How a name test matches a name.
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

impl NameMatch {
    fn matches(&self, node: &Node) -> bool {
        let Some(name) = node.name() else {
            return false;
        };
        match self {
            Self::Any => true,
            Self::Exact { namespace, local } => {
                name.local == *local && name.namespace.as_deref() == namespace.as_deref()
            }
            Self::Namespace(namespace) => name.namespace.as_deref() == Some(namespace.as_str()),
            Self::Local(local) => name.local == *local,
        }
    }
}

/// The node test of a pattern step, with the axis folded in: a child step only matches
/// nodes that can be children, an attribute step only attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StepTest {
    /// `node()` on the child axis: elements, text, comments, processing instructions.
    AnyChild,
    /// A node kind without name test (text, comment, processing instruction, or
    /// attribute for `@node()`).
    Kind(NodeKind),
    /// Elements or attributes with a name test.
    Named { kind: NodeKind, name: NameMatch },
    /// A test that no node on the axis can pass (`attribute()` on the child axis,
    /// `text()` on the attribute axis).
    Never,
}

impl StepTest {
    /// Whether some node of `kind` can pass the test.
    pub(crate) const fn can_match(&self, kind: NodeKind) -> bool {
        match self {
            Self::AnyChild => matches!(
                kind,
                NodeKind::Element
                    | NodeKind::Text
                    | NodeKind::Comment
                    | NodeKind::ProcessingInstruction
            ),
            Self::Kind(k) | Self::Named { kind: k, .. } => *k as u8 == kind as u8,
            Self::Never => false,
        }
    }

    pub(crate) fn matches(&self, node: &Node) -> bool {
        let kind = node.kind();
        match self {
            Self::AnyChild => matches!(
                kind,
                NodeKind::Element
                    | NodeKind::Text
                    | NodeKind::Comment
                    | NodeKind::ProcessingInstruction
            ),
            Self::Kind(k) => kind == *k,
            Self::Named { kind: k, name } => kind == *k && name.matches(node),
            Self::Never => false,
        }
    }
}

fn name_match(test: &NameTest) -> NameMatch {
    match test {
        NameTest::Name(q) => NameMatch::Exact {
            namespace: q.namespace.clone(),
            local: q.local.clone(),
        },
        NameTest::Any => NameMatch::Any,
        NameTest::AnyLocal { namespace, .. } => NameMatch::Namespace(namespace.clone()),
        NameTest::AnyNamespace { local } => NameMatch::Local(local.clone()),
    }
}

fn wildcard_match(name: Option<&NameOrWildcard>) -> NameMatch {
    match name {
        None | Some(NameOrWildcard::Wildcard) => NameMatch::Any,
        Some(NameOrWildcard::Name(q)) => NameMatch::Exact {
            namespace: q.namespace.clone(),
            local: q.local.clone(),
        },
    }
}

/// The test of a step on `axis` (child or attribute).
pub(crate) fn step_test(axis: PatternAxis, test: &ast::NodeTest) -> StepTest {
    match (axis, test) {
        (PatternAxis::Child, ast::NodeTest::Name(name)) => StepTest::Named {
            kind: NodeKind::Element,
            name: name_match(name),
        },
        (PatternAxis::Attribute, ast::NodeTest::Name(name)) => StepTest::Named {
            kind: NodeKind::Attribute,
            name: name_match(name),
        },
        (PatternAxis::Child, ast::NodeTest::Kind(kind)) => match kind {
            KindTest::AnyKind => StepTest::AnyChild,
            KindTest::Text => StepTest::Kind(NodeKind::Text),
            KindTest::Comment => StepTest::Kind(NodeKind::Comment),
            KindTest::ProcessingInstruction => StepTest::Kind(NodeKind::ProcessingInstruction),
            KindTest::Element(name) => StepTest::Named {
                kind: NodeKind::Element,
                name: wildcard_match(name.as_ref()),
            },
            KindTest::Attribute(_) => StepTest::Never,
        },
        (PatternAxis::Attribute, ast::NodeTest::Kind(kind)) => match kind {
            KindTest::AnyKind => StepTest::Kind(NodeKind::Attribute),
            KindTest::Attribute(name) => StepTest::Named {
                kind: NodeKind::Attribute,
                name: wildcard_match(name.as_ref()),
            },
            KindTest::Text
            | KindTest::Comment
            | KindTest::ProcessingInstruction
            | KindTest::Element(_) => StepTest::Never,
        },
    }
}

/// The predicates of a step.
#[derive(Debug, Clone)]
pub(crate) enum Predicates {
    None,
    /// Effective boolean values, in order, with the node as context item.
    Boolean(Vec<CompiledExpr>),
    /// The step as an axis step with its predicates (`child::test[p]...` or
    /// `attribute::test[p]...`), evaluated from the parent.
    FromParent(CompiledExpr),
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledStep {
    pub(crate) test: StepTest,
    predicates: Predicates,
}

/// A step of a path pattern.
#[derive(Debug, Clone)]
pub(crate) enum PathStep {
    Axis(CompiledStep),
    /// `(alternatives)[predicates]` (XSLT 3.0), the predicates being effective boolean
    /// values: a node matches it when it matches one of the alternatives, whole, and the
    /// predicates hold. After the first step, the alternatives are single relative steps
    /// and there are no predicates (`einvoice_xpath::parse_pattern`).
    Group {
        inner: Vec<Alternative>,
        predicates: Vec<CompiledExpr>,
    },
}

/// One alternative of a pattern.
#[derive(Debug, Clone)]
pub(crate) enum Alternative {
    /// `/`: a document node.
    DocumentRoot,
    /// `root()`: the root of a tree (always a document node here).
    RootFunction,
    Path {
        anchor: PatternAnchor,
        /// The steps, the matched node's last.
        steps: Vec<PathStep>,
        /// `links[i]` joins `steps[i]` and `steps[i + 1]`.
        links: Vec<PatternLink>,
    },
}

/// What kind of node an alternative can match, to index template rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Key {
    Document,
    Element(Option<String>, String),
    AnyElement,
    Attribute(Option<String>, String),
    AnyAttribute,
    /// `node()` on the child axis.
    AnyChild,
    Text,
    Comment,
    ProcessingInstruction,
    /// Nodes of several kinds (`root()`, a parenthesized pattern over several kinds).
    Generic,
}

impl Alternative {
    /// Whether every node of `kind` matches the alternative, which is then a lone step
    /// without predicates whose test does not depend on the name (`text()`, `node()`,
    /// `*`, `@*`): matching it evaluates nothing and cannot fail.
    pub(crate) fn matches_every_node_of(&self, kind: NodeKind) -> bool {
        let Self::Path {
            anchor: PatternAnchor::Relative,
            steps,
            ..
        } = self
        else {
            return false;
        };
        let [
            PathStep::Axis(CompiledStep {
                test,
                predicates: Predicates::None,
            }),
        ] = steps.as_slice()
        else {
            return false;
        };
        match test {
            StepTest::AnyChild => matches!(
                kind,
                NodeKind::Element
                    | NodeKind::Text
                    | NodeKind::Comment
                    | NodeKind::ProcessingInstruction
            ),
            StepTest::Kind(k)
            | StepTest::Named {
                kind: k,
                name: NameMatch::Any,
            } => *k == kind,
            StepTest::Named { .. } | StepTest::Never => false,
        }
    }

    /// Whether no node of `kind` matches the alternative, as its matching finds out
    /// without evaluating anything: the test of its last step, or `/`.
    pub(crate) fn excludes(&self, kind: NodeKind) -> bool {
        match self {
            Self::DocumentRoot | Self::RootFunction => kind != NodeKind::Document,
            Self::Path { steps, .. } => match steps.last() {
                None => true,
                Some(PathStep::Axis(step)) => !step.test.can_match(kind),
                Some(PathStep::Group { .. }) => false,
            },
        }
    }

    /// Whether the alternative is a lone relative step without predicates testing for
    /// nodes of `kind` named `{namespace}local`.
    pub(crate) fn is_lone_name_test(
        &self,
        kind: NodeKind,
        namespace: Option<&str>,
        local: &str,
    ) -> bool {
        let Self::Path {
            anchor: PatternAnchor::Relative,
            steps,
            ..
        } = self
        else {
            return false;
        };
        matches!(
            steps.as_slice(),
            [PathStep::Axis(CompiledStep {
                test: StepTest::Named {
                    kind: k,
                    name: NameMatch::Exact { namespace: ns, local: l },
                },
                predicates: Predicates::None,
            })] if *k == kind && ns.as_deref() == namespace && l == local
        )
    }

    /// The rule chains of Saxon that hold a rule with this pattern: one chain, except
    /// for `node()`, which Saxon splits by node kind (see [`Key::AnyChild`]).
    pub(crate) fn chain_keys(&self) -> Vec<Key> {
        match self {
            Self::RootFunction => vec![Key::Generic],
            Self::Path { steps, .. } if matches!(steps.last(), Some(PathStep::Group { .. })) => {
                let mut keys = Vec::new();
                self.keys(&mut keys);
                let element = |k: &Key| matches!(k, Key::Element(..) | Key::AnyElement);
                let attribute = |k: &Key| matches!(k, Key::Attribute(..) | Key::AnyAttribute);
                vec![if !keys.is_empty() && keys.iter().all(element) {
                    Key::AnyElement
                } else if !keys.is_empty() && keys.iter().all(attribute) {
                    Key::AnyAttribute
                } else if !keys.is_empty() && keys.iter().all(|k| *k == Key::Document) {
                    Key::Document
                } else {
                    Key::Generic
                }]
            }
            Self::DocumentRoot | Self::Path { .. } => {
                let mut keys = Vec::new();
                self.keys(&mut keys);
                keys
            }
        }
    }

    /// The keys under which the alternative must be indexed: a node can only match it if
    /// it falls under one of them.
    pub(crate) fn keys(&self, out: &mut Vec<Key>) {
        match self {
            Self::DocumentRoot | Self::RootFunction => out.push(Key::Document),
            Self::Path { anchor, steps, .. } => match steps.last() {
                None => {}
                Some(PathStep::Group { inner, .. }) => {
                    for alternative in inner {
                        alternative.keys(out);
                    }
                }
                Some(PathStep::Axis(last)) => match &last.test {
                    // Saxon splits a lone `node()` by node kind; any other pattern
                    // ending with `node()` can match nodes of several kinds.
                    StepTest::AnyChild => out.push(
                        if steps.len() == 1
                            && *anchor == PatternAnchor::Relative
                            && matches!(last.predicates, Predicates::None)
                        {
                            Key::AnyChild
                        } else {
                            Key::Generic
                        },
                    ),
                    StepTest::Kind(NodeKind::Text) => out.push(Key::Text),
                    StepTest::Kind(NodeKind::Comment) => out.push(Key::Comment),
                    StepTest::Kind(NodeKind::ProcessingInstruction) => {
                        out.push(Key::ProcessingInstruction);
                    }
                    StepTest::Kind(NodeKind::Attribute) => out.push(Key::AnyAttribute),
                    StepTest::Named { kind, name } => {
                        let attribute = *kind == NodeKind::Attribute;
                        out.push(match (name, attribute) {
                            (NameMatch::Exact { namespace, local }, false) => {
                                Key::Element(namespace.clone(), local.clone())
                            }
                            (NameMatch::Exact { namespace, local }, true) => {
                                Key::Attribute(namespace.clone(), local.clone())
                            }
                            (_, false) => Key::AnyElement,
                            (_, true) => Key::AnyAttribute,
                        });
                    }
                    StepTest::Kind(NodeKind::Document | NodeKind::Element) | StepTest::Never => {}
                },
            },
        }
    }
}

/// The one rule chain of Saxon-HE 12.10 that holds a rule for a whole union pattern
/// (`SimpleMode.addRule`, by the node kinds the union can match): the chain of that kind
/// when there is only one (for elements and attributes, the chain of any name), else
/// the generic chain. `None` when no alternative can match a node.
pub(crate) fn union_chain_key(alternatives: &[Alternative]) -> Option<Key> {
    const ELEMENT: u8 = 1;
    const ATTRIBUTE: u8 = 2;
    const DOCUMENT: u8 = 4;
    const TEXT: u8 = 8;
    const COMMENT: u8 = 16;
    const PROCESSING_INSTRUCTION: u8 = 32;
    const ANY: u8 = u8::MAX;
    let mut kinds = 0;
    for alternative in alternatives {
        if matches!(alternative, Alternative::RootFunction) {
            kinds |= ANY;
            continue;
        }
        let mut keys = Vec::new();
        alternative.keys(&mut keys);
        for key in &keys {
            kinds |= match key {
                Key::Element(..) | Key::AnyElement => ELEMENT,
                Key::Attribute(..) | Key::AnyAttribute => ATTRIBUTE,
                Key::Document => DOCUMENT,
                Key::Text => TEXT,
                Key::Comment => COMMENT,
                Key::ProcessingInstruction => PROCESSING_INSTRUCTION,
                Key::AnyChild => ELEMENT | TEXT | COMMENT | PROCESSING_INSTRUCTION,
                Key::Generic => ANY,
            };
        }
    }
    match kinds {
        0 => None,
        ELEMENT => Some(Key::AnyElement),
        ATTRIBUTE => Some(Key::AnyAttribute),
        DOCUMENT => Some(Key::Document),
        TEXT => Some(Key::Text),
        COMMENT => Some(Key::Comment),
        PROCESSING_INSTRUCTION => Some(Key::ProcessingInstruction),
        _ => Some(Key::Generic),
    }
}

fn is_fn(name: &ast::QName, local: &str) -> bool {
    name.local == local && name.namespace.as_deref() == Some(ast::FN_NAMESPACE)
}

/// Whether `last()` or `position()` is called anywhere in `expr`.
fn calls_focus(expr: &ast::Expr) -> bool {
    use ast::Expr as E;
    let steps = |path: &PathExpr| {
        path.steps.iter().any(|s| match s {
            Step::Axis(a) => a.predicates.iter().any(calls_focus),
            Step::Expr(e) => calls_focus(e),
        })
    };
    match expr {
        E::Literal(_) | E::VarRef(_) | E::ContextItem => false,
        E::FunctionCall { name, args } => {
            is_fn(name, "last") || is_fn(name, "position") || args.iter().any(calls_focus)
        }
        E::Sequence(items) => items.iter().any(calls_focus),
        E::Filter { base, predicates } => calls_focus(base) || predicates.iter().any(calls_focus),
        E::Path(path) => steps(path),
        E::Negate(e)
        | E::Castable { operand: e, .. }
        | E::Cast { operand: e, .. }
        | E::InstanceOf { operand: e, .. } => calls_focus(e),
        E::Union(a, b)
        | E::Arithmetic { lhs: a, rhs: b, .. }
        | E::Range(a, b)
        | E::Comparison { lhs: a, rhs: b, .. }
        | E::And(a, b)
        | E::Or(a, b) => calls_focus(a) || calls_focus(b),
        E::If {
            condition,
            then,
            otherwise,
        } => calls_focus(condition) || calls_focus(then) || calls_focus(otherwise),
        E::For { bindings, body } => {
            bindings.iter().any(|b| calls_focus(&b.sequence)) || calls_focus(body)
        }
        E::Quantified {
            bindings,
            satisfies,
            ..
        } => bindings.iter().any(|b| calls_focus(&b.sequence)) || calls_focus(satisfies),
    }
}

/// Functions whose result is never numeric.
const NON_NUMERIC_FUNCTIONS: &[&str] = &[
    "not",
    "true",
    "false",
    "boolean",
    "exists",
    "empty",
    "contains",
    "starts-with",
    "ends-with",
    "matches",
    "string",
    "normalize-space",
    "concat",
    "substring",
    "substring-before",
    "substring-after",
    "translate",
    "upper-case",
    "lower-case",
    "name",
    "local-name",
    "namespace-uri",
    "string-join",
    "replace",
    "tokenize",
    "root",
    "current",
];

/// Whether the value of `expr` can never be a numeric value.
fn never_numeric(expr: &ast::Expr) -> bool {
    use ast::Expr as E;
    match expr {
        E::Literal(literal) => matches!(literal, ast::Literal::String(_)),
        E::ContextItem
        | E::Comparison { .. }
        | E::And(..)
        | E::Or(..)
        | E::Quantified { .. }
        | E::InstanceOf { .. }
        | E::Castable { .. }
        | E::Union(..) => true,
        E::Path(path) => match path.steps.last() {
            None | Some(Step::Axis(_)) => true,
            Some(Step::Expr(e)) => never_numeric(e),
        },
        E::Filter { base, .. } => never_numeric(base),
        E::Cast { target, .. } => !matches!(
            target.atomic.kind,
            ast::AtomicKind::Integer | ast::AtomicKind::Decimal
        ),
        E::Sequence(items) => items.iter().all(never_numeric),
        E::If {
            then, otherwise, ..
        } => never_numeric(then) && never_numeric(otherwise),
        E::FunctionCall { name, .. } => {
            name.namespace.as_deref() == Some(ast::FN_NAMESPACE)
                && NON_NUMERIC_FUNCTIONS.contains(&name.local.as_str())
        }
        E::VarRef(_) | E::Negate(_) | E::Arithmetic { .. } | E::Range(..) | E::For { .. } => false,
    }
}

/// Compiles one alternative. Constructs that are refused are returned as errors.
pub(crate) fn compile_alternative(
    alternative: &PatternAlternative,
    context: &dyn StaticContext,
) -> std::result::Result<Alternative, String> {
    Ok(match alternative {
        PatternAlternative::DocumentRoot => Alternative::DocumentRoot,
        PatternAlternative::RootFunction => Alternative::RootFunction,
        PatternAlternative::Path(path) => {
            let mut steps = Vec::with_capacity(path.rest.len() + 1);
            let mut links = Vec::with_capacity(path.rest.len());
            steps.push(compile_path_step(&path.first, context)?);
            for (link, step) in &path.rest {
                links.push(*link);
                steps.push(compile_path_step(step, context)?);
            }
            Alternative::Path {
                anchor: path.anchor,
                steps,
                links,
            }
        }
    })
}

fn compile_path_step(
    step: &ast::PathStep,
    context: &dyn StaticContext,
) -> std::result::Result<PathStep, String> {
    match step {
        ast::PathStep::Axis(step) => compile_step(step, context).map(PathStep::Axis),
        ast::PathStep::Parenthesized { inner, predicates } => {
            let compiled = compile_predicates(predicates, context)?;
            if let Some((p, _)) = predicates
                .iter()
                .zip(&compiled)
                .find(|(p, c)| is_positional_compiled(p, c))
            {
                return Err(format!(
                    "the predicate [{p}] of a parenthesized pattern may depend on the position, which is not supported"
                ));
            }
            let inner = inner
                .alternatives
                .iter()
                .map(|a| compile_alternative(a, context))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(PathStep::Group {
                inner,
                predicates: compiled,
            })
        }
    }
}

fn compile_step(
    step: &PatternStep,
    context: &dyn StaticContext,
) -> std::result::Result<CompiledStep, String> {
    let test = step_test(step.axis, &step.test);
    if step.predicates.is_empty() {
        return Ok(CompiledStep {
            test,
            predicates: Predicates::None,
        });
    }
    let compiled = compile_predicates(&step.predicates, context)?;
    let positional = step
        .predicates
        .iter()
        .zip(&compiled)
        .any(|(p, c)| is_positional_compiled(p, c));
    let predicates = if positional {
        let axis_step = ast::Expr::Path(PathExpr {
            start: PathStart::Relative,
            steps: vec![Step::Axis(AxisStep {
                axis: match step.axis {
                    PatternAxis::Child => Axis::Child,
                    PatternAxis::Attribute => Axis::Attribute,
                },
                test: step.test.clone(),
                predicates: step.predicates.clone(),
            })],
        });
        Predicates::FromParent(compile(&axis_step, context).map_err(|e| e.to_string())?)
    } else {
        Predicates::Boolean(compiled)
    };
    Ok(CompiledStep { test, predicates })
}

fn compile_predicates(
    predicates: &[ast::Expr],
    context: &dyn StaticContext,
) -> std::result::Result<Vec<CompiledExpr>, String> {
    predicates
        .iter()
        .map(|p| compile(p, context).map_err(|e| e.to_string()))
        .collect()
}

/// Whether the value of a predicate can depend on the position of the node: it calls
/// `position()` or `last()`, or its value may be a number, as far as its form and its
/// static type tell (a reference to a boolean variable is not positional). Conservative:
/// `true` unless proved otherwise.
fn is_positional_compiled(predicate: &ast::Expr, compiled: &CompiledExpr) -> bool {
    calls_focus(predicate) || (!never_numeric(predicate) && !compiled.is_never_numeric())
}

/// Matching, given the dynamic context. Errors are the evaluator's, as raised: the
/// caller converts them (an error a host callback raised, such as a global variable
/// evaluated in a predicate, is given back by the runtime) and decides whether to
/// recover from them.
pub(crate) struct Matcher<'d, 'h> {
    pub(crate) dc: &'d DynamicContext<'h>,
}

impl Matcher<'_, '_> {
    /// Whether `node` matches `alternative`.
    pub(crate) fn matches(&self, alternative: &Alternative, node: &Node) -> Result<bool> {
        self.matches_for(alternative, node, node)
    }

    /// Whether `node` matches `alternative`, `matched` being the node matched against the
    /// whole pattern (`current()`).
    fn matches_for(&self, alternative: &Alternative, node: &Node, matched: &Node) -> Result<bool> {
        match alternative {
            Alternative::DocumentRoot | Alternative::RootFunction => {
                Ok(node.kind() == NodeKind::Document)
            }
            Alternative::Path {
                anchor,
                steps,
                links,
            } => {
                let Some(last) = steps.len().checked_sub(1) else {
                    return Ok(false);
                };
                self.match_step(steps, links, *anchor, last, node, matched)
            }
        }
    }

    /// Whether `node` matches `steps[index]` and what precedes it. `matched` is the node
    /// being matched against the whole pattern (`current()`).
    fn match_step(
        &self,
        steps: &[PathStep],
        links: &[PatternLink],
        anchor: PatternAnchor,
        index: usize,
        node: &Node,
        matched: &Node,
    ) -> Result<bool> {
        let holds = match &steps[index] {
            PathStep::Axis(step) => {
                step.test.matches(node) && self.predicates_hold(step, node, matched)?
            }
            PathStep::Group { inner, predicates } => {
                self.group_matches(inner, predicates, node, matched)?
            }
        };
        if !holds {
            return Ok(false);
        }
        let Some(previous) = index.checked_sub(1) else {
            return Ok(match anchor {
                PatternAnchor::Relative => true,
                PatternAnchor::Root => node
                    .parent()
                    .is_some_and(|p| p.kind() == NodeKind::Document),
                PatternAnchor::RootDescendant => node.root().kind() == NodeKind::Document,
            });
        };
        match links[previous] {
            PatternLink::Child => match node.parent() {
                Some(parent) => self.match_step(steps, links, anchor, previous, &parent, matched),
                None => Ok(false),
            },
            PatternLink::Descendant => {
                let mut ancestor = node.parent();
                while let Some(a) = ancestor {
                    if self.match_step(steps, links, anchor, previous, &a, matched)? {
                        return Ok(true);
                    }
                    ancestor = a.parent();
                }
                Ok(false)
            }
        }
    }

    /// Whether `node` matches a parenthesized step: one of its alternatives, then its
    /// predicates, with `node` as context item.
    fn group_matches(
        &self,
        inner: &[Alternative],
        predicates: &[CompiledExpr],
        node: &Node,
        matched: &Node,
    ) -> Result<bool> {
        let mut any = false;
        for alternative in inner {
            if self.matches_for(alternative, node, matched)? {
                any = true;
                break;
            }
        }
        if !any {
            return Ok(false);
        }
        let context = Context {
            focus: Some(Focus {
                item: Item::Node(node.clone()),
                position: 1,
                size: 1,
            }),
            current: Some(Item::Node(matched.clone())),
        };
        for predicate in predicates {
            if !predicate.effective_boolean_value(self.dc, &context)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn predicates_hold(&self, step: &CompiledStep, node: &Node, matched: &Node) -> Result<bool> {
        match &step.predicates {
            Predicates::None => Ok(true),
            Predicates::Boolean(predicates) => {
                let context = Context {
                    focus: Some(Focus {
                        item: Item::Node(node.clone()),
                        position: 1,
                        size: 1,
                    }),
                    current: Some(Item::Node(matched.clone())),
                };
                for predicate in predicates {
                    if !predicate.effective_boolean_value(self.dc, &context)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Predicates::FromParent(step_expr) => {
                let Some(parent) = node.parent() else {
                    return Ok(false);
                };
                let context = Context {
                    focus: Some(Focus {
                        item: Item::Node(parent),
                        position: 1,
                        size: 1,
                    }),
                    current: Some(Item::Node(matched.clone())),
                };
                Ok(step_expr
                    .evaluate(self.dc, &context)?
                    .iter()
                    .any(|item| item.as_node() == Some(node)))
            }
        }
    }
}
