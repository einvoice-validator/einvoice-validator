//! Preparation: a loaded [`Stylesheet`] becomes an [`Executable`], once, to be run on any
//! number of documents. Every XPath expression is compiled against its static context
//! (global variables, the local variables in scope, the stylesheet functions), every
//! variable gets a slot in the frame of its template, function or global variable,
//! named templates and modes are resolved, and the template rules of each mode are
//! ranked and indexed by the kind and name of the nodes they can match.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use einvoice_xpath::{
    self as ast, Avt, AvtPart, Expr, NamespaceMap, Occurrence, QName, SequenceType, parse_expr,
};
use einvoice_xpath_eval::{
    BindingValue, Cardinality, CompiledExpr, DocumentCache, FunctionId, RegexCache, StaticContext,
    ValueType, VariableId, VariableInfo, compile,
};
use einvoice_xslt::{
    ApplyMode, ApplyTemplates, Attribute, CallTemplate, Choose, Comment, Copy, CopyOf, Element,
    ForEach, GlobalKind, If, Instruction, LiteralElement, Message, Mode, NamespaceBinding,
    NamespaceNode, NextMatch, Number, NumberLevel, Priority, ProcessingInstruction, Sequence,
    Standalone, Stylesheet, Template, TemplateModes, ValueOf, Variable, VariableValue,
};

use crate::error::{PrepareError, PrepareErrors};
use crate::pattern::{self, Alternative, Key};
use crate::sink::Serialization;

/// A stylesheet ready to run: prepared once, run on any number of documents with
/// [`Executable::transform`](crate::Executable::transform).
#[derive(Debug)]
pub struct Executable {
    pub(crate) globals: Vec<Global>,
    pub(crate) templates: Vec<CompiledTemplate>,
    pub(crate) functions: Vec<CompiledFunction>,
    pub(crate) modes: Vec<CompiledMode>,
    pub(crate) default_mode: usize,
    pub(crate) serialization: Serialization,
    /// The static base URI, against which `document()` resolves relative URIs given as
    /// strings.
    pub(crate) base_uri: Option<String>,
    /// Whether some expression calls `document()`, which may load the source document
    /// again from its text: a transformation given its source as an owned `String` then
    /// keeps the text (see [`Executable::transform`](crate::Executable::transform)).
    pub(crate) calls_document: bool,
    /// How many bindings are evaluated as [`Eval::Learning`].
    pub(crate) learners: usize,
    /// The documents `document()` loaded through the resolver, parsed once for every
    /// transformation that loads the same text again.
    pub(crate) loaded: DocumentCache,
    /// The regular expressions compiled at evaluation, for every transformation.
    pub(crate) regexes: RegexCache,
    /// The names of elements and attributes that have rules of their own in some mode,
    /// numbered ([`CompiledMode::named_by_number`]).
    pub(crate) rule_names: RuleNames,
}

/// Numbers of names, by local name and namespace.
pub(crate) type RuleNames = HashMap<String, Vec<(Option<String>, u32)>>;

/// The number of an expanded name in `names`, if it has one.
pub(crate) fn rule_name_number(names: &RuleNames, name: &QName) -> Option<u32> {
    names.get(name.local.as_str()).and_then(|by_ns| {
        by_ns
            .iter()
            .find(|(ns, _)| ns.as_deref() == name.namespace.as_deref())
            .map(|(_, number)| *number)
    })
}

/// The template rules of each mode, and the numbers of the names they have rules for.
fn compile_modes(
    sheet: &Stylesheet,
    templates: &[CompiledTemplate],
    modes: &[Mode],
) -> (Vec<CompiledMode>, RuleNames) {
    let mut compiled: Vec<CompiledMode> = modes
        .iter()
        .map(|mode| rank_rules(sheet, templates, mode))
        .collect();
    let names = number_rule_names(&mut compiled);
    (compiled, names)
}

/// Numbers the names of elements and attributes that have rules of their own in the
/// modes, and gives each mode its chains of those names by number.
fn number_rule_names(modes: &mut [CompiledMode]) -> RuleNames {
    let mut names = RuleNames::new();
    let mut count = 0_u32;
    for mode in modes.iter_mut() {
        for attribute in [false, true] {
            let map = if attribute {
                &mode.attributes
            } else {
                &mode.elements
            };
            let mut numbered: Vec<(u32, Vec<u32>)> = Vec::new();
            for (local, by_ns) in map {
                for (namespace, ranks) in by_ns {
                    let entry = names.entry(local.clone()).or_default();
                    let number =
                        if let Some((_, number)) = entry.iter().find(|(ns, _)| ns == namespace) {
                            *number
                        } else {
                            entry.push((namespace.clone(), count));
                            count += 1;
                            count - 1
                        };
                    numbered.push((number, ranks.clone()));
                }
            }
            numbered.sort_by_key(|(number, _)| *number);
            if attribute {
                mode.attributes_by_number = numbered;
            } else {
                mode.elements_by_number = numbered;
            }
        }
    }
    names
}

/// A global variable or parameter.
#[derive(Debug)]
pub(crate) struct Global {
    pub(crate) name: QName,
    pub(crate) is_param: bool,
    pub(crate) line: u32,
    pub(crate) value: Value,
    pub(crate) as_type: Option<SequenceType>,
    /// Slots of the variables local to its sequence constructor.
    pub(crate) slots: usize,
}

/// How the value of a binding is computed.
#[derive(Debug)]
pub(crate) enum Value {
    Select(Box<CompiledExpr>),
    Content(Vec<Inst>),
    Absent,
}

/// A local variable, a template or function parameter: a slot of the frame.
#[derive(Debug)]
pub(crate) struct Binding {
    pub(crate) line: u32,
    pub(crate) name: QName,
    pub(crate) slot: usize,
    pub(crate) as_type: Option<SequenceType>,
    pub(crate) value: Value,
    /// When its value is computed.
    pub(crate) eval: Eval,
    /// Whether some expression in its scope refers to it (an unused variable is never
    /// evaluated, as Saxon removes it).
    pub(crate) used: bool,
}

/// When the value of a binding is computed, as Saxon-HE 12.10 computes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Eval {
    /// Where it is bound.
    Eager,
    /// When first referenced: a local variable referred to once, not in a loop, which
    /// Saxon inlines into the reference.
    Lazy,
    /// Saxon's `LearningEvaluator`: when first referenced, until the binding has been
    /// evaluated 20 times, its value each time computed (or, in a function body, 20
    /// times at all); from then on where it is bound. Only the first 40 evaluations
    /// count. `learner` indexes its counters, kept per transformation.
    Learning {
        learner: usize,
        /// In an `xsl:function` body, where every evaluation counts, read or not.
        every_counts: bool,
    },
}

/// `xsl:with-param`.
#[derive(Debug)]
pub(crate) struct WithParam {
    pub(crate) name: QName,
    pub(crate) as_type: Option<SequenceType>,
    pub(crate) value: Value,
    pub(crate) eval: Eval,
}

#[derive(Debug)]
pub(crate) struct CompiledTemplate {
    pub(crate) line: u32,
    pub(crate) params: Vec<Binding>,
    pub(crate) body: Vec<Inst>,
    pub(crate) slots: usize,
    /// The compiled alternatives of the match pattern (empty without one).
    pub(crate) pattern: Vec<Alternative>,
    /// The match pattern as written, and each of its alternatives, for warnings.
    pub(crate) pattern_text: String,
    pub(crate) alternative_texts: Vec<String>,
    /// Whether the body can end with an `xsl:call-template` in tail position (its last
    /// instruction, or the last of a branch of an `xsl:choose` or `xsl:if` that is): run
    /// as a tail call ([`has_tail_call`]).
    pub(crate) tail_calls: bool,
}

/// Whether a sequence constructor can end with an `xsl:call-template` in tail position:
/// its last instruction is one, or is an `xsl:choose` (or `xsl:if`) with a branch that
/// can.
fn has_tail_call(body: &[Inst]) -> bool {
    match body.last() {
        Some(Inst::CallTemplate(_)) => true,
        Some(Inst::Choose(CompiledChoose { whens, otherwise })) => {
            whens.iter().any(|w| has_tail_call(&w.body)) || has_tail_call(otherwise)
        }
        _ => false,
    }
}

#[derive(Debug)]
pub(crate) struct CompiledFunction {
    pub(crate) line: u32,
    pub(crate) name: QName,
    pub(crate) params: Vec<Binding>,
    pub(crate) as_type: Option<SequenceType>,
    pub(crate) body: Vec<Inst>,
    pub(crate) slots: usize,
}

/// The mode of `xsl:apply-templates`.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ModeRef {
    Mode(usize),
    Current,
}

/// An attribute value template.
#[derive(Debug)]
pub(crate) struct CompiledAvt(pub(crate) Vec<AvtItem>);

#[derive(Debug)]
pub(crate) enum AvtItem {
    Text(Box<str>),
    Expr(Box<CompiledExpr>),
}

/// An instruction of a sequence constructor.
#[derive(Debug)]
pub(crate) enum Inst {
    // The instructions on the path of a recursion through the templates hold their parts
    // in a struct of their own, which `Runtime::instruction` passes whole to the function
    // that runs it: a debug build keeps every binding of every arm of a match on the
    // stack, and that function is part of every level of the recursion.
    ApplyTemplates(CompiledApplyTemplates),
    NextMatch(CompiledNextMatch),
    CallTemplate(CompiledCallTemplate),
    /// `xsl:choose`, and `xsl:if` as a choose with one branch.
    Choose(CompiledChoose),
    ForEach(CompiledForEach),
    Variable(Box<Binding>),
    Sequence {
        line: u32,
        select: Box<CompiledExpr>,
    },
    ValueOf {
        line: u32,
        select: Option<Box<CompiledExpr>>,
        separator: Option<CompiledAvt>,
        body: Vec<Inst>,
    },
    Attribute {
        line: u32,
        name: CompiledAvt,
        namespace: Option<CompiledAvt>,
        namespaces: Arc<[NamespaceBinding]>,
        select: Option<Box<CompiledExpr>>,
        body: Vec<Inst>,
    },
    Comment {
        line: u32,
        body: Vec<Inst>,
    },
    Text(Box<str>),
    Number {
        line: u32,
        level: NumberLevel,
        /// `None`: nodes of the context node's kind and name.
        count: Option<CountPattern>,
    },
    /// A literal result element.
    Element(CompiledElement),
    /// `xsl:copy`.
    Copy {
        line: u32,
        body: Vec<Inst>,
    },
    /// `xsl:copy-of`.
    CopyOf {
        line: u32,
        select: Box<CompiledExpr>,
    },
    /// `xsl:element`.
    ComputedElement {
        line: u32,
        name: CompiledAvt,
        namespace: Option<CompiledAvt>,
        namespaces: Arc<[NamespaceBinding]>,
        body: Vec<Inst>,
    },
    /// `xsl:namespace`.
    Namespace {
        line: u32,
        name: CompiledAvt,
        select: Option<Box<CompiledExpr>>,
        body: Vec<Inst>,
    },
    /// `xsl:processing-instruction`.
    ProcessingInstruction {
        line: u32,
        name: CompiledAvt,
        select: Option<Box<CompiledExpr>>,
        body: Vec<Inst>,
    },
    /// `xsl:message`.
    Message {
        line: u32,
        terminate: Option<CompiledAvt>,
        body: Vec<Inst>,
    },
}

/// `xsl:apply-templates`.
#[derive(Debug)]
pub(crate) struct CompiledApplyTemplates {
    pub(crate) line: u32,
    pub(crate) select: Box<CompiledExpr>,
    pub(crate) mode: ModeRef,
    pub(crate) params: Vec<WithParam>,
}

/// `xsl:next-match`.
#[derive(Debug)]
pub(crate) struct CompiledNextMatch {
    pub(crate) line: u32,
    pub(crate) params: Vec<WithParam>,
}

/// `xsl:call-template`.
#[derive(Debug)]
pub(crate) struct CompiledCallTemplate {
    pub(crate) line: u32,
    pub(crate) template: usize,
    pub(crate) params: Vec<WithParam>,
}

/// `xsl:choose`, and `xsl:if` as a choose with one branch.
#[derive(Debug)]
pub(crate) struct CompiledChoose {
    pub(crate) whens: Vec<When>,
    pub(crate) otherwise: Vec<Inst>,
}

/// `xsl:for-each`.
#[derive(Debug)]
pub(crate) struct CompiledForEach {
    pub(crate) line: u32,
    pub(crate) select: Box<CompiledExpr>,
    pub(crate) body: Vec<Inst>,
}

/// A literal result element.
#[derive(Debug)]
pub(crate) struct CompiledElement {
    pub(crate) line: u32,
    pub(crate) name: Arc<QName>,
    pub(crate) namespaces: Arc<[NamespaceBinding]>,
    pub(crate) attributes: Vec<(Arc<QName>, CompiledAvt)>,
    pub(crate) body: Vec<Inst>,
}

/// A branch of `xsl:choose` (`xsl:when`), or `xsl:if`: an error in its test is
/// attributed to its own line, as Saxon attributes it.
#[derive(Debug)]
pub(crate) struct When {
    pub(crate) line: u32,
    pub(crate) test: Box<CompiledExpr>,
    pub(crate) body: Vec<Inst>,
}

/// The `count` pattern of `xsl:number`, and its text for warnings.
#[derive(Debug)]
pub(crate) struct CountPattern {
    pub(crate) alternatives: Vec<Alternative>,
    pub(crate) text: String,
}

/// A template rule as Saxon-HE 12.10 registers it: one per alternative of a match
/// pattern without explicit priority (`A | B` is split into rules, XSLT 2.0 section
/// 6.4), one for the whole pattern with an explicit priority.
#[derive(Debug)]
pub(crate) struct Rule {
    pub(crate) template: usize,
    /// The alternative of the template's pattern; `None` for the whole pattern.
    pub(crate) alternative: Option<usize>,
    /// The import precedence of the template's module, compared before the priority.
    pub(crate) precedence: u32,
    /// The priority, compared as Saxon compares it: as a double.
    pub(crate) priority: f64,
    /// Declaration order: the template's position.
    pub(crate) sequence: usize,
    /// The position of the alternative among the rules split from the same template
    /// without explicit priority; 0 for a pattern with an explicit priority (Saxon's
    /// `part`, which keeps `xsl:next-match` from running the same template twice).
    pub(crate) part: usize,
    /// The node kinds ([`kind_index`], one bit each) whose every node the rule is tried
    /// on matches its pattern, which is then matched without evaluating anything: the
    /// first alternative tried that can match the kind, as its test tells, is `text()`,
    /// `node()`, `*` or `@*` alone without predicates, or `cac:X` in the chain of that
    /// name ([`certain_kinds`]).
    pub(crate) certain: u8,
}

impl Rule {
    /// The rank of the rule against another, as Saxon ranks rules: import precedence,
    /// then priority (declaration order and part tell the rules apart beyond it).
    pub(crate) fn compare_rank(&self, other: &Self) -> std::cmp::Ordering {
        self.precedence
            .cmp(&other.precedence)
            .then(self.priority.total_cmp(&other.priority))
    }
}

/// The [`Rule::certain`] kinds of a rule whose pattern's alternatives are tried in the
/// order of `alternatives` and that is held by the chains of `keys`.
fn certain_kinds(alternatives: &[pattern::Alternative], keys: &[pattern::Key]) -> u8 {
    use einvoice_xpath_eval::NodeKind as K;
    let mut kinds = 0;
    for kind in [
        K::Document,
        K::Element,
        K::Attribute,
        K::Text,
        K::Comment,
        K::ProcessingInstruction,
    ] {
        // A rule held only by the chains of one name (a lone name test) is tried on nodes
        // of that name only.
        let by_name = !keys.is_empty()
            && keys.iter().all(|key| match key {
                pattern::Key::Element(ns, local) => {
                    kind == K::Element
                        && alternatives
                            .first()
                            .is_some_and(|a| a.is_lone_name_test(K::Element, ns.as_deref(), local))
                }
                pattern::Key::Attribute(ns, local) => {
                    kind == K::Attribute
                        && alternatives.first().is_some_and(|a| {
                            a.is_lone_name_test(K::Attribute, ns.as_deref(), local)
                        })
                }
                _ => false,
            });
        let first = alternatives.iter().find(|a| !a.excludes(kind));
        if by_name || first.is_some_and(|a| a.matches_every_node_of(kind)) {
            kinds |= 1 << kind_index(kind);
        }
    }
    kinds
}

/// Rule ranks of elements or attributes by local name, then namespace.
type ByName = HashMap<String, Vec<(Option<String>, Vec<u32>)>>;

/// The rules of a mode and Saxon's rule chains: the ranks of the rules that can match
/// nodes of one name (elements, attributes), of one kind, or of any kind, each chain in
/// rank order.
#[derive(Debug, Default)]
pub(crate) struct CompiledMode {
    pub(crate) name: String,
    /// In rank order: import precedence descending, priority descending, then
    /// declaration order descending, then part descending (Saxon puts a rule added later
    /// before the earlier ones of the same priority).
    pub(crate) rules: Vec<Rule>,
    elements: ByName,
    attributes: ByName,
    any_element: Vec<u32>,
    any_attribute: Vec<u32>,
    text: Vec<u32>,
    comment: Vec<u32>,
    processing_instruction: Vec<u32>,
    document: Vec<u32>,
    generic: Vec<u32>,
    /// What the rule search gives for a node of each kind ([`kind_index`]) whose name has
    /// no rule of its own, when it is known without matching any pattern.
    plans: [Plan; 6],
    /// The chains of `elements` and `attributes` by the number of their name in
    /// [`Executable::rule_names`], in number order.
    elements_by_number: Vec<(u32, Vec<u32>)>,
    attributes_by_number: Vec<(u32, Vec<u32>)>,
}

/// The outcome of the rule search ([`Runtime::find_rule`](crate::run)) for a node of a
/// kind, when the chain of its name is empty and no rule is current
/// (`xsl:apply-templates`, not `xsl:next-match`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Plan {
    /// Unknown without matching patterns: search the chains.
    #[default]
    Search,
    /// No rule can match: the built-in rule.
    Builtin,
    /// This rule matches every node of the kind, without evaluating anything, and every
    /// other rule that could match has a lower priority.
    Rule(u32),
}

/// The index of a node kind in [`CompiledMode`]'s plans.
pub(crate) const fn kind_index(kind: einvoice_xpath_eval::NodeKind) -> usize {
    use einvoice_xpath_eval::NodeKind as K;
    match kind {
        K::Document => 0,
        K::Element => 1,
        K::Attribute => 2,
        K::Text => 3,
        K::Comment => 4,
        K::ProcessingInstruction => 5,
    }
}

const EMPTY: &[u32] = &[];

impl CompiledMode {
    /// The chain of rules for elements (or attributes, `attribute`) of a name.
    pub(crate) fn named_chain(&self, attribute: bool, name: Option<&QName>) -> &[u32] {
        Self::named(
            if attribute {
                &self.attributes
            } else {
                &self.elements
            },
            name,
        )
    }

    /// The chain of rules for elements (or attributes, `attribute`) whose name has the
    /// number `number` in [`Executable::rule_names`].
    pub(crate) fn named_by_number(&self, attribute: bool, number: u32) -> &[u32] {
        let numbered = if attribute {
            &self.attributes_by_number
        } else {
            &self.elements_by_number
        };
        numbered
            .binary_search_by_key(&number, |(n, _)| *n)
            .map_or(EMPTY, |i| numbered[i].1.as_slice())
    }

    /// The rule search's outcome for a node of `kind` without rules for its name.
    pub(crate) const fn plan(&self, kind: einvoice_xpath_eval::NodeKind) -> Plan {
        self.plans[kind_index(kind)]
    }

    /// The chains Saxon searches for a node of that kind, `named` being the chain of its
    /// name ([`CompiledMode::named_chain`], empty for other kinds), in its order: the
    /// chain of the name, the chain of the kind, the generic chain.
    pub(crate) fn chains_with<'a>(
        &'a self,
        kind: einvoice_xpath_eval::NodeKind,
        named: &'a [u32],
    ) -> [&'a [u32]; 3] {
        [named, self.kind_chain(kind), &self.generic]
    }

    fn kind_chain(&self, kind: einvoice_xpath_eval::NodeKind) -> &[u32] {
        use einvoice_xpath_eval::NodeKind as K;
        match kind {
            K::Element => &self.any_element,
            K::Attribute => &self.any_attribute,
            K::Text => &self.text,
            K::Comment => &self.comment,
            K::ProcessingInstruction => &self.processing_instruction,
            K::Document => &self.document,
        }
    }

    /// The plans of the node kinds ([`Plan`]), once the rules are indexed: the search
    /// [`Runtime::find_rule`](crate::run) does, over the chain of the kind and the
    /// generic chain, when its outcome needs no pattern to be matched.
    fn plan_kinds(&mut self) {
        use einvoice_xpath_eval::NodeKind as K;
        for kind in [
            K::Document,
            K::Element,
            K::Attribute,
            K::Text,
            K::Comment,
            K::ProcessingInstruction,
        ] {
            let mut ranks = self.kind_chain(kind).iter().chain(&self.generic).copied();
            let plan = match ranks.next() {
                None => Plan::Builtin,
                Some(first) => {
                    let rule = &self.rules[first as usize];
                    let unconditional = rule.certain & (1 << kind_index(kind)) != 0;
                    if unconditional
                        && ranks.all(|r| {
                            self.rules[r as usize].compare_rank(rule) == std::cmp::Ordering::Less
                        })
                    {
                        Plan::Rule(first)
                    } else {
                        Plan::Search
                    }
                }
            };
            self.plans[kind_index(kind)] = plan;
        }
    }

    fn named<'a>(map: &'a ByName, name: Option<&QName>) -> &'a [u32] {
        let Some(name) = name else {
            return EMPTY;
        };
        map.get(name.local.as_str())
            .and_then(|by_ns| {
                by_ns
                    .iter()
                    .find(|(ns, _)| ns.as_deref() == name.namespace.as_deref())
            })
            .map_or(EMPTY, |(_, ranks)| ranks.as_slice())
    }

    fn index(&mut self, rank: u32, key: &Key) {
        let push_named = |map: &mut ByName, ns: &Option<String>, local: &str| {
            let by_ns = map.entry(local.to_owned()).or_default();
            match by_ns.iter_mut().find(|(n, _)| n == ns) {
                Some((_, ranks)) => ranks.push(rank),
                None => by_ns.push((ns.clone(), vec![rank])),
            }
        };
        match key {
            Key::Document => self.document.push(rank),
            Key::Element(ns, local) => push_named(&mut self.elements, ns, local),
            Key::Attribute(ns, local) => push_named(&mut self.attributes, ns, local),
            Key::AnyElement => self.any_element.push(rank),
            Key::AnyAttribute => self.any_attribute.push(rank),
            Key::AnyChild => {
                self.any_element.push(rank);
                self.text.push(rank);
                self.comment.push(rank);
                self.processing_instruction.push(rank);
            }
            Key::Text => self.text.push(rank),
            Key::Comment => self.comment.push(rank),
            Key::ProcessingInstruction => self.processing_instruction.push(rank),
            Key::Generic => self.generic.push(rank),
        }
    }
}

/// The local variables of one frame (a template, a function or a global variable)
/// during compilation.
#[derive(Debug, Default)]
struct Frame {
    slots: usize,
    /// In scope, innermost last.
    locals: Vec<(QName, VariableInfo)>,
    /// References seen, per slot.
    uses: RefCell<Vec<u32>>,
    /// Per slot, whether a reference is in a loop: an operand of an XPath expression
    /// evaluated repeatedly, or an `xsl:for-each` body the variable is declared outside.
    looping: RefCell<Vec<bool>>,
    /// Per slot, the number of `xsl:for-each` bodies around its declaration.
    depths: Vec<u32>,
    /// The number of `xsl:for-each` bodies around the instruction being compiled.
    depth: u32,
    /// The local variables whose value is being compiled, innermost last.
    enclosing: Vec<usize>,
    /// Every reference to a local variable: its slot, and the variables whose value holds
    /// it (`enclosing` then).
    references: RefCell<Vec<(usize, Vec<usize>)>>,
    /// Per slot, whether Saxon removes the variable: unused, or inlined into its one
    /// reference (the references in its value are then gone too).
    gone: Vec<bool>,
    /// Per slot, the variables whose value holds its declaration (`enclosing` then).
    declared_in: Vec<Vec<usize>>,
}

impl Frame {
    /// A new slot, not in scope yet ([`Frame::bind`]).
    fn reserve(&mut self) -> usize {
        let slot = self.slots;
        self.slots += 1;
        self.uses.borrow_mut().push(0);
        self.looping.borrow_mut().push(false);
        self.depths.push(self.depth);
        self.gone.push(false);
        self.declared_in.push(self.enclosing.clone());
        slot
    }

    /// Whether a reference to `target` held in the values of the variables `enclosing` is
    /// in the value of one for which `removed` holds, among those declared after `target`
    /// (a variable whose value holds the declaration of `target` holds it whole).
    fn removed_with(
        &self,
        target: usize,
        enclosing: &[usize],
        removed: &dyn Fn(usize) -> bool,
    ) -> bool {
        let declared_in = self.declared_in.get(target);
        enclosing
            .iter()
            .any(|e| !declared_in.is_some_and(|d| d.contains(e)) && removed(*e))
    }

    fn declare(
        &mut self,
        name: QName,
        cardinality: Cardinality,
        value_type: ValueType,
        globals: usize,
    ) -> usize {
        let slot = self.reserve();
        self.bind(slot, name, cardinality, value_type, globals);
        slot
    }

    /// Puts a reserved slot in scope under `name`.
    fn bind(
        &mut self,
        slot: usize,
        name: QName,
        cardinality: Cardinality,
        value_type: ValueType,
        globals: usize,
    ) {
        let id = VariableId(u32::try_from(globals + slot).unwrap_or(u32::MAX));
        self.locals.push((
            name,
            VariableInfo {
                id,
                cardinality,
                value_type,
            },
        ));
    }
}

/// The static context of an expression.
struct Scope<'a> {
    globals: &'a [(QName, VariableInfo)],
    /// How many globals Saxon has typed before the declaration being compiled (those
    /// declared before it): a reference to a later one is of unknown type for the type
    /// inference of the variables declared here.
    typed_globals: usize,
    functions: &'a [(QName, usize)],
    /// The declared result types of the functions.
    function_types: &'a [ValueType],
    frame: Option<&'a Frame>,
    /// The URI of the module of the declaration: the static base URI of `document()`.
    static_base: Option<&'a str>,
}

impl Scope<'_> {
    /// The local variable of that name in scope, innermost first, and its slot.
    fn local(&self, name: &QName) -> Option<(usize, VariableInfo)> {
        let frame = self.frame?;
        let (_, info) = frame
            .locals
            .iter()
            .rev()
            .find(|(n, _)| n.same_expanded_name(name))?;
        let slot = (info.id.0 as usize).saturating_sub(self.globals.len());
        Some((slot, info.clone()))
    }
}

impl StaticContext for Scope<'_> {
    fn variable(&self, name: &QName) -> Option<VariableInfo> {
        if let Some((_, info)) = self.local(name) {
            return Some(info);
        }
        let (index, (_, info)) = self
            .globals
            .iter()
            .enumerate()
            .find(|(_, (n, _))| n.same_expanded_name(name))?;
        Some(if index < self.typed_globals {
            info.clone()
        } else {
            VariableInfo {
                value_type: info.value_type.declared_later(),
                ..info.clone()
            }
        })
    }

    fn variable_reference(&self, name: &QName, in_loop: bool) -> Option<VariableInfo> {
        if let (Some(frame), Some((slot, _))) = (self.frame, self.local(name)) {
            if let Some(count) = frame.uses.borrow_mut().get_mut(slot) {
                *count += 1;
            }
            let outside = frame.depths.get(slot).is_some_and(|&d| frame.depth > d);
            if let Some(looping) = frame.looping.borrow_mut().get_mut(slot) {
                *looping |= in_loop || outside;
            }
            frame
                .references
                .borrow_mut()
                .push((slot, frame.enclosing.clone()));
        }
        self.variable(name)
    }

    fn function(&self, name: &QName, arity: usize) -> Option<FunctionId> {
        self.functions
            .iter()
            .position(|(n, a)| *a == arity && n.same_expanded_name(name))
            .map(|i| FunctionId(u32::try_from(i).unwrap_or(u32::MAX)))
    }

    fn function_type(&self, id: FunctionId) -> ValueType {
        self.function_types
            .get(id.0 as usize)
            .cloned()
            .unwrap_or_default()
    }

    fn xslt_functions(&self) -> bool {
        true
    }

    /// The parser has resolved the prefixed names of `system-property()`,
    /// `element-available()` and `function-available()` already (as `Q{uri}local`): a
    /// prefix the compiler asks for is not bound.
    fn namespace_uri(&self, _prefix: &str) -> Option<String> {
        None
    }

    fn element_available(&self, name: &QName) -> Option<bool> {
        einvoice_xslt::element_available(name.namespace.as_deref(), &name.local)
    }

    fn static_base_uri(&self) -> Option<String> {
        self.static_base.map(str::to_owned)
    }
}

/// Calls `f` on each sequence constructor an instruction holds (its bodies, the value of
/// a variable or of a parameter it passes).
fn for_each_body(inst: &Inst, f: &mut dyn FnMut(&[Inst])) {
    let params = |params: &[WithParam], f: &mut dyn FnMut(&[Inst])| {
        for p in params {
            if let Value::Content(body) = &p.value {
                f(body);
            }
        }
    };
    match inst {
        Inst::ApplyTemplates(CompiledApplyTemplates { params: p, .. })
        | Inst::NextMatch(CompiledNextMatch { params: p, .. })
        | Inst::CallTemplate(CompiledCallTemplate { params: p, .. }) => params(p, f),
        Inst::Choose(CompiledChoose { whens, otherwise }) => {
            for w in whens {
                f(&w.body);
            }
            f(otherwise);
        }
        Inst::Variable(b) => {
            if let Value::Content(body) = &b.value {
                f(body);
            }
        }
        Inst::ForEach(CompiledForEach { body, .. })
        | Inst::ValueOf { body, .. }
        | Inst::Attribute { body, .. }
        | Inst::Comment { body, .. }
        | Inst::Element(CompiledElement { body, .. })
        | Inst::Copy { body, .. }
        | Inst::ComputedElement { body, .. }
        | Inst::Namespace { body, .. }
        | Inst::ProcessingInstruction { body, .. }
        | Inst::Message { body, .. } => f(body),
        Inst::Sequence { .. } | Inst::Text(_) | Inst::Number { .. } | Inst::CopyOf { .. } => {}
    }
}

/// [`for_each_body`], mutably.
fn for_each_body_mut<'i>(inst: &'i mut Inst, f: &mut dyn FnMut(&'i mut Vec<Inst>)) {
    let params = |params: &'i mut Vec<WithParam>, f: &mut dyn FnMut(&'i mut Vec<Inst>)| {
        for p in params {
            if let Value::Content(body) = &mut p.value {
                f(body);
            }
        }
    };
    match inst {
        Inst::ApplyTemplates(CompiledApplyTemplates { params: p, .. })
        | Inst::NextMatch(CompiledNextMatch { params: p, .. })
        | Inst::CallTemplate(CompiledCallTemplate { params: p, .. }) => params(p, f),
        Inst::Choose(CompiledChoose { whens, otherwise }) => {
            for w in whens {
                f(&mut w.body);
            }
            f(otherwise);
        }
        Inst::Variable(b) => {
            if let Value::Content(body) = &mut b.value {
                f(body);
            }
        }
        Inst::ForEach(CompiledForEach { body, .. })
        | Inst::ValueOf { body, .. }
        | Inst::Attribute { body, .. }
        | Inst::Comment { body, .. }
        | Inst::Element(CompiledElement { body, .. })
        | Inst::Copy { body, .. }
        | Inst::ComputedElement { body, .. }
        | Inst::Namespace { body, .. }
        | Inst::ProcessingInstruction { body, .. }
        | Inst::Message { body, .. } => f(body),
        Inst::Sequence { .. } | Inst::Text(_) | Inst::Number { .. } | Inst::CopyOf { .. } => {}
    }
}

/// Whether the value of a binding depends on the position, the size or the current item
/// of the focus it is bound in (`fn:position()`, `fn:last()`, `fn:current()`, and the
/// instructions that pass the focus on, `xsl:call-template` and `xsl:next-match`), as
/// Saxon's dependencies tell it: such a value is never evaluated lazily.
fn depends_on_focus_position(value: &VariableValue) -> bool {
    match value {
        VariableValue::Select(expr) => expr_depends_on_focus_position(expr, true),
        VariableValue::Content(body) => instructions_depend_on_focus_position(body),
        VariableValue::Absent => false,
    }
}

fn instructions_depend_on_focus_position(instructions: &[Instruction]) -> bool {
    let expr = |e: &Expr| expr_depends_on_focus_position(e, true);
    let avt = |a: &Avt| {
        a.parts
            .iter()
            .any(|p| matches!(p, AvtPart::Expr(e) if expr(e)))
    };
    let params = |params: &[Variable]| params.iter().any(|p| depends_on_focus_position(&p.value));
    instructions.iter().any(|instruction| match instruction {
        Instruction::CallTemplate(_) | Instruction::NextMatch(_) => true,
        Instruction::ApplyTemplates(a) => a.select.as_ref().is_some_and(expr) || params(&a.params),
        Instruction::Choose(c) => {
            c.whens
                .iter()
                .any(|w| expr(&w.test) || instructions_depend_on_focus_position(&w.body))
                || c.otherwise
                    .as_ref()
                    .is_some_and(|o| instructions_depend_on_focus_position(o))
        }
        Instruction::If(i) => expr(&i.test) || instructions_depend_on_focus_position(&i.body),
        // The body has the focus of the items selected.
        Instruction::ForEach(f) => expr(&f.select),
        Instruction::Variable(v) => depends_on_focus_position(&v.value),
        Instruction::Sequence(q) => expr(&q.select),
        Instruction::ValueOf(v) => {
            v.select.as_ref().is_some_and(expr)
                || v.separator.as_ref().is_some_and(avt)
                || instructions_depend_on_focus_position(&v.body)
        }
        Instruction::Attribute(a) => {
            avt(&a.name)
                || a.namespace.as_ref().is_some_and(avt)
                || a.select.as_ref().is_some_and(expr)
                || instructions_depend_on_focus_position(&a.body)
        }
        Instruction::Comment(c) => instructions_depend_on_focus_position(&c.body),
        Instruction::LiteralElement(e) => {
            e.attributes.iter().any(|a| avt(&a.value))
                || instructions_depend_on_focus_position(&e.body)
        }
        Instruction::Copy(c) => instructions_depend_on_focus_position(&c.body),
        Instruction::CopyOf(c) => expr(&c.select),
        Instruction::Element(e) => {
            avt(&e.name)
                || e.namespace.as_ref().is_some_and(avt)
                || instructions_depend_on_focus_position(&e.body)
        }
        Instruction::Namespace(n) => {
            avt(&n.name)
                || n.select.as_ref().is_some_and(expr)
                || instructions_depend_on_focus_position(&n.body)
        }
        Instruction::ProcessingInstruction(p) => {
            avt(&p.name)
                || p.select.as_ref().is_some_and(expr)
                || instructions_depend_on_focus_position(&p.body)
        }
        Instruction::Message(m) => {
            m.terminate.as_ref().is_some_and(avt) || instructions_depend_on_focus_position(&m.body)
        }
        Instruction::Text(_) | Instruction::Number(_) => false,
    })
}

/// Whether `expr` calls `current()` anywhere, or `position()` or `last()` where the
/// focus is still the outer one (`outer`): not in a predicate, nor in a path step after
/// the first.
fn expr_depends_on_focus_position(expr: &Expr, outer: bool) -> bool {
    let at = |e: &Expr, outer: bool| expr_depends_on_focus_position(e, outer);
    let bindings = |bindings: &[ast::Binding]| bindings.iter().any(|b| at(&b.sequence, outer));
    match expr {
        Expr::FunctionCall { name, args } => {
            let fn_name = |local: &str| {
                name.namespace.as_deref() == Some(ast::FN_NAMESPACE) && name.local == local
            };
            fn_name("current")
                || (outer && (fn_name("position") || fn_name("last")))
                || args.iter().any(|a| at(a, outer))
        }
        Expr::Literal(_) | Expr::VarRef(_) | Expr::ContextItem => false,
        Expr::Sequence(items) => items.iter().any(|e| at(e, outer)),
        Expr::Filter { base, predicates } => {
            at(base, outer) || predicates.iter().any(|p| at(p, false))
        }
        Expr::Path(path) => path.steps.iter().enumerate().any(|(i, step)| {
            let step_outer = outer && i == 0 && path.start == ast::PathStart::Relative;
            match step {
                ast::Step::Axis(axis_step) => axis_step.predicates.iter().any(|p| at(p, false)),
                ast::Step::Expr(e) => at(e, step_outer),
            }
        }),
        Expr::Negate(e)
        | Expr::Castable { operand: e, .. }
        | Expr::Cast { operand: e, .. }
        | Expr::InstanceOf { operand: e, .. } => at(e, outer),
        Expr::Union(lhs, rhs)
        | Expr::Range(lhs, rhs)
        | Expr::And(lhs, rhs)
        | Expr::Or(lhs, rhs)
        | Expr::Arithmetic { lhs, rhs, .. }
        | Expr::Comparison { lhs, rhs, .. } => at(lhs, outer) || at(rhs, outer),
        Expr::If {
            condition,
            then,
            otherwise,
        } => at(condition, outer) || at(then, outer) || at(otherwise, outer),
        Expr::For { bindings: b, body } => bindings(b) || at(body, outer),
        Expr::Quantified {
            bindings: b,
            satisfies,
            ..
        } => bindings(b) || at(satisfies, outer),
    }
}

/// Whether an expression calls `fn:document` anywhere.
fn calls_document(expr: &Expr) -> bool {
    let any = |exprs: &[Expr]| exprs.iter().any(calls_document);
    let bindings = |bindings: &[ast::Binding]| bindings.iter().any(|b| calls_document(&b.sequence));
    match expr {
        Expr::FunctionCall { name, args } => {
            (name.namespace.as_deref() == Some(ast::FN_NAMESPACE) && name.local == "document")
                || any(args)
        }
        Expr::Literal(_) | Expr::VarRef(_) | Expr::ContextItem => false,
        Expr::Sequence(items) => any(items),
        Expr::Filter { base, predicates } => calls_document(base) || any(predicates),
        Expr::Path(path) => path.steps.iter().any(|step| match step {
            ast::Step::Axis(axis_step) => any(&axis_step.predicates),
            ast::Step::Expr(e) => calls_document(e),
        }),
        Expr::Negate(e)
        | Expr::Castable { operand: e, .. }
        | Expr::Cast { operand: e, .. }
        | Expr::InstanceOf { operand: e, .. } => calls_document(e),
        Expr::Union(lhs, rhs)
        | Expr::Range(lhs, rhs)
        | Expr::And(lhs, rhs)
        | Expr::Or(lhs, rhs)
        | Expr::Arithmetic { lhs, rhs, .. }
        | Expr::Comparison { lhs, rhs, .. } => calls_document(lhs) || calls_document(rhs),
        Expr::If {
            condition,
            then,
            otherwise,
        } => calls_document(condition) || calls_document(then) || calls_document(otherwise),
        Expr::For { bindings: b, body } => bindings(b) || calls_document(body),
        Expr::Quantified {
            bindings: b,
            satisfies,
            ..
        } => bindings(b) || calls_document(satisfies),
    }
}

/// Whether the predicates of a pattern alternative call `fn:document` anywhere.
fn pattern_calls_document(alternative: &ast::PatternAlternative) -> bool {
    let any = |exprs: &[Expr]| exprs.iter().any(calls_document);
    match alternative {
        ast::PatternAlternative::DocumentRoot | ast::PatternAlternative::RootFunction => false,
        ast::PatternAlternative::Path(path) => std::iter::once(&path.first)
            .chain(path.rest.iter().map(|(_, step)| step))
            .any(|step| match step {
                ast::PathStep::Axis(step) => any(&step.predicates),
                ast::PathStep::Parenthesized { inner, predicates } => {
                    inner.alternatives.iter().any(pattern_calls_document) || any(predicates)
                }
            }),
    }
}

fn cardinality_of(occurrence: Occurrence) -> Cardinality {
    match occurrence {
        Occurrence::One | Occurrence::ZeroOrOne => Cardinality::AtMostOne,
        Occurrence::ZeroOrMore | Occurrence::OneOrMore => Cardinality::Any,
    }
}

/// What a binding's value is, for the inference of its static type.
fn binding_value(value: &Value) -> BindingValue<'_> {
    match value {
        Value::Select(expr) => BindingValue::Select(expr),
        Value::Content(_) => BindingValue::Content,
        Value::Absent => BindingValue::Absent,
    }
}

/// What is statically known of the number of items of a binding's value.
fn binding_cardinality(as_type: Option<&SequenceType>, value: &Value) -> Cardinality {
    if let Some(t) = as_type {
        return cardinality_of(t.occurrence);
    }
    match value {
        Value::Select(expr) if !expr.is_at_most_one() => Cardinality::Any,
        Value::Select(_) | Value::Content(_) | Value::Absent => Cardinality::AtMostOne,
    }
}

struct Compiler<'s> {
    sheet: &'s Stylesheet,
    /// Names of literal result elements and their attributes, shared.
    names: Vec<Arc<QName>>,
    errors: Vec<PrepareError>,
    globals: Vec<(QName, VariableInfo)>,
    /// How many globals are declared before the declaration being compiled.
    typed_globals: usize,
    functions: Vec<(QName, usize)>,
    /// The declared result types of the functions, by identity.
    function_types: Vec<ValueType>,
    modes: Vec<Mode>,
    /// Whether some expression compiled so far calls `document()`.
    calls_document: bool,
    /// How many [`Eval::Learning`] bindings so far.
    learners: usize,
    /// Whether the bindings being compiled are in an `xsl:function` body.
    in_function: bool,
    /// The URI of the module of the declaration being compiled.
    module_uri: Option<String>,
}

/// Collects the modes named by templates and by `xsl:apply-templates`.
fn collect_modes(instructions: &[Instruction], modes: &mut BTreeSet<Mode>) {
    for instruction in instructions {
        match instruction {
            Instruction::ApplyTemplates(apply) => {
                if let ApplyMode::Mode(mode) = &apply.mode {
                    modes.insert(mode.clone());
                }
                collect_params(&apply.params, modes);
            }
            Instruction::NextMatch(n) => collect_params(&n.params, modes),
            Instruction::CallTemplate(c) => collect_params(&c.params, modes),
            Instruction::Choose(choose) => {
                for when in &choose.whens {
                    collect_modes(&when.body, modes);
                }
                if let Some(otherwise) = &choose.otherwise {
                    collect_modes(otherwise, modes);
                }
            }
            Instruction::If(i) => collect_modes(&i.body, modes),
            Instruction::ForEach(f) => collect_modes(&f.body, modes),
            Instruction::Variable(v) => collect_value(&v.value, modes),
            Instruction::ValueOf(v) => collect_modes(&v.body, modes),
            Instruction::Attribute(a) => collect_modes(&a.body, modes),
            Instruction::Comment(c) => collect_modes(&c.body, modes),
            Instruction::LiteralElement(e) => collect_modes(&e.body, modes),
            Instruction::Copy(c) => collect_modes(&c.body, modes),
            Instruction::Element(e) => collect_modes(&e.body, modes),
            Instruction::Namespace(n) => collect_modes(&n.body, modes),
            Instruction::ProcessingInstruction(p) => collect_modes(&p.body, modes),
            Instruction::Message(m) => collect_modes(&m.body, modes),
            Instruction::Sequence(_)
            | Instruction::Text(_)
            | Instruction::Number(_)
            | Instruction::CopyOf(_) => {}
        }
    }
}

fn collect_params(params: &[Variable], modes: &mut BTreeSet<Mode>) {
    for p in params {
        collect_value(&p.value, modes);
    }
}

fn collect_value(value: &VariableValue, modes: &mut BTreeSet<Mode>) {
    if let VariableValue::Content(body) = value {
        collect_modes(body, modes);
    }
}

impl Compiler<'_> {
    fn error(&mut self, line: u32, message: impl Into<String>) {
        self.errors.push(PrepareError {
            line,
            message: message.into(),
        });
    }

    fn scope<'a>(&'a self, frame: Option<&'a Frame>) -> Scope<'a> {
        Scope {
            globals: &self.globals,
            typed_globals: self.typed_globals,
            functions: &self.functions,
            function_types: &self.function_types,
            frame,
            static_base: self.module_uri.as_deref(),
        }
    }

    fn expr(&mut self, line: u32, expr: &Expr, frame: Option<&Frame>) -> Option<Box<CompiledExpr>> {
        self.calls_document |= calls_document(expr);
        let scope = self.scope(frame);
        match compile(expr, &scope) {
            Ok(compiled) => Some(Box::new(compiled)),
            Err(e) => {
                self.error(line, format!("{expr}: {e}"));
                None
            }
        }
    }

    fn avt(&mut self, line: u32, avt: &Avt, frame: Option<&Frame>) -> Option<CompiledAvt> {
        let mut items = Vec::with_capacity(avt.parts.len());
        let mut ok = true;
        for part in &avt.parts {
            match part {
                AvtPart::Text(text) => items.push(AvtItem::Text(text.as_str().into())),
                AvtPart::Expr(e) => match self.expr(line, e, frame) {
                    Some(c) => items.push(AvtItem::Expr(c)),
                    None => ok = false,
                },
            }
        }
        ok.then_some(CompiledAvt(items))
    }

    /// A shared copy of `name` (the prefix counts: it is serialized).
    fn intern(&mut self, name: &QName) -> Arc<QName> {
        if let Some(shared) = self.names.iter().find(|n| ***n == *name) {
            return Arc::clone(shared);
        }
        let shared = Arc::new(name.clone());
        self.names.push(Arc::clone(&shared));
        shared
    }

    fn mode_index(&self, mode: &Mode) -> usize {
        self.modes.iter().position(|m| m == mode).unwrap_or(0)
    }

    /// The value of a binding (its content is a temporary tree without `as`, a sequence
    /// with it, which the executor tells apart).
    fn value(&mut self, line: u32, value: &VariableValue, frame: &mut Frame) -> Value {
        match value {
            VariableValue::Select(expr) => self
                .expr(line, expr, Some(frame))
                .map_or(Value::Absent, Value::Select),
            VariableValue::Content(body) => Value::Content(self.sequence_constructor(body, frame)),
            VariableValue::Absent => Value::Absent,
        }
    }

    /// The `xsl:with-param` of an instruction: evaluated eagerly by `xsl:apply-templates`
    /// and `xsl:next-match` (`lazy` false), as Saxon evaluates them, lazily with Saxon's
    /// learning by `xsl:call-template`.
    fn with_params(
        &mut self,
        params: &[Variable],
        frame: &mut Frame,
        lazy: bool,
    ) -> Vec<WithParam> {
        let mut compiled = Vec::with_capacity(params.len());
        for p in params {
            let value = self.value(p.line, &p.value, frame);
            let eval = if lazy {
                self.learning(&p.value)
            } else {
                Eval::Eager
            };
            compiled.push(WithParam {
                name: p.name.clone(),
                as_type: p.as_type.clone(),
                value,
                eval,
            });
        }
        compiled
    }

    /// [`Eval::Learning`] with new counters, or [`Eval::Eager`] for a value that depends
    /// on the position, the size or the current item of its focus, which Saxon never
    /// evaluates lazily (`Expression.supportsLazyEvaluation`).
    fn learning(&mut self, value: &VariableValue) -> Eval {
        if depends_on_focus_position(value) {
            return Eval::Eager;
        }
        let learner = self.learners;
        self.learners += 1;
        Eval::Learning {
            learner,
            every_counts: self.in_function,
        }
    }

    /// Compiles a sequence constructor. Local variables are in scope for their
    /// following siblings and their descendants.
    fn sequence_constructor(
        &mut self,
        instructions: &[Instruction],
        frame: &mut Frame,
    ) -> Vec<Inst> {
        let scope_depth = frame.locals.len();
        let mut out = Vec::with_capacity(instructions.len());
        for instruction in instructions {
            if let Some(inst) = self.instruction(instruction, frame) {
                out.push(inst);
            }
        }
        frame.locals.truncate(scope_depth);
        out
    }

    /// When the local variables of a template, function or global variable are
    /// evaluated, once its whole body is compiled (`body`, and the default values of its
    /// parameters, `params`), as Saxon-HE 12.10 decides it (`LetExpression.optimize` and
    /// `setEvaluator`):
    ///
    /// - unused (no reference left once the unused variables are removed): never;
    /// - referred to once, not in a loop: inlined into the reference, evaluated when it
    ///   is ([`Eval::Lazy`]);
    /// - otherwise as Saxon counts its references, which it does not always count again
    ///   after inlining others: a variable keeps the references it had, less those in
    ///   the values of the variables removed since, unless it is optimized again: when the
    ///   variable right after it is removed (its action changes), or a variable before
    ///   it in scope has the variable right after it removed (which optimizes all that
    ///   follows again, `verifyReferences`). With two references or more (or one in a
    ///   loop), Saxon's learning ([`Eval::Learning`]); with one or none, a value of at
    ///   most one item is computed where it is bound (`LazyPullEvaluator` on an item), any
    ///   other when first read.
    fn decide_frame(&mut self, frame: &mut Frame, body: &mut [Inst], params: &mut [Binding]) {
        // The references left, per slot: none in the value of an unused variable. A
        // reference is always from a variable declared later, with a higher slot.
        let mut live = vec![0_u32; frame.slots];
        let is_variable = Self::variable_slots(body, params, frame.slots);
        for slot in (0..frame.slots).rev() {
            let count = frame
                .references
                .borrow()
                .iter()
                .filter(|(target, enclosing)| {
                    *target == slot
                        && !frame.removed_with(slot, enclosing, &|e| {
                            is_variable.get(e).copied().unwrap_or(false)
                                && live.get(e).copied().unwrap_or(0) == 0
                        })
                })
                .count();
            live[slot] = u32::try_from(count).unwrap_or(u32::MAX);
            let looping = frame.looping.borrow().get(slot).copied().unwrap_or(true);
            if let Some(gone) = frame.gone.get_mut(slot) {
                *gone = is_variable.get(slot).copied().unwrap_or(false)
                    && (live[slot] == 0 || (live[slot] == 1 && !looping));
            }
        }
        for param in params.iter_mut() {
            if let Value::Content(content) = &mut param.value {
                self.decide_sequence(frame, &live, content, false);
            }
        }
        self.decide_sequence(frame, &live, body, false);
    }

    /// Which slots are local variables (not parameters).
    fn variable_slots(body: &[Inst], params: &[Binding], slots: usize) -> Vec<bool> {
        fn walk(insts: &[Inst], out: &mut Vec<bool>) {
            for inst in insts {
                if let Inst::Variable(b) = inst
                    && let Some(v) = out.get_mut(b.slot)
                {
                    *v = true;
                }
                for_each_body(inst, &mut |body| walk(body, out));
            }
        }
        let mut out = vec![false; slots];
        for param in params {
            if let Value::Content(content) = &param.value {
                walk(content, &mut out);
            }
        }
        walk(body, &mut out);
        out
    }

    /// [`Compiler::decide_frame`] on one sequence constructor; `again`: a variable before
    /// it in scope is optimized again.
    fn decide_sequence(&mut self, frame: &Frame, live: &[u32], insts: &mut [Inst], again: bool) {
        let is_gone = |slot: usize| frame.gone.get(slot).copied().unwrap_or(false);
        let next_is_gone: Vec<bool> = (0..insts.len())
            .map(|i| matches!(insts.get(i + 1), Some(Inst::Variable(next)) if is_gone(next.slot)))
            .collect();
        let mut again = again;
        for (inst, next_is_gone) in insts.iter_mut().zip(next_is_gone) {
            if let Inst::Variable(binding) = inst {
                let slot = binding.slot;
                binding.used = live.get(slot).copied().unwrap_or(0) > 0;
                if !is_gone(slot) && binding.eval != Eval::Eager {
                    let looping = frame.looping.borrow().get(slot).copied().unwrap_or(true);
                    let registered = if again || next_is_gone {
                        live.get(slot).copied().unwrap_or(0)
                    } else {
                        let kept = frame
                            .references
                            .borrow()
                            .iter()
                            .filter(|(target, enclosing)| {
                                *target == slot && !frame.removed_with(slot, enclosing, &is_gone)
                            })
                            .count();
                        u32::try_from(kept).unwrap_or(u32::MAX)
                    };
                    binding.eval = if looping || registered >= 2 {
                        let learner = self.learners;
                        self.learners += 1;
                        Eval::Learning {
                            learner,
                            every_counts: self.in_function,
                        }
                    } else if matches!(&binding.value, Value::Select(e) if e.is_at_most_one()) {
                        Eval::Eager
                    } else {
                        Eval::Lazy
                    };
                }
            }
            let mut bodies: Vec<&mut Vec<Inst>> = Vec::new();
            for_each_body_mut(inst, &mut |body| bodies.push(body));
            for body in bodies {
                self.decide_sequence(frame, live, body, again);
            }
            if let Inst::Variable(binding) = inst
                && !is_gone(binding.slot)
                && next_is_gone
            {
                // Its action changes: what follows is optimized again.
                again = true;
            }
        }
    }

    /// Compiles one instruction. Each kind is compiled by a function of its own, so that
    /// the recursion over nested instructions holds, at each level, only what that
    /// level's instruction needs (the call stack of a debug build keeps every local of a
    /// function).
    fn instruction(&mut self, instruction: &Instruction, frame: &mut Frame) -> Option<Inst> {
        match instruction {
            Instruction::ApplyTemplates(apply) => self.apply_templates(apply, frame),
            Instruction::NextMatch(next) => Some(self.next_match(next, frame)),
            Instruction::CallTemplate(call) => self.call_template(call, frame),
            Instruction::Choose(choose) => self.choose(choose, frame),
            Instruction::If(i) => self.if_instruction(i, frame),
            Instruction::ForEach(f) => self.for_each(f, frame),
            Instruction::Variable(v) => Some(self.local_variable(v, frame)),
            Instruction::Sequence(s) => self.sequence(s, frame),
            Instruction::ValueOf(v) => self.value_of(v, frame),
            Instruction::Attribute(a) => self.attribute(a, frame),
            Instruction::Comment(c) => Some(self.comment(c, frame)),
            Instruction::Text(t) => Some(Inst::Text(t.value.as_str().into())),
            Instruction::Number(n) => self.number(n),
            Instruction::LiteralElement(e) => self.literal_element(e, frame),
            Instruction::Copy(c) => Some(self.copy(c, frame)),
            Instruction::CopyOf(c) => self.copy_of(c, frame),
            Instruction::Element(e) => self.computed_element(e, frame),
            Instruction::Namespace(n) => self.namespace(n, frame),
            Instruction::ProcessingInstruction(p) => self.processing_instruction(p, frame),
            Instruction::Message(m) => self.message(m, frame),
        }
    }

    /// `xsl:apply-templates`.
    fn apply_templates(&mut self, apply: &ApplyTemplates, frame: &mut Frame) -> Option<Inst> {
        let select = if let Some(e) = &apply.select {
            self.expr(apply.line, e, Some(frame))
        } else {
            let children = parse_expr("child::node()", &NamespaceMap::new()).ok()?;
            self.expr(apply.line, &children, Some(frame))
        };
        let mode = match &apply.mode {
            ApplyMode::Current => ModeRef::Current,
            ApplyMode::Mode(m) => ModeRef::Mode(self.mode_index(m)),
        };
        let params = self.with_params(&apply.params, frame, false);
        Some(Inst::ApplyTemplates(CompiledApplyTemplates {
            line: apply.line,
            select: select?,
            mode,
            params,
        }))
    }

    /// `xsl:next-match`.
    fn next_match(&mut self, next: &NextMatch, frame: &mut Frame) -> Inst {
        Inst::NextMatch(CompiledNextMatch {
            line: next.line,
            params: self.with_params(&next.params, frame, false),
        })
    }

    /// `xsl:call-template`.
    fn call_template(&mut self, call: &CallTemplate, frame: &mut Frame) -> Option<Inst> {
        let Some(template) = self.sheet.named_template_index(&call.name) else {
            self.error(
                call.line,
                format!("no template named {} (XTSE0650)", call.name),
            );
            return None;
        };
        let declared = &self.sheet.templates[template].params;
        if let Some(p) = call
            .params
            .iter()
            .find(|p| !declared.iter().any(|d| d.name.same_expanded_name(&p.name)))
        {
            let message = format!(
                "parameter {} is not declared by template {} (XTSE0680)",
                p.name, call.name
            );
            self.error(call.line, message);
            return None;
        }
        Some(Inst::CallTemplate(CompiledCallTemplate {
            line: call.line,
            template,
            params: self.with_params(&call.params, frame, true),
        }))
    }

    /// `xsl:choose`.
    fn choose(&mut self, choose: &Choose, frame: &mut Frame) -> Option<Inst> {
        let mut whens = Vec::with_capacity(choose.whens.len());
        for when in &choose.whens {
            whens.push(self.when(when.line, &when.test, &when.body, frame)?);
        }
        let otherwise = match &choose.otherwise {
            Some(body) => self.sequence_constructor(body, frame),
            None => Vec::new(),
        };
        Some(Inst::Choose(CompiledChoose { whens, otherwise }))
    }

    /// `xsl:when`, and `xsl:if` as a `xsl:choose` of one `xsl:when`.
    fn when(
        &mut self,
        line: u32,
        test: &Expr,
        body: &[Instruction],
        frame: &mut Frame,
    ) -> Option<When> {
        let test = self.expr(line, test, Some(frame));
        let body = self.sequence_constructor(body, frame);
        Some(When {
            line,
            test: test?,
            body,
        })
    }

    /// `xsl:if`.
    fn if_instruction(&mut self, i: &If, frame: &mut Frame) -> Option<Inst> {
        let when = self.when(i.line, &i.test, &i.body, frame)?;
        Some(Inst::Choose(CompiledChoose {
            whens: vec![when],
            otherwise: Vec::new(),
        }))
    }

    /// `xsl:for-each`.
    fn for_each(&mut self, f: &ForEach, frame: &mut Frame) -> Option<Inst> {
        let select = self.expr(f.line, &f.select, Some(frame));
        frame.depth += 1;
        let body = self.sequence_constructor(&f.body, frame);
        frame.depth -= 1;
        Some(Inst::ForEach(CompiledForEach {
            line: f.line,
            select: select?,
            body,
        }))
    }

    /// A local `xsl:variable`.
    fn local_variable(&mut self, v: &Variable, frame: &mut Frame) -> Inst {
        let slot = frame.reserve();
        frame.enclosing.push(slot);
        let value = self.value(v.line, &v.value, frame);
        frame.enclosing.pop();
        let cardinality = binding_cardinality(v.as_type.as_ref(), &value);
        let value_type = ValueType::local_variable(binding_value(&value), v.as_type.as_ref());
        frame.bind(
            slot,
            v.name.clone(),
            cardinality,
            value_type,
            self.globals.len(),
        );
        // Decided once the references in its scope are known (`sequence_constructor`).
        let eval = if depends_on_focus_position(&v.value) {
            Eval::Eager
        } else {
            Eval::Lazy
        };
        Inst::Variable(Box::new(Binding {
            line: v.line,
            name: v.name.clone(),
            slot,
            as_type: v.as_type.clone(),
            value,
            eval,
            used: false,
        }))
    }

    /// `xsl:sequence`.
    fn sequence(&mut self, s: &Sequence, frame: &mut Frame) -> Option<Inst> {
        Some(Inst::Sequence {
            line: s.line,
            select: self.expr(s.line, &s.select, Some(frame))?,
        })
    }

    /// `xsl:value-of`.
    fn value_of(&mut self, v: &ValueOf, frame: &mut Frame) -> Option<Inst> {
        let select = v.select.as_ref().map(|e| self.expr(v.line, e, Some(frame)));
        let separator = v
            .separator
            .as_ref()
            .map(|s| self.avt(v.line, s, Some(frame)));
        let body = self.sequence_constructor(&v.body, frame);
        Some(Inst::ValueOf {
            line: v.line,
            select: select.map_or(Some(None), |s| s.map(Some))?,
            separator: separator.map_or(Some(None), |s| s.map(Some))?,
            body,
        })
    }

    /// `xsl:attribute`.
    fn attribute(&mut self, a: &Attribute, frame: &mut Frame) -> Option<Inst> {
        let name = self.avt(a.line, &a.name, Some(frame));
        let namespace = a
            .namespace
            .as_ref()
            .map(|n| self.avt(a.line, n, Some(frame)));
        let select = a.select.as_ref().map(|e| self.expr(a.line, e, Some(frame)));
        let body = self.sequence_constructor(&a.body, frame);
        Some(Inst::Attribute {
            line: a.line,
            name: name?,
            namespace: namespace.map_or(Some(None), |n| n.map(Some))?,
            namespaces: Arc::clone(&a.namespaces),
            select: select.map_or(Some(None), |s| s.map(Some))?,
            body,
        })
    }

    /// `xsl:comment`.
    fn comment(&mut self, c: &Comment, frame: &mut Frame) -> Inst {
        Inst::Comment {
            line: c.line,
            body: self.sequence_constructor(&c.body, frame),
        }
    }

    /// `xsl:copy`.
    fn copy(&mut self, c: &Copy, frame: &mut Frame) -> Inst {
        Inst::Copy {
            line: c.line,
            body: self.sequence_constructor(&c.body, frame),
        }
    }

    /// `xsl:copy-of`.
    fn copy_of(&mut self, c: &CopyOf, frame: &mut Frame) -> Option<Inst> {
        Some(Inst::CopyOf {
            line: c.line,
            select: self.expr(c.line, &c.select, Some(frame))?,
        })
    }

    /// `xsl:element`.
    fn computed_element(&mut self, e: &Element, frame: &mut Frame) -> Option<Inst> {
        let name = self.avt(e.line, &e.name, Some(frame));
        let namespace = e
            .namespace
            .as_ref()
            .map(|n| self.avt(e.line, n, Some(frame)));
        let body = self.sequence_constructor(&e.body, frame);
        Some(Inst::ComputedElement {
            line: e.line,
            name: name?,
            namespace: namespace.map_or(Some(None), |n| n.map(Some))?,
            namespaces: Arc::clone(&e.namespaces),
            body,
        })
    }

    /// `xsl:namespace`.
    fn namespace(&mut self, n: &NamespaceNode, frame: &mut Frame) -> Option<Inst> {
        let name = self.avt(n.line, &n.name, Some(frame));
        let select = n.select.as_ref().map(|e| self.expr(n.line, e, Some(frame)));
        let body = self.sequence_constructor(&n.body, frame);
        Some(Inst::Namespace {
            line: n.line,
            name: name?,
            select: select.map_or(Some(None), |s| s.map(Some))?,
            body,
        })
    }

    /// `xsl:processing-instruction`.
    fn processing_instruction(
        &mut self,
        p: &ProcessingInstruction,
        frame: &mut Frame,
    ) -> Option<Inst> {
        let name = self.avt(p.line, &p.name, Some(frame));
        let select = p.select.as_ref().map(|e| self.expr(p.line, e, Some(frame)));
        let body = self.sequence_constructor(&p.body, frame);
        Some(Inst::ProcessingInstruction {
            line: p.line,
            name: name?,
            select: select.map_or(Some(None), |s| s.map(Some))?,
            body,
        })
    }

    /// `xsl:message`.
    fn message(&mut self, m: &Message, frame: &mut Frame) -> Option<Inst> {
        let terminate = m
            .terminate
            .as_ref()
            .map(|t| self.avt(m.line, t, Some(frame)));
        let body = self.sequence_constructor(&m.body, frame);
        Some(Inst::Message {
            line: m.line,
            terminate: terminate.map_or(Some(None), |t| t.map(Some))?,
            body,
        })
    }

    /// `xsl:number`.
    fn number(&mut self, n: &Number) -> Option<Inst> {
        let count = match &n.count {
            None => None,
            Some(p) => Some(CountPattern {
                alternatives: self.pattern(n.line, p)?,
                text: p.to_string(),
            }),
        };
        Some(Inst::Number {
            line: n.line,
            level: n.level,
            count,
        })
    }

    /// A literal result element.
    fn literal_element(&mut self, e: &LiteralElement, frame: &mut Frame) -> Option<Inst> {
        let mut attributes = Vec::with_capacity(e.attributes.len());
        let mut ok = true;
        for attribute in &e.attributes {
            match self.avt(e.line, &attribute.value, Some(frame)) {
                Some(value) => attributes.push((self.intern(&attribute.name), value)),
                None => ok = false,
            }
        }
        let body = self.sequence_constructor(&e.body, frame);
        if !ok {
            return None;
        }
        Some(Inst::Element(CompiledElement {
            line: e.line,
            name: self.intern(&e.name),
            namespaces: Arc::clone(&e.namespaces),
            attributes,
            body,
        }))
    }

    /// A pattern (template `match`, `xsl:number/@count`): global variables only.
    fn pattern(&mut self, line: u32, pattern: &ast::Pattern) -> Option<Vec<Alternative>> {
        let mut out = Vec::with_capacity(pattern.alternatives.len());
        for alternative in &pattern.alternatives {
            self.calls_document |= pattern_calls_document(alternative);
            let scope = self.scope(None);
            match pattern::compile_alternative(alternative, &scope) {
                Ok(a) => out.push(a),
                Err(message) => {
                    self.error(line, format!("pattern {alternative}: {message}"));
                    return None;
                }
            }
        }
        Some(out)
    }

    /// Template and function parameters, declared in order in `frame`.
    fn params(&mut self, params: &[Variable], frame: &mut Frame) -> Vec<Binding> {
        let mut out = Vec::with_capacity(params.len());
        for p in params {
            let value = self.value(p.line, &p.value, frame);
            let cardinality = p
                .as_type
                .as_ref()
                .map_or(Cardinality::Any, |t| cardinality_of(t.occurrence));
            let eval = self.learning(&p.value);
            let slot = frame.declare(
                p.name.clone(),
                cardinality,
                ValueType::declared(p.as_type.as_ref()),
                self.globals.len(),
            );
            out.push(Binding {
                line: p.line,
                name: p.name.clone(),
                slot,
                as_type: p.as_type.clone(),
                value,
                eval,
                used: true,
            });
        }
        out
    }

    fn template(&mut self, template: &Template) -> CompiledTemplate {
        self.typed_globals = self.globals_before(template.module, template.line);
        self.module_uri = self.module_uri_of(template.module);
        let mut frame = Frame::default();
        let mut params = self.params(&template.params, &mut frame);
        let mut body = self.sequence_constructor(&template.body, &mut frame);
        self.decide_frame(&mut frame, &mut body, &mut params);
        let (pattern, pattern_text, alternative_texts) = match &template.pattern {
            Some(p) => (
                self.pattern(template.line, p).unwrap_or_default(),
                p.to_string(),
                p.alternatives.iter().map(ToString::to_string).collect(),
            ),
            None => (Vec::new(), String::new(), Vec::new()),
        };
        CompiledTemplate {
            line: template.line,
            params,
            tail_calls: has_tail_call(&body),
            body,
            slots: frame.slots,
            pattern,
            pattern_text,
            alternative_texts,
        }
    }
}

impl Executable {
    /// Prepares a loaded stylesheet.
    ///
    /// # Errors
    ///
    /// Every problem found: XPath expressions that do not compile in their static
    /// context (unknown variables or functions), calls of undeclared templates, and the
    /// constructs refused because their result would differ from the reference engine's
    /// (instructions that construct nodes inside an `xsl:function` or a variable with
    /// `as`, a positional predicate in a parenthesized pattern, `system-property()` with
    /// a prefixed name).
    pub fn new(sheet: &Stylesheet) -> Result<Self, PrepareErrors> {
        let mut modes = BTreeSet::new();
        modes.insert(Mode::Default);
        for template in &sheet.templates {
            if template.pattern.is_some()
                && let TemplateModes::Listed(listed) = &template.modes
            {
                modes.extend(listed.iter().cloned());
            }
            collect_modes(&template.body, &mut modes);
            collect_params(&template.params, &mut modes);
        }
        for function in &sheet.functions {
            collect_modes(&function.body, &mut modes);
        }
        for global in &sheet.globals {
            collect_value(&global.variable.value, &mut modes);
        }
        let mut compiler = Compiler {
            sheet,
            names: Vec::new(),
            errors: Vec::new(),
            globals: Vec::new(),
            typed_globals: 0,
            functions: sheet
                .functions
                .iter()
                .map(|f| (f.name.clone(), f.params.len()))
                .collect(),
            function_types: sheet
                .functions
                .iter()
                .map(|f| ValueType::declared(f.as_type.as_ref()))
                .collect(),
            modes: modes.into_iter().collect(),
            calls_document: false,
            learners: 0,
            in_function: false,
            module_uri: None,
        };
        let globals = compiler.globals(sheet);
        let templates: Vec<CompiledTemplate> = sheet
            .templates
            .iter()
            .map(|t| compiler.template(t))
            .collect();
        let functions = sheet
            .functions
            .iter()
            .map(|f| {
                compiler.typed_globals = compiler.globals_before(f.module, f.line);
                compiler.module_uri = compiler.module_uri_of(f.module);
                let mut frame = Frame::default();
                let mut params = compiler.params(&f.params, &mut frame);
                compiler.in_function = true;
                let mut body = compiler.sequence_constructor(&f.body, &mut frame);
                compiler.decide_frame(&mut frame, &mut body, &mut params);
                compiler.in_function = false;
                CompiledFunction {
                    line: f.line,
                    name: f.name.clone(),
                    params,
                    as_type: f.as_type.clone(),
                    body,
                    slots: frame.slots,
                }
            })
            .collect();
        let (compiled_modes, rule_names) = compile_modes(sheet, &templates, &compiler.modes);
        if !compiler.errors.is_empty() {
            compiler.errors.sort_by_key(|e| e.line);
            return Err(PrepareErrors(compiler.errors));
        }
        let default_mode = compiler.mode_index(&Mode::Default);
        let serialization = sheet
            .output
            .map_or_else(Serialization::default, |o| Serialization {
                indent: o.indent == Some(true),
                omit_xml_declaration: o.omit_xml_declaration == Some(true),
                standalone: match o.standalone {
                    Some(Standalone::Yes) => Some(true),
                    Some(Standalone::No) => Some(false),
                    Some(Standalone::Omit) | None => None,
                },
            });
        Ok(Self {
            globals,
            templates,
            functions,
            modes: compiled_modes,
            default_mode,
            serialization,
            base_uri: None,
            calls_document: compiler.calls_document,
            learners: compiler.learners,
            loaded: DocumentCache::new(),
            regexes: RegexCache::new(),
            rule_names,
        })
    }

    /// Sets the static base URI of the stylesheet (its location), against which
    /// `document()` resolves relative URIs given as strings.
    #[must_use]
    pub fn with_base_uri(mut self, uri: impl Into<String>) -> Self {
        self.base_uri = Some(uri.into());
        self
    }
}

impl Compiler<'_> {
    /// How many global variables and parameters are declared before `line` of module
    /// `module`: Saxon types the declarations in document order, the modules of a lower
    /// import precedence first.
    fn globals_before(&self, module: usize, line: u32) -> usize {
        let precedence = self.sheet.precedence(module);
        self.sheet
            .globals
            .iter()
            .take_while(|g| {
                let p = self.sheet.precedence(g.module);
                p < precedence || (p == precedence && g.variable.line < line)
            })
            .count()
    }

    /// The URI of module `module`, the static base URI of its expressions.
    fn module_uri_of(&self, module: usize) -> Option<String> {
        self.sheet.modules.get(module).and_then(|m| m.uri.clone())
    }

    /// The global variables. Their static cardinality comes from their compiled value,
    /// which may refer to other globals: compiled again until it is stable. Their static
    /// type is inferred in declaration order from the types of the globals declared
    /// before them, and used from the second compilation on.
    fn globals(&mut self, sheet: &Stylesheet) -> Vec<Global> {
        self.globals = sheet
            .globals
            .iter()
            .enumerate()
            .map(|(i, g)| {
                (
                    g.variable.name.clone(),
                    VariableInfo {
                        id: VariableId(u32::try_from(i).unwrap_or(u32::MAX)),
                        cardinality: g
                            .variable
                            .as_type
                            .as_ref()
                            .map_or(Cardinality::Any, |t| cardinality_of(t.occurrence)),
                        value_type: ValueType::unknown(),
                    },
                )
            })
            .collect();
        let errors_before = self.errors.len();
        let mut compiled: Vec<Global> = Vec::new();
        for pass in 0..=sheet.globals.len() + 1 {
            self.errors.truncate(errors_before);
            compiled = Vec::with_capacity(sheet.globals.len());
            for (index, g) in sheet.globals.iter().enumerate() {
                let v = &g.variable;
                self.typed_globals = index;
                self.module_uri = self.module_uri_of(g.module);
                let mut frame = Frame::default();
                let mut value = self.value(v.line, &v.value, &mut frame);
                if let Value::Content(body) = &mut value {
                    self.decide_frame(&mut frame, body, &mut []);
                }
                let value_type = if g.kind == GlobalKind::Param {
                    ValueType::declared(v.as_type.as_ref())
                } else {
                    ValueType::global_variable(binding_value(&value), v.as_type.as_ref())
                };
                if let Some((_, info)) = self.globals.get_mut(index) {
                    info.value_type = value_type;
                }
                compiled.push(Global {
                    name: v.name.clone(),
                    is_param: g.kind == GlobalKind::Param,
                    line: v.line,
                    value,
                    as_type: v.as_type.clone(),
                    slots: frame.slots,
                });
            }
            // The types are final after the first compilation (each depends on the
            // globals declared before it only), which saw the later ones untyped.
            let mut changed = pass == 0;
            for (info, global) in self.globals.iter_mut().zip(&compiled) {
                let cardinality = binding_cardinality(global.as_type.as_ref(), &global.value);
                if info.1.cardinality != cardinality {
                    info.1.cardinality = cardinality;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        compiled
    }
}

/// The priority as Saxon compares priorities: a double.
fn as_double(priority: Priority) -> f64 {
    priority.to_string().parse().unwrap_or(f64::NAN)
}

/// The rules of `mode`, ranked and indexed.
fn rank_rules(sheet: &Stylesheet, templates: &[CompiledTemplate], mode: &Mode) -> CompiledMode {
    let mut rules = Vec::new();
    for (index, template) in sheet.templates.iter().enumerate() {
        if template.pattern.is_none() {
            continue;
        }
        let in_mode = match &template.modes {
            TemplateModes::All => true,
            TemplateModes::Listed(modes) => modes.contains(mode),
        };
        if !in_mode {
            continue;
        }
        // Saxon-HE 12.10 keeps a union pattern with an explicit priority as one rule
        // (Saxon-HE 9.9 split it like a pattern without one).
        let whole = template.rules.len() > 1 && template.rules.iter().all(|r| !r.is_default);
        for rule in &template.rules {
            rules.push(Rule {
                template: index,
                alternative: (!whole).then_some(rule.alternative),
                precedence: sheet.precedence(template.module),
                priority: as_double(rule.priority),
                sequence: index,
                part: if rule.is_default { rule.alternative } else { 0 },
                certain: 0,
            });
            if whole {
                break;
            }
        }
    }
    rules.sort_by(|a, b| {
        b.compare_rank(a)
            .then(b.sequence.cmp(&a.sequence))
            .then(b.part.cmp(&a.part))
    });
    let mut compiled = CompiledMode {
        name: match mode {
            Mode::Default => "#default".to_owned(),
            Mode::Named(q) => q.to_string(),
        },
        ..CompiledMode::default()
    };
    for (rank, rule) in rules.iter_mut().enumerate() {
        let rank = u32::try_from(rank).unwrap_or(u32::MAX);
        let pattern = &templates[rule.template].pattern;
        let (keys, tried) = match rule.alternative {
            Some(alternative) => (
                pattern
                    .get(alternative)
                    .map(pattern::Alternative::chain_keys)
                    .unwrap_or_default(),
                pattern.get(alternative..=alternative).unwrap_or_default(),
            ),
            None => (
                pattern::union_chain_key(pattern).into_iter().collect(),
                pattern.as_slice(),
            ),
        };
        rule.certain = certain_kinds(tried, &keys);
        for key in &keys {
            compiled.index(rank, key);
        }
    }
    compiled.rules = rules;
    compiled.plan_kinds();
    compiled
}
