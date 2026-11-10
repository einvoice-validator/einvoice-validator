//! Running an [`Executable`] on a document.
//!
//! Template rules are chosen as Saxon-HE chooses them: highest priority first, the last
//! declared on a tie (an ambiguity is reported as a warning, as Saxon does), then the
//! built-in rules of XSLT 2.0 section 6.6. A dynamic error while matching a pattern is
//! recovered as Saxon recovers it: a warning, and the rule does not match. Variables live in frames, one per template,
//! function or global variable invocation; the XPath evaluator reads them through the
//! [`Host`] interface, and calls stylesheet functions through it too.
//!
//! Evaluation order follows Saxon-HE 12.10 where it is observable (which dynamic errors
//! are raised): global variables are evaluated when first referenced; a local variable
//! that nothing refers to is never evaluated; one referred to once, not in a loop, is
//! evaluated when the reference is (Saxon inlines it); a value that depends on the
//! position, size or current item of its focus is evaluated where it is bound, and so are
//! the parameters `xsl:apply-templates` and `xsl:next-match` pass; any other local
//! variable, template parameter default or parameter `xsl:call-template` passes is
//! evaluated as Saxon's `LearningEvaluator` evaluates it: when first referenced, until
//! it has been evaluated 20 times with its value computed each time (in a function body,
//! 20 times at all), then where it is bound (see [`Eval`]). Saxon keeps these counts in
//! the compiled stylesheet, over every document it transforms with it; they are kept
//! here for one transformation, as Saxon on a stylesheet compiled for one document.
//! Arguments of stylesheet functions are evaluated before the call (Saxon evaluates them
//! lazily too).

use std::borrow::Cow;
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::sync::Arc;

use einvoice_xdm::{AtomicType, AtomicValue, DateTime};
use einvoice_xpath::{AtomicKind, ItemType, Occurrence, QName, SequenceType};
use einvoice_xpath_eval::{
    Argument, Clock, CompiledExpr, Context, DEFAULT_MAX_DEPTH, Document, DocumentError,
    DynamicContext, ErrorCode, FunctionId, Host, Item, LimitExceeded, Node, NodeAxis, NodeKind,
    NodeSelection, SelectionStorage, StackLimit, VariableId,
};
use einvoice_xslt::{NamespaceBinding, NumberLevel};

use crate::error::{LIMIT, Result, TransformError, err};
use crate::output::{CompactBuilder, CompactTransformation, ResultObserver};
use crate::pattern::{self, Matcher};
use crate::prepare::{
    AvtItem, Binding, CompiledApplyTemplates, CompiledAvt, CompiledCallTemplate, CompiledChoose,
    CompiledElement, CompiledForEach, CompiledFunction, CompiledNextMatch, CompiledTemplate,
    CountPattern, Eval, Executable, Inst, ModeRef, Plan, Value, WithParam, kind_index,
    rule_name_number,
};
use crate::sink::{
    Compact, SequenceSink, Serialization, Sink, Target, TextSink, TreeSink, WholeText, XmlWriter,
    attribute_name, copy_node, element_name, serialize,
};

/// Fetches the documents that `document()` asks for, by absolute URI. The engine never
/// reads files or the network itself.
pub trait DocumentResolver {
    /// The text of the document at `uri`, or `None` when it cannot be retrieved.
    fn resolve(&self, uri: &str) -> Option<String>;
}

/// The maximum nesting of invocations in a transformation, unless the caller sets another
/// one ([`TransformOptions::max_nesting`]): template rules, built-in template rules, named
/// templates and stylesheet functions invoked one inside another. It is
/// [`max_nesting_for_depth`] of [`DEFAULT_MAX_DEPTH`]: 576.
pub const DEFAULT_MAX_NESTING: u32 = max_nesting_for_depth(DEFAULT_MAX_DEPTH);

/// The most tail calls in a row in a transformation, unless the caller sets another
/// maximum ([`TransformOptions::max_tail_calls`]): 65 536. A template that ends with an
/// `xsl:call-template` runs the template it calls in its place, as Saxon does, without
/// stack in proportion to the recursion; the ISO Schematron skeleton recurses so over
/// each character of every XPath expression of a schema (8519 for the longest of the CEN
/// rules).
pub const DEFAULT_MAX_TAIL_CALLS: u32 = 1 << 16;

/// The nesting of invocations that the official stylesheets need on a document
/// `max_depth` deep, with room to spare: twice the depth, plus 64.
///
/// A compiled Schematron applies the template rules of each pattern to every element, one
/// element level inside another, so the nesting reaches the depth of the document; where
/// a rule fires, the ISO Schematron skeleton writes the location of the node with a
/// template that recurses over its ancestors (mode `schematron-get-full-path`), which
/// nests as deep again. The 64 levels above are for the few invocations around them
/// (the root template, the next rule in a chain) and for stylesheets that recurse
/// otherwise. On a document [`DEFAULT_MAX_DEPTH`] deep, this nesting fits in the 2 MiB
/// stack a thread is given by default, in a debug build too.
#[must_use]
pub const fn max_nesting_for_depth(max_depth: u32) -> u32 {
    max_depth.saturating_mul(2).saturating_add(64)
}

/// Options of a transformation.
pub struct TransformOptions<'r> {
    /// The implicit timezone in minutes east of UTC (Saxon takes the JVM's). Default: UTC.
    pub implicit_timezone: Option<i16>,
    /// The value of `fn:current-dateTime()`, with a timezone. Default: the system clock,
    /// read once per transformation; required on targets without one
    /// (`wasm32-unknown-unknown`).
    pub current_date_time: Option<DateTime>,
    /// Values of the global parameters (`xsl:param`), by name.
    pub parameters: Vec<(QName, Vec<Item>)>,
    /// Where `document()` gets documents other than the source document.
    pub resolver: Option<&'r dyn DocumentResolver>,
    /// The maximum element depth of the source document and of the documents `document()`
    /// loads (the document element is at depth 1): a document with an element nested
    /// deeper is refused with a `LIMIT` error ([`TransformError::limit`]). Default:
    /// [`DEFAULT_MAX_DEPTH`].
    pub max_depth: u32,
    /// The maximum nesting of template rules, built-in template rules, named templates
    /// and stylesheet functions invoked one inside another: beyond it, the transformation
    /// ends with a `LIMIT` error ([`TransformError::limit`]) instead of overflowing the
    /// stack. Default: [`DEFAULT_MAX_NESTING`]. A larger value needs a larger stack.
    pub max_nesting: u32,
    /// The most stack, in bytes, the invocations in progress and the expressions they
    /// evaluate may use, measured from the start of the transformation: beyond it, the
    /// transformation ends with a `LIMIT` error ([`LimitExceeded::Stack`]) instead of
    /// overflowing the stack. It bounds what the nesting alone cannot, the stack of each
    /// level, which depends on the stylesheet: a function calling itself from a nested
    /// expression needs much more than a template rule applied to a child. The thread
    /// must have this much stack and some more, for its caller and for the deepest level.
    /// Default: [`DEFAULT_MAX_STACK`].
    pub max_stack: u32,
    /// The most tail calls in a row (`xsl:call-template` at the end of a template, run in
    /// its place): beyond it, the transformation ends with a `LIMIT` error
    /// ([`LimitExceeded::TailCalls`]) rather than run for ever. Default:
    /// [`DEFAULT_MAX_TAIL_CALLS`].
    pub max_tail_calls: u32,
}

/// The default of [`TransformOptions::max_stack`]: 1.5 MiB, for a thread with the
/// default 2 MiB stack of a spawned thread; 256 KiB on WebAssembly. With it, the official
/// stylesheets run on a document [`DEFAULT_MAX_DEPTH`] deep, in a debug build too, and
/// the 512 KiB left are more than the caller and one level of invocation need.
///
/// A level of recursion through the templates (a named template, a template rule
/// applied to a child, a built-in rule) needs less than 3 KiB of stack, in a debug build
/// too, on Linux, macOS and Windows (`tests/limits.rs` checks it): the default holds the
/// 512 levels the Peppol stylesheets nest on a document [`DEFAULT_MAX_DEPTH`] deep (two
/// per element level), and more than 550 levels of named templates. Measured on x86-64
/// Linux, a debug build needs 1.2 KiB per named template, 2.1 KiB per template rule; a
/// release build 0.6 and 1.3 KiB. A level of stylesheet function, evaluated through the
/// XPath evaluator, needs several times more: a recursion through functions may end at
/// the stack limit before the nesting limit.
///
/// On WebAssembly, the stack measured is the program's stack in linear memory (1 MiB),
/// but what runs out first is the stack of the engine running the code, which the
/// program cannot measure: 512 KiB by default in wasmtime. A recursion stopped at 256 KiB
/// of the former uses at most 380 KB of the latter (measured on the `wasm-validate`
/// example of `einvoice-validator`, a function calling itself without end), and the
/// official stylesheets need less than half of it on a document [`DEFAULT_MAX_DEPTH`]
/// deep.
#[cfg(not(target_family = "wasm"))]
pub const DEFAULT_MAX_STACK: u32 = 3 << 19;

/// The default of [`TransformOptions::max_stack`] (see the other targets).
#[cfg(target_family = "wasm")]
pub const DEFAULT_MAX_STACK: u32 = 256 << 10;

impl Default for TransformOptions<'_> {
    fn default() -> Self {
        Self {
            implicit_timezone: None,
            current_date_time: None,
            parameters: Vec::new(),
            resolver: None,
            max_depth: DEFAULT_MAX_DEPTH,
            max_nesting: DEFAULT_MAX_NESTING,
            max_stack: DEFAULT_MAX_STACK,
            max_tail_calls: DEFAULT_MAX_TAIL_CALLS,
        }
    }
}

impl std::fmt::Debug for TransformOptions<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransformOptions")
            .field("implicit_timezone", &self.implicit_timezone)
            .field("current_date_time", &self.current_date_time)
            .field("parameters", &self.parameters)
            .field("resolver", &self.resolver.map(|_| "..."))
            .field("max_depth", &self.max_depth)
            .field("max_nesting", &self.max_nesting)
            .field("max_stack", &self.max_stack)
            .field("max_tail_calls", &self.max_tail_calls)
            .finish()
    }
}

/// The result of a transformation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transformation {
    /// The result document, serialized as `xsl:output` asks.
    pub output: String,
    /// Recoverable conditions reported as warnings, as Saxon reports them, each
    /// distinct message once in the order first reported, as Saxon's standard error
    /// reporter prints them: ambiguous template rule matches (`XTDE0540: ...`), and
    /// dynamic errors raised while matching a pattern, after which the node is taken
    /// not to match (`line <L>: <CODE>: An error occurred matching pattern {...}:
    /// <message>`, `L` the line of the `xsl:template` or `xsl:number`).
    pub warnings: Vec<String>,
    /// The messages of `xsl:message`, in the order they were made, each its content
    /// written as Saxon-HE 12.10's message handler writes it: as XML without
    /// declaration nor indentation (text escaped, elements as tags). A message without
    /// content is not kept, as Saxon prints nothing for it. The message of an
    /// `xsl:message` that terminates is the error's ([`TransformError::message`], code
    /// `XTMM9000`).
    pub messages: Vec<String>,
}

/// The result of [`Executable::transform_tree`]: the result tree.
#[derive(Debug, Clone)]
pub struct TreeTransformation {
    /// The result document (its nodes without base URI).
    pub document: Rc<Document>,
    /// The warnings, as [`Transformation::warnings`].
    pub warnings: Vec<String>,
    /// The messages, as [`Transformation::messages`].
    pub messages: Vec<String>,
}

impl Executable {
    /// Runs the stylesheet on a document: `xsl:apply-templates` on its document node in
    /// the default mode, the result serialized. `base_uri` is the URI of the document:
    /// the base URI of its nodes, and the URI under which `document()` loads it again. As
    /// in Saxon-HE 12.10, it is not its document URI: `document-uri(/)` is empty.
    ///
    /// The source text can be borrowed (`&str`, `&String`) or given (`String`), as for
    /// [`Document::parse`]: borrowed, the texts of its nodes are copied into the tree;
    /// given, the tree keeps the text and refers to it, so that the document is in memory
    /// once. A given text is kept as a copy only for a stylesheet that calls
    /// `document()` and a document with a `base_uri`, where `document()` may load the
    /// document again from its text.
    ///
    /// # Errors
    ///
    /// The document is not well-formed (or has a document type declaration), or the
    /// transformation raised a dynamic error.
    pub fn transform<'s>(
        &self,
        source: impl Into<Cow<'s, str>>,
        base_uri: Option<&str>,
    ) -> std::result::Result<Transformation, TransformError> {
        self.transform_with(source, base_uri, &TransformOptions::default())
    }

    /// [`Executable::transform`] with options.
    ///
    /// # Errors
    ///
    /// As [`Executable::transform`], and an invalid timezone or current date and time
    /// (or none given on a target without a system clock).
    pub fn transform_with<'s>(
        &self,
        source: impl Into<Cow<'s, str>>,
        base_uri: Option<&str>,
        options: &TransformOptions<'_>,
    ) -> std::result::Result<Transformation, TransformError> {
        let ((output, WholeText), warnings, messages) =
            self.run(source, base_uri, options, WholeText)?;
        Ok(Transformation {
            output,
            warnings,
            messages,
        })
    }

    /// Runs the stylesheet on a tree (`source`, a document node) and gives the result
    /// tree instead of its serialization, for a transformation whose result is read by
    /// another one. `document()` loads every document through the resolver of the
    /// options, the source's own URI included.
    ///
    /// # Errors
    ///
    /// The source is not a document node, or the transformation raised a dynamic error;
    /// as [`Executable::transform_with`] otherwise.
    pub fn transform_tree(
        &self,
        source: &Node,
        options: &TransformOptions<'_>,
    ) -> std::result::Result<TreeTransformation, TransformError> {
        if source.kind() != NodeKind::Document {
            return err(
                "XTDE0040",
                "the source of a tree transformation is a document node",
            );
        }
        let mut sink = TreeSink::new();
        let (warnings, messages) = self.execute(source, None, None, options, &mut sink)?;
        Ok(TreeTransformation {
            document: sink.finish()?,
            warnings,
            messages,
        })
    }

    /// The serialization parameters of the stylesheet's `xsl:output`.
    #[must_use]
    pub const fn serialization(&self) -> Serialization {
        self.serialization
    }

    /// [`Executable::transform_with`], the result kept as a
    /// [`CompactOutput`](crate::CompactOutput) instead of a string, and read as it is
    /// written by `observer`: a large result in less memory, read without parsing its
    /// text.
    ///
    /// # Errors
    ///
    /// As [`Executable::transform_with`].
    pub fn transform_compact<'s>(
        &self,
        source: impl Into<Cow<'s, str>>,
        base_uri: Option<&str>,
        options: &TransformOptions<'_>,
        observer: &mut dyn ResultObserver,
    ) -> std::result::Result<CompactTransformation, TransformError> {
        let target = Compact {
            builder: CompactBuilder::default(),
            observer,
        };
        // The pieces are all taken: nothing is left in the writer's buffer.
        let ((_, target), warnings, messages) = self.run(source, base_uri, options, target)?;
        let mut output = target.builder.finish();
        // The source tree is dropped: the room left in the buffers is released.
        output.shrink_to_fit();
        Ok(CompactTransformation {
            output,
            warnings,
            messages,
        })
    }

    /// Runs the stylesheet, its result written to `target`: what
    /// [`XmlWriter::finish`] returns, the warnings and the messages.
    #[expect(
        clippy::type_complexity,
        reason = "the writer's result, the warnings and the messages"
    )]
    fn run<'s, T: Target>(
        &self,
        source: impl Into<Cow<'s, str>>,
        base_uri: Option<&str>,
        options: &TransformOptions<'_>,
        target: T,
    ) -> std::result::Result<((String, T), Vec<String>, Vec<String>), TransformError> {
        let source = source.into();
        // The text `document()` parses again when asked for the source document.
        let source_text = match &source {
            Cow::Borrowed(text) => Some(Cow::Borrowed(*text)),
            Cow::Owned(text) if self.calls_document && base_uri.is_some() => {
                Some(Cow::Owned(text.clone()))
            }
            Cow::Owned(_) => None,
        };
        let document = Document::parse_with_max_depth(source, base_uri, options.max_depth)
            .map_err(|e| match e.limit() {
                Some(limit) => TransformError::limit_exceeded(limit),
                None => TransformError::new("SXXP0003", e.message()),
            })?;
        let mut writer = XmlWriter::new(self.serialization, target);
        let (warnings, messages) = self.execute(
            &document.root(),
            source_text,
            base_uri,
            options,
            &mut writer,
        )?;
        let finished = writer.finish()?;
        Ok((finished, warnings, messages))
    }

    /// Runs the stylesheet on the tree of `root`, its result written to `sink`: the
    /// warnings and the messages. `source_text` and `source_uri` are the text of the
    /// source and its URI, for `document()` asked for the source document by its URI.
    fn execute<'a>(
        &self,
        root: &Node,
        source_text: Option<Cow<'a, str>>,
        source_uri: Option<&'a str>,
        options: &'a TransformOptions<'a>,
        sink: &mut dyn Sink,
    ) -> std::result::Result<(Vec<String>, Vec<String>), TransformError> {
        let runtime = Runtime {
            exe: self,
            source_root: root.clone(),
            source_text: RefCell::new(source_text),
            source_uri,
            options,
            dc: Cell::new(None),
            globals: (0..self.globals.len())
                .map(|_| RefCell::new(GlobalState::Pending))
                .collect(),
            frames: RefCell::new(Vec::new()),
            documents: RefCell::new(Vec::new()),
            warnings: RefCell::new(Vec::new()),
            warned: RefCell::new(HashSet::new()),
            messages: RefCell::new(Vec::new()),
            stashed: RefCell::new(None),
            nesting: Cell::new(0),
            stack: StackLimit::from_here(options.max_stack),
            learners: (0..self.learners).map(|_| Rc::default()).collect(),
            argument_learners: RefCell::new(HashMap::new()),
            rule_name_numbers: RefCell::new(Vec::new()),
            selections: RefCell::new(Vec::new()),
        };
        let timezone = options.implicit_timezone.unwrap_or(0);
        let clock = match options.current_date_time {
            Some(now) => Clock::new(timezone, now)?,
            None => system_clock(timezone)?,
        };
        let dc = DynamicContext::new(&runtime, clock)
            .with_stack_limit(runtime.stack)
            .with_regex_cache(&self.regexes);
        runtime.dc.set(Some(&dc));
        runtime.apply_rule(root, 1, &Size::Known(1), self.default_mode, None, &[], sink)?;
        let warnings = runtime.warnings.take();
        let messages = runtime.messages.take();
        Ok((warnings, messages))
    }
}

/// The system clock, in the implicit timezone.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
fn system_clock(timezone: i16) -> Result<Clock> {
    Ok(Clock::system(timezone)?)
}

/// No system clock on this target: the caller must give the current date and time.
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
fn system_clock(_timezone: i16) -> Result<Clock> {
    Err(einvoice_xpath_eval::Error::new(
        ErrorCode::Unsupported,
        "no system clock on this target: give TransformOptions::current_date_time",
    )
    .into())
}

#[derive(Debug)]
enum GlobalState {
    Pending,
    Evaluating,
    Done(Rc<[Item]>),
}

/// A value computed when first read.
struct Thunk<'e> {
    value: &'e Value,
    as_type: Option<&'e SequenceType>,
    code: &'static str,
    context: Context,
    /// The mode and the current template rule, for a sequence constructor.
    mode: usize,
    rule: Option<u32>,
    /// The frame the expression's variables are read from.
    frame: Option<Weak<Frame<'e>>>,
    memo: OnceCell<Rc<[Item]>>,
    /// The learner to tell when the value is computed ([`Eval::Learning`]).
    report: Option<Rc<Learner>>,
    /// The frame, kept alive until the value is computed: for a parameter of a tail call
    /// (see [`Runtime::invoke`]), whose frame ends before the template it calls.
    hold: RefCell<Option<Rc<Frame<'e>>>>,
}

/// Dropping a value kept by the parameters of tail calls drops the frames they keep, and
/// the values not computed in those frames, and so on: a chain as long as the tail calls
/// (a parameter passed down a recursion and never read). It is dropped here in a loop,
/// not by a recursion of drops as deep as the chain.
impl Drop for Thunk<'_> {
    fn drop(&mut self) {
        let mut frames: Vec<Rc<Frame<'_>>> = self.hold.get_mut().take().into_iter().collect();
        while let Some(frame) = frames.pop() {
            // A frame still read elsewhere is not dropped here.
            let Ok(frame) = Rc::try_unwrap(frame) else {
                continue;
            };
            for slot in frame.slots.into_inner() {
                if let Slot::Lazy(thunk) = slot
                    && let Ok(mut thunk) = Rc::try_unwrap(thunk)
                {
                    frames.extend(thunk.hold.get_mut().take());
                }
            }
        }
    }
}

/// The argument of a stylesheet function call, computed when first read, in the frame
/// of the caller.
struct ArgumentThunk<'e> {
    argument: Argument,
    param: &'e Binding,
    function: &'e CompiledFunction,
    frame: Option<Weak<Frame<'e>>>,
    memo: OnceCell<Rc<[Item]>>,
    report: Option<Rc<Learner>>,
}

/// The counters of Saxon's `LearningEvaluator` for one binding ([`Eval::Learning`]).
#[derive(Debug, Default)]
struct Learner {
    /// Evaluations so far.
    count: Cell<u32>,
    /// Evaluations whose value was computed.
    completed: Cell<u32>,
    /// Whether the binding is now evaluated where it is bound.
    eager: Cell<bool>,
}

/// Saxon's `LearningEvaluator`: the evaluations whose completion counts, and how many
/// completed ones make the binding eager.
const LEARNING_LIMIT: u32 = 40;
const EAGER_AFTER: u32 = 20;

impl Learner {
    /// One more evaluation: `None` when the binding is now evaluated where it is bound,
    /// else whether the completion of this evaluation is to be reported
    /// ([`Learner::completed`]). With `every_counts` (a function body), each evaluation
    /// counts as completed.
    fn evaluation(&self, every_counts: bool) -> Option<bool> {
        if self.eager.get() {
            return None;
        }
        let count = self.count.get().saturating_add(1);
        self.count.set(count);
        if every_counts {
            if count >= EAGER_AFTER {
                self.eager.set(true);
            }
            return Some(false);
        }
        Some(count <= LEARNING_LIMIT)
    }

    /// The value of an evaluation was computed (`LearningEvaluator.reportCompletion`):
    /// eager from now on once 20 evaluations have, and all so far.
    fn completed(&self) {
        let completed = self.completed.get().saturating_add(1);
        self.completed.set(completed);
        if completed >= EAGER_AFTER && completed == self.count.get() {
            self.eager.set(true);
        }
    }
}

#[derive(Clone)]
enum Slot<'e> {
    Unset,
    Value(Rc<[Item]>),
    Lazy(Rc<Thunk<'e>>),
    Argument(Rc<ArgumentThunk<'e>>),
}

/// The variables of one invocation.
struct Frame<'e> {
    slots: RefCell<Vec<Slot<'e>>>,
}

impl Frame<'_> {
    fn new(slots: usize) -> Rc<Self> {
        Rc::new(Self {
            slots: RefCell::new(vec![Slot::Unset; slots]),
        })
    }
}

/// The XSLT part of the dynamic context of an instruction.
struct State<'c> {
    context: &'c Context,
    mode: usize,
    /// The rank of the current template rule in `mode`, if any.
    rule: Option<u32>,
}

type Params<'e> = Vec<(&'e QName, Slot<'e>)>;

/// An `xsl:call-template` in tail position, its parameters bound: the template to run in
/// place of the one whose body it ends.
struct TailCall<'e> {
    template: usize,
    params: Params<'e>,
}

struct Runtime<'e, 'a> {
    exe: &'e Executable,
    source_root: Node,
    /// The text of the source document, until `document()` loads it again (absent when
    /// the stylesheet never calls `document()` and the text was given to the tree).
    source_text: RefCell<Option<Cow<'a, str>>>,
    source_uri: Option<&'a str>,
    options: &'a TransformOptions<'a>,
    dc: Cell<Option<&'a DynamicContext<'a>>>,
    globals: Vec<RefCell<GlobalState>>,
    frames: RefCell<Vec<Rc<Frame<'e>>>>,
    /// Documents loaded by `document()`, by absolute URI.
    documents: RefCell<Vec<(String, Node)>>,
    /// The warnings, each distinct one once, in the order first reported.
    warnings: RefCell<Vec<String>>,
    warned: RefCell<HashSet<String>>,
    /// The messages of `xsl:message`.
    messages: RefCell<Vec<String>>,
    /// The error a host callback raised, given back when the evaluator returns it.
    stashed: RefCell<Option<TransformError>>,
    /// The invocations in progress: template rules, built-in template rules, named
    /// templates and stylesheet functions (at most `options.max_nesting`).
    nesting: Cell<u32>,
    /// The stack the transformation may use, from its start (`options.max_stack`): the
    /// invocations and the evaluations of the expressions check it.
    stack: StackLimit,
    /// The counters of the bindings evaluated as Saxon learns ([`Eval::Learning`]).
    learners: Vec<Rc<Learner>>,
    /// The same for the arguments of stylesheet function calls, by call site
    /// ([`Argument::site`]).
    argument_learners: RefCell<HashMap<usize, Rc<Learner>>>,
    /// The number in [`Executable::rule_names`] of each name of the source document, by
    /// [`Node::name_id`], as looked up.
    rule_name_numbers: RefCell<Vec<NameNumber>>,
    /// The storage of the selections of `xsl:apply-templates` done, for the next ones.
    selections: RefCell<Vec<SelectionStorage>>,
}

/// The size of the sequence of nodes a template rule is applied to, the context size of
/// the rule: known, or that of the children of a node, counted when first needed (the
/// built-in rule applies the rules to the children, and most children of a document
/// walked by a compiled Schematron get a rule with an empty body or the built-in rule).
enum Size<'n> {
    Known(usize),
    ChildrenOf(&'n Node, Cell<Option<usize>>),
}

impl Size<'_> {
    fn get(&self) -> usize {
        match self {
            Self::Known(size) => *size,
            Self::ChildrenOf(parent, counted) => {
                if let Some(size) = counted.get() {
                    return size;
                }
                let size = parent.children().count();
                counted.set(Some(size));
                size
            }
        }
    }
}

/// The number of a name of the source document in [`Executable::rule_names`].
#[derive(Debug, Clone, Copy)]
enum NameNumber {
    /// Not looked up yet.
    Unknown,
    /// No mode has rules for the name.
    NoRules,
    Known(u32),
}

/// An invocation counted in [`Runtime::nesting`], until it is dropped.
struct Nested<'n>(&'n Cell<u32>);

impl Drop for Nested<'_> {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
}

/// Sets the line of an error.
trait At<T> {
    fn at(self, line: u32) -> Result<T>;
}

impl<T> At<T> for Result<T> {
    fn at(self, line: u32) -> Result<T> {
        self.map_err(|e| e.at(line))
    }
}

/// The context of a template rule applied to `node`, at `position` in a sequence of
/// `size` nodes. Out of [`Runtime::apply_rule`], whose stack use is part of every level
/// of recursion through the template rules.
#[inline(never)]
fn node_context(node: &Node, position: usize, size: usize) -> Context {
    Context::at(Item::Node(node.clone()), position, size)
}

/// XPath type names, for messages.
fn type_name(t: &SequenceType) -> String {
    let item = match &t.item {
        ItemType::Atomic(a) => a.name.to_string(),
        ItemType::Kind(k) => format!("{k:?}"),
    };
    let occurrence = match t.occurrence {
        Occurrence::One => "",
        Occurrence::ZeroOrOne => "?",
        Occurrence::ZeroOrMore => "*",
        Occurrence::OneOrMore => "+",
    };
    format!("{item}{occurrence}")
}

const fn atomic_type(kind: AtomicKind) -> AtomicType {
    match kind {
        AtomicKind::String => AtomicType::String,
        AtomicKind::AnyUri => AtomicType::AnyUri,
        AtomicKind::Boolean => AtomicType::Boolean,
        AtomicKind::Integer => AtomicType::Integer,
        AtomicKind::Decimal => AtomicType::Decimal,
        AtomicKind::Date => AtomicType::Date,
    }
}

/// Converts a value to a sequence type with the function conversion rules (XPath 2.0
/// section 3.1.5): atomization, `xs:untypedAtomic` cast to the expected type, URI
/// promotion to `xs:string`, then the item type and cardinality checks.
fn convert(items: Vec<Item>, t: &SequenceType, code: &str) -> Result<Vec<Item>> {
    let count = items.len();
    let cardinality_ok = match t.occurrence {
        Occurrence::One => count == 1,
        Occurrence::ZeroOrOne => count <= 1,
        Occurrence::ZeroOrMore => true,
        Occurrence::OneOrMore => count >= 1,
    };
    if !cardinality_ok {
        return err(
            code,
            format!(
                "Required cardinality of value is {}; supplied value has {count} items",
                type_name(t)
            ),
        );
    }
    match &t.item {
        ItemType::Atomic(a) => {
            let target = atomic_type(a.kind);
            items
                .into_iter()
                .map(|item| {
                    let value = item.atomize();
                    let value = match value {
                        AtomicValue::UntypedAtomic(_) => value.cast(target)?,
                        AtomicValue::AnyUri(s) if target == AtomicType::String => {
                            AtomicValue::String(s)
                        }
                        v if v.is_instance_of(target) => v,
                        v => {
                            return err(
                                code,
                                format!(
                                    "Required item type of value is {}; supplied value has item type {}",
                                    a.name,
                                    v.atomic_type()
                                ),
                            );
                        }
                    };
                    Ok(Item::Atomic(value))
                })
                .collect()
        }
        ItemType::Kind(kind) => {
            let test = pattern::step_test(
                match kind {
                    einvoice_xpath::KindTest::Attribute(_) => {
                        einvoice_xpath::PatternAxis::Attribute
                    }
                    _ => einvoice_xpath::PatternAxis::Child,
                },
                &einvoice_xpath::NodeTest::Kind(kind.clone()),
            );
            for item in &items {
                let ok = match item {
                    Item::Node(n) => {
                        matches!(kind, einvoice_xpath::KindTest::AnyKind) || test.matches(n)
                    }
                    Item::Atomic(_) => false,
                };
                if !ok {
                    return err(
                        code,
                        format!("Required item type of value is {}", type_name(t)),
                    );
                }
            }
            Ok(items)
        }
    }
}

/// Inserts a space where a comment's text would contain `--` or end with `-` (XSLT 2.0
/// section 11.6).
fn comment_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_dash = false;
    for c in text.chars() {
        if c == '-' && previous_dash {
            out.push(' ');
        }
        previous_dash = c == '-';
        out.push(c);
    }
    if previous_dash {
        out.push(' ');
    }
    out
}

/// Resolves a URI reference against a base URI (RFC 3986 section 5.2, without the
/// query and fragment cases the rule sets never produce).
fn resolve_uri(reference: &str, base: Option<&str>) -> String {
    let has_scheme = reference.split_once(':').is_some_and(|(scheme, _)| {
        let mut chars = scheme.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    let Some(base) = base.filter(|_| !has_scheme) else {
        return reference.to_owned();
    };
    if reference.is_empty() {
        return base.to_owned();
    }
    let joined = if reference.starts_with('/') {
        match base.split_once(':') {
            Some((scheme, _)) => format!("{scheme}:{reference}"),
            None => reference.to_owned(),
        }
    } else {
        let directory = base.rfind('/').map_or("", |i| &base[..=i]);
        format!("{directory}{reference}")
    };
    // Remove `.` and `..` segments.
    let (prefix, path) = joined.split_once(':').unwrap_or(("", &joined));
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "." => {}
            ".." => {
                if segments.len() > 1 {
                    segments.pop();
                }
            }
            s => segments.push(s),
        }
    }
    let path = segments.join("/");
    if prefix.is_empty() {
        path
    } else {
        format!("{prefix}:{path}")
    }
}

impl<'e, 'a> Runtime<'e, 'a> {
    fn dc(&self) -> Result<&'a DynamicContext<'a>> {
        self.dc
            .get()
            .ok_or_else(|| TransformError::new("INTERNAL", "no dynamic context"))
    }

    /// Converts an evaluator error, giving back the error a host callback raised.
    fn xerr(&self, e: einvoice_xpath_eval::Error) -> TransformError {
        if e.code() == ErrorCode::Host
            && let Some(stashed) = self.stashed.borrow_mut().take()
        {
            return stashed;
        }
        e.into()
    }

    /// Counts one more invocation in progress, until the guard returned is dropped: a
    /// `LIMIT` error beyond `options.max_nesting`, or when the invocations in progress
    /// use more than `options.max_stack` bytes of stack. Every recursion of a
    /// transformation goes through an invocation (a template rule applied to a child, a
    /// named template or a function calling itself), so this bounds the depth of the
    /// call stack.
    fn nested(&self) -> Result<Nested<'_>> {
        let depth = self.nesting.get();
        if depth >= self.options.max_nesting {
            return Err(TransformError::limit_exceeded(LimitExceeded::Nesting {
                max: self.options.max_nesting,
            }));
        }
        self.check_stack()?;
        self.nesting.set(depth + 1);
        Ok(Nested(&self.nesting))
    }

    /// A `LIMIT` error when the invocations in progress use more than `options.max_stack`
    /// bytes of stack. Computing a lazy value checks it: a value may refer to another lazy
    /// one, and so on (a parameter passed down a recursion and read at its end), each
    /// computed inside the one that refers to it. The evaluator checks the same bound
    /// ([`DynamicContext::with_stack_limit`]) at each level of an expression, which may be
    /// evaluated at the deepest invocation.
    fn check_stack(&self) -> Result<()> {
        self.stack.check().map_err(TransformError::limit_exceeded)
    }

    /// Keeps an error for [`Runtime::xerr`] and gives the evaluator a placeholder.
    fn stash(&self, e: TransformError) -> einvoice_xpath_eval::Error {
        let message = e.to_string();
        *self.stashed.borrow_mut() = Some(e);
        einvoice_xpath_eval::Error::new(ErrorCode::Host, message)
    }

    fn evaluate(&self, expr: &CompiledExpr, context: &Context) -> Result<Vec<Item>> {
        expr.evaluate(self.dc()?, context).map_err(|e| self.xerr(e))
    }

    fn test(&self, expr: &CompiledExpr, context: &Context) -> Result<bool> {
        expr.effective_boolean_value(self.dc()?, context)
            .map_err(|e| self.xerr(e))
    }

    /// Runs `f` with `frame` as the frame variables are read from.
    fn in_frame<T>(
        &self,
        frame: Option<Rc<Frame<'e>>>,
        f: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let Some(frame) = frame else {
            return f();
        };
        self.frames.borrow_mut().push(frame);
        let result = f();
        self.frames.borrow_mut().pop();
        result
    }

    fn top_frame(&self) -> Option<Rc<Frame<'e>>> {
        self.frames.borrow().last().cloned()
    }

    fn set_slot(&self, slot: usize, value: Slot<'e>) -> Result<()> {
        let frames = self.frames.borrow();
        let Some(frame) = frames.last() else {
            return err("INTERNAL", "no frame for a local variable");
        };
        let mut slots = frame.slots.borrow_mut();
        let Some(s) = slots.get_mut(slot) else {
            return err("INTERNAL", "variable slot out of range");
        };
        *s = value;
        Ok(())
    }

    #[inline(never)]
    fn force(&self, thunk: &Thunk<'e>) -> Result<Rc<[Item]>> {
        if let Some(value) = thunk.memo.get() {
            return Ok(Rc::clone(value));
        }
        self.check_stack()?;
        let frame = match &thunk.frame {
            None => None,
            Some(weak) => Some(
                weak.upgrade()
                    .ok_or_else(|| TransformError::new("INTERNAL", "variable frame is gone"))?,
            ),
        };
        let state = State {
            context: &thunk.context,
            mode: thunk.mode,
            rule: thunk.rule,
        };
        let value = self.in_frame(frame, || {
            self.value(thunk.value, thunk.as_type, &state, thunk.code)
        })?;
        let _ = thunk.memo.set(Rc::clone(&value));
        thunk.hold.borrow_mut().take();
        if let Some(learner) = &thunk.report {
            learner.completed();
        }
        Ok(value)
    }

    #[inline(never)]
    fn force_argument(&self, thunk: &ArgumentThunk<'e>) -> Result<Rc<[Item]>> {
        if let Some(value) = thunk.memo.get() {
            return Ok(Rc::clone(value));
        }
        self.check_stack()?;
        let frame = match &thunk.frame {
            None => None,
            Some(weak) => Some(
                weak.upgrade()
                    .ok_or_else(|| TransformError::new("INTERNAL", "caller frame is gone"))?,
            ),
        };
        let items = self.in_frame(frame, || {
            thunk
                .argument
                .evaluate(self.dc()?)
                .map_err(|e| self.xerr(e))
        })?;
        let value: Rc<[Item]> = Self::argument_value(thunk.function, thunk.param, items)?.into();
        let _ = thunk.memo.set(Rc::clone(&value));
        if let Some(learner) = &thunk.report {
            learner.completed();
        }
        Ok(value)
    }

    /// An argument converted to the type of its parameter.
    fn argument_value(
        function: &CompiledFunction,
        param: &Binding,
        items: Vec<Item>,
    ) -> Result<Vec<Item>> {
        match &param.as_type {
            Some(t) => convert(items, t, "XPTY0004").map_err(|e| {
                TransformError::new(
                    e.code(),
                    format!(
                        "{} (argument ${} of {}())",
                        e.message(),
                        param.name,
                        function.name
                    ),
                )
            }),
            None => Ok(items),
        }
    }

    fn slot_value(&self, slot: &Slot<'e>) -> Result<Rc<[Item]>> {
        match slot {
            Slot::Value(v) => Ok(Rc::clone(v)),
            Slot::Lazy(thunk) => self.force(thunk),
            Slot::Argument(thunk) => self.force_argument(thunk),
            Slot::Unset => err("INTERNAL", "variable read before it is bound"),
        }
    }

    fn variable(&self, id: VariableId) -> Result<Rc<[Item]>> {
        let index = id.0 as usize;
        let globals = self.exe.globals.len();
        if index < globals {
            return self.global(index);
        }
        let slot = {
            let frames = self.frames.borrow();
            let Some(frame) = frames.last() else {
                return err("INTERNAL", "no frame for a local variable");
            };
            let slots = frame.slots.borrow();
            slots
                .get(index - globals)
                .cloned()
                .ok_or_else(|| TransformError::new("INTERNAL", "variable slot out of range"))?
        };
        self.slot_value(&slot)
    }

    fn global(&self, index: usize) -> Result<Rc<[Item]>> {
        let g = &self.exe.globals[index];
        {
            let mut state = self.globals[index].borrow_mut();
            match &*state {
                GlobalState::Done(v) => return Ok(Rc::clone(v)),
                GlobalState::Evaluating => {
                    return err(
                        "XTDE0640",
                        format!("Circular definition of global variable ${}", g.name),
                    )
                    .at(g.line);
                }
                GlobalState::Pending => *state = GlobalState::Evaluating,
            }
        }
        let supplied = if g.is_param {
            self.options
                .parameters
                .iter()
                .find(|(n, _)| n.same_expanded_name(&g.name))
        } else {
            None
        };
        let value = if let Some((_, items)) = supplied {
            match &g.as_type {
                Some(t) => convert(items.clone(), t, "XTTE0590"),
                None => Ok(items.clone()),
            }
            .map(Rc::from)
        } else {
            let context = Context::for_item(Item::Node(self.source_root.clone()));
            let state = State {
                context: &context,
                mode: self.exe.default_mode,
                rule: None,
            };
            let frame = (g.slots > 0).then(|| Frame::new(g.slots));
            let code = if g.is_param { "XTTE0590" } else { "XTTE0570" };
            self.in_frame(frame, || {
                self.value(&g.value, g.as_type.as_ref(), &state, code)
            })
        };
        let value = match value {
            Ok(v) => v,
            Err(e) => {
                *self.globals[index].borrow_mut() = GlobalState::Pending;
                return Err(e.at(g.line));
            }
        };
        *self.globals[index].borrow_mut() = GlobalState::Done(Rc::clone(&value));
        Ok(value)
    }

    /// The value of a binding, computed now.
    fn value(
        &self,
        value: &'e Value,
        as_type: Option<&'e SequenceType>,
        state: &State<'_>,
        code: &str,
    ) -> Result<Rc<[Item]>> {
        let items = match value {
            Value::Select(expr) => self.evaluate(expr, state.context)?,
            Value::Content(body) => {
                if as_type.is_some() {
                    let mut sink = SequenceSink::default();
                    self.exec(body, state, &mut sink)?;
                    sink.items
                } else {
                    let mut sink = TreeSink::new();
                    self.exec(body, state, &mut sink)?;
                    vec![Item::Node(sink.finish()?.root())]
                }
            }
            Value::Absent => {
                if as_type.is_some() {
                    Vec::new()
                } else {
                    vec![Item::Atomic(AtomicValue::String(String::new()))]
                }
            }
        };
        let items = match as_type {
            Some(t) => convert(items, t, code)?,
            None => items,
        };
        Ok(items.into())
    }

    /// A binding's slot: computed now, or later (see [`Eval`]).
    fn bind(
        &self,
        value: &'e Value,
        as_type: Option<&'e SequenceType>,
        eval: Eval,
        state: &State<'_>,
        code: &'static str,
    ) -> Result<Slot<'e>> {
        let report = match eval {
            Eval::Eager => None,
            Eval::Lazy => Some(None),
            Eval::Learning {
                learner,
                every_counts,
            } => self
                .learners
                .get(learner)
                .and_then(|l| l.evaluation(every_counts).map(|r| r.then(|| Rc::clone(l)))),
        };
        let Some(report) = report else {
            return Ok(Slot::Value(self.value(value, as_type, state, code)?));
        };
        Ok(Slot::Lazy(Rc::new(Thunk {
            value,
            as_type,
            code,
            context: state.context.clone(),
            mode: state.mode,
            rule: state.rule,
            frame: self.top_frame().map(|f| Rc::downgrade(&f)),
            memo: OnceCell::new(),
            report,
            hold: RefCell::new(None),
        })))
    }

    fn with_params(&self, params: &'e [WithParam], state: &State<'_>) -> Result<Params<'e>> {
        if params.is_empty() {
            return Ok(Vec::new());
        }
        params
            .iter()
            .map(|p| {
                Ok((
                    &p.name,
                    self.bind(&p.value, p.as_type.as_ref(), p.eval, state, "XTTE0590")?,
                ))
            })
            .collect()
    }

    /// Runs a sequence constructor. An `xsl:choose` (or `xsl:if`) that is its last
    /// instruction runs its branch in place, in the same loop: a template whose body is
    /// a choice (most templates that recurse) needs no stack frame for it.
    fn exec(&self, body: &'e [Inst], state: &State<'_>, sink: &mut dyn Sink) -> Result<()> {
        let mut body = body;
        while let Some((inst, rest)) = body.split_first() {
            if rest.is_empty()
                && let Inst::Choose(choose) = inst
            {
                body = self.branch(choose, state)?;
            } else {
                self.instruction(inst, state, sink)?;
                body = rest;
            }
        }
        Ok(())
    }

    /// Runs one instruction. Each instruction has a function of its own: this one is on
    /// the path of every recursion through the templates (`xsl:apply-templates` in a
    /// template rule, ...), whose stack use per level of the document must stay small,
    /// so that it holds none of their locals, and dispatches only the instructions that
    /// hold others or invoke templates, each passed whole (its parts in a struct of their
    /// own, one binding per arm); [`Runtime::other_instruction`] the rest.
    fn instruction(&self, inst: &'e Inst, state: &State<'_>, sink: &mut dyn Sink) -> Result<()> {
        match inst {
            Inst::ApplyTemplates(apply) => self.apply_templates(apply, state, sink),
            Inst::NextMatch(next) => self.next_match(next, state, sink),
            Inst::CallTemplate(call) => self.call_template(call, state, sink),
            Inst::Choose(choose) => self.choose(choose, state, sink),
            Inst::ForEach(for_each) => self.for_each(for_each, state, sink),
            Inst::Element(element) => self.element(element, state, sink),
            _ => self.other_instruction(inst, state, sink),
        }
    }

    /// The instructions [`Runtime::instruction`] does not dispatch itself.
    #[inline(never)]
    fn other_instruction(
        &self,
        inst: &'e Inst,
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        match inst {
            Inst::Variable(binding) => self.variable_instruction(binding, state),
            Inst::Sequence { line, select } => self.sequence(*line, select, state, sink),
            Inst::ValueOf {
                line,
                select,
                separator,
                body,
            } => self.value_of(
                *line,
                select.as_deref(),
                separator.as_ref(),
                body,
                state,
                sink,
            ),
            Inst::Attribute {
                line,
                name,
                namespace,
                namespaces,
                select,
                body,
            } => self.attribute(
                *line,
                (name, namespace.as_ref()),
                namespaces,
                select.as_deref(),
                body,
                state,
                sink,
            ),
            Inst::CopyOf { line, select } => self.copy_of(*line, select, state, sink),
            Inst::Copy { line, body } => self.copy(*line, body, state, sink),
            Inst::ComputedElement {
                line,
                name,
                namespace,
                namespaces,
                body,
            } => self.computed_element(
                *line,
                name,
                namespace.as_ref(),
                namespaces,
                body,
                state,
                sink,
            ),
            Inst::Namespace {
                line,
                name,
                select,
                body,
            } => self.namespace(*line, name, select.as_deref(), body, state, sink),
            Inst::ProcessingInstruction {
                line,
                name,
                select,
                body,
            } => self.processing_instruction(*line, name, select.as_deref(), body, state, sink),
            Inst::Message {
                line,
                terminate,
                body,
            } => self.message(*line, terminate.as_ref(), body, state),
            Inst::Comment { line, body } => self.comment(*line, body, state, sink),
            Inst::Text(text) => sink.text(text),
            Inst::Number { line, level, count } => {
                self.number_instruction(*line, *level, count.as_ref(), state, sink)
            }
            Inst::ApplyTemplates(_)
            | Inst::NextMatch(_)
            | Inst::CallTemplate(_)
            | Inst::Choose(_)
            | Inst::ForEach(_)
            | Inst::Element(_) => self.instruction(inst, state, sink),
        }
    }

    fn apply_templates(
        &self,
        apply: &'e CompiledApplyTemplates,
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
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
        // A selection of nodes read from the tree (`@*|*`), or any value. Each step in a
        // function of its own: the stack of a level of a recursion holds as little as
        // it can (a debug build keeps every local of a function on its stack).
        let Some(nodes) = self.select_in_place(line, select, state)? else {
            return self.apply_to_value(line, select, mode, params, state, sink);
        };
        let params = self.with_params(params, state).at(line)?;
        let size = Size::Known(nodes.len());
        let mut position = 0;
        while let Some(node) = nodes.get(position) {
            position += 1;
            self.apply_rule(&node, position, &size, mode, None, &params, sink)
                .at(line)?;
        }
        self.selections.borrow_mut().push(nodes.into_storage());
        Ok(())
    }

    /// The nodes `select` selects, read in place from the tree, if it can be.
    fn select_in_place(
        &self,
        line: u32,
        select: &CompiledExpr,
        state: &State<'_>,
    ) -> Result<Option<NodeSelection>> {
        let storage = self.selections.borrow_mut().pop().unwrap_or_default();
        select
            .select_in_place(self.dc()?, state.context, storage)
            .map_err(|e| self.xerr(e))
            .at(line)
    }

    /// `xsl:apply-templates` to the value of `select`.
    fn apply_to_value(
        &self,
        line: u32,
        select: &'e CompiledExpr,
        mode: usize,
        params: &'e [WithParam],
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let items = self.evaluate(select, state.context).at(line)?;
        let params = self.with_params(params, state).at(line)?;
        let size = items.len();
        for (i, item) in items.iter().enumerate() {
            match item {
                Item::Node(node) => self
                    .apply_rule(node, i + 1, &Size::Known(size), mode, None, &params, sink)
                    .at(line)?,
                // As Saxon, an XSLT 3.0 processor: no pattern of the subset matches an
                // atomic value, whose built-in rule writes it as text.
                Item::Atomic(_) => sink.text(&item.string_value()).at(line)?,
            }
        }
        Ok(())
    }

    fn next_match(
        &self,
        next: &'e CompiledNextMatch,
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let CompiledNextMatch { line, ref params } = *next;
        let Some(rank) = state.rule else {
            return err("XTDE0560", "xsl:next-match without a current template rule").at(line);
        };
        let Some(focus) = &state.context.focus else {
            return err("XTDE0560", "xsl:next-match without a context item").at(line);
        };
        let Item::Node(node) = &focus.item else {
            return err("XTDE0560", "xsl:next-match on an atomic value").at(line);
        };
        let params = self.with_params(params, state).at(line)?;
        self.apply_rule(
            node,
            focus.position,
            &Size::Known(focus.size),
            state.mode,
            Some(rank),
            &params,
            sink,
        )
        .at(line)
    }

    fn call_template(
        &self,
        call: &'e CompiledCallTemplate,
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let params = self.with_params(&call.params, state).at(call.line)?;
        self.invoke(
            call.template,
            state.context,
            state.mode,
            state.rule,
            &params,
            sink,
        )
        .at(call.line)
    }

    fn choose(
        &self,
        choose: &'e CompiledChoose,
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let branch = self.branch(choose, state)?;
        self.exec(branch, state, sink)
    }

    /// The branch of an `xsl:choose` to run: that of the first `xsl:when` whose test is
    /// true, or the `xsl:otherwise`. Out of [`Runtime::exec`] and [`Runtime::choose`],
    /// whose stack use is part of every level of recursion through the templates.
    #[inline(never)]
    fn branch(&self, choose: &'e CompiledChoose, state: &State<'_>) -> Result<&'e [Inst]> {
        for when in &choose.whens {
            if self.test(&when.test, state.context).at(when.line)? {
                return Ok(&when.body);
            }
        }
        Ok(&choose.otherwise)
    }

    fn for_each(
        &self,
        for_each: &'e CompiledForEach,
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let CompiledForEach {
            line,
            ref select,
            ref body,
        } = *for_each;
        let items = self.evaluate(select, state.context).at(line)?;
        let size = items.len();
        for (i, item) in items.into_iter().enumerate() {
            let context = Context::at(item, i + 1, size);
            let inner = State {
                context: &context,
                mode: state.mode,
                rule: None,
            };
            self.exec(body, &inner, sink)?;
        }
        Ok(())
    }

    #[inline(never)]
    fn sequence(
        &self,
        line: u32,
        select: &'e CompiledExpr,
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        for item in self.evaluate(select, state.context).at(line)? {
            sink.item(&item).at(line)?;
        }
        Ok(())
    }

    #[inline(never)]
    fn value_of(
        &self,
        line: u32,
        select: Option<&'e CompiledExpr>,
        separator: Option<&'e CompiledAvt>,
        body: &'e [Inst],
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let separator = match separator {
            Some(avt) => Some(self.avt(avt, state).at(line)?),
            None => None,
        };
        let mut parts = TextSink::new();
        let text = if let Some(expr) = select {
            for item in self.evaluate(expr, state.context).at(line)? {
                parts.item(&item)?;
            }
            parts.finish(separator.as_deref().unwrap_or(" "))
        } else {
            self.exec(body, state, &mut parts)?;
            parts.finish(separator.as_deref().unwrap_or(""))
        };
        sink.text(&text).at(line)
    }

    #[inline(never)]
    #[expect(
        clippy::too_many_arguments,
        reason = "the parts of the instruction, its dynamic context and its sink"
    )]
    fn attribute(
        &self,
        line: u32,
        (name, namespace): (&'e CompiledAvt, Option<&'e CompiledAvt>),
        namespaces: &[NamespaceBinding],
        select: Option<&'e CompiledExpr>,
        body: &'e [Inst],
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let name = self.avt(name, state).at(line)?;
        let namespace = match namespace {
            Some(avt) => Some(self.avt(avt, state).at(line)?),
            None => None,
        };
        let name = attribute_name(&name, namespace.as_deref(), namespaces).at(line)?;
        let value = self.simple_content(select, body, state).at(line)?;
        sink.attribute(&name, &value).at(line)
    }

    #[inline(never)]
    fn comment(
        &self,
        line: u32,
        body: &'e [Inst],
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let mut parts = TextSink::new();
        self.exec(body, state, &mut parts)?;
        sink.comment(&comment_text(&parts.finish(""))).at(line)
    }

    #[inline(never)]
    fn number_instruction(
        &self,
        line: u32,
        level: NumberLevel,
        count: Option<&'e CountPattern>,
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let text = self.number(line, level, count, state).at(line)?;
        sink.text(&text).at(line)
    }

    fn element(
        &self,
        element: &'e CompiledElement,
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        self.start_element(element, state, sink)?;
        self.exec(&element.body, state, sink)?;
        sink.end_element().at(element.line)
    }

    /// The start tag of a literal result element: its name, namespaces and attributes.
    /// Out of [`Runtime::element`], whose stack use is part of every level of recursion
    /// through the templates.
    #[inline(never)]
    fn start_element(
        &self,
        element: &'e CompiledElement,
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        sink.start_element(&element.name, &element.namespaces)
            .at(element.line)?;
        for (attribute, value) in &element.attributes {
            self.attribute_value(element.line, attribute, value, state, sink)?;
        }
        Ok(())
    }

    /// An attribute of a literal result element.
    #[inline(never)]
    fn attribute_value(
        &self,
        line: u32,
        name: &QName,
        value: &'e CompiledAvt,
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let value = self.avt(value, state).at(line)?;
        sink.attribute(name, &value).at(line)
    }

    /// `xsl:copy`: a copy of the context item, the body evaluated for a document node or
    /// an element only, which keeps its namespace nodes.
    #[inline(never)]
    fn copy(
        &self,
        line: u32,
        body: &'e [Inst],
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let Some(focus) = &state.context.focus else {
            return err("XTTE0945", "xsl:copy without a context item").at(line);
        };
        let Item::Node(node) = &focus.item else {
            return sink.item(&focus.item).at(line);
        };
        match node.kind() {
            NodeKind::Document => {
                sink.start_document().at(line)?;
                self.exec(body, state, sink)?;
                sink.end_document().at(line)
            }
            NodeKind::Element => {
                let Some(name) = node.name() else {
                    return err("INTERNAL", "element without name");
                };
                let namespaces: Arc<[NamespaceBinding]> = node.namespaces_in_scope().into();
                sink.start_copied_element(name, &namespaces).at(line)?;
                self.exec(body, state, sink)?;
                sink.end_element().at(line)
            }
            // Of the other kinds, a copy of the node alone: the body is not evaluated.
            _ => copy_node(sink, node).at(line),
        }
    }

    /// `xsl:copy-of`: deep copies of the nodes selected, the atomic values as they are.
    #[inline(never)]
    fn copy_of(
        &self,
        line: u32,
        select: &'e CompiledExpr,
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        for item in self.evaluate(select, state.context).at(line)? {
            match &item {
                Item::Node(node) => copy_node(sink, node).at(line)?,
                Item::Atomic(_) => sink.item(&item).at(line)?,
            }
        }
        Ok(())
    }

    /// `xsl:element`: an element named by the computed name, in the namespace it names,
    /// without namespace nodes of its own: the binding its name needs is added when its
    /// start tag is complete (namespace fixup, after those of `xsl:namespace`).
    #[expect(
        clippy::too_many_arguments,
        reason = "the parts of the instruction, its dynamic context and its sink"
    )]
    #[inline(never)]
    fn computed_element(
        &self,
        line: u32,
        name: &'e CompiledAvt,
        namespace: Option<&'e CompiledAvt>,
        namespaces: &[NamespaceBinding],
        body: &'e [Inst],
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let name = self.element_name(line, name, namespace, namespaces, state)?;
        sink.start_element(&name, &Arc::from(Vec::new())).at(line)?;
        self.exec(body, state, sink)?;
        sink.end_element().at(line)
    }

    /// The name of `xsl:element`.
    #[inline(never)]
    fn element_name(
        &self,
        line: u32,
        name: &'e CompiledAvt,
        namespace: Option<&'e CompiledAvt>,
        namespaces: &[NamespaceBinding],
        state: &State<'_>,
    ) -> Result<QName> {
        let text = self.avt(name, state).at(line)?;
        let namespace = match namespace {
            Some(avt) => Some(self.avt(avt, state).at(line)?),
            None => None,
        };
        element_name(&text, namespace.as_deref(), namespaces).at(line)
    }

    /// `xsl:namespace`: a namespace node on the element being constructed (XSLT 2.0
    /// section 11.7).
    #[inline(never)]
    fn namespace(
        &self,
        line: u32,
        name: &'e CompiledAvt,
        select: Option<&'e CompiledExpr>,
        body: &'e [Inst],
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let prefix = self.avt(name, state).at(line)?;
        let prefix = prefix.trim_matches(einvoice_xpath::is_xml_whitespace);
        let uri = self.simple_content(select, body, state).at(line)?;
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
        sink.namespace((!prefix.is_empty()).then_some(prefix), &uri)
            .at(line)
    }

    /// `xsl:processing-instruction` (XSLT 2.0 section 11.6): the target, an `NCName` other
    /// than `xml` in any case; the value without leading whitespace, `?>` written `? >`.
    #[inline(never)]
    fn processing_instruction(
        &self,
        line: u32,
        name: &'e CompiledAvt,
        select: Option<&'e CompiledExpr>,
        body: &'e [Inst],
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let target = self.avt(name, state).at(line)?;
        let target = target.trim_matches(einvoice_xpath::is_xml_whitespace);
        if !einvoice_xpath::is_ncname(target) || target.eq_ignore_ascii_case("xml") {
            return err(
                "XTDE0890",
                format!("invalid processing instruction name {target:?}"),
            )
            .at(line);
        }
        let value = self.simple_content(select, body, state).at(line)?;
        let value = value
            .trim_start_matches(einvoice_xpath::is_xml_whitespace)
            .replace("?>", "? >");
        sink.processing_instruction(target, &value).at(line)
    }

    /// `xsl:message`: its content, a document node, written as Saxon writes messages
    /// (see [`Transformation::messages`]); with `terminate` `yes`, the transformation ends
    /// with the error `XTMM9000`.
    #[inline(never)]
    fn message(
        &self,
        line: u32,
        terminate: Option<&'e CompiledAvt>,
        body: &'e [Inst],
        state: &State<'_>,
    ) -> Result<()> {
        let terminate = match terminate {
            None => false,
            Some(avt) => {
                let value = self.avt(avt, state).at(line)?;
                match value.trim_matches(einvoice_xpath::is_xml_whitespace) {
                    "yes" | "true" | "1" => true,
                    "no" | "false" | "0" => false,
                    other => {
                        return err(
                            "XTDE0030",
                            format!("invalid value {other:?} of xsl:message/@terminate"),
                        )
                        .at(line);
                    }
                }
            }
        };
        let mut content = TreeSink::new();
        self.exec(body, state, &mut content)?;
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

    fn variable_instruction(&self, binding: &'e Binding, state: &State<'_>) -> Result<()> {
        if !binding.used {
            return Ok(());
        }
        let slot = self
            .bind(
                &binding.value,
                binding.as_type.as_ref(),
                binding.eval,
                state,
                "XTTE0570",
            )
            .at(binding.line)?;
        self.set_slot(binding.slot, slot).at(binding.line)
    }

    /// The value of `xsl:attribute`: `select` joined with spaces, or the content.
    fn simple_content(
        &self,
        select: Option<&'e CompiledExpr>,
        body: &'e [Inst],
        state: &State<'_>,
    ) -> Result<String> {
        let mut parts = TextSink::new();
        if let Some(expr) = select {
            for item in self.evaluate(expr, state.context)? {
                parts.item(&item)?;
            }
            Ok(parts.finish(" "))
        } else {
            self.exec(body, state, &mut parts)?;
            Ok(parts.finish(""))
        }
    }

    fn avt(&self, avt: &'e CompiledAvt, state: &State<'_>) -> Result<String> {
        if let [AvtItem::Text(text)] = avt.0.as_slice() {
            return Ok(text.to_string());
        }
        let mut out = String::new();
        for item in &avt.0 {
            match item {
                AvtItem::Text(text) => out.push_str(text),
                AvtItem::Expr(expr) => {
                    for (i, value) in self.evaluate(expr, state.context)?.iter().enumerate() {
                        if i > 0 {
                            out.push(' ');
                        }
                        out.push_str(&value.atomize().to_string());
                    }
                }
            }
        }
        Ok(out)
    }

    /// Finds the best rule for `node` in `mode` (ranked after `after`, for
    /// `xsl:next-match`) and runs it, or the built-in rule.
    #[expect(
        clippy::too_many_arguments,
        reason = "the focus, the mode, the rule and the parameters of the invocation"
    )]
    fn apply_rule(
        &self,
        node: &Node,
        position: usize,
        size: &Size<'_>,
        mode: usize,
        after: Option<u32>,
        params: &[(&'e QName, Slot<'e>)],
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let Some(rank) = self.find_rule(mode, node, after)? else {
            return self.builtin(node, mode, params, sink);
        };
        let Some(index) = self.rule_template(mode, rank)? else {
            return Ok(());
        };
        let context = node_context(node, position, size.get());
        self.invoke(index, &context, mode, Some(rank), params, sink)
    }

    /// The template of the rule of rank `rank` in `mode`, to invoke; `None` when it has
    /// nothing to bind or run, its invocation counted only, as [`Runtime::invoke`] counts
    /// it. Out of [`Runtime::apply_rule`], whose stack use is part of every level of
    /// recursion through the template rules.
    #[inline(never)]
    fn rule_template(&self, mode: usize, rank: u32) -> Result<Option<usize>> {
        let index = self.exe.modes[mode].rules[rank as usize].template;
        let template = &self.exe.templates[index];
        if template.body.is_empty() && template.params.is_empty() {
            let _nested = self.nested().at(template.line)?;
            return Ok(None);
        }
        Ok(Some(index))
    }

    /// The chain of rules of `mode` for the name of `node`, an element or (`attribute`)
    /// an attribute. The names of the source document are looked up once per
    /// transformation ([`Executable::rule_names`]), then by number in each mode.
    fn named_chain(&self, mode: usize, node: &Node, attribute: bool) -> &'e [u32] {
        let exe: &'e Executable = self.exe;
        let compiled = &exe.modes[mode];
        let id = node
            .name_id()
            .filter(|_| Rc::ptr_eq(node.document(), self.source_root.document()));
        let Some(id) = id else {
            return compiled.named_chain(attribute, node.name());
        };
        let number = {
            let mut numbers = self.rule_name_numbers.borrow_mut();
            if numbers.is_empty() {
                numbers.resize(
                    self.source_root.document().name_count(),
                    NameNumber::Unknown,
                );
            }
            match numbers.get_mut(id as usize) {
                Some(NameNumber::Known(number)) => *number,
                Some(NameNumber::NoRules) => return &[],
                Some(entry) => {
                    let found = node
                        .name()
                        .and_then(|name| rule_name_number(&exe.rule_names, name));
                    *entry = found.map_or(NameNumber::NoRules, NameNumber::Known);
                    match found {
                        Some(number) => number,
                        None => return &[],
                    }
                }
                None => return compiled.named_chain(attribute, node.name()),
            }
        };
        compiled.named_by_number(attribute, number)
    }

    /// Whether `node` matches the pattern of a rule. A dynamic error while matching is
    /// recovered as Saxon recovers it (see [`Runtime::pattern_error`]).
    fn rule_matches(&self, rank: u32, mode: usize, node: &Node) -> Result<bool> {
        let rule = &self.exe.modes[mode].rules[rank as usize];
        if rule.certain & (1 << kind_index(node.kind())) != 0 {
            // Every node of its kind the rule is tried on matches it.
            return Ok(true);
        }
        let template = &self.exe.templates[rule.template];
        let matcher = Matcher { dc: self.dc()? };
        // A union pattern with an explicit priority is one rule: its alternatives in
        // order, the first error ending the match.
        let (alternatives, text) = match rule.alternative {
            Some(index) => (
                template.pattern.get(index..=index).unwrap_or_default(),
                template
                    .alternative_texts
                    .get(index)
                    .map_or("", String::as_str),
            ),
            None => (template.pattern.as_slice(), template.pattern_text.as_str()),
        };
        let outcome = alternatives
            .iter()
            .try_fold(false, |found, a| Ok(found || matcher.matches(a, node)?));
        outcome.or_else(|e| self.pattern_error(e, template.line, text))
    }

    /// A dynamic error raised while matching a pattern (of a template rule, or the
    /// `count` pattern of `xsl:number`), as Saxon-HE 12.10 handles it
    /// (`Pattern.matchesItem`): the node does not match, and the error is reported as
    /// a warning, whatever its code, except a circularity (`XTDE0640`), which ends the
    /// transformation. A global variable whose evaluation failed is not kept in error:
    /// the next reference evaluates it again (`GlobalVariable.actuallyEvaluate`), in a
    /// pattern or elsewhere. The refusals, limits and bugs of this crate (`UNSUPPORTED`,
    /// `LIMIT`, `INTERNAL`) are never recovered: Saxon would not have raised them.
    fn pattern_error(
        &self,
        e: einvoice_xpath_eval::Error,
        line: u32,
        pattern: &str,
    ) -> Result<bool> {
        let e = self.xerr(e);
        if matches!(
            e.code(),
            "XTDE0640" | "UNSUPPORTED" | LIMIT | "INTERNAL" | "HOST"
        ) {
            return Err(e.at(line));
        }
        self.warn(format!(
            "line {line}: {}: An error occurred matching pattern {{{pattern}}}: {}",
            e.code(),
            e.message()
        ));
        Ok(false)
    }

    /// Reports a warning, once: Saxon's standard error reporter prints each distinct
    /// warning once.
    fn warn(&self, message: String) {
        if self.warned.borrow_mut().insert(message.clone()) {
            self.warnings.borrow_mut().push(message);
        }
    }

    /// The rule for `node` in `mode`, chosen exactly as Saxon-HE 12.10 chooses it
    /// (`SimpleMode.getRule` and `searchRuleChain`): the chains of the node's name, of
    /// its kind, then the generic chain, each in rank order; a rule of higher priority
    /// replaces the best so far; on a tie the later declared rule wins and the ambiguity
    /// is reported as an XTDE0540 warning. For `xsl:next-match` (`after` is the current
    /// rule) only the rules ranked below the current one are candidates (lower
    /// priority, or same priority and declared before it, or an earlier part of the
    /// same template), and a tie between two parts of one template goes to the part
    /// found last.
    fn find_rule(&self, mode: usize, node: &Node, after: Option<u32>) -> Result<Option<u32>> {
        let compiled = &self.exe.modes[mode];
        let rules = &compiled.rules;
        let kind = node.kind();
        let named = match kind {
            NodeKind::Element => self.named_chain(mode, node, false),
            NodeKind::Attribute => self.named_chain(mode, node, true),
            _ => &[],
        };
        if after.is_none() && named.is_empty() {
            match compiled.plan(kind) {
                Plan::Builtin => return Ok(None),
                Plan::Rule(rank) => return Ok(Some(rank)),
                Plan::Search => {}
            }
        }
        let below_current = |rank: u32| -> bool {
            let Some(current) = after else {
                return true;
            };
            let (r, c) = (&rules[rank as usize], &rules[current as usize]);
            match r.compare_rank(c) {
                std::cmp::Ordering::Less => true,
                std::cmp::Ordering::Greater => false,
                std::cmp::Ordering::Equal => {
                    r.sequence < c.sequence || (r.sequence == c.sequence && r.part < c.part)
                }
            }
        };
        let mut best: Option<u32> = None;
        for chain in compiled.chains_with(kind, named) {
            for &rank in chain {
                if !below_current(rank) {
                    continue;
                }
                let Some(chosen) = best else {
                    if self.rule_matches(rank, mode, node)? {
                        best = Some(rank);
                    }
                    continue;
                };
                let (head, b) = (&rules[rank as usize], &rules[chosen as usize]);
                match head.compare_rank(b) {
                    std::cmp::Ordering::Less => break,
                    std::cmp::Ordering::Greater => {
                        if self.rule_matches(rank, mode, node)? {
                            best = Some(rank);
                        }
                    }
                    std::cmp::Ordering::Equal => {
                        if !self.rule_matches(rank, mode, node)? {
                            continue;
                        }
                        if head.template != b.template {
                            self.ambiguity(node, compiled, b, head);
                        }
                        // The later declared rule; between two parts of one template,
                        // the higher part, or for xsl:next-match the one found last.
                        let keep = b.sequence > head.sequence
                            || (b.sequence == head.sequence
                                && after.is_none()
                                && b.part > head.part);
                        best = Some(if keep { chosen } else { rank });
                        break;
                    }
                }
            }
        }
        Ok(best)
    }

    /// Reports a tie between `a`, the best rule so far, and `b`, as Saxon's
    /// `reportAmbiguity` does, naming them in that order.
    fn ambiguity(
        &self,
        node: &Node,
        mode: &crate::prepare::CompiledMode,
        a: &crate::prepare::Rule,
        b: &crate::prepare::Rule,
    ) {
        let (ta, tb) = (
            &self.exe.templates[a.template],
            &self.exe.templates[b.template],
        );
        let later = if a.sequence > b.sequence { ta } else { tb };
        self.warn(format!(
            "XTDE0540: Ambiguous rule match for {} in mode {}: matches the template rules on line {} and line {}; the one on line {} is used",
            describe(node),
            mode.name,
            ta.line,
            tb.line,
            later.line
        ));
    }

    /// The built-in template rule for `node`. For a document node or an element, it
    /// applies the template rules to the children, one level of recursion counted as an
    /// invocation ([`Runtime::nested`]).
    fn builtin(
        &self,
        node: &Node,
        mode: usize,
        params: &[(&'e QName, Slot<'e>)],
        sink: &mut dyn Sink,
    ) -> Result<()> {
        match node.kind() {
            NodeKind::Document | NodeKind::Element => {
                let _nested = self.nested()?;
                // The children are counted only if a template rule applied to one runs.
                let size = Size::ChildrenOf(node, Cell::new(None));
                for (i, child) in node.children().enumerate() {
                    self.apply_rule(&child, i + 1, &size, mode, None, params, sink)?;
                }
                Ok(())
            }
            NodeKind::Text | NodeKind::Attribute => sink.text(&node.string_value()),
            NodeKind::Comment | NodeKind::ProcessingInstruction => Ok(()),
        }
    }

    /// Runs a template: its parameters bound from `params` or their default values,
    /// then its body. Counted as an invocation ([`Runtime::nested`]).
    ///
    /// An `xsl:call-template` in tail position of the body (its last instruction, or the
    /// last of the branch of an `xsl:choose` or `xsl:if` that is) runs as Saxon runs it,
    /// in the same invocation rather than one more inside it (Saxon's tail calls): a
    /// template that recurses so, over each character of a string for instance, needs no
    /// stack in proportion to its recursion. The frame of an invocation replaced is kept
    /// by the parameters it passes that are not computed yet, until they are
    /// ([`Runtime::exec_tail`]). At most `max_tail_calls` in a row.
    fn invoke(
        &self,
        index: usize,
        context: &Context,
        mode: usize,
        rule: Option<u32>,
        params: &[(&'e QName, Slot<'e>)],
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let template = &self.exe.templates[index];
        if template.tail_calls {
            return self.invoke_with_tail_calls(index, context, mode, rule, params, sink);
        }
        let _nested = self.nested().at(template.line)?;
        // The frame pushed and popped here, not by `in_frame`: a closure and a generic
        // function would be two more stack frames in each level of the recursion.
        let framed = self.push_frame(template.slots);
        let state = State {
            context,
            mode,
            rule,
        };
        let mut result = self.bind_params(template, params, &state);
        if result.is_ok() {
            result = self.exec(&template.body, &state, sink);
        }
        if framed {
            self.pop_frame();
        }
        result.at(template.line)
    }

    /// Pushes a new frame of `slots` slots, if `slots` is not 0: whether it did. Out of
    /// [`Runtime::invoke`], whose stack use is part of every level of recursion through
    /// the templates.
    #[inline(never)]
    fn push_frame(&self, slots: usize) -> bool {
        if slots == 0 {
            return false;
        }
        self.frames.borrow_mut().push(Frame::new(slots));
        true
    }

    /// Pops the frame [`Runtime::push_frame`] pushed.
    #[inline(never)]
    fn pop_frame(&self) {
        self.frames.borrow_mut().pop();
    }

    /// [`Runtime::invoke`] of a template with an `xsl:call-template` in tail position
    /// (see [`CompiledTemplate::tail_calls`](crate::prepare::CompiledTemplate)): the
    /// templates it calls so run in a loop, in place of the one that calls them. Out of
    /// [`Runtime::invoke`], whose stack use is part of every level of recursion through
    /// the templates.
    #[inline(never)]
    fn invoke_with_tail_calls(
        &self,
        index: usize,
        context: &Context,
        mode: usize,
        rule: Option<u32>,
        params: &[(&'e QName, Slot<'e>)],
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let _nested = self.nested().at(self.exe.templates[index].line)?;
        let state = State {
            context,
            mode,
            rule,
        };
        let mut current = index;
        let mut passed: Option<Params<'e>> = None;
        let mut tail_calls = 0_u32;
        loop {
            let template = &self.exe.templates[current];
            let frame = (template.slots > 0).then(|| Frame::new(template.slots));
            let params = passed.as_deref().unwrap_or(params);
            let tail = self
                .in_frame(frame, || {
                    self.bind_params(template, params, &state)?;
                    self.exec_tail(&template.body, &state, sink)
                })
                .at(template.line)?;
            let Some(tail) = tail else {
                return Ok(());
            };
            tail_calls += 1;
            if tail_calls > self.options.max_tail_calls {
                return Err(TransformError::limit_exceeded(LimitExceeded::TailCalls {
                    max: self.options.max_tail_calls,
                })
                .at(template.line));
            }
            current = tail.template;
            passed = Some(tail.params);
        }
    }

    /// Runs a sequence constructor in tail position of a template body: its
    /// `xsl:call-template` in tail position, if any, is not run but returned, its
    /// parameters bound, for [`Runtime::invoke`] to run in place of the template.
    fn exec_tail(
        &self,
        body: &'e [Inst],
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<Option<Box<TailCall<'e>>>> {
        let Some((last, rest)) = body.split_last() else {
            return Ok(None);
        };
        self.exec(rest, state, sink)?;
        match last {
            Inst::CallTemplate(CompiledCallTemplate {
                line,
                template,
                params,
            }) => {
                let params = self.with_params(params, state).at(*line)?;
                // The values not computed yet read this frame, which ends now.
                let frame = self.top_frame();
                for (_, slot) in &params {
                    if let Slot::Lazy(thunk) = slot {
                        thunk.hold.borrow_mut().clone_from(&frame);
                    }
                }
                Ok(Some(Box::new(TailCall {
                    template: *template,
                    params,
                })))
            }
            Inst::Choose(choose) => {
                let branch = self.branch(choose, state)?;
                self.exec_tail(branch, state, sink)
            }
            other => {
                self.instruction(other, state, sink)?;
                Ok(None)
            }
        }
    }

    /// Binds the parameters of a template being invoked, in its frame: from `params`, or
    /// their default values. Out of [`Runtime::invoke`], whose stack use is part of every
    /// level of recursion through the templates.
    #[inline(never)]
    fn bind_params(
        &self,
        template: &'e CompiledTemplate,
        params: &[(&'e QName, Slot<'e>)],
        state: &State<'_>,
    ) -> Result<()> {
        for param in &template.params {
            let passed = params
                .iter()
                .find(|(name, _)| name.same_expanded_name(&param.name));
            let slot = match (passed, &param.as_type) {
                (Some((_, slot)), None) => slot.clone(),
                (Some((_, slot)), Some(t)) => {
                    let items = self.slot_value(slot)?.to_vec();
                    Slot::Value(convert(items, t, "XTTE0590").at(param.line)?.into())
                }
                (None, as_type) => self
                    .bind(
                        &param.value,
                        as_type.as_ref(),
                        param.eval,
                        state,
                        "XTTE0590",
                    )
                    .at(param.line)?,
            };
            self.set_slot(param.slot, slot)?;
        }
        Ok(())
    }

    /// Calls a stylesheet function. Counted as an invocation ([`Runtime::nested`]). Its
    /// arguments are bound as Saxon binds them: those evaluated before the call as they
    /// are, the others lazily, with Saxon's learning per call site.
    fn call(&self, id: FunctionId, args: Vec<Argument>) -> Result<Vec<Item>> {
        let function = &self.exe.functions[id.0 as usize];
        let _nested = self.nested().at(function.line)?;
        let frame = self.bind_arguments(function, args)?;
        self.function_body(function, frame)
    }

    /// The frame of a function call, its parameters bound to the arguments. Out of
    /// [`Runtime::call`], whose stack use is part of every level of a recursion through
    /// functions.
    #[inline(never)]
    fn bind_arguments(
        &self,
        function: &'e CompiledFunction,
        args: Vec<Argument>,
    ) -> Result<Rc<Frame<'e>>> {
        let caller = self.top_frame().map(|f| Rc::downgrade(&f));
        let frame = Frame::new(function.slots);
        for (param, argument) in function.params.iter().zip(args) {
            let learner = argument.site().map(|site| {
                Rc::clone(self.argument_learners.borrow_mut().entry(site).or_default())
            });
            let report = match &learner {
                None => None,
                Some(l) => l.evaluation(false),
            };
            let slot = match report {
                None => {
                    let items = argument.evaluate(self.dc()?).map_err(|e| self.xerr(e))?;
                    Slot::Value(Self::argument_value(function, param, items)?.into())
                }
                Some(report) => Slot::Argument(Rc::new(ArgumentThunk {
                    argument,
                    param,
                    function,
                    frame: caller.clone(),
                    memo: OnceCell::new(),
                    report: if report { learner } else { None },
                })),
            };
            frame.slots.borrow_mut()[param.slot] = slot;
        }
        Ok(frame)
    }

    /// Runs the body of a function in its frame.
    #[inline(never)]
    fn function_body(
        &self,
        function: &'e CompiledFunction,
        frame: Rc<Frame<'e>>,
    ) -> Result<Vec<Item>> {
        // A stylesheet function has no focus and no current template rule.
        let context = Context::new();
        let state = State {
            context: &context,
            mode: self.exe.default_mode,
            rule: None,
        };
        let mut sink = SequenceSink::default();
        self.in_frame(Some(frame), || self.exec(&function.body, &state, &mut sink))
            .at(function.line)?;
        match &function.as_type {
            Some(t) => convert(sink.items, t, "XTTE0780").at(function.line),
            None => Ok(sink.items),
        }
    }

    fn number(
        &self,
        line: u32,
        level: NumberLevel,
        count: Option<&'e CountPattern>,
        state: &State<'_>,
    ) -> Result<String> {
        let Some(Item::Node(node)) = state.context.focus.as_ref().map(|f| &f.item) else {
            return err("XTTE0990", "xsl:number needs a node as context item");
        };
        let matcher = Matcher { dc: self.dc()? };
        let counted = |n: &Node| -> Result<bool> {
            match count {
                // As a template rule's pattern, an error is recovered (Saxon's
                // `Navigator` matches the count pattern with `matchesItem`).
                Some(pattern) => pattern
                    .alternatives
                    .iter()
                    .try_fold(false, |found, a| Ok(found || matcher.matches(a, n)?))
                    .or_else(|e| self.pattern_error(e, line, &pattern.text)),
                None => Ok(n.kind() == node.kind()
                    && match (n.name(), node.name()) {
                        (Some(a), Some(b)) => a.same_expanded_name(b),
                        (None, None) => true,
                        _ => false,
                    }),
            }
        };
        let position = |n: &Node| -> Result<usize> {
            let mut p = 1;
            for sibling in n.axis(NodeAxis::PrecedingSibling) {
                if counted(&sibling)? {
                    p += 1;
                }
            }
            Ok(p)
        };
        let mut numbers = Vec::new();
        for ancestor in node.axis(NodeAxis::AncestorOrSelf) {
            if counted(&ancestor)? {
                numbers.push(position(&ancestor)?);
                if level == NumberLevel::Single {
                    break;
                }
            }
        }
        numbers.reverse();
        Ok(numbers
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join("."))
    }

    fn load_document(&self, uri: &str, base: Option<&str>) -> Result<Node> {
        let absolute = resolve_uri(uri, base.or(self.exe.base_uri.as_deref()));
        if let Some((_, node)) = self.documents.borrow().iter().find(|(u, _)| *u == absolute) {
            return Ok(node.clone());
        }
        // As Saxon: the source document, asked for by its base URI, is parsed again and
        // is a different tree. Loaded once: the tree is kept under its URI above.
        let is_source = self.source_uri == Some(absolute.as_str());
        let text: Cow<'_, str> = if is_source {
            self.source_text.take().ok_or_else(|| {
                TransformError::new(
                    "INTERNAL",
                    "document() loads the source document, whose text was not kept",
                )
            })?
        } else {
            // The resolver's text is given to the tree, not copied.
            Cow::Owned(
                self.options
                    .resolver
                    .and_then(|r| r.resolve(&absolute))
                    .ok_or_else(|| {
                        TransformError::new(
                            "FODC0002",
                            format!("cannot retrieve the document at {absolute}"),
                        )
                    })?,
            )
        };
        // A loaded document has a document URI, as one in Saxon's document pool. A text
        // from the resolver is parsed once for every transformation of the stylesheet.
        let parsed = match text {
            Cow::Owned(text) if !is_source => {
                self.exe
                    .loaded
                    .parse_loaded(text, &absolute, self.options.max_depth)
            }
            text => Document::parse_loaded_with_max_depth(text, &absolute, self.options.max_depth),
        };
        let node = parsed
            .map_err(|e| loaded_document_error(&absolute, &e))?
            .root();
        self.documents.borrow_mut().push((absolute, node.clone()));
        Ok(node)
    }
}

/// A node, for messages: its path from the root, written as Saxon-HE 12.10 writes it
/// (`Navigator.getPath`): `/r/x[2]/@a`, `/r/text()[1]`, the outermost element without
/// position. Saxon's error reporter prints each distinct warning once, so the path tells
/// apart exactly the nodes Saxon's tells apart.
fn describe(node: &Node) -> String {
    let name = |n: &Node| n.name().map(ToString::to_string).unwrap_or_default();
    // The position among the preceding siblings of the same kind and, for a node with a
    // name, the same name (`Navigator.getNumberSimple`).
    let number = |n: &Node| {
        1 + n
            .axis(NodeAxis::PrecedingSibling)
            .filter(|s| {
                s.kind() == n.kind()
                    && match (s.name(), n.name()) {
                        (Some(a), Some(b)) => a.same_expanded_name(b),
                        (None, None) => true,
                        _ => false,
                    }
            })
            .count()
    };
    // The path of each node from the root down, from its parent's: a loop rather than a
    // recursion over the ancestors, whose number only the document bounds.
    let lineage: Vec<Node> = node.axis(NodeAxis::AncestorOrSelf).collect();
    let mut path = String::new();
    let mut parent_kind: Option<NodeKind> = None;
    for n in lineage.iter().rev() {
        let parent_path = std::mem::take(&mut path);
        // A step under the parent, which is "/" for the document node.
        let under_parent = |step: String| {
            format!(
                "{}/{step}",
                if parent_path == "/" {
                    ""
                } else {
                    parent_path.as_str()
                }
            )
        };
        path = match n.kind() {
            NodeKind::Document => "/".to_owned(),
            NodeKind::Element => match parent_kind {
                None => name(n),
                Some(NodeKind::Document) => format!("/{}", name(n)),
                Some(_) => format!("{parent_path}/{}[{}]", name(n), number(n)),
            },
            NodeKind::Attribute => format!("{parent_path}/@{}", name(n)),
            NodeKind::Text => under_parent(format!("text()[{}]", number(n))),
            NodeKind::Comment => under_parent(format!("comment()[{}]", number(n))),
            NodeKind::ProcessingInstruction => {
                under_parent(format!("processing-instruction()[{}]", number(n)))
            }
        };
        parent_kind = Some(n.kind());
    }
    path
}

/// The error for a document that `document()` could not parse: `LIMIT` for one beyond
/// the maximum depth, else `FODC0002`, as Saxon reports a document it cannot parse.
fn loaded_document_error(uri: &str, e: &DocumentError) -> TransformError {
    match e.limit() {
        Some(limit) => TransformError::limit_exceeded(limit).in_document(uri),
        None => TransformError::new("FODC0002", format!("{uri}: {}", e.message())),
    }
}

impl Host for Runtime<'_, '_> {
    fn variable(&self, id: VariableId) -> einvoice_xpath_eval::Result<Rc<[Item]>> {
        Runtime::variable(self, id).map_err(|e| self.stash(e))
    }

    fn call_function(
        &self,
        id: FunctionId,
        args: Vec<Argument>,
        _dc: &DynamicContext<'_>,
    ) -> einvoice_xpath_eval::Result<Vec<Item>> {
        self.call(id, args).map_err(|e| self.stash(e))
    }

    fn document(&self, uri: &str, base: Option<&str>) -> einvoice_xpath_eval::Result<Node> {
        self.load_document(uri, base).map_err(|e| self.stash(e))
    }

    fn system_property(&self, name: &QName) -> String {
        // Saxon-HE 12.10's values.
        einvoice_xslt::system_property(name.namespace.as_deref(), &name.local).to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{comment_text, resolve_uri};

    #[test]
    fn comments_never_hold_double_dashes() {
        assert_eq!(comment_text("a--b-"), "a- -b- ");
        assert_eq!(comment_text("-x"), "-x");
        assert_eq!(comment_text("---"), "- - - ");
    }

    #[test]
    fn uris_resolve_against_the_base() {
        assert_eq!(resolve_uri("b.xml", Some("file:/d/a.xml")), "file:/d/b.xml");
        assert_eq!(
            resolve_uri("../c/b.xml", Some("file:/d/e/a.xml")),
            "file:/d/c/b.xml"
        );
        assert_eq!(
            resolve_uri("file:/x.xml", Some("file:/d/a.xml")),
            "file:/x.xml"
        );
        assert_eq!(resolve_uri("/x.xml", Some("file:/d/a.xml")), "file:/x.xml");
        assert_eq!(resolve_uri("x.xml", None), "x.xml");
        assert_eq!(resolve_uri("", Some("file:/d/a.xml")), "file:/d/a.xml");
    }
}
