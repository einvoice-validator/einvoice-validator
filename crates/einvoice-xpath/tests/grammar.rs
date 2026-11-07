//! Precedence, associativity and the shape of the tree, written as S-expressions.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

use einvoice_xpath::{
    ArithmeticOp, AxisStep, CompareKind, ComparisonOp, Expr, FN_NAMESPACE, KindTest, Literal,
    NameOrWildcard, NameTest, NamespaceMap, NodeTest, Occurrence, PathStart, Pattern,
    PatternAlternative, PatternAnchor, PatternAxis, PatternLink, Quantifier, Step, XS_NAMESPACE,
    parse_avt, parse_expr, parse_pattern, parse_sequence_type,
};

fn ns() -> NamespaceMap {
    NamespaceMap::new()
        .bind("xs", XS_NAMESPACE)
        .bind("p", "urn:p")
        .bind("u", "urn:u")
}

fn node_test(test: &NodeTest) -> String {
    match test {
        NodeTest::Name(NameTest::Name(q)) => match &q.namespace {
            Some(uri) => format!("{{{uri}}}{}", q.local),
            None => q.local.clone(),
        },
        NodeTest::Name(NameTest::Any) => "*".into(),
        NodeTest::Name(NameTest::AnyLocal { namespace, .. }) => format!("{{{namespace}}}*"),
        NodeTest::Name(NameTest::AnyNamespace { local }) => format!("*:{local}"),
        NodeTest::Kind(kind) => kind.to_string(),
    }
}

fn list(items: &[Expr]) -> String {
    items.iter().map(shape).collect::<Vec<_>>().join(" ")
}

fn preds(predicates: &[Expr]) -> String {
    predicates
        .iter()
        .map(|p| ["[", &shape(p), "]"].concat())
        .collect()
}

fn comparison(op: ComparisonOp) -> &'static str {
    let (kind, general) = match op {
        ComparisonOp::General(k) => (k, true),
        ComparisonOp::Value(k) => (k, false),
    };
    match (kind, general) {
        (CompareKind::Eq, true) => "=",
        (CompareKind::Ne, true) => "!=",
        (CompareKind::Lt, true) => "<",
        (CompareKind::Le, true) => "<=",
        (CompareKind::Gt, true) => ">",
        (CompareKind::Ge, true) => ">=",
        (CompareKind::Eq, false) => "eq",
        (CompareKind::Ne, false) => "ne",
        (CompareKind::Lt, false) => "lt",
        (CompareKind::Le, false) => "le",
        (CompareKind::Gt, false) => "gt",
        (CompareKind::Ge, false) => "ge",
    }
}

fn arithmetic(op: ArithmeticOp) -> &'static str {
    match op {
        ArithmeticOp::Add => "+",
        ArithmeticOp::Subtract => "-",
        ArithmeticOp::Multiply => "*",
        ArithmeticOp::Divide => "div",
        ArithmeticOp::IntegerDivide => "idiv",
        ArithmeticOp::Modulo => "mod",
    }
}

/// A fully bracketed rendering of the tree.
fn shape(expr: &Expr) -> String {
    match expr {
        Expr::Literal(Literal::String(s)) => format!("'{s}'"),
        Expr::Literal(Literal::Integer(n)) => format!("int:{n}"),
        Expr::Literal(Literal::Decimal(n)) => format!("dec:{n}"),
        Expr::Literal(Literal::Double(n)) => format!("dbl:{n}"),
        Expr::VarRef(q) => format!("${q}"),
        Expr::ContextItem => ".".into(),
        Expr::FunctionCall { name, args } => {
            let ns = if name.namespace.as_deref() == Some(FN_NAMESPACE) {
                "fn:"
            } else {
                ""
            };
            format!("({ns}{name} {})", list(args)).replace(" )", ")")
        }
        Expr::Sequence(items) => format!("(seq {})", list(items)).replace(" )", ")"),
        Expr::Filter { base, predicates } => {
            format!("(filter {}{})", shape(base), preds(predicates))
        }
        Expr::Path(path) => {
            let steps: Vec<String> = path
                .steps
                .iter()
                .map(|step| match step {
                    Step::Axis(AxisStep {
                        axis,
                        test,
                        predicates,
                    }) => {
                        format!("{}::{}{}", axis.name(), node_test(test), preds(predicates))
                    }
                    Step::Expr(e) => shape(e),
                })
                .collect();
            let start = if path.start == PathStart::Root {
                "root "
            } else {
                ""
            };
            format!("(path {start}{})", steps.join(" ")).replace(" )", ")")
        }
        Expr::Negate(e) => format!("(neg {})", shape(e)),
        Expr::Castable { operand, target } => format!("(castable {} {target})", shape(operand)),
        Expr::InstanceOf { operand, target } => format!("(instance {} {target})", shape(operand)),
        Expr::Union(a, b) => format!("(| {} {})", shape(a), shape(b)),
        Expr::Arithmetic { op, lhs, rhs } => {
            format!("({} {} {})", arithmetic(*op), shape(lhs), shape(rhs))
        }
        Expr::Range(a, b) => format!("(to {} {})", shape(a), shape(b)),
        Expr::Comparison { op, lhs, rhs } => {
            format!("({} {} {})", comparison(*op), shape(lhs), shape(rhs))
        }
        Expr::And(a, b) => format!("(and {} {})", shape(a), shape(b)),
        Expr::Or(a, b) => format!("(or {} {})", shape(a), shape(b)),
        Expr::If {
            condition,
            then,
            otherwise,
        } => {
            format!(
                "(if {} {} {})",
                shape(condition),
                shape(then),
                shape(otherwise)
            )
        }
        Expr::For { bindings, body } => {
            let b: Vec<String> = bindings
                .iter()
                .map(|b| format!("${} {}", b.variable, shape(&b.sequence)))
                .collect();
            format!("(for [{}] {})", b.join(", "), shape(body))
        }
        Expr::Quantified {
            quantifier,
            bindings,
            satisfies,
        } => {
            let q = match quantifier {
                Quantifier::Some => "some",
                Quantifier::Every => "every",
            };
            let b: Vec<String> = bindings
                .iter()
                .map(|b| format!("${} {}", b.variable, shape(&b.sequence)))
                .collect();
            format!("({q} [{}] {})", b.join(", "), shape(satisfies))
        }
    }
}

fn parsed(src: &str) -> String {
    shape(&parse_expr(src, &ns()).unwrap_or_else(|e| panic!("{src}: {e}")))
}

#[track_caller]
fn check(cases: &[(&str, &str)]) {
    for (src, expected) in cases {
        assert_eq!(parsed(src), *expected, "{src}");
    }
}

#[track_caller]
fn syntax_error(src: &str) {
    let err = parse_expr(src, &ns()).expect_err(src);
    assert_eq!(err.kind, einvoice_xpath::ErrorKind::Syntax, "{src}: {err}");
}

#[test]
fn arithmetic_precedence_and_left_associativity() {
    check(&[
        ("1 + 2 * 3", "(+ int:1 (* int:2 int:3))"),
        ("1 * 2 + 3", "(+ (* int:1 int:2) int:3)"),
        ("1 - 2 - 3", "(- (- int:1 int:2) int:3)"),
        ("8 div 4 div 2", "(div (div int:8 int:4) int:2)"),
        ("7 mod 3 * 2", "(* (mod int:7 int:3) int:2)"),
        ("7 idiv 2 + 1", "(+ (idiv int:7 int:2) int:1)"),
        ("1 - (2 - 3)", "(- int:1 (- int:2 int:3))"),
        ("-1 + 2", "(+ (neg int:1) int:2)"),
        ("- - 1", "(neg (neg int:1))"),
        ("--1", "(neg (neg int:1))"),
        ("1 - -1", "(- int:1 (neg int:1))"),
    ]);
}

#[test]
fn union_binds_tighter_than_multiplication_and_looser_than_unary() {
    check(&[
        (
            "a | b * c",
            "(* (| (path child::a) (path child::b)) (path child::c))",
        ),
        (
            "a * b | c",
            "(* (path child::a) (| (path child::b) (path child::c)))",
        ),
        (
            "a | b | c",
            "(| (| (path child::a) (path child::b)) (path child::c))",
        ),
        ("-a | b", "(| (neg (path child::a)) (path child::b))"),
        ("-a/b", "(neg (path child::a child::b))"),
    ]);
}

#[test]
fn logical_comparison_and_range_levels() {
    check(&[
        (
            "a or b and c",
            "(or (path child::a) (and (path child::b) (path child::c)))",
        ),
        (
            "a and b or c",
            "(or (and (path child::a) (path child::b)) (path child::c))",
        ),
        (
            "a or b or c",
            "(or (or (path child::a) (path child::b)) (path child::c))",
        ),
        (
            "a = b or c",
            "(or (= (path child::a) (path child::b)) (path child::c))",
        ),
        (
            "a = b and c != d",
            "(and (= (path child::a) (path child::b)) (!= (path child::c) (path child::d)))",
        ),
        ("1 + 2 = 3", "(= (+ int:1 int:2) int:3)"),
        ("1 to 2 + 3", "(to int:1 (+ int:2 int:3))"),
        ("1 to 3 = 2", "(= (to int:1 int:3) int:2)"),
        ("count(x) eq 1", "(eq (fn:count (path child::x)) int:1)"),
        ("a ne b", "(ne (path child::a) (path child::b))"),
        ("a lt b", "(lt (path child::a) (path child::b))"),
        ("a le b", "(le (path child::a) (path child::b))"),
        ("a gt b", "(gt (path child::a) (path child::b))"),
        ("a ge b", "(ge (path child::a) (path child::b))"),
        ("a < b", "(< (path child::a) (path child::b))"),
        ("a <= b", "(<= (path child::a) (path child::b))"),
        ("a > b", "(> (path child::a) (path child::b))"),
        ("a >= b", "(>= (path child::a) (path child::b))"),
    ]);
    // Comparisons and ranges are not associative.
    syntax_error("1 = 2 = 3");
    syntax_error("1 eq 2 eq 3");
    syntax_error("1 to 2 to 3");
}

#[test]
fn type_operators() {
    check(&[
        (
            "$a castable as xs:integer = true()",
            "(= (castable $a xs:integer) (fn:true))",
        ),
        ("$a castable as xs:date?", "(castable $a xs:date?)"),
        (
            "-$a castable as xs:integer",
            "(castable (neg $a) xs:integer)",
        ),
        ("$a instance of element()", "(instance $a element())"),
        ("$a instance of xs:string*", "(instance $a xs:string*)"),
        (
            "$a instance of attribute() or $b",
            "(or (instance $a attribute()) $b)",
        ),
        (
            "($a instance of xs:integer) * 2",
            "(* (instance $a xs:integer) int:2)",
        ),
        (
            "$a | $b instance of node()+",
            "(| $a (instance $b node()+))",
        ),
    ]);
    // The occurrence indicator is taken greedily: `* 2` cannot follow it.
    syntax_error("$a instance of xs:integer * 2");
}

#[test]
fn sequence_for_quantified_and_if() {
    check(&[
        ("1, 2, 3", "(seq int:1 int:2 int:3)"),
        ("()", "(seq)"),
        ("(1, (2, 3))", "(seq int:1 (seq int:2 int:3))"),
        ("((a))", "(path child::a)"),
        (
            "for $x in 1 to 3 return $x * 2",
            "(for [$x (to int:1 int:3)] (* $x int:2))",
        ),
        (
            "for $x in a, $y in b return $x",
            "(for [$x (path child::a), $y (path child::b)] $x)",
        ),
        (
            "some $x in a satisfies $x = 1",
            "(some [$x (path child::a)] (= $x int:1))",
        ),
        (
            "every $x in a, $y in b satisfies $x",
            "(every [$x (path child::a), $y (path child::b)] $x)",
        ),
        (
            "some $x in a satisfies b, c",
            "(seq (some [$x (path child::a)] (path child::b)) (path child::c))",
        ),
        (
            "if (a) then b else c or d",
            "(if (path child::a) (path child::b) (or (path child::c) (path child::d)))",
        ),
        (
            "if (a, b) then 1 else 2",
            "(if (seq (path child::a) (path child::b)) int:1 int:2)",
        ),
        (
            "(if (a) then b else c) or d",
            "(or (if (path child::a) (path child::b) (path child::c)) (path child::d))",
        ),
    ]);
}

#[test]
fn keywords_are_names_where_an_operand_is_expected() {
    check(&[
        ("div div div", "(div (path child::div) (path child::div))"),
        ("for", "(path child::for)"),
        ("if/then", "(path child::if child::then)"),
        ("and and or", "(and (path child::and) (path child::or))"),
        ("* * *", "(* (path child::*) (path child::*))"),
        ("a-b", "(path child::a-b)"),
        ("a - b", "(- (path child::a) (path child::b))"),
        ("$XR-URL-REGEX", "$XR-URL-REGEX"),
    ]);
}

#[test]
fn paths_steps_and_abbreviations() {
    check(&[
        ("/", "(path root)"),
        ("/a", "(path root child::a)"),
        ("//a", "(path root descendant-or-self::node() child::a)"),
        (
            "a//b",
            "(path child::a descendant-or-self::node() child::b)",
        ),
        ("../@x", "(path parent::node() attribute::x)"),
        ("@*", "(path attribute::*)"),
        ("attribute()", "(path attribute::attribute())"),
        ("child::attribute()", "(path child::attribute())"),
        ("self::node()", "(path self::node())"),
        (
            "preceding-sibling::*[1]",
            "(path preceding-sibling::*[int:1])",
        ),
        ("ancestor-or-self::*", "(path ancestor-or-self::*)"),
        (
            "following-sibling::p:x",
            "(path following-sibling::{urn:p}x)",
        ),
        (
            "preceding::a/ancestor::b",
            "(path preceding::a ancestor::b)",
        ),
        ("p:*", "(path child::{urn:p}*)"),
        ("*:x", "(path child::*:x)"),
        (
            "text() | comment() | processing-instruction()",
            "(| (| (path child::text()) (path child::comment())) (path child::processing-instruction()))",
        ),
        ("a[1][b]", "(path child::a[int:1][(path child::b)])"),
        ("(a)[1]", "(filter (path child::a)[int:1])"),
        ("$x[1]/b", "(path (filter $x[int:1]) child::b)"),
        (".", "."),
        ("./a", "(path . child::a)"),
        (
            "a/xs:decimal(b)",
            "(path child::a (xs:decimal (path child::b)))",
        ),
        (
            "//(a|b)/c",
            "(path root descendant-or-self::node() (| (path child::a) (path child::b)) child::c)",
        ),
        ("/ = 1", "(= (path root) int:1)"),
        ("(/) * 5", "(* (path root) int:5)"),
        ("document-uri()", "(fn:document-uri)"),
        ("u:f(1, 'a')", "(u:f int:1 'a')"),
    ]);
    // The leading-lone-slash rule: `*` after `/` is a name test, so `5` is unexpected.
    syntax_error("/ * 5");
}

#[test]
fn literals_keep_kind_and_text() {
    check(&[
        ("1", "int:1"),
        ("12345678901234567890123", "int:12345678901234567890123"),
        ("1.50", "dec:1.50"),
        (".5", "dec:.5"),
        ("1.", "dec:1."),
        ("1e3", "dbl:1e3"),
        ("1.5E-2", "dbl:1.5E-2"),
        ("'it''s'", "'it's'"),
        (r#""say ""hi""""#, r#"'say "hi"'"#),
    ]);
    syntax_error("10div 3");
    syntax_error("'unterminated");
}

#[test]
fn names_resolve_against_the_given_namespaces() {
    let ns = NamespaceMap::new()
        .bind("p", "urn:p")
        .with_default_element_namespace("urn:default");
    let expr = parse_expr("a/@b/p:c/f()/$v/xml:lang", &ns).unwrap();
    let Expr::Path(path) = expr else { panic!() };
    let namespaces: Vec<Option<String>> = path
        .steps
        .iter()
        .map(|step| match step {
            Step::Axis(AxisStep {
                test: NodeTest::Name(NameTest::Name(q)),
                ..
            }) => q.namespace.clone(),
            Step::Expr(Expr::FunctionCall { name, .. } | Expr::VarRef(name)) => {
                name.namespace.clone()
            }
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        namespaces,
        [
            Some("urn:default".to_owned()),
            None,
            Some("urn:p".to_owned()),
            Some(FN_NAMESPACE.to_owned()),
            None,
            Some(einvoice_xpath::XML_NAMESPACE.to_owned()),
        ]
    );
}

#[test]
fn errors_carry_positions() {
    let err = parse_expr("a + )", &ns()).unwrap_err();
    assert_eq!((err.offset, err.column), (4, 5), "{err}");
    assert_eq!(
        err.to_string(),
        "syntax error at column 5: expected an expression, found `)`"
    );
    let err = parse_expr("'é' = q:x", &ns()).unwrap_err();
    assert_eq!(err.kind, einvoice_xpath::ErrorKind::UndeclaredPrefix);
    assert_eq!((err.offset, err.column), (7, 7), "{err}");
}

#[test]
fn sequence_types() {
    for (src, occurrence) in [
        ("xs:string*", Occurrence::ZeroOrMore),
        ("xs:anyURI?", Occurrence::ZeroOrOne),
        ("xs:boolean", Occurrence::One),
        ("node()+", Occurrence::OneOrMore),
        ("element(p:report)", Occurrence::One),
        ("element()?", Occurrence::ZeroOrOne),
    ] {
        let t = parse_sequence_type(src, &ns()).unwrap();
        assert_eq!(t.occurrence, occurrence, "{src}");
        assert_eq!(t.to_string(), src);
    }
    let t = parse_sequence_type("element(p:report)", &ns()).unwrap();
    let einvoice_xpath::ItemType::Kind(KindTest::Element(Some(NameOrWildcard::Name(name)))) =
        t.item
    else {
        panic!("{t:?}")
    };
    assert_eq!(name.namespace.as_deref(), Some("urn:p"));
}

fn alternatives(src: &str) -> Pattern {
    parse_pattern(src, &ns()).unwrap_or_else(|e| panic!("{src}: {e}"))
}

#[test]
fn patterns() {
    let p = alternatives("node() | @*");
    assert_eq!(p.alternatives.len(), 2);
    let p = alternatives("/");
    assert_eq!(p.alternatives, [PatternAlternative::DocumentRoot]);
    let p = alternatives("/ | root()");
    assert_eq!(
        p.alternatives,
        [
            PatternAlternative::DocumentRoot,
            PatternAlternative::RootFunction
        ]
    );
    let p = alternatives("//p:a[1]/b//@c");
    let [PatternAlternative::Path(path)] = p.alternatives.as_slice() else {
        panic!("{p:?}")
    };
    assert_eq!(path.anchor, PatternAnchor::RootDescendant);
    assert_eq!(path.first.predicates.len(), 1);
    let links: Vec<(PatternLink, PatternAxis)> =
        path.rest.iter().map(|(l, s)| (*l, s.axis)).collect();
    assert_eq!(
        links,
        [
            (PatternLink::Child, PatternAxis::Child),
            (PatternLink::Descendant, PatternAxis::Attribute)
        ]
    );
    let p = alternatives("(/p:a | /p:b)[$x and $y]");
    let [PatternAlternative::Parenthesized { inner, predicates }] = p.alternatives.as_slice()
    else {
        panic!("{p:?}")
    };
    assert_eq!(inner.alternatives.len(), 2);
    assert_eq!(predicates.len(), 1);
    let p = alternatives("attribute()");
    let [PatternAlternative::Path(path)] = p.alternatives.as_slice() else {
        panic!("{p:?}")
    };
    assert_eq!(path.first.axis, PatternAxis::Attribute);
    for src in [
        "p:a/p:b[x = 1]",
        "/*/p:a[1]",
        "text()",
        "@*",
        "child::attribute()",
        "(/p:a | /p:b)[$x][2]",
        "/ | root() | *:x | p:*",
    ] {
        let p = alternatives(src);
        assert_eq!(
            parse_pattern(&p.to_string(), &ns()).unwrap(),
            p,
            "{src} -> {p}"
        );
    }
}

#[test]
fn attribute_value_templates() {
    let avt = parse_avt("a{{b}}{concat('}', $x)}c{1}", &ns()).unwrap();
    assert_eq!(avt.parts.len(), 4, "{avt:?}");
    assert_eq!(avt.to_string(), r#"a{{b}}{concat("}", $x)}c{1}"#);
    assert_eq!(
        parse_avt("plain {{text}}", &ns())
            .unwrap()
            .as_literal()
            .as_deref(),
        Some("plain {text}")
    );
    assert_eq!(parse_avt("", &ns()).unwrap().parts, []);
    for bad in ["a}b", "{}", "{a", "{a b}", "{1 +}"] {
        let err = parse_avt(bad, &ns()).expect_err(bad);
        assert_eq!(err.kind, einvoice_xpath::ErrorKind::Syntax, "{bad}: {err}");
    }
}

/// Trees whose naive printing would read back differently: the printer must add
/// parentheses (or not drop them) in each case.
#[test]
fn printing_hazards_round_trip() {
    for src in [
        "($a instance of xs:integer) * 2",
        "($a instance of xs:integer) + 2",
        "$b | ($a instance of element())",
        "f(($a instance of xs:integer), 1)",
        "(/) * 5",
        "(/) div 2",
        "(/), a",
        "-(/)",
        "a[/]",
        "($x[1])[2]",
        "(a/b)[1]",
        "(a)/b",
        "a/(b, c)",
        "(1, 2), 3",
        "(if (a) then b else c) + 1",
        "a - -b",
        "--a",
        "(for $x in a return $x) | b",
        "some $x in a satisfies (b, c)",
        "if (a) then (b, c) else ()",
        "$a castable as xs:integer? * 2",
        "(1 to 2) to 3",
        "(a = b) = c",
        "a - (b - c)",
        "a | (b | c)",
        "@attribute | child::attribute()",
        "1./a",
        "'{}' = \"\"\"\"",
    ] {
        let tree = parse_expr(src, &ns()).unwrap_or_else(|e| panic!("{src}: {e}"));
        let printed = tree.to_string();
        let again = parse_expr(&printed, &ns())
            .unwrap_or_else(|e| panic!("{src} printed as {printed}: {e}"));
        assert_eq!(again, tree, "{src} printed as {printed}");
    }
}
