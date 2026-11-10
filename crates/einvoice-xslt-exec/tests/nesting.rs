//! Stylesheets at the limits of the loader (issue #33): instructions nested
//! [`MAX_NESTING`] deep holding expressions [`MAX_DEPTH`] deep are prepared and run on a
//! thread with the default stack of a spawned thread, 2 MiB, in the build being tested
//! (debug for `cargo test`); deep expressions evaluated inside a deep recursion end with
//! the stack limit of the transformation for a recursion through functions, which runs
//! on the call stack, with the nesting limit for a recursion through templates, which
//! does not (the limit on tail calls in a row, for a tail recursion), never a stack
//! overflow; random nestings around the limits are loaded exactly up to them, and never
//! abort the process.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

use einvoice_xpath::{ErrorKind, MAX_DEPTH};
use einvoice_xpath_eval::LimitExceeded;
use einvoice_xslt::{LoadErrorKind, MAX_NESTING, load};
use einvoice_xslt_exec::{Executable, TransformOptions};
use proptest::prelude::*;

/// The limits, as counts.
const DEPTH: usize = MAX_DEPTH as usize;
const NESTING: usize = MAX_NESTING as usize;

/// The default stack of a spawned thread.
const STACK: usize = 2 << 20;

fn on_a_default_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

const HEAD: &str = r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema">
<xsl:output omit-xml-declaration="yes"/>"#;

/// `n` times `open`, `inner`, `n` times `close`.
fn wrapped(open: &str, inner: &str, close: &str, n: usize) -> String {
    format!("{}{inner}{}", open.repeat(n), close.repeat(n))
}

/// Elements `a` nested one more than the deepest path of the expressions.
fn source() -> String {
    wrapped("<a>", "1", "</a>", DEPTH + 1)
}

/// Expressions at the maximum depth that evaluate without error from the document node.
fn expressions() -> Vec<(&'static str, String)> {
    let chain = |op: &str, operand: &str| vec![operand; DEPTH].join(op);
    let bindings = (0..DEPTH - 1)
        .map(|i| format!("$x{i} in 1"))
        .collect::<Vec<_>>()
        .join(", ");
    vec![
        ("parentheses", wrapped("(", "1", ")", DEPTH - 1)),
        ("function calls", wrapped("not(", "true()", ")", DEPTH - 1)),
        (
            "function arguments",
            wrapped("concat('x', ", "'a'", ")", DEPTH - 1),
        ),
        ("unary minus", format!("{}1", "-".repeat(DEPTH - 1))),
        ("path steps", vec!["a"; DEPTH].join("/")),
        ("predicates", format!(".{}", "[1]".repeat(DEPTH - 1))),
        ("nested predicates", wrapped("a[", "1", "]", DEPTH - 1)),
        ("if", wrapped("if (1) then ", "1", " else 0", DEPTH - 1)),
        (
            "for",
            format!("{}1", "for $x in 1 return ".repeat(DEPTH - 1)),
        ),
        (
            "some",
            format!("{}true()", "some $x in 1 satisfies ".repeat(DEPTH - 1)),
        ),
        ("bindings", format!("for {bindings} return 1")),
        ("sequences", wrapped("(1, ", "1", ")", DEPTH - 1)),
        ("or chain", chain(" or ", "false()")),
        ("+ chain", chain(" + ", "1")),
        ("* chain", chain(" * ", "1")),
        ("| chain", chain(" | ", ".")),
    ]
}

/// A stylesheet whose instructions nest `NESTING` deep (the content of the innermost
/// `e` element), holding `expr` in every kind of attribute there.
fn at_both_limits(expr: &str) -> String {
    // The template body is level 1, the content of `r` level 2, of the k-th `xsl:if`
    // level 2 + k, of `e` (in the 61st) level 64.
    let ifs = NESTING - 3;
    format!(
        r#"{HEAD}<xsl:template match='/'><r>{}<e a="{{{expr}}}"><xsl:attribute name="b" select="{expr}"/><xsl:value-of select="{expr}" separator="{{{expr}}}"/><xsl:variable name="v" select="{expr}"/><xsl:value-of select="$v"/></e>{}</r></xsl:template></xsl:stylesheet>"#,
        "<xsl:if test='1'>".repeat(ifs),
        "</xsl:if>".repeat(ifs)
    )
}

#[test]
fn a_stylesheet_at_both_limits_is_prepared_and_run() {
    on_a_default_stack(|| {
        for (name, expr) in expressions() {
            let sheet = load(&at_both_limits(&expr)).unwrap_or_else(|e| panic!("{name}: {e}"));
            let executable = Executable::new(&sheet).unwrap_or_else(|e| panic!("{name}: {e}"));
            let output = executable
                .transform(source(), None)
                .unwrap_or_else(|e| panic!("{name}: {e}"))
                .output;
            assert!(output.contains("><e a=\""), "{name}: {output}");
        }
        // One level deeper, either way, is refused when loaded.
        let (_, expr) = &expressions()[0];
        let errors = load(&at_both_limits(&format!("({expr})"))).unwrap_err();
        assert!(errors.0.iter().all(|e| matches!(&e.kind,
            LoadErrorKind::XPath { error, .. } if error.kind == ErrorKind::TooDeep)));
        let deeper = at_both_limits("1")
            .replace("<e a=", "<xsl:if test='1'><e a=")
            .replace("</e>", "</e></xsl:if>");
        let errors = load(&deeper).unwrap_err();
        assert_eq!(
            errors.0[0].kind,
            LoadErrorKind::TooDeep { max: MAX_NESTING }
        );
    });
}

/// A recursion through a function that evaluates a deep expression at each level ends
/// with the stack limit, whether the limit is found by an invocation or by the
/// evaluation of the expression: here the 128 KiB left on the thread above the limit are
/// less than the evaluation of the expression needs in a debug build.
#[test]
fn deep_expressions_in_a_deep_function_recursion_end_with_the_stack_limit() {
    for (name, expr) in expressions() {
        let xslt = format!(
            r#"{HEAD}<xsl:function name='f:t' xmlns:f='urn:f'><xsl:param name='c'/><xsl:for-each select="$c"><xsl:value-of select="{expr}"/></xsl:for-each><xsl:sequence select="f:t($c)"/></xsl:function><xsl:template match='/' xmlns:f='urn:f'><r><xsl:sequence select="f:t(.)"/></r></xsl:template></xsl:stylesheet>"#
        );
        let executable = Executable::new(&load(&xslt).unwrap()).unwrap();
        let outcome = on_a_default_stack(move || {
            let options = TransformOptions {
                max_nesting: u32::MAX,
                max_stack: u32::try_from(STACK - (128 << 10)).unwrap(),
                ..TransformOptions::default()
            };
            executable
                .transform_with(source(), None, &options)
                .map(|t| t.output)
        });
        let error = outcome.unwrap_err();
        assert_eq!(error.code(), "LIMIT", "{name}: {error}");
        assert_eq!(
            error.limit(),
            Some(LimitExceeded::Stack {
                max: u32::try_from(STACK - (128 << 10)).unwrap()
            }),
            "{name}"
        );
    }
}

/// The same recursion through a named template (the call is not the last instruction of
/// the template: not a tail call, which would run in place) runs on the heap, not on the
/// call stack: the deep expressions are evaluated at each of 4096 levels on the 2 MiB
/// stack, and the recursion ends with the nesting limit.
#[test]
fn deep_expressions_in_a_deep_template_recursion_end_with_the_nesting_limit() {
    const LEVELS: u32 = 4096;
    for (name, expr) in expressions() {
        let xslt = format!(
            r#"{HEAD}<xsl:template match='/'><r><xsl:call-template name='t'/></r></xsl:template><xsl:template name='t'><xsl:value-of select="{expr}"/><xsl:call-template name='t'/><xsl:sequence select="()"/></xsl:template></xsl:stylesheet>"#
        );
        let executable = Executable::new(&load(&xslt).unwrap()).unwrap();
        let outcome = on_a_default_stack(move || {
            let options = TransformOptions {
                max_nesting: LEVELS,
                max_stack: u32::try_from(STACK - (128 << 10)).unwrap(),
                ..TransformOptions::default()
            };
            executable
                .transform_with(source(), None, &options)
                .map(|t| t.output)
        });
        let error = outcome.unwrap_err();
        assert_eq!(error.code(), "LIMIT", "{name}: {error}");
        assert_eq!(
            error.limit(),
            Some(LimitExceeded::Nesting { max: LEVELS }),
            "{name}"
        );
    }
}

/// The same recursion as tail calls (the call is the last instruction), which run in
/// place: the stack does not grow with it, and the recursion ends with the limit on tail
/// calls in a row, the deep expressions evaluated at each call on the 2 MiB stack.
#[test]
fn deep_expressions_in_a_tail_recursion_end_with_the_tail_call_limit() {
    for (name, expr) in expressions() {
        let xslt = format!(
            r#"{HEAD}<xsl:template match='/'><r><xsl:call-template name='t'/></r></xsl:template><xsl:template name='t'><xsl:value-of select="{expr}"/><xsl:call-template name='t'/></xsl:template></xsl:stylesheet>"#
        );
        let executable = Executable::new(&load(&xslt).unwrap()).unwrap();
        let outcome = on_a_default_stack(move || {
            let options = TransformOptions {
                max_stack: u32::try_from(STACK - (128 << 10)).unwrap(),
                max_tail_calls: 1000,
                ..TransformOptions::default()
            };
            executable
                .transform_with(source(), None, &options)
                .map(|t| t.output)
        });
        let error = outcome.unwrap_err();
        assert_eq!(error.code(), "LIMIT", "{name}: {error}");
        assert_eq!(
            error.limit(),
            Some(LimitExceeded::TailCalls { max: 1000 }),
            "{name}"
        );
    }
}

/// An instruction nesting a sequence constructor.
#[derive(Debug, Clone, Copy)]
enum Level {
    If,
    When,
    Otherwise,
    ForEach,
    Variable,
    WithParam,
    ValueOf,
    Attribute,
    Comment,
    Element,
}

impl Level {
    fn tags(self) -> (&'static str, &'static str) {
        match self {
            Self::If => ("<xsl:if test='1'>", "</xsl:if>"),
            Self::When => (
                "<xsl:choose><xsl:when test='1'>",
                "</xsl:when></xsl:choose>",
            ),
            Self::Otherwise => (
                "<xsl:choose><xsl:when test='0'/><xsl:otherwise>",
                "</xsl:otherwise></xsl:choose>",
            ),
            Self::ForEach => ("<xsl:for-each select='.'>", "</xsl:for-each>"),
            Self::Variable => (
                "<xsl:variable name='v'>",
                "</xsl:variable><xsl:value-of select='$v'/>",
            ),
            Self::WithParam => (
                "<xsl:call-template name='t'><xsl:with-param name='p'>",
                "</xsl:with-param></xsl:call-template>",
            ),
            Self::ValueOf => ("<xsl:value-of>", "</xsl:value-of>"),
            Self::Attribute => ("<xsl:attribute name='a'>", "</xsl:attribute>"),
            Self::Comment => ("<xsl:comment>", "</xsl:comment>"),
            Self::Element => ("<e>", "</e>"),
        }
    }
}

fn level() -> impl Strategy<Value = Level> {
    prop_oneof![
        Just(Level::If),
        Just(Level::When),
        Just(Level::Otherwise),
        Just(Level::ForEach),
        Just(Level::Variable),
        Just(Level::WithParam),
        Just(Level::ValueOf),
        Just(Level::Attribute),
        Just(Level::Comment),
        Just(Level::Element),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Random nestings of instructions, the innermost evaluating an expression of
    /// random depth: loaded exactly when neither is beyond its limit, then prepared and
    /// run (to a result or a dynamic error), all on a thread with a 2 MiB stack.
    #[test]
    fn random_nestings_never_abort(
        levels in prop::collection::vec(level(), 0..2 * NESTING),
        depth in 1..2 * DEPTH,
    ) {
        let expr = wrapped("not(", "true()", ")", depth - 1);
        let (open, close): (String, String) = levels.iter().fold(
            (String::new(), String::new()),
            |(open, close), level| {
                let (o, c) = level.tags();
                (open + o, c.to_owned() + &close)
            },
        );
        // The template body is level 1, the content of `r` level 2.
        let nesting = levels.len() + 2;
        let xslt = format!(
            "{HEAD}<xsl:template name='t'><xsl:param name='p'/><xsl:value-of select='$p'/></xsl:template><xsl:template match='/'><r>{open}<xsl:value-of select='{expr}'/>{close}</r></xsl:template></xsl:stylesheet>"
        );
        let accepted = nesting <= NESTING && depth <= DEPTH;
        let outcome = on_a_default_stack(move || match load(&xslt) {
            Ok(sheet) => {
                let executable = Executable::new(&sheet).unwrap_or_else(|e| panic!("{e}"));
                let _ = executable.transform("<a/>", None);
                Ok(())
            }
            Err(errors) => Err(errors),
        });
        match outcome {
            Ok(()) => prop_assert!(accepted, "nesting {nesting}, depth {depth}"),
            Err(errors) => {
                prop_assert!(!accepted, "nesting {}, depth {}: {}", nesting, depth, errors);
                for error in &errors.0 {
                    let too_deep = match &error.kind {
                        LoadErrorKind::TooDeep { max } => *max == MAX_NESTING,
                        LoadErrorKind::XPath { error, .. } => error.kind == ErrorKind::TooDeep,
                        _ => false,
                    };
                    prop_assert!(too_deep, "{}", error);
                }
            }
        }
    }
}
