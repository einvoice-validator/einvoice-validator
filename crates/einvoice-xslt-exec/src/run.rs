//! Running an [`Executable`] on a document.
//!
//! Template rules are chosen as Saxon-HE chooses them: highest priority first, the last
//! declared on a tie (an ambiguity is reported as a warning, as Saxon does), then the
//! built-in rules of XSLT 2.0 section 6.6. A dynamic error while matching a pattern is
//! recovered as Saxon recovers it: a warning, and the rule does not match. Variables live in frames, one per template,
//! function or global variable invocation; the XPath evaluator reads them through the
//! [`Host`] interface, and calls stylesheet functions through it too.
//!
//! Evaluation order follows Saxon where it is observable (which dynamic errors are
//! raised): global variables are evaluated when first referenced; a local variable or
//! parameter that nothing refers to is never evaluated; one whose `select` may return
//! several items is evaluated when first referenced; any other is evaluated where it is
//! declared.

use std::borrow::Cow;
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use einvoice_xdm::{AtomicType, AtomicValue, DateTime};
use einvoice_xpath::{AtomicKind, ItemType, Occurrence, QName, SequenceType};
use einvoice_xpath_eval::{
    Clock, CompiledExpr, Context, DEFAULT_MAX_DEPTH, Document, DocumentError, DynamicContext,
    ErrorCode, FunctionId, Host, Item, LimitExceeded, Node, NodeAxis, NodeKind, VariableId,
};
use einvoice_xslt::{NamespaceBinding, NumberLevel};

use crate::error::{LIMIT, Result, TransformError, err};
use crate::pattern::{self, Matcher};
use crate::prepare::{
    AvtItem, Binding, CompiledAvt, CompiledTemplate, CountPattern, Executable, Inst, ModeRef,
    Value, When, WithParam,
};
use crate::sink::{SequenceSink, Sink, TextSink, TreeSink, XmlWriter, attribute_name};

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
    /// The most stack, in bytes, the invocations in progress may use, measured from the
    /// start of the transformation: beyond it, the transformation ends with a `LIMIT`
    /// error ([`LimitExceeded::Stack`]) instead of overflowing the stack. It bounds what
    /// the nesting alone cannot, the stack of each level, which depends on the
    /// stylesheet: a function calling itself from a nested expression needs much more
    /// than a template rule applied to a child. The thread must have this much stack and
    /// some more, for its caller and for the deepest level. Default:
    /// [`DEFAULT_MAX_STACK`].
    pub max_stack: u32,
}

/// The default of [`TransformOptions::max_stack`]: 1.5 MiB, for a thread with the
/// default 2 MiB stack of a spawned thread; 640 KiB on WebAssembly, whose stack is 1 MiB.
/// With it, the official stylesheets run on a document [`DEFAULT_MAX_DEPTH`] deep, in a
/// debug build too, and the 512 KiB left (384 KiB on WebAssembly) are more than the
/// caller and one level of invocation need.
#[cfg(not(target_family = "wasm"))]
pub const DEFAULT_MAX_STACK: u32 = 3 << 19;

/// The default of [`TransformOptions::max_stack`] (see the other targets).
#[cfg(target_family = "wasm")]
pub const DEFAULT_MAX_STACK: u32 = 640 << 10;

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
        let root = document.root();
        let runtime = Runtime {
            exe: self,
            source_root: root.clone(),
            source_text: RefCell::new(source_text),
            source_uri: base_uri,
            options,
            dc: Cell::new(None),
            globals: (0..self.globals.len())
                .map(|_| RefCell::new(GlobalState::Pending))
                .collect(),
            frames: RefCell::new(Vec::new()),
            documents: RefCell::new(Vec::new()),
            warnings: RefCell::new(Vec::new()),
            warned: RefCell::new(HashSet::new()),
            stashed: RefCell::new(None),
            nesting: Cell::new(0),
            stack_base: stack_position(),
        };
        let timezone = options.implicit_timezone.unwrap_or(0);
        let clock = match options.current_date_time {
            Some(now) => Clock::new(timezone, now)?,
            None => system_clock(timezone)?,
        };
        let dc = DynamicContext::new(&runtime, clock);
        runtime.dc.set(Some(&dc));
        let mut writer = XmlWriter::new(self.serialization);
        runtime.apply_rule(&root, 1, 1, self.default_mode, None, &[], &mut writer)?;
        let output = writer.finish()?;
        let warnings = runtime.warnings.take();
        Ok(Transformation { output, warnings })
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
    expr: &'e CompiledExpr,
    as_type: Option<&'e SequenceType>,
    code: &'static str,
    context: Context,
    /// The frame the expression's variables are read from.
    frame: Option<Weak<Frame<'e>>>,
    memo: OnceCell<Rc<[Item]>>,
}

#[derive(Clone)]
enum Slot<'e> {
    Unset,
    Value(Rc<[Item]>),
    Lazy(Rc<Thunk<'e>>),
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
    /// The error a host callback raised, given back when the evaluator returns it.
    stashed: RefCell<Option<TransformError>>,
    /// The invocations in progress: template rules, built-in template rules, named
    /// templates and stylesheet functions (at most `options.max_nesting`).
    nesting: Cell<u32>,
    /// The position of the stack when the transformation started (see
    /// [`stack_position`]).
    stack_base: usize,
}

/// The position of the call stack: the address of a local of this function. Two of them
/// measure the stack used between two calls (stacks grow downwards on every supported
/// target, the WebAssembly shadow stack included; the distance is taken either way).
#[inline(never)]
fn stack_position() -> usize {
    let marker = 0_u8;
    std::hint::black_box(std::ptr::addr_of!(marker)).addr()
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
        let used = self.stack_base.abs_diff(stack_position());
        if u32::try_from(used).map_or(true, |used| used > self.options.max_stack) {
            return Err(TransformError::limit_exceeded(LimitExceeded::Stack {
                max: self.options.max_stack,
            }));
        }
        self.nesting.set(depth + 1);
        Ok(Nested(&self.nesting))
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

    fn force(&self, thunk: &Thunk<'e>) -> Result<Rc<[Item]>> {
        if let Some(value) = thunk.memo.get() {
            return Ok(Rc::clone(value));
        }
        let frame = match &thunk.frame {
            None => None,
            Some(weak) => Some(
                weak.upgrade()
                    .ok_or_else(|| TransformError::new("INTERNAL", "variable frame is gone"))?,
            ),
        };
        let items = self.in_frame(frame, || self.evaluate(thunk.expr, &thunk.context))?;
        let items = match thunk.as_type {
            Some(t) => convert(items, t, thunk.code)?,
            None => items,
        };
        let value: Rc<[Item]> = items.into();
        let _ = thunk.memo.set(Rc::clone(&value));
        Ok(value)
    }

    fn slot_value(&self, slot: &Slot<'e>) -> Result<Rc<[Item]>> {
        match slot {
            Slot::Value(v) => Ok(Rc::clone(v)),
            Slot::Lazy(thunk) => self.force(thunk),
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

    /// A binding's slot: computed now, or later when it may be a sequence.
    fn bind(
        &self,
        value: &'e Value,
        as_type: Option<&'e SequenceType>,
        lazy: bool,
        state: &State<'_>,
        code: &'static str,
    ) -> Result<Slot<'e>> {
        if lazy && let Value::Select(expr) = value {
            return Ok(Slot::Lazy(Rc::new(Thunk {
                expr,
                as_type,
                code,
                context: state.context.clone(),
                frame: self.top_frame().map(|f| Rc::downgrade(&f)),
                memo: OnceCell::new(),
            })));
        }
        Ok(Slot::Value(self.value(value, as_type, state, code)?))
    }

    fn with_params(&self, params: &'e [WithParam], state: &State<'_>) -> Result<Params<'e>> {
        params
            .iter()
            .map(|p| {
                Ok((
                    &p.name,
                    self.bind(&p.value, p.as_type.as_ref(), p.lazy, state, "XTTE0590")?,
                ))
            })
            .collect()
    }

    fn exec(&self, body: &'e [Inst], state: &State<'_>, sink: &mut dyn Sink) -> Result<()> {
        for inst in body {
            self.instruction(inst, state, sink)?;
        }
        Ok(())
    }

    /// Runs one instruction. Each instruction has a function of its own: this one is on
    /// the path of every recursion through the templates (`xsl:apply-templates` in a
    /// template rule, ...), whose stack use per level of the document must stay small,
    /// so that it holds none of their locals, and dispatches only the instructions that
    /// hold others or invoke templates; [`Runtime::other_instruction`] the rest.
    fn instruction(&self, inst: &'e Inst, state: &State<'_>, sink: &mut dyn Sink) -> Result<()> {
        match inst {
            Inst::ApplyTemplates {
                line,
                select,
                mode,
                params,
            } => self.apply_templates(*line, select, mode, params, state, sink),
            Inst::NextMatch { line, params } => self.next_match(*line, params, state, sink),
            Inst::CallTemplate {
                line,
                template,
                params,
            } => self.call_template(*line, *template, params, state, sink),
            Inst::Choose { whens, otherwise } => self.choose(whens, otherwise, state, sink),
            Inst::ForEach { line, select, body } => self.for_each(*line, select, body, state, sink),
            Inst::Element {
                line,
                name,
                namespaces,
                attributes,
                body,
            } => self.element(*line, name, namespaces, attributes, body, state, sink),
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
                namespaces,
                select,
                body,
            } => self.attribute(
                *line,
                name,
                namespaces,
                select.as_deref(),
                body,
                state,
                sink,
            ),
            Inst::Comment { line, body } => self.comment(*line, body, state, sink),
            Inst::Text(text) => sink.text(text),
            Inst::Number { line, level, count } => {
                self.number_instruction(*line, *level, count.as_ref(), state, sink)
            }
            Inst::ApplyTemplates { .. }
            | Inst::NextMatch { .. }
            | Inst::CallTemplate { .. }
            | Inst::Choose { .. }
            | Inst::ForEach { .. }
            | Inst::Element { .. } => self.instruction(inst, state, sink),
        }
    }

    fn apply_templates(
        &self,
        line: u32,
        select: &'e CompiledExpr,
        mode: &ModeRef,
        params: &'e [WithParam],
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let items = self.evaluate(select, state.context).at(line)?;
        let mode = match mode {
            ModeRef::Mode(m) => *m,
            ModeRef::Current => state.mode,
        };
        let params = self.with_params(params, state).at(line)?;
        let size = items.len();
        for (i, item) in items.iter().enumerate() {
            match item {
                Item::Node(node) => self
                    .apply_rule(node, i + 1, size, mode, None, &params, sink)
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
        line: u32,
        params: &'e [WithParam],
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
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
            focus.size,
            state.mode,
            Some(rank),
            &params,
            sink,
        )
        .at(line)
    }

    fn call_template(
        &self,
        line: u32,
        template: usize,
        params: &'e [WithParam],
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let params = self.with_params(params, state).at(line)?;
        self.invoke(
            template,
            state.context,
            state.mode,
            state.rule,
            &params,
            sink,
        )
        .at(line)
    }

    fn choose(
        &self,
        whens: &'e [When],
        otherwise: &'e [Inst],
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        for when in whens {
            if self.test(&when.test, state.context).at(when.line)? {
                return self.exec(&when.body, state, sink);
            }
        }
        self.exec(otherwise, state, sink)
    }

    fn for_each(
        &self,
        line: u32,
        select: &'e CompiledExpr,
        body: &'e [Inst],
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
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
        name: &'e CompiledAvt,
        namespaces: &[NamespaceBinding],
        select: Option<&'e CompiledExpr>,
        body: &'e [Inst],
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        let name = self.avt(name, state).at(line)?;
        let name = attribute_name(&name, namespaces).at(line)?;
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

    #[expect(
        clippy::too_many_arguments,
        reason = "the parts of the instruction, its dynamic context and its sink"
    )]
    fn element(
        &self,
        line: u32,
        name: &QName,
        namespaces: &[NamespaceBinding],
        attributes: &'e [(Arc<QName>, CompiledAvt)],
        body: &'e [Inst],
        state: &State<'_>,
        sink: &mut dyn Sink,
    ) -> Result<()> {
        sink.start_element(name, namespaces).at(line)?;
        for (attribute, value) in attributes {
            self.attribute_value(line, attribute, value, state, sink)?;
        }
        self.exec(body, state, sink)?;
        sink.end_element().at(line)
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

    fn variable_instruction(&self, binding: &'e Binding, state: &State<'_>) -> Result<()> {
        if !binding.used {
            return Ok(());
        }
        let slot = self
            .bind(
                &binding.value,
                binding.as_type.as_ref(),
                binding.lazy,
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
        size: usize,
        mode: usize,
        after: Option<u32>,
        params: &[(&'e QName, Slot<'e>)],
        sink: &mut dyn Sink,
    ) -> Result<()> {
        match self.find_rule(mode, node, after)? {
            Some(rank) => {
                let template = self.exe.modes[mode].rules[rank as usize].template;
                let context = Context::at(Item::Node(node.clone()), position, size);
                self.invoke(template, &context, mode, Some(rank), params, sink)
            }
            None => self.builtin(node, mode, params, sink),
        }
    }

    /// Whether `node` matches the pattern of a rule. A dynamic error while matching is
    /// recovered as Saxon recovers it (see [`Runtime::pattern_error`]).
    fn rule_matches(&self, rank: u32, mode: usize, node: &Node) -> Result<bool> {
        let rule = &self.exe.modes[mode].rules[rank as usize];
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
        let below_current = |rank: u32| -> bool {
            let Some(current) = after else {
                return true;
            };
            let (r, c) = (&rules[rank as usize], &rules[current as usize]);
            match r.priority.total_cmp(&c.priority) {
                std::cmp::Ordering::Less => true,
                std::cmp::Ordering::Greater => false,
                std::cmp::Ordering::Equal => {
                    r.sequence < c.sequence || (r.sequence == c.sequence && r.part < c.part)
                }
            }
        };
        let mut best: Option<u32> = None;
        for chain in compiled.chains(node.kind(), node.name()) {
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
                match head.priority.total_cmp(&b.priority) {
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
                let children: Vec<Node> = node.children().collect();
                let size = children.len();
                for (i, child) in children.iter().enumerate() {
                    self.apply_rule(child, i + 1, size, mode, None, params, sink)?;
                }
                Ok(())
            }
            NodeKind::Text | NodeKind::Attribute => sink.text(&node.string_value()),
            NodeKind::Comment | NodeKind::ProcessingInstruction => Ok(()),
        }
    }

    /// Runs a template: its parameters bound from `params` or their default values,
    /// then its body. Counted as an invocation ([`Runtime::nested`]).
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
        let _nested = self.nested().at(template.line)?;
        let frame = (template.slots > 0).then(|| Frame::new(template.slots));
        let state = State {
            context,
            mode,
            rule,
        };
        self.in_frame(frame, || {
            self.bind_params(template, params, &state)?;
            self.exec(&template.body, &state, sink)
        })
        .at(template.line)
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
                        param.lazy,
                        state,
                        "XTTE0590",
                    )
                    .at(param.line)?,
            };
            self.set_slot(param.slot, slot)?;
        }
        Ok(())
    }

    /// Calls a stylesheet function. Counted as an invocation ([`Runtime::nested`]).
    fn call(&self, id: FunctionId, args: Vec<Vec<Item>>) -> Result<Vec<Item>> {
        let function = &self.exe.functions[id.0 as usize];
        let _nested = self.nested().at(function.line)?;
        let frame = Frame::new(function.slots);
        for (param, arg) in function.params.iter().zip(args) {
            let value = match &param.as_type {
                Some(t) => convert(arg, t, "XPTY0004").map_err(|e| {
                    TransformError::new(
                        e.code(),
                        format!(
                            "{} (argument ${} of {}())",
                            e.message(),
                            param.name,
                            function.name
                        ),
                    )
                })?,
                None => arg,
            };
            frame.slots.borrow_mut()[param.slot] = Slot::Value(value.into());
        }
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
        let text: Cow<'_, str> = if self.source_uri == Some(absolute.as_str()) {
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
        // A loaded document has a document URI, as one in Saxon's document pool.
        let node = Document::parse_loaded_with_max_depth(text, &absolute, self.options.max_depth)
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
        args: Vec<Vec<Item>>,
    ) -> einvoice_xpath_eval::Result<Vec<Item>> {
        self.call(id, args).map_err(|e| self.stash(e))
    }

    fn document(&self, uri: &str, base: Option<&str>) -> einvoice_xpath_eval::Result<Node> {
        self.load_document(uri, base).map_err(|e| self.stash(e))
    }

    fn system_property(&self, name: &QName) -> String {
        // Saxon-HE 12.10's values.
        if name.namespace.as_deref() != Some(einvoice_xslt::XSLT_NAMESPACE) {
            return String::new();
        }
        match name.local.as_str() {
            "version" => "3.0",
            "vendor" => "Saxonica",
            "vendor-url" => "http://www.saxonica.com/",
            "product-name" => "SAXON",
            "product-version" => "HE 12.10",
            _ => "",
        }
        .to_owned()
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
