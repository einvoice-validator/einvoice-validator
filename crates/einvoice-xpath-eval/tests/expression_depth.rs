//! Expressions at the maximum depth of the parser ([`MAX_DEPTH`], issue #33) compile
//! and evaluate on a thread with the default stack of a spawned thread, 2 MiB, in the
//! build being tested (debug for `cargo test`); random nestings around the limit are
//! accepted exactly up to it, and never abort the process; a host's [`StackLimit`] ends
//! an evaluation with a [`LimitExceeded::Stack`] error.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]
// Proptest is not a dependency on wasm targets.
#![cfg(not(target_family = "wasm"))]

use einvoice_xpath::{ErrorKind, Expr, MAX_DEPTH, NamespaceMap, XS_NAMESPACE, parse_expr};
use einvoice_xpath_eval::{
    Clock, Context, Document, DynamicContext, ErrorCode, Item, LimitExceeded, NoHost,
    SimpleStaticContext, StackLimit, compile,
};
use std::rc::Rc;

use proptest::prelude::*;

/// The depth limit, as a count.
const MAX: usize = MAX_DEPTH as usize;

fn on_a_default_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

fn ns() -> NamespaceMap {
    NamespaceMap::new().bind("xs", XS_NAMESPACE)
}

/// `n` times `open`, `inner`, `n` times `close`.
fn wrapped(open: &str, inner: &str, close: &str, n: usize) -> String {
    format!("{}{inner}{}", open.repeat(n), close.repeat(n))
}

/// Elements `a` nested one more than the deepest path of the expressions, so that every
/// step finds a node.
fn document() -> Rc<Document> {
    Document::parse(wrapped("<a>", "1", "</a>", MAX + 1), None).unwrap()
}

/// Every form of nesting at the maximum depth (or nesting of the parser).
fn at_the_limit() -> Vec<(&'static str, String)> {
    let chain = |op: &str, operand: &str| vec![operand; MAX].join(op);
    let bindings = (0..MAX - 1)
        .map(|i| format!("$x{i} in 1"))
        .collect::<Vec<_>>()
        .join(", ");
    vec![
        ("parentheses", wrapped("(", "1", ")", MAX - 1)),
        ("function calls", wrapped("not(", "true()", ")", MAX - 1)),
        (
            "function arguments",
            wrapped("concat('x', ", "'a'", ")", MAX - 1),
        ),
        ("sum", wrapped("sum(", "1", ")", MAX - 1)),
        ("unary minus", format!("{}1", "-".repeat(MAX - 1))),
        ("negated parentheses", wrapped("-(", "1", ")", MAX - 1)),
        (
            "cast as",
            wrapped("(", "1", ") cast as xs:integer", MAX - 1),
        ),
        (
            "castable as",
            wrapped("(", "1", ") castable as xs:boolean", MAX - 1),
        ),
        (
            "instance of",
            wrapped("(", "1", ") instance of xs:boolean", MAX - 1),
        ),
        ("path steps", vec!["a"; MAX].join("/")),
        (
            "absolute path steps",
            format!("/{}", vec!["a"; MAX - 1].join("/")),
        ),
        ("descendant steps", "//a".repeat((MAX - 1) / 2)),
        ("filter steps", vec!["(.)"; MAX].join("/")),
        (
            "predicates of a filter",
            format!(".{}", "[1]".repeat(MAX - 1)),
        ),
        (
            "predicates of a step",
            format!("a{}", "[1]".repeat(MAX - 1)),
        ),
        ("nested predicates", wrapped("a[", "1", "]", MAX - 1)),
        ("if", wrapped("if (1) then ", "1", " else 0", MAX - 1)),
        (
            "if conditions",
            wrapped("if (", "1", ") then 1 else 0", MAX - 1),
        ),
        ("for", format!("{}1", "for $x in 1 return ".repeat(MAX - 1))),
        (
            "for sequences",
            wrapped("for $x in ", "1", " return $x", MAX - 1),
        ),
        (
            "some",
            format!("{}true()", "some $x in 1 satisfies ".repeat(MAX - 1)),
        ),
        (
            "every",
            format!("{}true()", "every $x in 1 satisfies ".repeat(MAX - 1)),
        ),
        ("bindings", format!("for {bindings} return 1")),
        ("sequences", wrapped("(1, ", "1", ")", MAX - 1)),
        ("or chain", chain(" or ", "false()")),
        ("and chain", chain(" and ", "true()")),
        ("+ chain", chain(" + ", "1")),
        ("- chain", chain(" - ", "1")),
        ("* chain", chain(" * ", "1")),
        ("div chain", chain(" div ", "1")),
        ("idiv chain", chain(" idiv ", "1")),
        ("mod chain", chain(" mod ", "1")),
        ("| chain", chain(" | ", ".")),
        ("=", wrapped("true() = (", "true()", ")", MAX - 1)),
        ("eq", wrapped("true() eq (", "true()", ")", MAX - 1)),
        ("to", wrapped("1 to (", "1", ")", MAX - 1)),
    ]
}

/// Compiles and evaluates `text` from the document node: a value or a dynamic error,
/// never a refusal or a bug of the evaluator.
fn evaluate(text: &str) -> Result<usize, einvoice_xpath_eval::Error> {
    let expr = parse_expr(text, &ns()).unwrap_or_else(|e| panic!("{text}: {e}"));
    let compiled = compile(&expr, &SimpleStaticContext::new()).unwrap_or_else(|e| panic!("{e}"));
    let doc = document();
    let dc = DynamicContext::new(&NoHost, Clock::from_unix_millis(0, 0).unwrap());
    let result = compiled
        .evaluate(&dc, &Context::for_item(Item::Node(doc.root())))
        .map(|items| items.len());
    if let Err(e) = &result {
        assert!(
            !matches!(e.code(), ErrorCode::Internal | ErrorCode::Unsupported),
            "{e}"
        );
    }
    result
}

#[test]
fn every_form_of_nesting_compiles_and_evaluates_at_the_limit() {
    on_a_default_stack(|| {
        for (name, text) in at_the_limit() {
            let outcome = evaluate(&text);
            assert!(
                outcome.is_ok() || name == "cast as" || name == "=" || name == "eq",
                "{name}: {outcome:?}"
            );
        }
    });
}

#[test]
fn a_stack_limit_ends_the_evaluation() {
    on_a_default_stack(|| {
        let text = vec!["1"; MAX].join(" + ");
        let expr = parse_expr(&text, &ns()).unwrap();
        let compiled = compile(&expr, &SimpleStaticContext::new()).unwrap();
        let doc = document();
        let context = Context::for_item(Item::Node(doc.root()));
        let clock = Clock::from_unix_millis(0, 0).unwrap();
        // Enough: the sum.
        let limit = StackLimit::from_here(1 << 20);
        assert_eq!(limit.max(), 1 << 20);
        assert!(limit.check().is_ok());
        let dc = DynamicContext::new(&NoHost, clock).with_stack_limit(limit);
        let items = compiled.evaluate(&dc, &context).unwrap();
        assert_eq!(items[0].string_value(), MAX.to_string());
        // Too little: a limit error, as the host would raise it.
        let dc = DynamicContext::new(&NoHost, clock).with_stack_limit(StackLimit::from_here(2048));
        let error = compiled.evaluate(&dc, &context).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Limit);
        assert_eq!(error.code().as_str(), "LIMIT");
        assert_eq!(error.limit(), Some(LimitExceeded::Stack { max: 2048 }));
        // Never set aside by `or`, as dynamic errors of the first operand are.
        let or = format!("({}) = 0 or true()", vec!["1"; MAX - 2].join(" + "));
        let compiled = compile(
            &parse_expr(&or, &ns()).unwrap(),
            &SimpleStaticContext::new(),
        )
        .unwrap();
        let dc = DynamicContext::new(&NoHost, clock).with_stack_limit(StackLimit::from_here(2048));
        let error = compiled.effective_boolean_value(&dc, &context).unwrap_err();
        assert_eq!(error.limit(), Some(LimitExceeded::Stack { max: 2048 }));
        // Without a limit, nothing is checked.
        let dc = DynamicContext::new(&NoHost, clock);
        assert!(compiled.effective_boolean_value(&dc, &context).unwrap());
    });
}

/// A form of nesting around an expression `e`: the text before and after it, and how
/// much deeper it is than `e` (the nesting of the parser is always one level more).
#[derive(Debug, Clone, Copy)]
enum Form {
    Parentheses,
    Not,
    Negate,
    Add,
    Chain(usize),
    If,
    Then,
    For,
    Some,
    Filter,
    Predicate,
    Path,
    Sequence,
    Cast,
    Compare,
}

impl Form {
    fn around(self, e: &mut (String, String)) -> usize {
        let (open, close, deeper): (&str, String, usize) = match self {
            Self::Parentheses => ("(", ")".into(), 0),
            Self::Not => ("not(", ")".into(), 1),
            Self::Negate => ("-(", ")".into(), 1),
            Self::Add => ("1 + (", ")".into(), 1),
            Self::Chain(k) => ("(", format!("){}", " + 1".repeat(k)), k),
            Self::If => ("if (", ") then 1 else 0".into(), 1),
            Self::Then => ("if (1) then ", " else 0".into(), 1),
            Self::For => ("for $x in ", " return $x".into(), 1),
            Self::Some => ("some $x in ", " satisfies true()".into(), 1),
            Self::Filter => (".[", "]".into(), 1),
            Self::Predicate => ("a[", "]".into(), 1),
            Self::Path => ("(", ")/a".into(), 1),
            Self::Sequence => ("(1, ", ")".into(), 1),
            Self::Cast => ("(", ") cast as xs:string".into(), 1),
            Self::Compare => ("1 = (", ")".into(), 1),
        };
        e.0.push_str(open);
        e.1.insert_str(0, &close);
        deeper
    }
}

fn form() -> impl Strategy<Value = Form> {
    prop_oneof![
        Just(Form::Parentheses),
        Just(Form::Not),
        Just(Form::Negate),
        Just(Form::Add),
        (1..8_usize).prop_map(Form::Chain),
        Just(Form::If),
        Just(Form::Then),
        Just(Form::For),
        Just(Form::Some),
        Just(Form::Filter),
        Just(Form::Predicate),
        Just(Form::Path),
        Just(Form::Sequence),
        Just(Form::Cast),
        Just(Form::Compare),
    ]
}

/// The text of the forms applied around `1`, the outermost first, its depth and the
/// nesting of the parser (the top-level expression is one level).
fn build(forms: &[Form]) -> (String, usize, usize) {
    let mut parts = (String::new(), String::new());
    let mut depth = 1;
    for form in forms {
        depth += form.around(&mut parts);
    }
    (format!("{}1{}", parts.0, parts.1), depth, forms.len() + 1)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Random nestings around the limit: read exactly when neither the depth nor the
    /// nesting is beyond it, and then printed, read again, compiled and evaluated, all
    /// on a thread with a 2 MiB stack.
    #[test]
    fn random_nestings_never_abort(forms in prop::collection::vec(form(), 0..2 * MAX)) {
        let (text, depth, nesting) = build(&forms);
        let accepted = depth <= MAX && nesting <= MAX;
        let outcome = on_a_default_stack(move || {
            match parse_expr(&text, &ns()) {
                Ok(expr) => {
                    let again: Expr = parse_expr(&expr.to_string(), &ns()).unwrap();
                    assert_eq!(again, expr);
                    let _ = evaluate(&text);
                    Ok(())
                }
                Err(e) => Err(e.kind),
            }
        });
        match outcome {
            Ok(()) => prop_assert!(accepted, "depth {depth}, nesting {nesting}"),
            Err(kind) => {
                prop_assert_eq!(kind, ErrorKind::TooDeep);
                prop_assert!(!accepted, "depth {}, nesting {}", depth, nesting);
            }
        }
    }
}
