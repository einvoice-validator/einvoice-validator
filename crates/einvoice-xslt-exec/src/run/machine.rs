//! The executor of sequence constructors: a loop over a stack of tasks kept on the heap,
//! not a recursion of the call stack.
//!
//! The instructions of a template body, and what they start, are tasks: a sequence
//! constructor being run (a template body, the content of an element being constructed,
//! a branch of `xsl:choose`, the body of `xsl:for-each` for an item, the content of an
//! instruction collected before it is used: a variable bound where it is declared,
//! `xsl:attribute`, `xsl:value-of`, `xsl:comment`, ...) with what ends it (the end of
//! the template invocation, the end tag, the use of the content); the template rules
//! applied to each node `xsl:apply-templates` selects; the built-in rule applied to each
//! child of a node; the items of `xsl:for-each`. Each is pushed on [`Machine::tasks`],
//! the context of a template invocation or of an item on [`Machine::states`], what an
//! `xsl:apply-templates`, a built-in rule, an `xsl:for-each` or a collected content is
//! at on a stack of its own, and [`Runtime::drive`] runs the task on top until none is
//! left. An `xsl:apply-templates` that ends a sequence constructor (as in every template
//! rule of a compiled Schematron) takes its place, and ends it once its rules are done;
//! a sequence constructor whose instructions start no task runs in place ([`is_leaf`]).
//! A template invoked by another (a compiled Schematron applies its rules to the
//! children of each element, and writes the path of each node it reports on with a
//! template applied to the node's parent, then to its parent, and so on) adds a task and
//! a state to these stacks, and nothing to the call stack: a document nested as deep as
//! the maximum depth is validated with the same call stack as a flat one, the stack of a
//! WebAssembly engine included.
//!
//! What runs on the call stack: the evaluation of an XPath expression, and what it calls
//! back, each a run of its own with a machine of its own: a stylesheet function, a global
//! variable, a local variable or parameter computed when first read, a parameter
//! computed where it is passed. These nest as deep as a stylesheet recurses through
//! them, bounded by `max_nesting` and `max_stack` (see [`Runtime::enter`]).
//!
//! An error ends the run: the tasks left are dropped, innermost first, each adding the
//! line of its instruction when the error has none yet, as each level of a recursion
//! would add it on its way out, and the frames and the count of invocations go back to
//! what they were when the run started.

use std::rc::Rc;
use std::sync::Arc;

use einvoice_xpath::QName;
use einvoice_xpath_eval::tree::Axis;
use einvoice_xpath_eval::{
    CompiledExpr, Context, Focus, Item, LimitExceeded, Node, NodeKind, NodeSelection,
};
use einvoice_xslt::NamespaceBinding;

use super::{At, Deferral, Params, Runtime, Slot, State, comment_text, convert, node_context};
use crate::error::{Result, TransformError, err};
use crate::prepare::{
    Binding, CompiledApplyTemplates, CompiledAvt, CompiledCallTemplate, CompiledElement,
    CompiledForEach, CompiledNextMatch, Inst, ModeRef, Value,
};
use crate::sink::{
    SequenceSink, Serialization, Sink, TextSink, TreeSink, attribute_name, copy_node, serialize,
};

/// The index in [`Machine::states`] that stands for the state a run starts in (given by
/// its caller, not owned by the machine).
const BASE: u32 = u32::MAX;

/// The work of a run in progress (see the module): reused from one run to the next.
#[derive(Default)]
pub(super) struct Machine<'e> {
    /// The tasks, innermost last.
    tasks: Vec<Task<'e>>,
    /// The contexts of the template invocations and of the items of `xsl:for-each` in
    /// progress, innermost last.
    states: States,
    /// The `xsl:apply-templates` in progress ([`Task::Apply`]), innermost last.
    applies: Vec<Applying<'e>>,
    /// The built-in rules applying the rules to the children of a node
    /// ([`Task::Walk`]), innermost last.
    walks: Vec<Walking<'e>>,
    /// The `xsl:for-each` in progress ([`Task::Loop`]), innermost last.
    loops: Vec<Looping<'e>>,
    /// The instructions whose content is being collected ([`End::Collect`]), innermost
    /// last: the result of the run goes to the sink of the caller when there is none.
    collects: Vec<Collecting<'e>>,
}

impl<'e> Machine<'e> {
    fn clear(&mut self) {
        self.tasks.clear();
        self.states.clear();
        self.applies.clear();
        self.walks.clear();
        self.loops.clear();
        self.collects.clear();
    }

    /// Pushes `body`, to run in the state at `at`, then `end`.
    fn push_body(&mut self, body: &'e [Inst], at: u32, end: End) {
        self.tasks.push(Task::Body {
            body,
            state: at,
            end,
        });
    }

    /// The lines that the tasks in progress, innermost first, add to an error that ends
    /// the run (see [`Runtime::run`]): the line of the instruction that would add it on
    /// the way out of a recursion.
    fn add_lines(&self, mut e: TransformError) -> TransformError {
        let (mut applies, mut walks, mut collects) =
            (self.applies.len(), self.walks.len(), self.collects.len());
        // The line of what ends a sequence constructor.
        let mut end_line = |end: End| match end {
            End::Return { line, .. } | End::Tail { line, .. } => Some(line),
            End::Collect => {
                collects = collects.saturating_sub(1);
                self.collects.get(collects).and_then(|c| c.what.line())
            }
            End::Nothing | End::Element(_) | End::Document(_) => None,
        };
        for task in self.tasks.iter().rev() {
            let lines = match task {
                Task::Body { end, .. } => [end_line(*end), None],
                Task::Apply => {
                    applies = applies.saturating_sub(1);
                    match self.applies.get(applies) {
                        Some(applying) => [applying.line, end_line(applying.then)],
                        None => [None, None],
                    }
                }
                Task::Walk => {
                    walks = walks.saturating_sub(1);
                    [self.walks.get(walks).and_then(|w| w.line), None]
                }
                Task::Loop => [None, None],
            };
            for line in lines.into_iter().flatten() {
                e = e.at(line);
            }
        }
        e
    }
}

/// The XSLT dynamic context of an invocation or of an item of `xsl:for-each`.
struct Owned {
    context: Context,
    mode: usize,
    rule: Option<u32>,
}

/// The states in progress, innermost last: the first `live` of `slots`. The slots after
/// them hold no item (their contexts are emptied when they are popped) and are reused: a
/// state is written field by field into its slot, not moved in and out of the vector.
#[derive(Default)]
struct States {
    slots: Vec<Owned>,
    live: usize,
}

impl States {
    fn get(&self, at: usize) -> Option<&Owned> {
        self.slots.get(..self.live).and_then(|live| live.get(at))
    }

    /// The index of the next state.
    fn next(&self) -> Result<u32> {
        u32::try_from(self.live)
            .ok()
            .filter(|&at| at != BASE)
            .ok_or_else(|| TransformError::new("INTERNAL", "too many states"))
    }

    /// Pushes a new state: its index.
    fn push(&mut self, context: Context, mode: usize, rule: Option<u32>) -> Result<u32> {
        let at = self.next()?;
        match self.slots.get_mut(self.live) {
            Some(slot) => {
                slot.context = context;
                slot.mode = mode;
                slot.rule = rule;
            }
            None => self.slots.push(Owned {
                context,
                mode,
                rule,
            }),
        }
        self.live += 1;
        Ok(at)
    }

    /// Pushes the state of a template rule applied to `node`, at `position` in a sequence
    /// of `size` nodes: its index.
    fn push_node(
        &mut self,
        node: &Node,
        (position, size): (usize, usize),
        mode: usize,
        rule: u32,
    ) -> Result<u32> {
        let at = self.next()?;
        let Some(slot) = self.slots.get_mut(self.live) else {
            return self.push(node_context(node, position, size), mode, Some(rule));
        };
        slot.context.focus = Some(Focus {
            item: Item::Node(node.clone()),
            position,
            size,
        });
        slot.context.current = Some(Item::Node(node.clone()));
        slot.mode = mode;
        slot.rule = Some(rule);
        self.live += 1;
        Ok(at)
    }

    /// Pops the innermost state, its items dropped.
    fn pop(&mut self) {
        let Some(live) = self.live.checked_sub(1) else {
            return;
        };
        self.live = live;
        if let Some(slot) = self.slots.get_mut(live) {
            slot.context.focus = None;
            slot.context.current = None;
        }
    }

    fn clear(&mut self) {
        self.slots.clear();
        self.live = 0;
    }
}

/// The state at `at` (an index in [`Machine::states`], or [`BASE`]).
fn state_of<'s>(states: &'s States, at: u32, base: &'s State<'_>) -> State<'s> {
    match states.get(at as usize) {
        Some(owned) => State {
            context: &owned.context,
            mode: owned.mode,
            rule: owned.rule,
        },
        None => State {
            context: base.context,
            mode: base.mode,
            rule: base.rule,
        },
    }
}

/// The sink instructions write to: the innermost collecting one, else the run's.
fn current<'s>(collects: &'s mut [Collecting<'_>], root: &'s mut dyn Sink) -> &'s mut dyn Sink {
    match collects.last_mut().map(|c| &mut c.sink) {
        Some(Collector::Text(sink)) => sink,
        Some(Collector::Tree(sink)) => &mut **sink,
        Some(Collector::Sequence(sink)) => sink,
        None => root,
    }
}

/// A sink collecting the content of an instruction.
enum Collector {
    Text(TextSink),
    Tree(Box<TreeSink>),
    Sequence(SequenceSink),
}

/// The content of an instruction being collected, and what it is for.
struct Collecting<'e> {
    what: Collect<'e>,
    sink: Collector,
}

/// The parameters passed to the rules applied to a sequence of nodes, shared by them and
/// by the built-in rules applied below them; `None` when there are none.
type Shared<'e> = Option<Rc<[(&'e QName, Slot<'e>)]>>;

fn shared(params: Params<'_>) -> Shared<'_> {
    (!params.is_empty()).then(|| params.into())
}

fn slice<'p, 'e>(params: &'p Shared<'e>) -> &'p [(&'e QName, Slot<'e>)] {
    params.as_deref().unwrap_or_default()
}

/// The items `xsl:apply-templates` selects: nodes read in place from the tree, or any
/// value.
enum Items {
    Nodes(NodeSelection),
    Values(Vec<Item>),
}

impl Items {
    /// The item at `index`: a node, or an atomic value (`Err`).
    fn node(&self, index: usize) -> Option<std::result::Result<Node, &Item>> {
        match self {
            Self::Nodes(nodes) => nodes.get(index).map(Ok),
            Self::Values(items) => items.get(index).map(|item| match item {
                Item::Node(node) => Ok(node.clone()),
                Item::Atomic(_) => Err(item),
            }),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Nodes(nodes) => nodes.len(),
            Self::Values(items) => items.len(),
        }
    }
}

/// An `xsl:apply-templates` in progress: the rule of each item in turn (`position`
/// items done).
struct Applying<'e> {
    items: Items,
    position: usize,
    mode: usize,
    params: Shared<'e>,
    /// The line of the instruction (none for the rule applied to the document node as the
    /// transformation starts).
    line: Option<u32>,
    /// What ends the sequence constructor the instruction ends, once its rules are done
    /// (see [`Runtime::body_step`]).
    then: End,
}

/// The built-in rule of a document node or an element in progress: the rule of each
/// child in turn, the children counted only if a template rule needs the context size.
struct Walking<'e> {
    parent: Node,
    children: Axis,
    position: usize,
    size: Option<usize>,
    mode: usize,
    params: Shared<'e>,
    /// The line of the instruction that applied the rules (inherited by the built-in
    /// rules below).
    line: Option<u32>,
}

/// An `xsl:for-each` in progress: its body for each item in turn, the state of the item
/// being run (`started`) on top of [`Machine::states`].
struct Looping<'e> {
    items: std::vec::IntoIter<Item>,
    position: usize,
    size: usize,
    body: &'e [Inst],
    mode: usize,
    started: bool,
}

/// What a node gets in a mode: the template of a rule, the built-in rule, or nothing to
/// run (a rule whose template has neither body nor parameters, its invocation counted).
#[derive(Clone, Copy)]
enum Found {
    Template { index: usize, rank: u32 },
    BuiltIn,
    Nothing,
}

enum Task<'e> {
    /// A sequence constructor: `body`, the instructions left, to run in turn in the
    /// state at `state`, then `end`.
    Body {
        body: &'e [Inst],
        state: u32,
        end: End,
    },
    /// The `xsl:apply-templates` on top of [`Machine::applies`].
    Apply,
    /// The built-in rule on top of [`Machine::walks`].
    Walk,
    /// The `xsl:for-each` on top of [`Machine::loops`].
    Loop,
}

/// What ends a sequence constructor, once its instructions and the tasks they started
/// are done.
#[derive(Clone, Copy)]
enum End {
    Nothing,
    /// The end of a template invocation: its frame (`framed`) and its state (`owned`)
    /// popped, its invocation no longer counted.
    Return {
        line: u32,
        framed: bool,
        owned: bool,
    },
    /// The same, for a template with an `xsl:call-template` in tail position: when the
    /// body ends with one, the template it calls runs in place of the template, in the
    /// same state and invocation (`calls` in a row so far), its line and frame those of
    /// the template running.
    Tail {
        line: u32,
        framed: bool,
        owned: bool,
        calls: u32,
    },
    /// The end tag of an element, at the line of its instruction.
    Element(u32),
    /// The end of a document node (`xsl:copy` of one).
    Document(u32),
    /// The content collected on top of [`Machine::collects`], used.
    Collect,
}

/// What the content collected for an instruction is used for.
enum Collect<'e> {
    /// `xsl:value-of` without `select`.
    ValueOf {
        line: u32,
        separator: Option<String>,
    },
    /// `xsl:attribute` without `select`.
    Attribute {
        line: u32,
        name: QName,
    },
    /// `xsl:namespace` without `select`.
    Namespace {
        line: u32,
        prefix: String,
    },
    /// `xsl:processing-instruction` without `select`.
    ProcessingInstruction {
        line: u32,
        target: String,
    },
    Comment {
        line: u32,
    },
    Message {
        line: u32,
        terminate: bool,
    },
    /// A variable whose value is a sequence constructor, computed where it is bound.
    Variable(&'e Binding),
}

impl Collect<'_> {
    /// The line set on the errors raised while the content is computed, if any: that of
    /// the instructions whose content is computed with the line of the instruction set
    /// on its errors.
    const fn line(&self) -> Option<u32> {
        match self {
            Self::Attribute { line, .. }
            | Self::Namespace { line, .. }
            | Self::ProcessingInstruction { line, .. } => Some(*line),
            Self::Variable(binding) => Some(binding.line),
            Self::ValueOf { .. } | Self::Comment { .. } | Self::Message { .. } => None,
        }
    }

    /// The sink that collects the content.
    fn collector(&self) -> Collector {
        match self {
            Self::Variable(binding) if binding.as_type.is_some() => {
                Collector::Sequence(SequenceSink::default())
            }
            Self::Message { .. } | Self::Variable(_) => Collector::Tree(Box::new(TreeSink::new())),
            Self::ValueOf { .. }
            | Self::Attribute { .. }
            | Self::Namespace { .. }
            | Self::ProcessingInstruction { .. }
            | Self::Comment { .. } => Collector::Text(TextSink::new()),
        }
    }
}

impl<'e> Runtime<'e, '_> {
    /// Runs a sequence constructor in `state`, its result written to `sink`.
    pub(super) fn exec(
        &self,
        body: &'e [Inst],
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        // Without an instruction that starts a task, in place.
        if body.iter().all(is_leaf) {
            return self.leaves(body, state, sink);
        }
        self.run(
            |m| {
                m.push_body(body, BASE, End::Nothing);
                Ok(())
            },
            state,
            sink,
        )
    }

    /// Applies the template rules of `mode` to `root`, as a transformation starts.
    pub(super) fn apply_to_root(
        &self,
        root: &Node,
        mode: usize,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let context = Context::new();
        let state = State {
            context: &context,
            mode,
            rule: None,
        };
        let start = |m: &mut Machine<'e>| {
            m.applies.push(Applying {
                items: Items::Values(vec![Item::Node(root.clone())]),
                position: 0,
                mode,
                params: None,
                line: None,
                then: End::Nothing,
            });
            m.tasks.push(Task::Apply);
            Ok(())
        };
        self.run(start, &state, sink)
    }

    /// Runs the tasks `start` pushes and every task they start (see the module).
    fn run(
        &self,
        start: impl FnOnce(&mut Machine<'e>) -> Result<()>,
        base: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        // A run is started inside the evaluation of an expression (a function, a value
        // computed when first read): the call stack grows with them.
        self.check_stack()?;
        let mut machine = self.machines.borrow_mut().pop().unwrap_or_default();
        let frames = self.frames.borrow().len();
        let nesting = self.nesting.get();
        let result = start(&mut machine)
            .and_then(|()| self.drive(&mut machine, base, sink))
            .map_err(|e| {
                self.frames.borrow_mut().truncate(frames);
                self.nesting.set(nesting);
                machine.add_lines(e)
            });
        machine.clear();
        self.machines.borrow_mut().push(machine);
        result
    }

    /// Runs the task on top until none is left.
    fn drive(&self, m: &mut Machine<'e>, base: &State<'_>, root: &mut dyn Sink) -> Result<()> {
        while let Some(task) = m.tasks.last() {
            match task {
                Task::Body { .. } => self.body_step(m, base, root)?,
                Task::Apply => self.apply_step(m, base, root)?,
                Task::Walk => self.walk_step(m, base, root)?,
                Task::Loop => Self::loop_step(m)?,
            }
        }
        Ok(())
    }

    /// The instructions of the body on top, in turn, until one starts a task, which runs
    /// first; then its end. Its last instruction runs in place of the body when nothing
    /// ends it; an `xsl:choose` last runs its branch in place of the body; an
    /// `xsl:apply-templates` last, its task in place of the body, which runs the end of
    /// the body once its rules are done; and in a template run with tail calls, an
    /// `xsl:call-template` last the template it calls in place of the template.
    fn body_step(&self, m: &mut Machine<'e>, base: &State<'_>, root: &mut dyn Sink) -> Result<()> {
        let depth = m.tasks.len();
        'task: loop {
            let Some(&Task::Body {
                body: mut instructions,
                state: at,
                end,
            }) = m.tasks.last()
            else {
                return err("INTERNAL", "no body to run");
            };
            // The instructions left are kept here, and in the task only when it waits
            // for the tasks an instruction pushed.
            loop {
                let Some((inst, rest)) = instructions.split_first() else {
                    m.tasks.pop();
                    return self.end(m, end, root);
                };
                instructions = rest;
                if rest.is_empty() {
                    match (inst, end) {
                        (Inst::Choose(choose), _) => {
                            instructions = self.branch(choose, &state_of(&m.states, at, base))?;
                            continue;
                        }
                        (Inst::CallTemplate(call), End::Tail { .. }) => {
                            self.tail_call(m, call, at, base)?;
                            continue 'task;
                        }
                        // The task of the instruction replaces the body, and runs its
                        // end when its rules are done.
                        (Inst::ApplyTemplates(apply), _) => {
                            m.tasks.pop();
                            return self.apply_instruction(m, apply, at, base, end, root);
                        }
                        (_, End::Nothing) => {
                            m.tasks.pop();
                            return self.instruction(m, inst, at, base, root);
                        }
                        // The end runs once the tasks the instruction starts are done.
                        _ => {}
                    }
                }
                self.instruction(m, inst, at, base, root)?;
                // Instructions only push tasks: the body is still on top when none was
                // pushed.
                if m.tasks.len() != depth {
                    if let Some(Task::Body { body, .. }) =
                        depth.checked_sub(1).and_then(|i| m.tasks.get_mut(i))
                    {
                        *body = instructions;
                    }
                    return Ok(());
                }
            }
        }
    }

    /// What ends a sequence constructor whose task was just popped.
    fn end(&self, m: &mut Machine<'e>, end: End, root: &mut dyn Sink) -> Result<()> {
        match end {
            End::Nothing => Ok(()),
            End::Return { framed, owned, .. } | End::Tail { framed, owned, .. } => {
                if framed {
                    self.pop_frame();
                }
                if owned {
                    m.states.pop();
                }
                self.leave();
                Ok(())
            }
            End::Element(line) => current(&mut m.collects, root).end_element().at(line),
            End::Document(line) => current(&mut m.collects, root).end_document().at(line),
            End::Collect => {
                let Some(Collecting { what, sink }) = m.collects.pop() else {
                    return err("INTERNAL", "content collected without a sink");
                };
                self.collected(what, sink, current(&mut m.collects, root))
            }
        }
    }

    /// The rules of the items of the `xsl:apply-templates` on top, in turn, until one
    /// starts a task, which runs first.
    fn apply_step(&self, m: &mut Machine<'e>, base: &State<'_>, root: &mut dyn Sink) -> Result<()> {
        let depth = m.tasks.len();
        loop {
            let Some(Applying {
                items,
                position,
                mode,
                params,
                line,
                ..
            }) = m.applies.last_mut()
            else {
                return err("INTERNAL", "no xsl:apply-templates to run");
            };
            let Some(next) = items.node(*position) else {
                m.tasks.pop();
                let Some(Applying { items, then, .. }) = m.applies.pop() else {
                    return err("INTERNAL", "no xsl:apply-templates to end");
                };
                if let Items::Nodes(nodes) = items {
                    self.selections.borrow_mut().push(nodes.into_storage());
                }
                return self.end(m, then, root);
            };
            *position += 1;
            let node = match next {
                Ok(node) => node,
                // As Saxon, an XSLT 3.0 processor: no pattern of the subset matches an
                // atomic value, whose built-in rule writes it as text.
                Err(atomic) => {
                    current(&mut m.collects, root).text(&atomic.string_value())?;
                    continue;
                }
            };
            let (position, size, mode) = (*position, items.len(), *mode);
            match self.rule_for(&node, mode, None)? {
                Found::Nothing => continue,
                Found::Template { index, rank } => {
                    let params = params.clone();
                    let at = m.states.push_node(&node, (position, size), mode, rank)?;
                    self.invoke(m, index, (at, true), slice(&params), base)?;
                }
                Found::BuiltIn => {
                    let (params, line) = (params.clone(), *line);
                    self.builtin(m, node, mode, params, line, root)?;
                }
            }
            if m.tasks.len() != depth {
                return self.started(m, base, root);
            }
        }
    }

    /// After a rule started tasks: the body of a template, when the task on top is one,
    /// run at once rather than after a return to [`Runtime::drive`]; a body that is a
    /// single `xsl:apply-templates` (the template rule a compiled Schematron applies to
    /// the nodes no rule of a pattern matches) replaced by the task of the instruction
    /// at once. A body step never runs another step itself: the call stack stays the
    /// same.
    fn started(&self, m: &mut Machine<'e>, base: &State<'_>, root: &mut dyn Sink) -> Result<()> {
        match m.tasks.last() {
            Some(&Task::Body {
                body: [Inst::ApplyTemplates(apply)],
                state: at,
                end,
            }) => {
                m.tasks.pop();
                self.apply_instruction(m, apply, at, base, end, root)
            }
            Some(Task::Body { .. }) => self.body_step(m, base, root),
            _ => Ok(()),
        }
    }

    /// The rules of the children of the built-in rule on top, in turn, until one starts
    /// a task, which runs first.
    fn walk_step(&self, m: &mut Machine<'e>, base: &State<'_>, root: &mut dyn Sink) -> Result<()> {
        let depth = m.tasks.len();
        loop {
            let Some(Walking {
                parent,
                children,
                position,
                size,
                mode,
                params,
                line,
            }) = m.walks.last_mut()
            else {
                return err("INTERNAL", "no built-in rule to run");
            };
            let Some(child) = children.next() else {
                m.tasks.pop();
                m.walks.pop();
                self.leave();
                return Ok(());
            };
            *position += 1;
            let (position, mode) = (*position, *mode);
            match self.rule_for(&child, mode, None)? {
                Found::Nothing => continue,
                Found::Template { index, rank } => {
                    // The children are counted only if a template rule applied to one runs.
                    let size = *size.get_or_insert_with(|| parent.children().count());
                    let params = params.clone();
                    let at = m.states.push_node(&child, (position, size), mode, rank)?;
                    self.invoke(m, index, (at, true), slice(&params), base)?;
                }
                Found::BuiltIn => {
                    let (params, line) = (params.clone(), *line);
                    self.builtin(m, child, mode, params, line, root)?;
                }
            }
            if m.tasks.len() != depth {
                return self.started(m, base, root);
            }
        }
    }

    /// The body of the `xsl:for-each` on top for its next item.
    fn loop_step(m: &mut Machine<'e>) -> Result<()> {
        let Some(Looping {
            items,
            position,
            size,
            body,
            mode,
            started,
        }) = m.loops.last_mut()
        else {
            return err("INTERNAL", "no xsl:for-each to run");
        };
        let next = items.next();
        let started = std::mem::replace(started, true);
        // The state of the previous item ends.
        if started {
            m.states.pop();
        }
        let Some(item) = next else {
            m.tasks.pop();
            m.loops.pop();
            return Ok(());
        };
        *position += 1;
        let context = Context::at(item, *position, *size);
        let (body, mode) = (*body, *mode);
        let at = m.states.push(context, mode, None)?;
        m.push_body(body, at, End::Nothing);
        Ok(())
    }

    /// The rule for `node` in `mode` (ranked after `after`, for `xsl:next-match`).
    fn rule_for(&self, node: &Node, mode: usize, after: Option<u32>) -> Result<Found> {
        let Some(rank) = self.find_rule(mode, node, after)? else {
            return Ok(Found::BuiltIn);
        };
        Ok(match self.rule_template(mode, rank)? {
            Some(index) => Found::Template { index, rank },
            None => Found::Nothing,
        })
    }

    /// The built-in rule for `node`: for a document node or an element, the rules
    /// applied to its children, counted as an invocation ([`Runtime::enter`]) until they
    /// are done; the text of a text node or an attribute.
    fn builtin(
        &self,
        m: &mut Machine<'e>,
        node: Node,
        mode: usize,
        params: Shared<'e>,
        line: Option<u32>,
        root: &mut dyn Sink,
    ) -> Result<()> {
        match node.kind() {
            NodeKind::Document | NodeKind::Element => {
                self.enter()?;
                m.walks.push(Walking {
                    children: node.children(),
                    parent: node,
                    position: 0,
                    size: None,
                    mode,
                    params,
                    line,
                });
                m.tasks.push(Task::Walk);
                Ok(())
            }
            NodeKind::Text | NodeKind::Attribute => {
                current(&mut m.collects, root).text(&node.string_value())
            }
            NodeKind::Comment | NodeKind::ProcessingInstruction => Ok(()),
        }
    }

    /// Invokes a template in the state at `at` (`owned` when the invocation pushed it,
    /// to pop at its end): its parameters bound from `params` or their default values,
    /// its body pushed. Counted as an invocation ([`Runtime::enter`]) until its body is
    /// done.
    ///
    /// A template with an `xsl:call-template` in tail position of its body (its last
    /// instruction, or the last of the branch of an `xsl:choose` or `xsl:if` that is)
    /// runs it as Saxon runs it, in place of the template rather than inside it (Saxon's
    /// tail calls): a template that recurses so, over each character of a string for
    /// instance, takes neither nesting nor tasks in proportion to its recursion. The
    /// frame of an invocation replaced is kept by the parameters it passes that are not
    /// computed yet, until they are ([`Runtime::tail_call`]). At most `max_tail_calls`
    /// in a row.
    fn invoke(
        &self,
        m: &mut Machine<'e>,
        index: usize,
        (at, owned): (u32, bool),
        params: &[(&'e QName, Slot<'e>)],
        base: &State<'_>,
    ) -> Result<()> {
        let template = &self.exe.templates[index];
        if let Err(e) = self.enter() {
            if owned {
                m.states.pop();
            }
            return Err(e.at(template.line));
        }
        let (line, framed) = (template.line, self.push_frame(template.slots));
        let end = if template.tail_calls {
            End::Tail {
                line,
                framed,
                owned,
                calls: 0,
            }
        } else {
            End::Return {
                line,
                framed,
                owned,
            }
        };
        m.push_body(&template.body, at, end);
        if template.params.is_empty() {
            return Ok(());
        }
        self.bind_params(template, params, &state_of(&m.states, at, base))
    }

    /// An `xsl:call-template` in tail position of the body on top, a template run with
    /// tail calls: the template it calls replaces the template, its parameters bound in
    /// a frame of its own, in the same state.
    fn tail_call(
        &self,
        m: &mut Machine<'e>,
        call: &'e CompiledCallTemplate,
        at: u32,
        base: &State<'_>,
    ) -> Result<()> {
        let params = self
            .with_params(&call.params, &state_of(&m.states, at, base))
            .at(call.line)?;
        // The values not computed yet read this frame, which ends now.
        let frame = self.top_frame();
        for (_, slot) in &params {
            if let Slot::Lazy(thunk) = slot {
                thunk.hold.borrow_mut().clone_from(&frame);
            }
        }
        let Some(Task::Body {
            body,
            end:
                End::Tail {
                    line,
                    framed,
                    calls,
                    ..
                },
            ..
        }) = m.tasks.last_mut()
        else {
            return err(
                "INTERNAL",
                "a tail call outside a template run with tail calls",
            );
        };
        // The frame of the template whose body ends.
        if *framed {
            self.pop_frame();
            *framed = false;
        }
        *calls += 1;
        if *calls > self.options.max_tail_calls {
            return Err(TransformError::limit_exceeded(LimitExceeded::TailCalls {
                max: self.options.max_tail_calls,
            }));
        }
        let template = &self.exe.templates[call.template];
        *line = template.line;
        *framed = self.push_frame(template.slots);
        *body = &template.body;
        if template.params.is_empty() {
            return Ok(());
        }
        self.bind_params(template, &params, &state_of(&m.states, at, base))
    }

    /// Runs one instruction: what it does at once, and the tasks it starts. Runs for
    /// every instruction: it dispatches only, each instruction run by a function of its
    /// own, out of line, so that it holds none of their locals.
    fn instruction(
        &self,
        m: &mut Machine<'e>,
        inst: &'e Inst,
        at: u32,
        base: &State<'_>,
        root: &mut dyn Sink,
    ) -> Result<()> {
        match inst {
            Inst::Choose(choose) => {
                let body = self.branch(choose, &state_of(&m.states, at, base))?;
                if !body.is_empty() {
                    m.push_body(body, at, End::Nothing);
                }
                Ok(())
            }
            Inst::ApplyTemplates(apply) => {
                self.apply_instruction(m, apply, at, base, End::Nothing, root)
            }
            Inst::NextMatch(next) => self.next_match(m, next, at, base, root),
            Inst::CallTemplate(call) => self.call_template(m, call, at, base),
            Inst::ForEach(for_each) => self.for_each_instruction(m, for_each, at, base),
            Inst::Element(element) => self.element(m, element, at, base, root),
            Inst::Variable(binding) => self.variable_instruction(m, binding, at, base, root),
            Inst::Sequence { .. } | Inst::CopyOf { .. } | Inst::Text(_) | Inst::Number { .. } => {
                self.leaf(
                    inst,
                    &state_of(&m.states, at, base),
                    current(&mut m.collects, root),
                )
            }
            Inst::Copy { .. } | Inst::ComputedElement { .. } => {
                self.constructed(m, inst, at, base, root)
            }
            Inst::ValueOf { .. }
            | Inst::Attribute { .. }
            | Inst::Namespace { .. }
            | Inst::ProcessingInstruction { .. }
            | Inst::Message { .. }
            | Inst::Comment { .. } => self.text_instruction(m, inst, at, base, root),
        }
    }

    /// `xsl:apply-templates`: its task, if it selects anything, ended by `then`; else
    /// `then` at once.
    #[inline(never)]
    fn apply_instruction(
        &self,
        m: &mut Machine<'e>,
        apply: &'e CompiledApplyTemplates,
        at: u32,
        base: &State<'_>,
        then: End,
        root: &mut dyn Sink,
    ) -> Result<()> {
        let state = state_of(&m.states, at, base);
        if self.apply_templates(apply, &state, then, &mut m.applies)? {
            m.tasks.push(Task::Apply);
            return Ok(());
        }
        self.end(m, then, root)
    }

    /// `xsl:call-template`: the template invoked in the state of the instruction.
    #[inline(never)]
    fn call_template(
        &self,
        m: &mut Machine<'e>,
        call: &'e CompiledCallTemplate,
        at: u32,
        base: &State<'_>,
    ) -> Result<()> {
        let params = self
            .with_params(&call.params, &state_of(&m.states, at, base))
            .at(call.line)?;
        self.invoke(m, call.template, (at, false), &params, base)
            .at(call.line)
    }

    /// `xsl:for-each`: its task, if it selects anything.
    #[inline(never)]
    fn for_each_instruction(
        &self,
        m: &mut Machine<'e>,
        for_each: &'e CompiledForEach,
        at: u32,
        base: &State<'_>,
    ) -> Result<()> {
        if let Some(looping) = self.for_each(for_each, &state_of(&m.states, at, base))? {
            m.loops.push(looping);
            m.tasks.push(Task::Loop);
        }
        Ok(())
    }

    /// A literal result element: its start tag, then its content, in place when no
    /// instruction of it starts a task ([`is_leaf`]), else as a task ended by its end
    /// tag.
    #[inline(never)]
    fn element(
        &self,
        m: &mut Machine<'e>,
        element: &'e CompiledElement,
        at: u32,
        base: &State<'_>,
        root: &mut dyn Sink,
    ) -> Result<()> {
        let state = state_of(&m.states, at, base);
        let sink = current(&mut m.collects, root);
        self.start_element(element, &state, sink)?;
        if element.body.iter().all(is_leaf) {
            self.leaves(&element.body, &state, sink)?;
            return sink.end_element().at(element.line);
        }
        m.push_body(&element.body, at, End::Element(element.line));
        Ok(())
    }

    /// `xsl:copy` and `xsl:element`: the start of the node constructed, then its content
    /// as a task ended by the end of the node.
    #[inline(never)]
    fn constructed(
        &self,
        m: &mut Machine<'e>,
        inst: &'e Inst,
        at: u32,
        base: &State<'_>,
        root: &mut dyn Sink,
    ) -> Result<()> {
        let state = state_of(&m.states, at, base);
        let sink = current(&mut m.collects, root);
        match inst {
            Inst::Copy { line, body } => {
                if let Some(end) = Self::copy(*line, &state, sink)? {
                    m.push_body(body, at, end);
                }
                Ok(())
            }
            Inst::ComputedElement {
                line,
                name,
                namespace,
                namespaces,
                body,
            } => {
                let line = *line;
                let name = self.element_name(line, name, namespace.as_ref(), namespaces, &state)?;
                sink.start_element(&name, &Arc::from(Vec::new())).at(line)?;
                m.push_body(body, at, End::Element(line));
                Ok(())
            }
            _ => err("INTERNAL", "not an instruction that constructs a node"),
        }
    }

    /// Runs an instruction whose value is text, computed from `select` or from its
    /// content: what it writes at once, or its content, collected.
    #[inline(never)]
    fn text_instruction(
        &self,
        m: &mut Machine<'e>,
        inst: &'e Inst,
        at: u32,
        base: &State<'_>,
        root: &mut dyn Sink,
    ) -> Result<()> {
        let state = state_of(&m.states, at, base);
        let (line, select, body, collect) = self.text_parts(inst, &state)?;
        let Some(select) = select else {
            return self.collect(m, collect, body, at, base, root);
        };
        let text = self.selected_text(&collect, line, select, &state)?;
        Self::use_text(collect, &text, current(&mut m.collects, root))
    }

    /// The line of an instruction whose value is text, its `select`, its content, and
    /// what its value is for, its name, separator, prefix or target computed.
    #[expect(
        clippy::type_complexity,
        reason = "the parts of the instruction, as the instruction holds them"
    )]
    fn text_parts(
        &self,
        inst: &'e Inst,
        state: &State<'_>,
    ) -> Result<(u32, &'e Option<Box<CompiledExpr>>, &'e [Inst], Collect<'e>)> {
        Ok(match inst {
            Inst::ValueOf {
                line,
                select,
                separator,
                body,
            } => {
                let separator = match separator {
                    Some(avt) => Some(self.avt(avt, state).at(*line)?),
                    None => None,
                };
                let collect = Collect::ValueOf {
                    line: *line,
                    separator,
                };
                (*line, select, body, collect)
            }
            Inst::Attribute {
                line,
                name,
                namespace,
                namespaces,
                select,
                body,
            } => {
                let name =
                    self.attribute_name(*line, name, namespace.as_ref(), namespaces, state)?;
                (
                    *line,
                    select,
                    body,
                    Collect::Attribute { line: *line, name },
                )
            }
            Inst::Namespace {
                line,
                name,
                select,
                body,
            } => {
                let prefix = self.avt(name, state).at(*line)?;
                let prefix = prefix
                    .trim_matches(einvoice_xpath::is_xml_whitespace)
                    .to_owned();
                (
                    *line,
                    select,
                    body,
                    Collect::Namespace {
                        line: *line,
                        prefix,
                    },
                )
            }
            Inst::ProcessingInstruction {
                line,
                name,
                select,
                body,
            } => {
                let target = self.processing_instruction_target(*line, name, state)?;
                let collect = Collect::ProcessingInstruction {
                    line: *line,
                    target,
                };
                (*line, select, body, collect)
            }
            Inst::Message {
                line,
                terminate,
                body,
            } => {
                let terminate = self.terminates(*line, terminate.as_ref(), state)?;
                (
                    *line,
                    &None,
                    body,
                    Collect::Message {
                        line: *line,
                        terminate,
                    },
                )
            }
            Inst::Comment { line, body } => (*line, &None, body, Collect::Comment { line: *line }),
            _ => return err("INTERNAL", "not an instruction whose value is text"),
        })
    }

    /// Runs an instruction that starts no task ([`is_leaf`]) in `state`, its result
    /// written to `sink`.
    #[inline(never)]
    fn leaf(&self, inst: &'e Inst, state: &State<'_>, sink: &mut dyn Sink) -> Result<()> {
        match inst {
            Inst::Text(text) => sink.text(text),
            Inst::Sequence { line, select } => self.sequence(*line, select, state, sink),
            Inst::CopyOf { line, select } => self.copy_of(*line, select, state, sink),
            Inst::Number { line, level, count } => {
                self.number_instruction(*line, *level, count.as_ref(), state, sink)
            }
            Inst::Element(element) => {
                self.start_element(element, state, sink)?;
                sink.end_element().at(element.line)
            }
            Inst::Variable(binding) => self.bind_variable(binding, state),
            Inst::ValueOf { .. }
            | Inst::Attribute { .. }
            | Inst::Namespace { .. }
            | Inst::ProcessingInstruction { .. } => {
                let (line, select, _, collect) = self.text_parts(inst, state)?;
                let Some(select) = select else {
                    return err("INTERNAL", "an instruction with content run as a leaf");
                };
                let text = self.selected_text(&collect, line, select, state)?;
                Self::use_text(collect, &text, sink)
            }
            _ => err("INTERNAL", "an instruction that starts tasks run as a leaf"),
        }
    }

    /// The text value of an instruction with `select`: the items it selects joined
    /// with spaces, or with the separator of `xsl:value-of`.
    fn selected_text(
        &self,
        collect: &Collect<'e>,
        line: u32,
        select: &'e CompiledExpr,
        state: &State<'_>,
    ) -> Result<String> {
        let items = self.evaluate(select, state.context);
        let mut parts = TextSink::new();
        if let Collect::ValueOf { separator, .. } = collect {
            for item in items.at(line)? {
                parts.item(&item)?;
            }
            return Ok(parts.finish(separator.as_deref().unwrap_or(" ")));
        }
        items
            .and_then(|items| items.iter().try_for_each(|item| parts.item(item)))
            .at(line)?;
        Ok(parts.finish(" "))
    }

    /// Writes the text value of an instruction (not `xsl:message`, nor a variable).
    fn use_text(collect: Collect<'e>, text: &str, sink: &mut dyn Sink) -> Result<()> {
        match collect {
            Collect::ValueOf { line, .. } => sink.text(text).at(line),
            Collect::Attribute { line, name } => sink.attribute(&name, text).at(line),
            Collect::Namespace { line, prefix } => Self::namespace(line, &prefix, text, sink),
            Collect::ProcessingInstruction { line, target } => {
                Self::processing_instruction(line, &target, text, sink)
            }
            Collect::Comment { line } => sink.comment(&comment_text(text)).at(line),
            Collect::Message { .. } | Collect::Variable(_) => {
                err("INTERNAL", "not an instruction whose value is text")
            }
        }
    }

    /// The `xsl:apply-templates` to run, pushed on `applies` if it selects anything:
    /// whether it does.
    fn apply_templates(
        &self,
        apply: &'e CompiledApplyTemplates,
        state: &State<'_>,
        then: End,
        applies: &mut Vec<Applying<'e>>,
    ) -> Result<bool> {
        let CompiledApplyTemplates {
            line,
            ref select,
            mode,
            ref params,
        } = *apply;
        let mode = match mode {
            ModeRef::Mode(m) => m,
            ModeRef::Current => state.mode,
        };
        // A selection of nodes read from the tree (`@*|*`), or any value.
        let items = match self.select_in_place(line, select, state)? {
            Some(nodes) => Items::Nodes(nodes),
            None => Items::Values(self.evaluate(select, state.context).at(line)?),
        };
        let params = self.with_params(params, state).at(line)?;
        if items.len() == 0 {
            if let Items::Nodes(nodes) = items {
                self.selections.borrow_mut().push(nodes.into_storage());
            }
            return Ok(false);
        }
        applies.push(Applying {
            items,
            position: 0,
            mode,
            params: shared(params),
            line: Some(line),
            then,
        });
        Ok(true)
    }

    /// `xsl:next-match`: the next rule for the current node, below the current rule.
    #[inline(never)]
    fn next_match(
        &self,
        m: &mut Machine<'e>,
        next: &'e CompiledNextMatch,
        at: u32,
        base: &State<'_>,
        root: &mut dyn Sink,
    ) -> Result<()> {
        let CompiledNextMatch { line, ref params } = *next;
        let (node, position, size, mode, rank, params) = {
            let state = state_of(&m.states, at, base);
            let Some(rank) = state.rule else {
                return err("XTDE0560", "xsl:next-match without a current template rule").at(line);
            };
            let Some(focus) = &state.context.focus else {
                return err("XTDE0560", "xsl:next-match without a context item").at(line);
            };
            let Item::Node(node) = &focus.item else {
                return err("XTDE0560", "xsl:next-match on an atomic value").at(line);
            };
            let params = self.with_params(params, &state).at(line)?;
            (
                node.clone(),
                focus.position,
                focus.size,
                state.mode,
                rank,
                params,
            )
        };
        match self.rule_for(&node, mode, Some(rank)).at(line)? {
            Found::Nothing => Ok(()),
            Found::Template { index, rank } => {
                let at = m
                    .states
                    .push_node(&node, (position, size), mode, rank)
                    .at(line)?;
                self.invoke(m, index, (at, true), &params, base).at(line)
            }
            Found::BuiltIn => self
                .builtin(m, node, mode, shared(params), Some(line), root)
                .at(line),
        }
    }

    /// The `xsl:for-each` to run, if it selects anything.
    fn for_each(
        &self,
        for_each: &'e CompiledForEach,
        state: &State<'_>,
    ) -> Result<Option<Looping<'e>>> {
        let items = self
            .evaluate(&for_each.select, state.context)
            .at(for_each.line)?;
        if items.is_empty() {
            return Ok(None);
        }
        Ok(Some(Looping {
            size: items.len(),
            items: items.into_iter(),
            position: 0,
            body: &for_each.body,
            mode: state.mode,
            started: false,
        }))
    }

    /// `xsl:copy` of the context item: what ends the node copied, for a document node or
    /// an element, whose content is the body; a copy of the node alone otherwise.
    fn copy(line: u32, state: &State<'_>, sink: &mut dyn Sink) -> Result<Option<End>> {
        let Some(focus) = &state.context.focus else {
            return err("XTTE0945", "xsl:copy without a context item").at(line);
        };
        let Item::Node(node) = &focus.item else {
            sink.item(&focus.item).at(line)?;
            return Ok(None);
        };
        match node.kind() {
            NodeKind::Document => {
                sink.start_document().at(line)?;
                Ok(Some(End::Document(line)))
            }
            NodeKind::Element => {
                let Some(name) = node.name() else {
                    return err("INTERNAL", "element without name");
                };
                // A copied element keeps its namespace nodes.
                let namespaces: Arc<[NamespaceBinding]> = node.namespaces_in_scope().into();
                sink.start_copied_element(name, &namespaces).at(line)?;
                Ok(Some(End::Element(line)))
            }
            // Of the other kinds, a copy of the node alone: the body is not evaluated.
            _ => {
                copy_node(sink, node).at(line)?;
                Ok(None)
            }
        }
    }

    /// Collects the content `body` of an instruction, for `what`: in place when no
    /// instruction of it starts a task ([`is_leaf`]), else as a task.
    fn collect(
        &self,
        m: &mut Machine<'e>,
        what: Collect<'e>,
        body: &'e [Inst],
        at: u32,
        base: &State<'_>,
        root: &mut dyn Sink,
    ) -> Result<()> {
        let mut sink = what.collector();
        if !body.iter().all(is_leaf) {
            m.collects.push(Collecting { what, sink });
            m.push_body(body, at, End::Collect);
            return Ok(());
        }
        let state = state_of(&m.states, at, base);
        let content = match &mut sink {
            Collector::Text(text) => self.leaves(body, &state, text),
            Collector::Tree(tree) => self.leaves(body, &state, &mut **tree),
            Collector::Sequence(sequence) => self.leaves(body, &state, sequence),
        };
        match what.line() {
            Some(line) => content.at(line)?,
            None => content?,
        }
        self.collected(what, sink, current(&mut m.collects, root))
    }

    /// Runs a sequence constructor whose instructions start no task ([`is_leaf`]).
    fn leaves(&self, body: &'e [Inst], state: &State<'_>, sink: &mut dyn Sink) -> Result<()> {
        body.iter()
            .try_for_each(|inst| self.leaf(inst, state, sink))
    }

    /// Uses the content collected for an instruction.
    fn collected(
        &self,
        what: Collect<'e>,
        collector: Collector,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        match (what, collector) {
            (Collect::Message { line, terminate }, Collector::Tree(content)) => {
                self.message(line, terminate, *content)
            }
            (Collect::Variable(binding), collector) => {
                let line = binding.line;
                let items = match collector {
                    Collector::Tree(tree) => vec![Item::Node(tree.finish().at(line)?.root())],
                    Collector::Sequence(sequence) => sequence.items,
                    Collector::Text(_) => {
                        return err("INTERNAL", "a variable collected as text");
                    }
                };
                let items = match &binding.as_type {
                    Some(t) => convert(items, t, "XTTE0570").at(line)?,
                    None => items,
                };
                self.set_slot(binding.slot, Slot::Value(items.into()))
                    .at(line)
            }
            (what, Collector::Text(parts)) => {
                let text = match &what {
                    Collect::ValueOf { separator, .. } => {
                        parts.finish(separator.as_deref().unwrap_or(""))
                    }
                    _ => parts.finish(""),
                };
                Self::use_text(what, &text, sink)
            }
            _ => err("INTERNAL", "content collected by the wrong sink"),
        }
    }

    /// The name of `xsl:attribute`.
    fn attribute_name(
        &self,
        line: u32,
        name: &'e CompiledAvt,
        namespace: Option<&'e CompiledAvt>,
        namespaces: &[NamespaceBinding],
        state: &State<'_>,
    ) -> Result<QName> {
        let name = self.avt(name, state).at(line)?;
        let namespace = match namespace {
            Some(avt) => Some(self.avt(avt, state).at(line)?),
            None => None,
        };
        attribute_name(&name, namespace.as_deref(), namespaces).at(line)
    }

    /// `xsl:namespace`: a namespace node on the element being constructed (XSLT 2.0
    /// section 11.7).
    fn namespace(line: u32, prefix: &str, uri: &str, sink: &mut dyn Sink) -> Result<()> {
        if !(prefix.is_empty() || einvoice_xpath::is_ncname(prefix)) || prefix == "xmlns" {
            return err(
                "XTDE0920",
                format!("invalid namespace prefix {prefix:?} in xsl:namespace"),
            )
            .at(line);
        }
        let xml = einvoice_xpath::XML_NAMESPACE;
        if (prefix == "xml") != (uri == xml) {
            return err(
                "XTDE0925",
                "the xml prefix and the XML namespace are bound to each other only",
            )
            .at(line);
        }
        if uri.is_empty() {
            return err("XTDE0930", "xsl:namespace with an empty namespace URI").at(line);
        }
        if prefix == "xml" {
            return Ok(());
        }
        sink.namespace((!prefix.is_empty()).then_some(prefix), uri)
            .at(line)
    }

    /// The target of `xsl:processing-instruction` (XSLT 2.0 section 11.6): an `NCName`
    /// other than `xml` in any case.
    fn processing_instruction_target(
        &self,
        line: u32,
        name: &'e CompiledAvt,
        state: &State<'_>,
    ) -> Result<String> {
        let target = self.avt(name, state).at(line)?;
        let target = target.trim_matches(einvoice_xpath::is_xml_whitespace);
        if !einvoice_xpath::is_ncname(target) || target.eq_ignore_ascii_case("xml") {
            return err(
                "XTDE0890",
                format!("invalid processing instruction name {target:?}"),
            )
            .at(line);
        }
        Ok(target.to_owned())
    }

    /// `xsl:processing-instruction`: the value without leading whitespace, `?>` written
    /// `? >`.
    fn processing_instruction(
        line: u32,
        target: &str,
        value: &str,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let value = value
            .trim_start_matches(einvoice_xpath::is_xml_whitespace)
            .replace("?>", "? >");
        sink.processing_instruction(target, &value).at(line)
    }

    /// Whether `xsl:message` terminates the transformation (its `terminate` attribute).
    fn terminates(
        &self,
        line: u32,
        terminate: Option<&'e CompiledAvt>,
        state: &State<'_>,
    ) -> Result<bool> {
        let Some(avt) = terminate else {
            return Ok(false);
        };
        let value = self.avt(avt, state).at(line)?;
        match value.trim_matches(einvoice_xpath::is_xml_whitespace) {
            "yes" | "true" | "1" => Ok(true),
            "no" | "false" | "0" => Ok(false),
            other => err(
                "XTDE0030",
                format!("invalid value {other:?} of xsl:message/@terminate"),
            )
            .at(line),
        }
    }

    /// `xsl:message`: its content, a document node, written as Saxon writes messages
    /// (see [`Transformation::messages`](crate::Transformation::messages)); with
    /// `terminate` `yes`, the transformation ends with the error `XTMM9000`.
    fn message(&self, line: u32, terminate: bool, content: TreeSink) -> Result<()> {
        let document = content.finish()?;
        let root = document.root();
        let text = if root.children().next().is_some() {
            let settings = Serialization {
                indent: false,
                omit_xml_declaration: true,
                standalone: None,
            };
            let text = serialize(&root, settings, true).at(line)?;
            self.messages.borrow_mut().push(text.clone());
            text
        } else {
            String::new()
        };
        if terminate {
            return err("XTMM9000", text).at(line);
        }
        Ok(())
    }

    /// `xsl:variable`: its value bound in the frame, computed now or later (see
    /// [`Eval`](crate::prepare::Eval)); a sequence constructor computed now is collected,
    /// the variable bound when it is done.
    #[inline(never)]
    fn variable_instruction(
        &self,
        m: &mut Machine<'e>,
        binding: &'e Binding,
        at: u32,
        base: &State<'_>,
        root: &mut dyn Sink,
    ) -> Result<()> {
        if !binding.used {
            return Ok(());
        }
        let state = state_of(&m.states, at, base);
        match (self.deferral(binding.eval), &binding.value) {
            (Deferral::Now, Value::Content(body)) => {
                self.collect(m, Collect::Variable(binding), body, at, base, root)
            }
            (deferral, _) => self.bind_local(binding, deferral, &state),
        }
    }

    /// `xsl:variable` whose value is not a sequence constructor computed now ([`is_leaf`]).
    fn bind_variable(&self, binding: &'e Binding, state: &State<'_>) -> Result<()> {
        if !binding.used {
            return Ok(());
        }
        let deferral = self.deferral(binding.eval);
        if let (Deferral::Now, Value::Content(_)) = (&deferral, &binding.value) {
            return err("INTERNAL", "a variable with content run as a leaf");
        }
        self.bind_local(binding, deferral, state)
    }

    /// Binds a variable whose value is computed now, not from a sequence constructor, or
    /// later.
    fn bind_local(
        &self,
        binding: &'e Binding,
        deferral: Deferral,
        state: &State<'_>,
    ) -> Result<()> {
        let line = binding.line;
        let as_type = binding.as_type.as_ref();
        let slot = match deferral {
            Deferral::Now => Slot::Value(
                self.value(&binding.value, as_type, state, "XTTE0570")
                    .at(line)?,
            ),
            Deferral::Later(report) => {
                self.thunk(&binding.value, as_type, state, "XTTE0570", report)
            }
        };
        self.set_slot(binding.slot, slot).at(line)
    }
}

/// Whether an instruction starts no task: it runs in place, in a sequence constructor
/// run in place ([`Runtime::leaves`]). Literal text, `xsl:sequence`, `xsl:copy-of`,
/// `xsl:number`, a literal result element without content, an instruction whose value
/// is text computed from `select`, a variable whose value is not a sequence constructor.
fn is_leaf(inst: &Inst) -> bool {
    match inst {
        Inst::Text(_) | Inst::Sequence { .. } | Inst::CopyOf { .. } | Inst::Number { .. } => true,
        Inst::Element(element) => element.body.is_empty(),
        Inst::ValueOf { select, .. }
        | Inst::Attribute { select, .. }
        | Inst::Namespace { select, .. }
        | Inst::ProcessingInstruction { select, .. } => select.is_some(),
        Inst::Variable(binding) => !matches!(binding.value, Value::Content(_)),
        Inst::ApplyTemplates(_)
        | Inst::NextMatch(_)
        | Inst::CallTemplate(_)
        | Inst::Choose(_)
        | Inst::ForEach(_)
        | Inst::Copy { .. }
        | Inst::ComputedElement { .. }
        | Inst::Message { .. }
        | Inst::Comment { .. } => false,
    }
}
