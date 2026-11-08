//! Constructs outside the supported subset are refused with an error that names them, at
//! the right position, and are never parsed as something else.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

use einvoice_xpath::{
    ErrorKind, NamespaceMap, XS_NAMESPACE, parse_expr, parse_pattern, parse_sequence_type,
};

fn ns() -> NamespaceMap {
    NamespaceMap::new()
        .bind("xs", XS_NAMESPACE)
        .bind("p", "urn:p")
}

#[track_caller]
fn refused(
    result: Result<impl std::fmt::Debug, einvoice_xpath::Error>,
    src: &str,
    offset: usize,
    needle: &str,
) {
    let err = result.expect_err(src);
    assert_eq!(err.kind, ErrorKind::Unsupported, "{src}: {err}");
    assert_eq!(err.offset, offset, "{src}: {err}");
    assert!(
        err.message.contains(needle),
        "{src}: `{}` lacks `{needle}`",
        err.message
    );
    assert!(
        err.to_string().starts_with("unsupported at column "),
        "{err}"
    );
}

#[test]
fn unsupported_expressions() {
    for (src, offset, needle) in [
        ("a intersect b", 2, "`intersect` operator"),
        ("a except b", 2, "`except` operator"),
        ("a union b", 2, "`union` operator"),
        ("a treat as xs:integer", 2, "`treat as`"),
        ("a is b", 2, "node comparison `is`"),
        ("a << b", 2, "node comparison `<<`"),
        ("a >> b", 2, "node comparison `>>`"),
        ("1 (: comment :) + 2", 2, "XPath comment"),
        ("+1", 0, "unary `+`"),
        ("1 + +1", 4, "unary `+`"),
        ("descendant::a", 0, "axis `descendant::`"),
        ("a/following::b", 2, "axis `following::`"),
        ("namespace::*", 0, "axis `namespace::`"),
        ("a ! b", 2, "simple map operator"),
        ("a || b", 2, "`||`"),
        ("a => f()", 2, "arrow operator"),
        ("let $x := 1 return $x", 0, "`let`"),
        (
            "typeswitch (a) case xs:integer return 1 default return 2",
            0,
            "`typeswitch`",
        ),
        ("function($x) { $x }", 0, "inline function"),
        ("processing-instruction('pi')", 23, "with a name argument"),
        ("document-node()", 0, "kind test `document-node()`"),
        ("schema-element(p:a)", 0, "kind test `schema-element()`"),
        ("schema-attribute(p:a)", 0, "kind test `schema-attribute()`"),
        ("element(p:a, xs:string)", 11, "type name in `element()`"),
        ("attribute(*, xs:string)", 11, "type name in `attribute()`"),
        ("$a instance of item()", 15, "`item()`"),
        ("$a instance of empty-sequence()", 15, "`empty-sequence()`"),
        ("$a instance of xs:double", 15, "atomic type `xs:double`"),
        (
            "$a castable as xs:dateTime",
            15,
            "atomic type `xs:dateTime`",
        ),
        ("$a cast as xs:double", 11, "atomic type `xs:double`"),
    ] {
        refused(parse_expr(src, &ns()), src, offset, needle);
    }
}

#[test]
fn unsupported_sequence_types() {
    for (src, offset, needle) in [
        ("item()*", 0, "`item()`"),
        ("empty-sequence()", 0, "`empty-sequence()`"),
        ("xs:double", 0, "atomic type `xs:double`"),
        ("document-node()", 0, "kind test `document-node()`"),
    ] {
        refused(parse_sequence_type(src, &ns()), src, offset, needle);
    }
    let err = parse_sequence_type("p:mytype", &ns()).unwrap_err();
    assert_eq!(err.kind, ErrorKind::UnknownType, "{err}");
}

#[test]
fn unsupported_patterns() {
    for (src, offset, needle) in [
        ("id('x')", 0, "`id()` pattern"),
        ("key('k', 'v')", 0, "`key()` pattern"),
        ("doc('a.xml')", 0, "`doc()` pattern"),
        ("a/descendant::b", 2, "axis `descendant::` in a pattern"),
        ("self::a", 0, "axis `self::` in a pattern"),
        (
            "following-sibling::a",
            0,
            "axis `following-sibling::` in a pattern",
        ),
        ("$x", 0, "variable reference as a pattern"),
        (".[1]", 0, "predicate pattern"),
        (
            "r/(a | b)[@x]",
            0,
            "a predicate on a parenthesized step after the first",
        ),
        (
            "r/(a | b)[1]/c",
            0,
            "a predicate on a parenthesized step after the first",
        ),
        (
            "r/(a/b | c)/d",
            0,
            "a parenthesized step after the first whose alternatives are not single steps",
        ),
        (
            "x | r/(/a | b)",
            4,
            "an absolute path or a parenthesized step in a parenthesized pattern after a step",
        ),
        (
            "r/(/ | b)",
            0,
            "`/` or `root()` in a parenthesized pattern after a step",
        ),
        (
            "((a | b)[1]/c)/d",
            0,
            "a parenthesized pattern nested in a parenthesized step",
        ),
        (
            "/(a/b)[@x]",
            1,
            "after `/` or `//`, a parenthesized step whose alternatives are not single steps",
        ),
        ("root()/a", 6, "after `root()`"),
        ("root()[1]", 6, "after `root()`"),
        ("a union b", 2, "`union` operator in a pattern"),
        ("a except b", 2, "`except` operator in a pattern"),
        ("a[. intersect b]", 4, "`intersect` operator"),
    ] {
        refused(parse_pattern(src, &ns()), src, offset, needle);
    }
}

#[test]
fn syntax_errors_are_not_unsupported() {
    for src in [
        "a +",
        "f(",
        "a[1",
        "1 2",
        "child::",
        "bogus::a",
        "$",
        "@",
        "if (a) then b",
        "for $x in a",
        "a/if(b)",
    ] {
        let err = parse_expr(src, &ns()).expect_err(src);
        assert_eq!(err.kind, ErrorKind::Syntax, "{src}: {err}");
    }
    let err = parse_expr("q:a", &ns()).unwrap_err();
    assert_eq!(err.kind, ErrorKind::UndeclaredPrefix, "{err}");
    let err = parse_pattern("a b", &ns()).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Syntax, "{err}");
}
