//! The limits that keep a transformation within a bounded call stack (issue #29): the
//! maximum element depth of the documents ([`TransformOptions::max_depth`]) and the
//! maximum nesting of template and function invocations
//! ([`TransformOptions::max_nesting`]); and the call stack of a recursion through the
//! templates, which does not grow with it (issue #39). Every test runs on a thread with
//! the default stack of a spawned thread, 2 MiB, in the build being tested (debug for
//! `cargo test`): what is accepted must run there, what is beyond a limit must be a
//! `LIMIT` error, never a stack overflow, which would abort the test binary.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

mod common;

use einvoice_xpath_eval::{DEFAULT_MAX_DEPTH, LimitExceeded};
use einvoice_xslt_exec::{
    DEFAULT_MAX_NESTING, DEFAULT_MAX_STACK, DEFAULT_MAX_TAIL_CALLS, DocumentResolver, Executable,
    TransformError, TransformOptions, max_nesting_for_depth,
};

/// The default stack of a spawned thread.
const STACK: usize = 2 << 20;

/// Runs `f` on a thread with a 2 MiB stack.
fn on_a_default_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

const HEAD: &str = r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
<xsl:output omit-xml-declaration="yes"/>"#;
/// The namespaces the literal result elements copy from [`HEAD`], sorted by prefix as
/// Saxon writes them.
const NS: &str = r#" xmlns:f="urn:f" xmlns:xs="http://www.w3.org/2001/XMLSchema""#;

/// A result element `r` with the namespaces of [`HEAD`], holding `text`.
fn r(text: &str) -> String {
    format!("<r{NS}>{text}</r>")
}

fn prepare(body: &str) -> Executable {
    let xslt = format!("{HEAD}\n{body}\n</xsl:stylesheet>");
    Executable::new(&einvoice_xslt::load(&xslt).unwrap_or_else(|e| panic!("{e}")))
        .unwrap_or_else(|e| panic!("{e}"))
}

/// `<x>` elements nested `depth` deep, the innermost holding `inner`.
fn nested(depth: usize, inner: &str) -> String {
    format!("{}{inner}{}", "<x>".repeat(depth), "</x>".repeat(depth))
}

/// The 1-based line and column of byte `offset` of `text`.
fn position(text: &str, offset: usize) -> (u32, u32) {
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before[before.rfind('\n').map_or(0, |i| i + 1)..]
        .chars()
        .count()
        + 1;
    (u32::try_from(line).unwrap(), u32::try_from(column).unwrap())
}

/// The official CEN UBL example 1 with elements of another namespace nested inside
/// `ext:UBLExtensions/ext:UBLExtension/ext:ExtensionContent`, so that the document is
/// `depth` deep, and the position of the start tag of its element at depth `at`.
fn deep_invoice(depth: usize, at: usize) -> (String, (u32, u32)) {
    let root = common::workspace_root();
    let text = common::read(&common::join(
        &root,
        "artefacts/cen-en16931-ubl-1.3.17/ubl/examples/ubl-tc434-example1.xml",
    ))
    .unwrap();
    let start = text.find("<Invoice").unwrap();
    let end = start + text[start..].find('>').unwrap() + 1;
    // Invoice, UBLExtensions, UBLExtension and ExtensionContent are depths 1 to 4.
    let n = depth - 4;
    let head = format!(
        "{}<ext:UBLExtensions xmlns:ext=\"urn:oasis:names:specification:ubl:schema:xsd:CommonExtensionComponents-2\"><ext:UBLExtension><ext:ExtensionContent><y:x xmlns:y=\"urn:y\">",
        &text[..end]
    );
    let document = format!(
        "{head}{}{}</ext:ExtensionContent></ext:UBLExtension></ext:UBLExtensions>{}",
        "<y:x>".repeat(n - 1),
        "</y:x>".repeat(n),
        &text[end..]
    );
    let offset = if at <= 4 {
        0
    } else {
        // The element at depth 5 is the first <y:x ...>, then one "<y:x>" per level.
        head.len() - "<y:x xmlns:y=\"urn:y\">".len()
            + if at == 5 {
                0
            } else {
                "<y:x xmlns:y=\"urn:y\">".len() + (at - 6) * "<y:x>".len()
            }
    };
    let at = position(&document, offset);
    (document, at)
}

fn limit<T: std::fmt::Debug>(result: Result<T, TransformError>) -> LimitExceeded {
    let error = result.unwrap_err();
    assert_eq!(error.code(), "LIMIT", "{error}");
    error.limit().unwrap_or_else(|| panic!("no limit: {error}"))
}

#[test]
fn the_defaults() {
    assert_eq!(DEFAULT_MAX_DEPTH, 256);
    assert_eq!(DEFAULT_MAX_NESTING, 2 * 256 + 64);
    assert_eq!(
        max_nesting_for_depth(DEFAULT_MAX_DEPTH),
        DEFAULT_MAX_NESTING
    );
    assert_eq!(max_nesting_for_depth(u32::MAX), u32::MAX);
    let options = TransformOptions::default();
    assert_eq!(
        (options.max_depth, options.max_nesting),
        (DEFAULT_MAX_DEPTH, DEFAULT_MAX_NESTING)
    );
}

/// The issue's case: the official CEN stylesheet on a document nested 256 deep (the
/// default maximum) runs on a 2 MiB stack and reports what Saxon reports for the example
/// (nothing is fatal); 257 deep is refused with the position of the deepest element; the
/// issue's 1500 is refused just the same.
#[test]
fn the_cen_stylesheet_at_the_maximum_depth() {
    on_a_default_stack(|| {
        let root = common::workspace_root();
        let executable = common::executable(&root).unwrap();
        let (document, _) = deep_invoice(256, 256);
        let svrl = executable
            .transform(document.as_str(), None)
            .unwrap()
            .output;
        let canonical = einvoice_svrl::canonicalize(&svrl, &|_| false).unwrap();
        assert!(!canonical.contains("flag=\"fatal\""), "{canonical}");
        assert!(canonical.contains("fired-rule"), "{canonical}");
        for depth in [257, 1500] {
            let (document, at) = deep_invoice(depth, 257);
            assert_eq!(
                limit(executable.transform(document.as_str(), None)),
                LimitExceeded::Depth {
                    max: 256,
                    line: at.0,
                    column: at.1
                }
            );
        }
        // A larger maximum accepts it.
        let (document, _) = deep_invoice(300, 300);
        let options = TransformOptions {
            max_depth: 300,
            max_nesting: max_nesting_for_depth(300),
            ..TransformOptions::default()
        };
        executable
            .transform_with(document.as_str(), None, &options)
            .unwrap();
    });
}

/// The same with both Peppol stylesheets, when the Peppol cache is built (see
/// `peppol_equivalence.rs`). The Peppol rules fire on the innermost, empty, element, whose
/// location the ISO skeleton writes with a template recursing over its ancestors: the
/// nesting reaches twice the depth.
#[test]
fn the_peppol_stylesheets_at_the_maximum_depth() {
    let root = common::workspace_root();
    let Some(corpus) = common::peppol::corpus(&root).unwrap_or_else(|e| panic!("{e}")) else {
        assert!(
            !common::peppol::required(),
            "no Peppol corpus; {} is set",
            common::peppol::REQUIRED_VAR
        );
        eprintln!("SKIPPED: no Peppol corpus in {}", common::peppol::BASE);
        return;
    };
    for stylesheet in corpus.stylesheets {
        on_a_default_stack(move || {
            let executable = common::prepare(&stylesheet.xslt).unwrap();
            let (document, _) = deep_invoice(256, 256);
            let svrl = executable
                .transform(document.as_str(), None)
                .unwrap()
                .output;
            einvoice_svrl::canonicalize(&svrl, &|_| false).unwrap();
            let (document, at) = deep_invoice(257, 257);
            assert_eq!(
                limit(executable.transform(document.as_str(), None)),
                LimitExceeded::Depth {
                    max: 256,
                    line: at.0,
                    column: at.1
                },
                "{}",
                stylesheet.name
            );
        });
    }
}

/// Every way a stylesheet can recurse without end ends with a `LIMIT` error on a 2 MiB
/// stack, with the default limits: template rules, named templates, functions, and
/// templates applied from the content of a variable, a value-of, an attribute or a
/// comment, or from a function called in a pattern (not recovered as a warning). Which
/// limit comes first depends on the stack each level needs, which depends on the build:
/// the nesting (576), or the stack used (1.5 MiB). A named template, the cheapest level,
/// always reaches the nesting first.
#[test]
fn unbounded_recursion_is_a_limit() {
    let stylesheets = [
        r#"<xsl:template match="/"><xsl:apply-templates select="*"/></xsl:template>
<xsl:template match="*"><r><xsl:apply-templates select="."/></r></xsl:template>"#,
        r#"<xsl:template match="/"><xsl:call-template name="t"/></xsl:template>
<xsl:template name="t"><xsl:param name="n" select="0"/><xsl:call-template name="t"><xsl:with-param name="n" select="$n + 1"/></xsl:call-template></xsl:template>"#,
        r#"<xsl:function name="f:r" as="xs:integer"><xsl:param name="n" as="xs:integer"/><xsl:sequence select="if ($n lt 0) then 0 else 1 + f:r($n + 1)"/></xsl:function>
<xsl:template match="/"><r><xsl:value-of select="f:r(0)"/></r></xsl:template>"#,
        r#"<xsl:function name="f:r"><xsl:param name="n"/><xsl:choose><xsl:when test="$n lt 0"><xsl:sequence select="0"/></xsl:when><xsl:otherwise><xsl:for-each select="1"><xsl:variable name="m" select="f:r($n + 1)"/><xsl:sequence select="$m"/></xsl:for-each></xsl:otherwise></xsl:choose></xsl:function>
<xsl:template match="/"><r><xsl:value-of select="f:r(0)"/></r></xsl:template>"#,
        r#"<xsl:template match="node()"><xsl:variable name="v"><xsl:apply-templates select="."/></xsl:variable><xsl:value-of select="$v"/></xsl:template>"#,
        r#"<xsl:template match="node()"><xsl:value-of><xsl:apply-templates select="."/></xsl:value-of></xsl:template>"#,
        r#"<xsl:template match="/"><r><xsl:apply-templates select="*"/></r></xsl:template>
<xsl:template match="*"><xsl:attribute name="a"><xsl:apply-templates select="."/></xsl:attribute></xsl:template>"#,
        r#"<xsl:template match="node()"><xsl:comment><xsl:apply-templates select="."/></xsl:comment></xsl:template>"#,
        r#"<xsl:template match="/"><xsl:next-match/></xsl:template>
<xsl:template match="node()" priority="-1"><xsl:apply-templates select="/"/></xsl:template>"#,
        r#"<xsl:function name="f:r" as="xs:boolean"><xsl:param name="n" as="xs:integer"/><xsl:sequence select="f:r($n + 1)"/></xsl:function>
<xsl:template match="x[f:r(0)]"><m/></xsl:template>"#,
    ];
    for (i, body) in stylesheets.into_iter().enumerate() {
        let outcome = on_a_default_stack(move || {
            let executable = prepare(body);
            executable.transform("<x><x/></x>", None).map(|t| t.output)
        });
        let error = outcome.unwrap_err();
        assert_eq!(error.code(), "LIMIT", "{body}: {error}");
        let message = error.to_string();
        match error.limit() {
            Some(LimitExceeded::Nesting { max: 576 }) => assert!(
                message.contains(
                    "LIMIT: templates and functions invoked one inside another more than 576 deep"
                ),
                "{message}"
            ),
            Some(LimitExceeded::Stack { max }) if max == DEFAULT_MAX_STACK && i != 1 => assert!(
                message.contains("use more than 1572864 bytes of stack"),
                "{message}"
            ),
            // A named template calling itself last: tail calls, run in place, which need
            // no stack.
            Some(LimitExceeded::TailCalls { max }) if max == DEFAULT_MAX_TAIL_CALLS && i == 1 => {
                assert!(
                    message.contains(
                        "ending with a call of another template more than 65536 times in a row"
                    ),
                    "{message}"
                );
            }
            other => panic!("{body}: {other:?}"),
        }
    }
}

/// Recursion that ends is not limited below the maximum nesting and stack, which are
/// configurable.
#[test]
fn bounded_recursion_and_the_configurable_limits() {
    // `$total` is read at each level: computed there (xsl:call-template passes it lazily).
    // The call is not the last instruction: not a tail call.
    let template = prepare(
        r#"<xsl:template match="/"><r><xsl:call-template name="sum"><xsl:with-param name="n" select="xs:integer(/n)"/></xsl:call-template></r></xsl:template>
<xsl:template name="sum"><xsl:param name="n"/><xsl:param name="total" select="0"/><xsl:choose><xsl:when test="$n eq 0 or $total lt 0"><xsl:value-of select="$total"/></xsl:when><xsl:otherwise><xsl:call-template name="sum"><xsl:with-param name="n" select="$n - 1"/><xsl:with-param name="total" select="$total + $n"/></xsl:call-template><xsl:sequence select="()"/></xsl:otherwise></xsl:choose></xsl:template>"#,
    );
    // The same as tail calls.
    let tail = prepare(
        r#"<xsl:template match="/"><r><xsl:call-template name="sum"><xsl:with-param name="n" select="xs:integer(/n)"/></xsl:call-template></r></xsl:template>
<xsl:template name="sum"><xsl:param name="n"/><xsl:param name="total" select="0"/><xsl:choose><xsl:when test="$n eq 0 or $total lt 0"><xsl:value-of select="$total"/></xsl:when><xsl:otherwise><xsl:call-template name="sum"><xsl:with-param name="n" select="$n - 1"/><xsl:with-param name="total" select="$total + $n"/></xsl:call-template></xsl:otherwise></xsl:choose></xsl:template>"#,
    );
    let function = prepare(
        r#"<xsl:function name="f:sum" as="xs:integer"><xsl:param name="n" as="xs:integer"/><xsl:sequence select="if ($n eq 0) then 0 else $n + f:sum($n - 1)"/></xsl:function>
<xsl:template match="/"><r><xsl:value-of select="f:sum(xs:integer(/n))"/></r></xsl:template>"#,
    );
    on_a_default_stack(move || {
        // The template rule, then 551 named templates (n = 550 down to 0), or 41
        // functions: a function level, through the XPath evaluator, uses the call stack,
        // a template level none.
        let run = |e: &Executable, n: u32, options: &TransformOptions<'_>| {
            e.transform_with(format!("<n>{n}</n>").as_str(), None, options)
                .map(|t| t.output)
        };
        let defaults = TransformOptions::default();
        assert_eq!(run(&template, 550, &defaults).unwrap(), r("151525"));
        assert_eq!(run(&function, 40, &defaults).unwrap(), r("820"));
        let nesting = TransformOptions {
            max_nesting: 10,
            ..TransformOptions::default()
        };
        for e in [&template, &function] {
            assert_eq!(run(e, 8, &nesting).unwrap(), r("36"));
            assert_eq!(
                limit(run(e, 9, &nesting)),
                LimitExceeded::Nesting { max: 10 }
            );
        }
        // The function's argument is computed lazily, inside its body, as Saxon computes
        // it: one evaluation inside another, on the call stack. The named templates run
        // as tasks, on the heap: the stack does not bound them.
        let stack = TransformOptions {
            max_stack: 64 << 10,
            ..TransformOptions::default()
        };
        assert_eq!(run(&function, 0, &stack).unwrap(), r("0"));
        assert_eq!(
            limit(run(&function, 200, &stack)),
            LimitExceeded::Stack { max: 64 << 10 }
        );
        assert_eq!(run(&template, 200, &stack).unwrap(), r("20100"));
        assert_eq!(run(&template, 550, &stack).unwrap(), r("151525"));
        // Tail calls need neither nesting nor stack, and are counted.
        assert_eq!(run(&tail, 60_000, &defaults).unwrap(), r("1800030000"));
        assert_eq!(run(&tail, 200, &nesting).unwrap(), r("20100"));
        assert_eq!(run(&tail, 200, &stack).unwrap(), r("20100"));
        let tail_calls = TransformOptions {
            max_tail_calls: 8,
            ..TransformOptions::default()
        };
        assert_eq!(run(&tail, 8, &tail_calls).unwrap(), r("36"));
        assert_eq!(
            limit(run(&tail, 9, &tail_calls)),
            LimitExceeded::TailCalls { max: 8 }
        );
    });
}

/// The least `max_stack` with which `executable` transforms `input` (to 16 bytes), the
/// depth and the nesting not limited: the stack the transformation uses, as the limit
/// measures it. At most [`DEFAULT_MAX_STACK`].
fn least_stack(executable: &Executable, input: &str) -> u32 {
    let runs = |max_stack: u32| {
        let options = TransformOptions {
            max_depth: u32::MAX,
            max_nesting: u32::MAX,
            max_stack,
            ..TransformOptions::default()
        };
        match executable.transform_with(input, None, &options) {
            Ok(_) => true,
            Err(e) if e.limit() == Some(LimitExceeded::Stack { max: max_stack }) => false,
            Err(e) => panic!("{e}"),
        }
    };
    let (mut low, mut high) = (0, DEFAULT_MAX_STACK);
    assert!(runs(high), "{input:.80}: more than the default stack");
    while high - low > 16 {
        let middle = low + (high - low) / 2;
        if runs(middle) {
            high = middle;
        } else {
            low = middle;
        }
    }
    high
}

/// A recursion through the templates does not use the call stack (issue #39): the stack
/// a transformation uses, as the limit measures it, is the same (to the 16 bytes of
/// [`least_stack`]) at two depths of recursion, in the build being tested (debug for
/// `cargo test`), on every platform the tests run on: a named template calling itself
/// (not as a tail call), template rules applied to the children (`xsl:apply-templates`
/// in a template rule, as the ISO Schematron skeleton writes them), the built-in rules,
/// 256 and 4096 levels; the CEN stylesheet on a document 128 and [`DEFAULT_MAX_DEPTH`]
/// deep, no rule firing on the nested elements, and with a rule firing and reporting on
/// every one of them (UBL-DT-01 on amounts with three fraction digits), whose location
/// the ISO Schematron skeleton writes with a template applied to each ancestor in turn.
#[test]
fn the_stack_does_not_grow_with_the_recursion() {
    on_a_default_stack(|| {
        let named = prepare(
            r#"<xsl:template match="/"><r><xsl:call-template name="down"><xsl:with-param name="n" select="xs:integer(/n)"/></xsl:call-template></r></xsl:template>
<xsl:template name="down"><xsl:param name="n"/><xsl:choose><xsl:when test="$n eq 0"><xsl:value-of select="$n"/></xsl:when><xsl:otherwise><xsl:call-template name="down"><xsl:with-param name="n" select="$n - 1"/></xsl:call-template><xsl:sequence select="()"/></xsl:otherwise></xsl:choose></xsl:template>"#,
        );
        let rules = prepare(
            r#"<xsl:template match="x"><xsl:apply-templates select="@*|*"/></xsl:template>"#,
        );
        let built_in =
            prepare(r#"<xsl:template match="/"><r><xsl:apply-templates/></r></xsl:template>"#);
        let cen = common::executable(&common::workspace_root()).unwrap();
        let number = |n: usize| format!("<n>{n}</n>");
        let elements = |n: usize| nested(n, "");
        let invoice = |n: usize| deep_invoice(n, n).0;
        let amounts = |n: usize| {
            format!(
                r#"<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2">{}1.123{}</Invoice>"#,
                "<Amount>".repeat(n - 1),
                "</Amount>".repeat(n - 1)
            )
        };
        let stack =
            |executable: &Executable, input: &dyn Fn(usize) -> String, depths: [usize; 2]| {
                depths.map(|depth| least_stack(executable, &input(depth)))
            };
        let measured = [
            ("named template", stack(&named, &number, [256, 4096])),
            ("template rule", stack(&rules, &elements, [256, 4096])),
            ("built-in rule", stack(&built_in, &elements, [256, 4096])),
            ("CEN stylesheet", stack(&cen, &invoice, [128, 256])),
            (
                "CEN stylesheet, reporting",
                stack(&cen, &amounts, [128, 256]),
            ),
        ];
        assert!(
            measured
                .iter()
                .all(|&(_, [shallow, deep])| deep <= shallow + 16),
            "{measured:?}"
        );
    });
}

/// A lazy value that refers to another, and so on, is computed inside the one that refers
/// to it: a parameter passed down a recursion and read only at its end (Saxon passes the
/// parameters of `xsl:call-template` lazily) is a chain of computations as deep as the
/// recursion, bounded by the stack limit too.
#[test]
fn chains_of_lazy_values_are_bounded() {
    let template = prepare(
        r#"<xsl:template match="/"><r><xsl:call-template name="sum"><xsl:with-param name="n" select="xs:integer(/n)"/></xsl:call-template></r></xsl:template>
<xsl:template name="sum"><xsl:param name="n"/><xsl:param name="total" select="0"/><xsl:choose><xsl:when test="$n eq 0"><xsl:value-of select="$total"/></xsl:when><xsl:otherwise><xsl:call-template name="sum"><xsl:with-param name="n" select="$n - 1"/><xsl:with-param name="total" select="$total + $n"/></xsl:call-template></xsl:otherwise></xsl:choose></xsl:template>"#,
    );
    on_a_default_stack(move || {
        let run = |n: u32| {
            template
                .transform(format!("<n>{n}</n>").as_str(), None)
                .map(|t| t.output)
        };
        assert_eq!(run(40).unwrap(), r("820"));
        assert_eq!(
            limit(run(550)),
            LimitExceeded::Stack {
                max: DEFAULT_MAX_STACK
            }
        );
    });
}

/// The built-in template rules recurse once per element level: a document deeper than
/// the nesting allows (with a raised maximum depth) is a `LIMIT` error.
#[test]
fn built_in_rules_are_counted() {
    let executable =
        prepare(r#"<xsl:template match="/"><r><xsl:apply-templates/></r></xsl:template>"#);
    on_a_default_stack(move || {
        let options = TransformOptions {
            max_depth: 100_000,
            ..TransformOptions::default()
        };
        // The template rule of the document node, then the built-in rules of 575
        // elements: 576 invocations nested.
        let text = nested(575, "t");
        assert_eq!(
            executable
                .transform_with(text.as_str(), None, &options)
                .unwrap()
                .output,
            r("t")
        );
        let text = nested(576, "t");
        assert_eq!(
            limit(executable.transform_with(text.as_str(), None, &options)),
            LimitExceeded::Nesting { max: 576 }
        );
    });
}

/// What walks a whole tree without template rules does it without recursion: copying a
/// document 100 000 deep to the result and into a temporary tree, querying it, and
/// writing the path of its innermost node in a warning (an ambiguous rule match).
#[test]
fn deep_trees_are_walked_without_recursion() {
    const DEPTH: usize = 100_000;
    let options = || TransformOptions {
        max_depth: 100_000,
        ..TransformOptions::default()
    };
    let copy = prepare(r#"<xsl:template match="/"><xsl:sequence select="."/></xsl:template>"#);
    let tree = prepare(
        r#"<xsl:template match="/"><xsl:variable name="t"><xsl:sequence select="/"/></xsl:variable><r><xsl:value-of select="count($t//x), string($t), count(($t//x)[last()]/ancestor::*)"/></r></xsl:template>"#,
    );
    let ambiguous = prepare(
        r#"<xsl:template match="/"><r><xsl:apply-templates select="//x[not(*)]"/></r></xsl:template>
<xsl:template match="x"><a/></xsl:template>
<xsl:template match="x"><b/></xsl:template>"#,
    );
    on_a_default_stack(move || {
        let text = nested(DEPTH, "t");
        assert_eq!(
            copy.transform_with(text.as_str(), None, &options())
                .unwrap()
                .output,
            text
        );
        assert_eq!(
            tree.transform_with(text.as_str(), None, &options())
                .unwrap()
                .output,
            r(&format!("{DEPTH} t {}", DEPTH - 1))
        );
        let result = ambiguous
            .transform_with(text.as_str(), None, &options())
            .unwrap();
        assert_eq!(result.output, r("<b/>"));
        assert_eq!(result.warnings.len(), 1, "{:?}", result.warnings);
        let path = format!("/x{}", "/x[1]".repeat(DEPTH - 1));
        assert!(
            result.warnings[0].contains(&format!("Ambiguous rule match for {path} in mode")),
            "{}",
            result.warnings[0].chars().take(200).collect::<String>()
        );
    });
}

struct Deep;

impl DocumentResolver for Deep {
    fn resolve(&self, uri: &str) -> Option<String> {
        let depth = uri.strip_prefix("urn:deep:")?.parse().ok()?;
        Some(nested(depth, ""))
    }
}

/// A document loaded by `document()` has the same maximum depth: beyond it, a `LIMIT`
/// error naming the document, also from a pattern (not recovered as a warning).
#[test]
fn documents_loaded_by_document_have_the_maximum_depth() {
    let executable = prepare(
        r#"<xsl:template match="/"><r><xsl:value-of select="count(document(string(/d))//x)"/></r></xsl:template>"#,
    );
    let in_pattern = prepare(
        r#"<xsl:template match="d[count(document(string(.))//x) gt 0]"><m/></xsl:template>"#,
    );
    on_a_default_stack(move || {
        let options = TransformOptions {
            resolver: Some(&Deep),
            ..TransformOptions::default()
        };
        let ok = executable
            .transform_with("<d>urn:deep:256</d>", None, &options)
            .unwrap();
        assert_eq!(ok.output, r("256"));
        for sheet in [&executable, &in_pattern] {
            let error = sheet
                .transform_with("<d>urn:deep:257</d>", None, &options)
                .unwrap_err();
            assert_eq!(
                error.limit(),
                Some(LimitExceeded::Depth {
                    max: 256,
                    line: 1,
                    column: 1 + 3 * 256
                }),
                "{error}"
            );
            assert!(error.message().starts_with("urn:deep:257: "), "{error}");
        }
    });
}
