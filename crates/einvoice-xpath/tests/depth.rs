//! The maximum depth of an expression or pattern (issue #33): every form of nesting is
//! read up to [`MAX_DEPTH`] and refused one level beyond, with an
//! [`ErrorKind::TooDeep`] error, never a stack overflow. Every test runs on a thread with
//! the default stack of a spawned thread, 2 MiB, in the build being tested (debug for
//! `cargo test`): the tree read at the limit is printed, parsed again, cloned, compared
//! and dropped there too.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

use einvoice_xpath::{
    Error, ErrorKind, MAX_DEPTH, NamespaceMap, XS_NAMESPACE, parse_avt, parse_expr, parse_pattern,
};

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

/// A form of nesting: the text with `n` levels of it, and the `n` whose depth (or
/// nesting of the parser) is exactly [`MAX_DEPTH`].
struct Case {
    name: &'static str,
    text: fn(usize) -> String,
    at_limit: usize,
}

/// Every form of nesting the grammar has.
#[expect(clippy::too_many_lines, reason = "one entry per form of nesting")]
fn cases() -> Vec<Case> {
    let mut cases = vec![
        // The top-level expression is one level, each parenthesized one another.
        Case {
            name: "parentheses",
            text: |n| wrapped("(", "1", ")", n),
            at_limit: MAX - 1,
        },
        Case {
            name: "function calls",
            text: |n| wrapped("not(", "true()", ")", n),
            at_limit: MAX - 1,
        },
        Case {
            name: "function arguments",
            text: |n| wrapped("concat('x', ", "'a'", ")", n),
            at_limit: MAX - 1,
        },
        Case {
            name: "unary minus",
            text: |n| format!("{}1", "-".repeat(n)),
            at_limit: MAX - 1,
        },
        Case {
            name: "negated parentheses",
            text: |n| wrapped("-(", "1", ")", n),
            at_limit: MAX - 1,
        },
        Case {
            name: "cast as",
            text: |n| wrapped("(", "1", ") cast as xs:integer", n),
            at_limit: MAX - 1,
        },
        Case {
            name: "castable as",
            text: |n| wrapped("(", "1", ") castable as xs:boolean", n),
            at_limit: MAX - 1,
        },
        Case {
            name: "instance of",
            text: |n| wrapped("(", "1", ") instance of xs:boolean", n),
            at_limit: MAX - 1,
        },
        // A path is one level per step, `/` one more, `//` two (its
        // `descendant-or-self::node()` step).
        Case {
            name: "path steps",
            text: |n| vec!["a"; n].join("/"),
            at_limit: MAX,
        },
        Case {
            name: "absolute path steps",
            text: |n| format!("/{}", vec!["a"; n].join("/")),
            at_limit: MAX - 1,
        },
        Case {
            name: "descendant steps",
            text: |n| "//a".repeat(n),
            at_limit: (MAX - 1) / 2,
        },
        Case {
            name: "parent steps",
            text: |n| vec![".."; n].join("/"),
            at_limit: MAX,
        },
        Case {
            name: "predicates of a filter",
            text: |n| format!(".{}", "[1]".repeat(n)),
            at_limit: MAX - 1,
        },
        Case {
            name: "predicates of a step",
            text: |n| format!("a{}", "[1]".repeat(n)),
            at_limit: MAX - 1,
        },
        Case {
            name: "nested predicates",
            text: |n| wrapped("a[", "1", "]", n),
            at_limit: MAX - 1,
        },
        Case {
            name: "filter steps",
            text: |n| vec!["(.)"; n].join("/"),
            at_limit: MAX,
        },
        Case {
            name: "if",
            text: |n| wrapped("if (1) then ", "1", " else 0", n),
            at_limit: MAX - 1,
        },
        Case {
            name: "if conditions",
            text: |n| wrapped("if (", "1", ") then 1 else 0", n),
            at_limit: MAX - 1,
        },
        Case {
            name: "for",
            text: |n| format!("{}1", "for $x in 1 return ".repeat(n)),
            at_limit: MAX - 1,
        },
        Case {
            name: "for sequences",
            text: |n| wrapped("for $x in ", "1", " return $x", n),
            at_limit: MAX - 1,
        },
        Case {
            name: "some",
            text: |n| format!("{}true()", "some $x in 1 satisfies ".repeat(n)),
            at_limit: MAX - 1,
        },
        Case {
            name: "every",
            text: |n| format!("{}true()", "every $x in 1 satisfies ".repeat(n)),
            at_limit: MAX - 1,
        },
        // Each binding binds inside the one before it.
        Case {
            name: "bindings of for",
            text: |n| format!("for {} return 1", bindings(n)),
            at_limit: MAX - 1,
        },
        Case {
            name: "bindings of some",
            text: |n| format!("some {} satisfies true()", bindings(n)),
            at_limit: MAX - 1,
        },
        Case {
            name: "bindings of every",
            text: |n| format!("every {} satisfies true()", bindings(n)),
            at_limit: MAX - 1,
        },
        Case {
            name: "sequences",
            text: |n| wrapped("(1, ", "1", ")", n),
            at_limit: MAX - 1,
        },
    ];
    // Left-associative operators: a chain of `n` operands is `n` deep.
    for (name, text) in [
        (
            "or chain",
            (|n| vec!["1"; n].join(" or ")) as fn(usize) -> String,
        ),
        ("and chain", |n| vec!["1"; n].join(" and ")),
        ("+ chain", |n| vec!["1"; n].join(" + ")),
        ("- chain", |n| vec!["1"; n].join(" - ")),
        ("* chain", |n| vec!["1"; n].join(" * ")),
        ("div chain", |n| vec!["1"; n].join(" div ")),
        ("idiv chain", |n| vec!["1"; n].join(" idiv ")),
        ("mod chain", |n| vec!["1"; n].join(" mod ")),
        ("| chain", |n| vec!["."; n].join(" | ")),
    ] {
        cases.push(Case {
            name,
            text,
            at_limit: MAX,
        });
    }
    // Comparisons and `to` do not chain: each nests in parentheses.
    for (name, text) in [
        (
            "=",
            (|n| wrapped("1 = (", "1", ")", n)) as fn(usize) -> String,
        ),
        ("!=", |n| wrapped("1 != (", "1", ")", n)),
        ("<", |n| wrapped("1 < (", "1", ")", n)),
        ("eq", |n| wrapped("1 eq (", "1", ")", n)),
        ("ge", |n| wrapped("1 ge (", "1", ")", n)),
        ("to", |n| wrapped("1 to (", "1", ")", n)),
    ] {
        cases.push(Case {
            name,
            text,
            at_limit: MAX - 1,
        });
    }
    cases
}

/// `$x0 in 1, $x1 in 1, ...`
fn bindings(n: usize) -> String {
    (0..n)
        .map(|i| format!("$x{i} in 1"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn too_deep(error: &Error, what: &str) {
    assert_eq!(error.kind, ErrorKind::TooDeep, "{what}: {error}");
    assert!(
        error.message.contains(&format!(
            "nested deeper than the maximum depth of {MAX_DEPTH}"
        )),
        "{what}: {error}"
    );
}

#[test]
fn the_maximum_depth() {
    assert_eq!(MAX_DEPTH, 128);
}

#[test]
fn every_form_of_nesting_up_to_the_limit_and_beyond() {
    on_a_default_stack(|| {
        for case in cases() {
            let text = (case.text)(case.at_limit);
            let expr = parse_expr(&text, &ns()).unwrap_or_else(|e| panic!("{}: {e}", case.name));
            // Printed and parsed again, still at the limit.
            let printed = expr.to_string();
            let again =
                parse_expr(&printed, &ns()).unwrap_or_else(|e| panic!("{}: {e}", case.name));
            assert_eq!(again, expr, "{}", case.name);
            let copy = expr.clone();
            assert_eq!(copy, expr, "{}", case.name);
            assert_ne!(format!("{copy:?}"), "");
            drop((expr, again, copy));
            let deeper = (case.text)(case.at_limit + 1);
            let error = parse_expr(&deeper, &ns()).unwrap_err();
            too_deep(&error, case.name);
            assert!(error.offset < deeper.len(), "{}: {error}", case.name);
        }
    });
}

#[test]
fn where_a_too_deep_expression_is_reported() {
    // The 128th `(` opens the 129th level.
    let error = parse_expr(&wrapped("(", "1", ")", 200), &ns()).unwrap_err();
    too_deep(&error, "parentheses");
    assert_eq!((error.offset, error.column), (128, 129));
    // The 128th `+` makes the 129th level.
    let chain = vec!["1"; 200].join("+");
    let error = parse_expr(&chain, &ns()).unwrap_err();
    too_deep(&error, "+ chain");
    assert_eq!(error.offset, 2 * 128 - 1);
    assert_eq!(
        error.to_string(),
        "too deep at column 256: expression nested deeper than the maximum depth of 128"
    );
}

#[test]
fn a_million_levels_never_overflow() {
    on_a_default_stack(|| {
        const LEVELS: usize = 1_000_000;
        for case in cases() {
            let text = (case.text)(LEVELS);
            too_deep(&parse_expr(&text, &ns()).unwrap_err(), case.name);
        }
    });
}

/// A form of nesting of patterns: its name, the text with `n` levels of it, and the `n`
/// at the limit.
type PatternCase = (&'static str, fn(usize) -> String, usize);

#[test]
fn patterns_up_to_the_limit_and_beyond() {
    on_a_default_stack(|| {
        let patterns: [PatternCase; 6] = [
            ("pattern steps", |n| vec!["a"; n].join("/"), MAX),
            ("descendant pattern steps", |n| vec!["a"; n].join("//"), MAX),
            (
                "pattern predicates",
                |n| format!("a{}", "[1]".repeat(n)),
                MAX - 1,
            ),
            (
                "nested pattern predicates",
                |n| wrapped("a[", "1", "]", n),
                MAX - 1,
            ),
            // A pattern is not an expression: its parentheses are counted from the first.
            ("parenthesized patterns", |n| wrapped("(", "a", ")", n), MAX),
            (
                "parenthesized alternatives",
                |n| wrapped("(a | ", "a", ")", n),
                MAX,
            ),
        ];
        for (name, text, at_limit) in patterns {
            let pattern =
                parse_pattern(&text(at_limit), &ns()).unwrap_or_else(|e| panic!("{name}: {e}"));
            let again = parse_pattern(&pattern.to_string(), &ns())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(again, pattern, "{name}");
            too_deep(
                &parse_pattern(&text(at_limit + 1), &ns()).unwrap_err(),
                name,
            );
            too_deep(&parse_pattern(&text(1_000_000), &ns()).unwrap_err(), name);
        }
    });
}

#[test]
fn attribute_value_templates_up_to_the_limit_and_beyond() {
    on_a_default_stack(|| {
        for case in cases() {
            // Each expression of a template has the maximum depth of its own.
            let expr = (case.text)(case.at_limit);
            let avt = format!("a{{{expr}}}b{{{expr}}}");
            let parsed = parse_avt(&avt, &ns()).unwrap_or_else(|e| panic!("{}: {e}", case.name));
            assert_eq!(
                parse_avt(&parsed.to_string(), &ns()).unwrap(),
                parsed,
                "{}",
                case.name
            );
            let deeper = format!("a{{{expr}}}b{{{}}}", (case.text)(case.at_limit + 1));
            too_deep(&parse_avt(&deeper, &ns()).unwrap_err(), case.name);
        }
    });
}

/// Long lists that do not nest are not limited: arguments, the items of a sequence, the
/// alternatives of a pattern.
#[test]
fn flat_lists_are_not_limited() {
    on_a_default_stack(|| {
        let n = 10 * MAX;
        let args = format!("concat({})", vec!["'a'"; n].join(", "));
        parse_expr(&args, &ns()).unwrap();
        let items = format!("({})", vec!["1"; n].join(", "));
        parse_expr(&items, &ns()).unwrap();
        let alternatives = vec!["a/b"; n].join(" | ");
        assert_eq!(
            parse_pattern(&alternatives, &ns())
                .unwrap()
                .alternatives
                .len(),
            n
        );
        let parts = "{1}x".repeat(n);
        assert_eq!(parse_avt(&parts, &ns()).unwrap().parts.len(), 2 * n);
    });
}
