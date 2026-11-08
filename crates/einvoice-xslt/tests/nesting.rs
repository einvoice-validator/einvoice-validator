//! The maximum nesting of instructions (issue #33): every way of nesting sequence
//! constructors is read up to [`MAX_NESTING`] levels and refused one level beyond, with
//! a [`LoadErrorKind::TooDeep`] error at the line of the first content too deep, never a
//! stack overflow; the expressions of a stylesheet, attribute value templates included,
//! are bounded by [`MAX_DEPTH`]. Every test runs on a thread with the default stack of
//! a spawned thread, 2 MiB, in the build being tested (debug for `cargo test`).

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

use einvoice_xpath::{ErrorKind, MAX_DEPTH};
use einvoice_xslt::{LoadErrorKind, LoadErrors, MAX_NESTING, Stylesheet, load};

/// The nesting limit, as a count.
const MAX: usize = MAX_NESTING as usize;

fn on_a_default_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

const HEAD: &str =
    r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">"#;

/// One way of nesting: the start and end tags around the content of one level.
type Level = (&'static str, &'static str);

/// Every instruction whose content is a sequence constructor, and literal result
/// elements; each level on its own line.
const LEVELS: [(&str, Level); 11] = [
    ("xsl:if", ("<xsl:if test='1'>\n", "</xsl:if>")),
    (
        "xsl:when",
        (
            "<xsl:choose><xsl:when test='1'>\n",
            "</xsl:when></xsl:choose>",
        ),
    ),
    (
        "xsl:otherwise",
        (
            "<xsl:choose><xsl:when test='0'/><xsl:otherwise>\n",
            "</xsl:otherwise></xsl:choose>",
        ),
    ),
    (
        "xsl:for-each",
        ("<xsl:for-each select='.'>\n", "</xsl:for-each>"),
    ),
    (
        "xsl:variable",
        ("<xsl:variable name='v'>\n", "</xsl:variable>"),
    ),
    (
        "xsl:with-param",
        (
            "<xsl:call-template name='t'><xsl:with-param name='p'>\n",
            "</xsl:with-param></xsl:call-template>",
        ),
    ),
    ("xsl:value-of", ("<xsl:value-of>\n", "</xsl:value-of>")),
    (
        "xsl:attribute",
        ("<xsl:attribute name='a'>\n", "</xsl:attribute>"),
    ),
    ("xsl:comment", ("<xsl:comment>\n", "</xsl:comment>")),
    ("literal element", ("<e>\n", "</e>")),
    ("xsl:text in a literal element", ("<e>\n", "</e>")),
];

/// A template whose body nests `n` levels, the content of the innermost being `inner`:
/// the body of the template is level 1, so `inner` is at level `n + 1`.
fn template(level: Level, n: usize, inner: &str) -> String {
    format!(
        "{HEAD}\n<xsl:template name='t'><xsl:param name='p'/></xsl:template>\n<xsl:template match='/'>\n{}{inner}{}</xsl:template>\n</xsl:stylesheet>",
        level.0.repeat(n),
        level.1.repeat(n)
    )
}

fn too_deep(errors: &LoadErrors) -> Vec<u32> {
    errors
        .0
        .iter()
        .map(|e| {
            assert_eq!(e.kind, LoadErrorKind::TooDeep { max: MAX_NESTING }, "{e}");
            assert_eq!(
                e.to_string(),
                format!(
                    "line {}: instructions nested deeper than the maximum of {MAX_NESTING} levels",
                    e.line
                )
            );
            e.line
        })
        .collect()
}

#[test]
fn the_maximum_nesting() {
    assert_eq!(MAX_NESTING, 64);
}

#[test]
fn every_way_of_nesting_up_to_the_limit_and_beyond() {
    on_a_default_stack(|| {
        for (name, level) in LEVELS {
            let inner = if name.starts_with("xsl:text") {
                "<xsl:text>x</xsl:text>"
            } else {
                "x"
            };
            let at_limit = template(level, MAX - 1, inner);
            let sheet: Stylesheet = load(&at_limit).unwrap_or_else(|e| panic!("{name}: {e}"));
            let copy = sheet.clone();
            assert_eq!(copy, sheet, "{name}");
            drop((sheet, copy));
            // The content of the 64th level is one level too deep, reported at its line:
            // HEAD, the template t and the template / are the first 3 lines, then a line
            // per level, the content of the innermost starting on the line of its start
            // tag (the text `x` after a line break), or on the next line (`xsl:text`, the
            // line break before it being whitespace stripped).
            let errors = load(&template(level, MAX, inner)).unwrap_err();
            let line = 3 + MAX + usize::from(name.starts_with("xsl:text"));
            assert_eq!(too_deep(&errors), [u32::try_from(line).unwrap()], "{name}");
        }
    });
}

#[test]
fn a_million_levels_never_overflow() {
    on_a_default_stack(|| {
        // Without line breaks, every level is on line 4.
        let errors = load(&template(("<e>", "</e>"), 1_000_000, "x")).unwrap_err();
        assert_eq!(too_deep(&errors), [4]);
        for (name, level) in LEVELS {
            let errors = load(&template(level, 100_000, "x")).unwrap_err();
            assert_eq!(too_deep(&errors).len(), 1, "{name}");
        }
    });
}

#[test]
fn every_declaration_starts_at_level_one() {
    on_a_default_stack(|| {
        let level = ("<xsl:if test='1'>", "</xsl:if>");
        let nested = |n: usize| {
            format!(
                "{}<xsl:sequence select='1'/>{}",
                level.0.repeat(n),
                level.1.repeat(n)
            )
        };
        let declarations = |n: usize| {
            format!(
                "{HEAD}<xsl:variable name='g'>{0}</xsl:variable><xsl:param name='q'>{0}</xsl:param><xsl:function name='f:f' xmlns:f='urn:f'><xsl:param name='a'/>{0}</xsl:function><xsl:template match='/'><xsl:param name='p'>{0}</xsl:param>{0}</xsl:template></xsl:stylesheet>",
                nested(n)
            )
        };
        load(&declarations(MAX - 1)).unwrap_or_else(|e| panic!("{e}"));
        let errors = load(&declarations(MAX)).unwrap_err();
        assert_eq!(too_deep(&errors).len(), 5);
    });
}

#[test]
fn deep_expressions_and_attribute_value_templates() {
    on_a_default_stack(|| {
        // `not(...)` `n` deep.
        let not = |n: usize| format!("{}true(){}", "not(".repeat(n - 1), ")".repeat(n - 1));
        let depth = MAX_DEPTH as usize;
        // At the deepest level of instructions (the content of `xsl:for-each` at level
        // 64), every kind of attribute holding an expression, a pattern (where a
        // predicate is one level below its step) or an attribute value template, each at
        // the maximum depth (`extra` 0) or one level deeper (`extra` 1).
        let attributes = |extra: usize| {
            let (expr, predicate) = (not(depth + extra), not(depth - 1 + extra));
            let level = ("<xsl:if test='1'>", "</xsl:if>");
            format!(
                "{HEAD}<xsl:template match='a[{predicate}]'>{}<e a='{{{expr}}}' b='x{{{expr}}}y'><xsl:attribute name='n{{{expr}}}' select='{expr}'/><xsl:value-of select='{expr}' separator='{{{expr}}}'/><xsl:for-each select='{expr}'><xsl:number count='a[{predicate}]'/></xsl:for-each><xsl:variable name='v' select='{expr}'/><xsl:sequence select='{expr}'/><xsl:apply-templates select='{expr}'><xsl:with-param name='w' select='{expr}'/></xsl:apply-templates><xsl:choose><xsl:when test='{expr}'/></xsl:choose></e>{}</xsl:template></xsl:stylesheet>",
                level.0.repeat(MAX - 3),
                level.1.repeat(MAX - 3)
            )
        };
        load(&attributes(0)).unwrap_or_else(|e| panic!("{e}"));
        let errors = load(&attributes(1)).unwrap_err();
        assert_eq!(errors.0.len(), 14, "{errors}");
        for error in &errors.0 {
            match &error.kind {
                LoadErrorKind::XPath { error, .. } => assert_eq!(error.kind, ErrorKind::TooDeep),
                other => panic!("{other:?}"),
            }
        }
    });
}
