//! Preparation: a loaded [`Stylesheet`] becomes an [`Executable`], once, to be run on any
//! number of documents. Every XPath expression is compiled against its static context
//! (global variables, the local variables in scope, the stylesheet functions), every
//! variable gets a slot in the frame of its template, function or global variable,
//! named templates and modes are resolved, and the template rules of each mode are
//! ranked and indexed by the kind and name of the nodes they can match.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use einvoice_xpath::{
    self as ast, Avt, AvtPart, Expr, NamespaceMap, Occurrence, QName, SequenceType, parse_expr,
};
use einvoice_xpath_eval::{
    Cardinality, CompiledExpr, FunctionId, StaticContext, VariableId, VariableInfo, compile,
};
use einvoice_xslt::{
    ApplyMode, GlobalKind, Instruction, Mode, NamespaceBinding, NumberLevel, Priority, Standalone,
    Stylesheet, Template, TemplateModes, Variable, VariableValue,
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
    /// Evaluated when first referenced rather than where it is declared: a `select`
    /// that may return more than one item (Saxon evaluates the others eagerly).
    pub(crate) lazy: bool,
    /// Whether some expression in its scope refers to it (an unused variable is never
    /// evaluated, as Saxon removes it).
    pub(crate) used: bool,
}

/// `xsl:with-param`.
#[derive(Debug)]
pub(crate) struct WithParam {
    pub(crate) name: QName,
    pub(crate) as_type: Option<SequenceType>,
    pub(crate) value: Value,
    pub(crate) lazy: bool,
}

#[derive(Debug)]
pub(crate) struct CompiledTemplate {
    pub(crate) line: u32,
    pub(crate) params: Vec<Binding>,
    pub(crate) body: Vec<Inst>,
    pub(crate) slots: usize,
    /// The compiled alternatives of the match pattern (empty without one).
    pub(crate) pattern: Vec<Alternative>,
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
    ApplyTemplates {
        line: u32,
        select: Box<CompiledExpr>,
        mode: ModeRef,
        params: Vec<WithParam>,
    },
    NextMatch {
        line: u32,
        params: Vec<WithParam>,
    },
    CallTemplate {
        line: u32,
        template: usize,
        params: Vec<WithParam>,
    },
    /// `xsl:choose`, and `xsl:if` as a choose with one branch.
    Choose {
        line: u32,
        whens: Vec<(Box<CompiledExpr>, Vec<Inst>)>,
        otherwise: Vec<Inst>,
    },
    ForEach {
        line: u32,
        select: Box<CompiledExpr>,
        body: Vec<Inst>,
    },
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
        count: Option<Vec<Alternative>>,
    },
    Element {
        line: u32,
        name: Arc<QName>,
        namespaces: Arc<[NamespaceBinding]>,
        attributes: Vec<(Arc<QName>, CompiledAvt)>,
        body: Vec<Inst>,
    },
}

/// A template rule as Saxon-HE 12.10 registers it: one per alternative of a match
/// pattern without explicit priority (`A | B` is split into rules, XSLT 2.0 section
/// 6.4), one for the whole pattern with an explicit priority.
#[derive(Debug)]
pub(crate) struct Rule {
    pub(crate) template: usize,
    /// The alternative of the template's pattern; `None` for the whole pattern.
    pub(crate) alternative: Option<usize>,
    /// The priority, compared as Saxon compares it: as a double.
    pub(crate) priority: f64,
    /// Declaration order: the template's position.
    pub(crate) sequence: usize,
    /// The position of the alternative among the rules split from the same template
    /// without explicit priority; 0 for a pattern with an explicit priority (Saxon's
    /// `part`, which keeps `xsl:next-match` from running the same template twice).
    pub(crate) part: usize,
}

/// Rule ranks of elements or attributes by local name, then namespace.
type ByName = HashMap<String, Vec<(Option<String>, Vec<u32>)>>;

/// The rules of a mode and Saxon's rule chains: the ranks of the rules that can match
/// nodes of one name (elements, attributes), of one kind, or of any kind, each chain in
/// rank order.
#[derive(Debug, Default)]
pub(crate) struct CompiledMode {
    pub(crate) name: String,
    /// In rank order: priority descending, then declaration order descending, then part
    /// descending (Saxon puts a rule added later before the earlier ones of the same
    /// priority).
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
}

const EMPTY: &[u32] = &[];

impl CompiledMode {
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

    /// The chains Saxon searches for a node of that kind and name, in its order: the
    /// chain of the name, the chain of the kind, the generic chain.
    pub(crate) fn chains(
        &self,
        kind: einvoice_xpath_eval::NodeKind,
        name: Option<&QName>,
    ) -> [&[u32]; 3] {
        use einvoice_xpath_eval::NodeKind as K;
        let kind_chain: (&[u32], &[u32]) = match kind {
            K::Element => (Self::named(&self.elements, name), &self.any_element),
            K::Attribute => (Self::named(&self.attributes, name), &self.any_attribute),
            K::Text => (EMPTY, &self.text),
            K::Comment => (EMPTY, &self.comment),
            K::ProcessingInstruction => (EMPTY, &self.processing_instruction),
            K::Document => (EMPTY, &self.document),
        };
        [kind_chain.0, kind_chain.1, &self.generic]
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
}

impl Frame {
    fn declare(&mut self, name: QName, cardinality: Cardinality, globals: usize) -> usize {
        let slot = self.slots;
        self.slots += 1;
        self.uses.borrow_mut().push(0);
        let id = VariableId(u32::try_from(globals + slot).unwrap_or(u32::MAX));
        self.locals.push((name, VariableInfo { id, cardinality }));
        slot
    }
}

/// The static context of an expression.
struct Scope<'a> {
    globals: &'a [(QName, VariableInfo)],
    functions: &'a [(QName, usize)],
    frame: Option<&'a Frame>,
    /// Set when the compiler asked for a namespace prefix (only `system-property()`
    /// does), which this crate cannot resolve.
    asked_prefix: Cell<bool>,
}

impl StaticContext for Scope<'_> {
    fn variable(&self, name: &QName) -> Option<VariableInfo> {
        if let Some(frame) = self.frame
            && let Some((_, info)) = frame
                .locals
                .iter()
                .rev()
                .find(|(n, _)| n.same_expanded_name(name))
        {
            let globals = self.globals.len();
            let slot = (info.id.0 as usize).saturating_sub(globals);
            if let Some(count) = frame.uses.borrow_mut().get_mut(slot) {
                *count += 1;
            }
            return Some(*info);
        }
        self.globals
            .iter()
            .find(|(n, _)| n.same_expanded_name(name))
            .map(|(_, info)| *info)
    }

    fn function(&self, name: &QName, arity: usize) -> Option<FunctionId> {
        self.functions
            .iter()
            .position(|(n, a)| *a == arity && n.same_expanded_name(name))
            .map(|i| FunctionId(u32::try_from(i).unwrap_or(u32::MAX)))
    }

    fn xslt_functions(&self) -> bool {
        true
    }

    fn namespace_uri(&self, _prefix: &str) -> Option<String> {
        self.asked_prefix.set(true);
        None
    }
}

fn cardinality_of(occurrence: Occurrence) -> Cardinality {
    match occurrence {
        Occurrence::One | Occurrence::ZeroOrOne => Cardinality::AtMostOne,
        Occurrence::ZeroOrMore | Occurrence::OneOrMore => Cardinality::Any,
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
    functions: Vec<(QName, usize)>,
    modes: Vec<Mode>,
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
            Instruction::Sequence(_) | Instruction::Text(_) | Instruction::Number(_) => {}
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

/// Where a sequence constructor's result goes, for the refusals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Content {
    /// Nodes may be constructed.
    Nodes,
    /// A sequence: only `xsl:sequence`, variables and control flow (the body of an
    /// `xsl:function`, a variable with `as`).
    Sequence,
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
            functions: &self.functions,
            frame,
            asked_prefix: Cell::new(false),
        }
    }

    fn expr(&mut self, line: u32, expr: &Expr, frame: Option<&Frame>) -> Option<Box<CompiledExpr>> {
        let scope = self.scope(frame);
        match compile(expr, &scope) {
            Ok(compiled) => Some(Box::new(compiled)),
            Err(e) => {
                let message = if scope.asked_prefix.get() {
                    format!(
                        "{expr}: system-property() with a prefixed name is not supported (the in-scope namespaces of instructions are not kept)"
                    )
                } else {
                    format!("{expr}: {e}")
                };
                self.error(line, message);
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

    /// The value of a binding. Its content is a sequence when it has `as`.
    fn value(
        &mut self,
        line: u32,
        value: &VariableValue,
        as_type: Option<&SequenceType>,
        frame: &mut Frame,
    ) -> Value {
        match value {
            VariableValue::Select(expr) => self
                .expr(line, expr, Some(frame))
                .map_or(Value::Absent, Value::Select),
            VariableValue::Content(body) => {
                let content = if as_type.is_some() {
                    Content::Sequence
                } else {
                    Content::Nodes
                };
                Value::Content(self.sequence_constructor(body, frame, content))
            }
            VariableValue::Absent => Value::Absent,
        }
    }

    fn with_params(&mut self, params: &[Variable], frame: &mut Frame) -> Vec<WithParam> {
        params
            .iter()
            .map(|p| {
                let value = self.value(p.line, &p.value, p.as_type.as_ref(), frame);
                let lazy = matches!(&value, Value::Select(e) if !e.is_at_most_one());
                WithParam {
                    name: p.name.clone(),
                    as_type: p.as_type.clone(),
                    value,
                    lazy,
                }
            })
            .collect()
    }

    /// Compiles a sequence constructor. Local variables are in scope for their
    /// following siblings and their descendants.
    fn sequence_constructor(
        &mut self,
        instructions: &[Instruction],
        frame: &mut Frame,
        content: Content,
    ) -> Vec<Inst> {
        let scope_depth = frame.locals.len();
        let mut out = Vec::with_capacity(instructions.len());
        for instruction in instructions {
            if let Some(inst) = self.instruction(instruction, frame, content) {
                out.push(inst);
            }
        }
        frame.locals.truncate(scope_depth);
        let uses = frame.uses.borrow();
        for inst in &mut out {
            if let Inst::Variable(binding) = inst {
                binding.used = uses.get(binding.slot).is_some_and(|&n| n > 0);
            }
        }
        out
    }

    fn refuse_in_sequence(&mut self, line: u32, what: &str, content: Content) -> bool {
        if content == Content::Sequence {
            self.error(
                line,
                format!(
                    "{what} in a sequence (an xsl:function body or a variable with `as`) is not supported"
                ),
            );
            return true;
        }
        false
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one arm per instruction of the supported subset"
    )]
    fn instruction(
        &mut self,
        instruction: &Instruction,
        frame: &mut Frame,
        content: Content,
    ) -> Option<Inst> {
        Some(match instruction {
            Instruction::ApplyTemplates(apply) => {
                if self.refuse_in_sequence(apply.line, "xsl:apply-templates", content) {
                    return None;
                }
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
                let params = self.with_params(&apply.params, frame);
                Inst::ApplyTemplates {
                    line: apply.line,
                    select: select?,
                    mode,
                    params,
                }
            }
            Instruction::NextMatch(next) => {
                if self.refuse_in_sequence(next.line, "xsl:next-match", content) {
                    return None;
                }
                Inst::NextMatch {
                    line: next.line,
                    params: self.with_params(&next.params, frame),
                }
            }
            Instruction::CallTemplate(call) => {
                if self.refuse_in_sequence(call.line, "xsl:call-template", content) {
                    return None;
                }
                let Some(template) = self.sheet.templates.iter().position(|t| {
                    t.name
                        .as_ref()
                        .is_some_and(|n| n.same_expanded_name(&call.name))
                }) else {
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
                Inst::CallTemplate {
                    line: call.line,
                    template,
                    params: self.with_params(&call.params, frame),
                }
            }
            Instruction::Choose(choose) => {
                let mut whens = Vec::with_capacity(choose.whens.len());
                for when in &choose.whens {
                    let test = self.expr(when.line, &when.test, Some(frame));
                    let body = self.sequence_constructor(&when.body, frame, content);
                    whens.push((test?, body));
                }
                let otherwise = choose
                    .otherwise
                    .as_ref()
                    .map(|body| self.sequence_constructor(body, frame, content))
                    .unwrap_or_default();
                Inst::Choose {
                    line: choose.line,
                    whens,
                    otherwise,
                }
            }
            Instruction::If(i) => {
                let test = self.expr(i.line, &i.test, Some(frame));
                let body = self.sequence_constructor(&i.body, frame, content);
                Inst::Choose {
                    line: i.line,
                    whens: vec![(test?, body)],
                    otherwise: Vec::new(),
                }
            }
            Instruction::ForEach(f) => {
                let select = self.expr(f.line, &f.select, Some(frame));
                let body = self.sequence_constructor(&f.body, frame, content);
                Inst::ForEach {
                    line: f.line,
                    select: select?,
                    body,
                }
            }
            Instruction::Variable(v) => {
                let value = self.value(v.line, &v.value, v.as_type.as_ref(), frame);
                let cardinality = binding_cardinality(v.as_type.as_ref(), &value);
                let lazy = matches!(&value, Value::Select(e) if !e.is_at_most_one());
                let slot = frame.declare(v.name.clone(), cardinality, self.globals.len());
                Inst::Variable(Box::new(Binding {
                    line: v.line,
                    name: v.name.clone(),
                    slot,
                    as_type: v.as_type.clone(),
                    value,
                    lazy,
                    used: false,
                }))
            }
            Instruction::Sequence(s) => Inst::Sequence {
                line: s.line,
                select: self.expr(s.line, &s.select, Some(frame))?,
            },
            Instruction::ValueOf(v) => {
                if self.refuse_in_sequence(v.line, "xsl:value-of", content) {
                    return None;
                }
                let select = v.select.as_ref().map(|e| self.expr(v.line, e, Some(frame)));
                let separator = v
                    .separator
                    .as_ref()
                    .map(|s| self.avt(v.line, s, Some(frame)));
                let body = self.sequence_constructor(&v.body, frame, Content::Nodes);
                Inst::ValueOf {
                    line: v.line,
                    select: select.map_or(Some(None), |s| s.map(Some))?,
                    separator: separator.map_or(Some(None), |s| s.map(Some))?,
                    body,
                }
            }
            Instruction::Attribute(a) => {
                if self.refuse_in_sequence(a.line, "xsl:attribute", content) {
                    return None;
                }
                let name = self.avt(a.line, &a.name, Some(frame));
                let select = a.select.as_ref().map(|e| self.expr(a.line, e, Some(frame)));
                let body = self.sequence_constructor(&a.body, frame, Content::Nodes);
                Inst::Attribute {
                    line: a.line,
                    name: name?,
                    namespaces: Arc::clone(&a.namespaces),
                    select: select.map_or(Some(None), |s| s.map(Some))?,
                    body,
                }
            }
            Instruction::Comment(c) => {
                if self.refuse_in_sequence(c.line, "xsl:comment", content) {
                    return None;
                }
                Inst::Comment {
                    line: c.line,
                    body: self.sequence_constructor(&c.body, frame, Content::Nodes),
                }
            }
            Instruction::Text(t) => {
                if self.refuse_in_sequence(t.line, "text", content) {
                    return None;
                }
                Inst::Text(t.value.as_str().into())
            }
            Instruction::Number(n) => {
                if self.refuse_in_sequence(n.line, "xsl:number", content) {
                    return None;
                }
                let count = match &n.count {
                    None => None,
                    Some(p) => Some(self.pattern(n.line, p)?),
                };
                Inst::Number {
                    line: n.line,
                    level: n.level,
                    count,
                }
            }
            Instruction::LiteralElement(e) => {
                if self.refuse_in_sequence(e.line, "a literal result element", content) {
                    return None;
                }
                let mut attributes = Vec::with_capacity(e.attributes.len());
                let mut ok = true;
                for attribute in &e.attributes {
                    match self.avt(e.line, &attribute.value, Some(frame)) {
                        Some(value) => attributes.push((self.intern(&attribute.name), value)),
                        None => ok = false,
                    }
                }
                let body = self.sequence_constructor(&e.body, frame, Content::Nodes);
                if !ok {
                    return None;
                }
                Inst::Element {
                    line: e.line,
                    name: self.intern(&e.name),
                    namespaces: Arc::clone(&e.namespaces),
                    attributes,
                    body,
                }
            }
        })
    }

    /// A pattern (template `match`, `xsl:number/@count`): global variables only.
    fn pattern(&mut self, line: u32, pattern: &ast::Pattern) -> Option<Vec<Alternative>> {
        let mut out = Vec::with_capacity(pattern.alternatives.len());
        for alternative in &pattern.alternatives {
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
            let value = self.value(p.line, &p.value, p.as_type.as_ref(), frame);
            let cardinality = p
                .as_type
                .as_ref()
                .map_or(Cardinality::Any, |t| cardinality_of(t.occurrence));
            let lazy = matches!(&value, Value::Select(e) if !e.is_at_most_one());
            let slot = frame.declare(p.name.clone(), cardinality, self.globals.len());
            out.push(Binding {
                line: p.line,
                name: p.name.clone(),
                slot,
                as_type: p.as_type.clone(),
                value,
                lazy,
                used: true,
            });
        }
        out
    }

    fn template(&mut self, template: &Template) -> CompiledTemplate {
        let mut frame = Frame::default();
        let params = self.params(&template.params, &mut frame);
        let body = self.sequence_constructor(&template.body, &mut frame, Content::Nodes);
        let pattern = match &template.pattern {
            Some(p) => self.pattern(template.line, p).unwrap_or_default(),
            None => Vec::new(),
        };
        CompiledTemplate {
            line: template.line,
            params,
            body,
            slots: frame.slots,
            pattern,
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
            functions: sheet
                .functions
                .iter()
                .map(|f| (f.name.clone(), f.params.len()))
                .collect(),
            modes: modes.into_iter().collect(),
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
                let mut frame = Frame::default();
                let params = compiler.params(&f.params, &mut frame);
                let body = compiler.sequence_constructor(&f.body, &mut frame, Content::Sequence);
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
        let compiled_modes = compiler
            .modes
            .iter()
            .map(|mode| rank_rules(sheet, &templates, mode))
            .collect();
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
    /// The global variables. Their static cardinality comes from their compiled value,
    /// which may refer to other globals: compiled again until it is stable.
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
                    },
                )
            })
            .collect();
        let errors_before = self.errors.len();
        let mut compiled: Vec<Global> = Vec::new();
        for _ in 0..=sheet.globals.len() {
            self.errors.truncate(errors_before);
            compiled = sheet
                .globals
                .iter()
                .map(|g| {
                    let v = &g.variable;
                    let mut frame = Frame::default();
                    let value = self.value(v.line, &v.value, v.as_type.as_ref(), &mut frame);
                    Global {
                        name: v.name.clone(),
                        is_param: g.kind == GlobalKind::Param,
                        line: v.line,
                        value,
                        as_type: v.as_type.clone(),
                        slots: frame.slots,
                    }
                })
                .collect();
            let mut changed = false;
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
                priority: as_double(rule.priority),
                sequence: index,
                part: if rule.is_default { rule.alternative } else { 0 },
            });
            if whole {
                break;
            }
        }
    }
    rules.sort_by(|a, b| {
        b.priority
            .total_cmp(&a.priority)
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
    for (rank, rule) in rules.iter().enumerate() {
        let rank = u32::try_from(rank).unwrap_or(u32::MAX);
        let pattern = &templates[rule.template].pattern;
        match rule.alternative {
            Some(alternative) => {
                if let Some(alternative) = pattern.get(alternative) {
                    for key in alternative.chain_keys() {
                        compiled.index(rank, &key);
                    }
                }
            }
            None => {
                if let Some(key) = pattern::union_chain_key(pattern) {
                    compiled.index(rank, &key);
                }
            }
        }
    }
    compiled.rules = rules;
    compiled
}
