//! Property: printing a syntax tree and parsing the text gives back the same tree.
//!
//! The generators build every tree shape the parser can produce (and only those: a
//! one-item sequence, a filter without predicate or a relative path made of a single
//! filter step cannot come out of the parser, since each is written like its only
//! operand).

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

use einvoice_xpath::{
    ArithmeticOp, AtomicKind, AtomicType, Axis, AxisStep, Binding, CompareKind, ComparisonOp, Expr,
    FN_NAMESPACE, ItemType, KindTest, Literal, NameOrWildcard, NameTest, NamespaceMap, NodeTest,
    Occurrence, PathExpr, PathPattern, PathStart, Pattern, PatternAlternative, PatternAnchor,
    PatternAxis, PatternLink, PatternStep, QName, Quantifier, SequenceType, SingleType, Step,
    XS_NAMESPACE, parse_expr, parse_pattern, parse_sequence_type,
};
use proptest::prelude::*;

const P: &str = "urn:p";

fn ns() -> NamespaceMap {
    NamespaceMap::new().bind("xs", XS_NAMESPACE).bind("p", P)
}

/// Local names, keywords among them: they are names wherever an operand is expected.
const LOCALS: [&str; 12] = [
    "a",
    "b",
    "div",
    "for",
    "if",
    "text",
    "and",
    "instance",
    "a-b",
    "x.y",
    "child",
    "attribute",
];
/// Function names: no reserved function name (`if`, `text`, `node`, ...).
const FUNCTIONS: [&str; 6] = ["f", "count", "for", "root", "document-uri", "a-b"];

fn local() -> impl Strategy<Value = String> {
    proptest::sample::select(&LOCALS[..]).prop_map(str::to_owned)
}

/// A name in no namespace or in `p:`.
fn name() -> impl Strategy<Value = QName> {
    (any::<bool>(), local()).prop_map(|(prefixed, local)| QName {
        prefix: prefixed.then(|| "p".to_owned()),
        namespace: prefixed.then(|| P.to_owned()),
        local,
    })
}

fn function_name() -> impl Strategy<Value = QName> {
    (any::<bool>(), proptest::sample::select(&FUNCTIONS[..])).prop_map(|(prefixed, local)| QName {
        prefix: prefixed.then(|| "p".to_owned()),
        namespace: Some(if prefixed { P } else { FN_NAMESPACE }.to_owned()),
        local: local.to_owned(),
    })
}

fn literal() -> impl Strategy<Value = Literal> {
    prop_oneof![
        "[a-z'\"{} ]{0,6}".prop_map(Literal::String),
        "[0-9]{1,25}".prop_map(Literal::Integer),
        prop_oneof!["[0-9]{1,3}\\.[0-9]{0,3}", "\\.[0-9]{1,3}"].prop_map(Literal::Decimal),
        "([0-9]{1,2}(\\.[0-9]{0,2})?|\\.[0-9]{1,2})[eE][+-]?[0-9]{1,2}".prop_map(Literal::Double),
    ]
}

fn name_or_wildcard() -> impl Strategy<Value = Option<NameOrWildcard>> {
    prop_oneof![
        Just(None),
        Just(Some(NameOrWildcard::Wildcard)),
        name().prop_map(|n| Some(NameOrWildcard::Name(n)))
    ]
}

fn kind_test() -> impl Strategy<Value = KindTest> {
    prop_oneof![
        Just(KindTest::AnyKind),
        Just(KindTest::Text),
        Just(KindTest::Comment),
        Just(KindTest::ProcessingInstruction),
        name_or_wildcard().prop_map(KindTest::Element),
        name_or_wildcard().prop_map(KindTest::Attribute),
    ]
}

fn node_test() -> impl Strategy<Value = NodeTest> {
    prop_oneof![
        name().prop_map(|n| NodeTest::Name(NameTest::Name(n))),
        Just(NodeTest::Name(NameTest::Any)),
        Just(NodeTest::Name(NameTest::AnyLocal {
            prefix: "p".into(),
            namespace: P.into()
        })),
        local().prop_map(|local| NodeTest::Name(NameTest::AnyNamespace { local })),
        kind_test().prop_map(NodeTest::Kind),
    ]
}

fn axis() -> impl Strategy<Value = Axis> {
    proptest::sample::select(
        &[
            Axis::Child,
            Axis::Attribute,
            Axis::SelfAxis,
            Axis::DescendantOrSelf,
            Axis::FollowingSibling,
            Axis::Parent,
            Axis::Ancestor,
            Axis::AncestorOrSelf,
            Axis::PrecedingSibling,
            Axis::Preceding,
        ][..],
    )
}

fn atomic_type() -> impl Strategy<Value = AtomicType> {
    proptest::sample::select(&["string", "anyURI", "boolean", "integer", "decimal", "date"][..])
        .prop_map(|local| AtomicType {
            name: QName {
                prefix: Some("xs".into()),
                namespace: Some(XS_NAMESPACE.into()),
                local: local.into(),
            },
            kind: AtomicKind::from_local_name(local).unwrap(),
        })
}

fn sequence_type() -> impl Strategy<Value = SequenceType> {
    (
        prop_oneof![
            atomic_type().prop_map(ItemType::Atomic),
            kind_test().prop_map(ItemType::Kind)
        ],
        proptest::sample::select(
            &[
                Occurrence::One,
                Occurrence::ZeroOrOne,
                Occurrence::ZeroOrMore,
                Occurrence::OneOrMore,
            ][..],
        ),
    )
        .prop_map(|(item, occurrence)| SequenceType { item, occurrence })
}

fn variable() -> impl Strategy<Value = QName> {
    name()
}

fn leaf() -> impl Strategy<Value = Expr> {
    prop_oneof![
        literal().prop_map(Expr::Literal),
        variable().prop_map(Expr::VarRef),
        Just(Expr::ContextItem),
        Just(Expr::Sequence(Vec::new())),
        Just(Expr::Path(PathExpr {
            start: PathStart::Root,
            steps: Vec::new()
        })),
        (axis(), node_test()).prop_map(|(axis, test)| Expr::Path(PathExpr {
            start: PathStart::Relative,
            steps: vec![Step::Axis(AxisStep {
                axis,
                test,
                predicates: Vec::new()
            })]
        })),
    ]
}

fn boxed(e: Expr) -> Box<Expr> {
    Box::new(e)
}

fn binding(inner: BoxedStrategy<Expr>) -> impl Strategy<Value = Vec<Binding>> {
    proptest::collection::vec(
        (variable(), inner).prop_map(|(variable, sequence)| Binding { variable, sequence }),
        1..3,
    )
}

fn step(inner: BoxedStrategy<Expr>) -> impl Strategy<Value = Step> {
    prop_oneof![
        (
            axis(),
            node_test(),
            proptest::collection::vec(inner.clone(), 0..2)
        )
            .prop_map(|(axis, test, predicates)| Step::Axis(AxisStep {
                axis,
                test,
                predicates
            })),
        inner.prop_map(Step::Expr),
    ]
}

fn path(inner: BoxedStrategy<Expr>) -> impl Strategy<Value = Expr> {
    (any::<bool>(), proptest::collection::vec(step(inner), 1..4)).prop_filter_map(
        "a lone filter step is not a path",
        |(rooted, steps)| {
            let start = if rooted {
                PathStart::Root
            } else {
                PathStart::Relative
            };
            if start == PathStart::Relative && steps.len() == 1 && matches!(steps[0], Step::Expr(_))
            {
                return None;
            }
            Some(Expr::Path(PathExpr { start, steps }))
        },
    )
}

fn comparison_op() -> impl Strategy<Value = ComparisonOp> {
    let kind = proptest::sample::select(
        &[
            CompareKind::Eq,
            CompareKind::Ne,
            CompareKind::Lt,
            CompareKind::Le,
            CompareKind::Gt,
            CompareKind::Ge,
        ][..],
    );
    (any::<bool>(), kind).prop_map(|(general, k)| {
        if general {
            ComparisonOp::General(k)
        } else {
            ComparisonOp::Value(k)
        }
    })
}

fn arithmetic_op() -> impl Strategy<Value = ArithmeticOp> {
    proptest::sample::select(
        &[
            ArithmeticOp::Add,
            ArithmeticOp::Subtract,
            ArithmeticOp::Multiply,
            ArithmeticOp::Divide,
            ArithmeticOp::IntegerDivide,
            ArithmeticOp::Modulo,
        ][..],
    )
}

fn expr() -> impl Strategy<Value = Expr> {
    leaf().prop_recursive(5, 48, 4, |inner| {
        let inner = inner.boxed();
        let two = (inner.clone(), inner.clone());
        prop_oneof![
            (
                function_name(),
                proptest::collection::vec(inner.clone(), 0..3)
            )
                .prop_map(|(name, args)| Expr::FunctionCall { name, args }),
            proptest::collection::vec(inner.clone(), 2..4).prop_map(Expr::Sequence),
            (
                inner.clone(),
                proptest::collection::vec(inner.clone(), 1..3)
            )
                .prop_map(|(base, predicates)| Expr::Filter {
                    base: boxed(base),
                    predicates
                }),
            path(inner.clone()),
            inner.clone().prop_map(|e| Expr::Negate(boxed(e))),
            (inner.clone(), atomic_type(), any::<bool>()).prop_map(|(e, atomic, allows_empty)| {
                Expr::Castable {
                    operand: boxed(e),
                    target: SingleType {
                        atomic,
                        allows_empty,
                    },
                }
            }),
            (inner.clone(), sequence_type()).prop_map(|(e, target)| Expr::InstanceOf {
                operand: boxed(e),
                target
            }),
            two.clone()
                .prop_map(|(a, b)| Expr::Union(boxed(a), boxed(b))),
            (arithmetic_op(), two.clone()).prop_map(|(op, (a, b))| Expr::Arithmetic {
                op,
                lhs: boxed(a),
                rhs: boxed(b)
            }),
            two.clone()
                .prop_map(|(a, b)| Expr::Range(boxed(a), boxed(b))),
            (comparison_op(), two.clone()).prop_map(|(op, (a, b))| Expr::Comparison {
                op,
                lhs: boxed(a),
                rhs: boxed(b)
            }),
            two.clone().prop_map(|(a, b)| Expr::And(boxed(a), boxed(b))),
            two.prop_map(|(a, b)| Expr::Or(boxed(a), boxed(b))),
            (inner.clone(), inner.clone(), inner.clone()).prop_map(|(c, t, o)| Expr::If {
                condition: boxed(c),
                then: boxed(t),
                otherwise: boxed(o)
            }),
            (binding(inner.clone()), inner.clone()).prop_map(|(bindings, body)| Expr::For {
                bindings,
                body: boxed(body)
            }),
            (any::<bool>(), binding(inner.clone()), inner).prop_map(
                |(some, bindings, satisfies)| Expr::Quantified {
                    quantifier: if some {
                        Quantifier::Some
                    } else {
                        Quantifier::Every
                    },
                    bindings,
                    satisfies: boxed(satisfies)
                }
            ),
        ]
    })
}

fn pattern_step() -> impl Strategy<Value = PatternStep> {
    (
        any::<bool>(),
        node_test(),
        proptest::collection::vec(leaf(), 0..2),
    )
        .prop_map(|(attribute, test, predicates)| PatternStep {
            axis: if attribute {
                PatternAxis::Attribute
            } else {
                PatternAxis::Child
            },
            test,
            predicates,
        })
}

fn path_pattern() -> impl Strategy<Value = PatternAlternative> {
    let link = prop_oneof![Just(PatternLink::Child), Just(PatternLink::Descendant)];
    let anchor = proptest::sample::select(
        &[
            PatternAnchor::Relative,
            PatternAnchor::Root,
            PatternAnchor::RootDescendant,
        ][..],
    );
    (
        anchor,
        pattern_step(),
        proptest::collection::vec((link, pattern_step()), 0..3),
    )
        .prop_map(|(anchor, first, rest)| {
            PatternAlternative::Path(PathPattern {
                anchor,
                first,
                rest,
            })
        })
}

fn pattern() -> impl Strategy<Value = Pattern> {
    let alternative = prop_oneof![
        Just(PatternAlternative::DocumentRoot),
        Just(PatternAlternative::RootFunction),
        path_pattern()
    ];
    alternative
        .prop_recursive(2, 8, 3, |inner| {
            prop_oneof![
                inner.clone(),
                (
                    proptest::collection::vec(inner, 1..3),
                    proptest::collection::vec(expr(), 1..2)
                )
                    .prop_map(|(alternatives, predicates)| {
                        PatternAlternative::Parenthesized {
                            inner: Box::new(Pattern { alternatives }),
                            predicates,
                        }
                    }),
            ]
        })
        .prop_map(|a| vec![a])
        .prop_flat_map(|first| {
            proptest::collection::vec(
                prop_oneof![Just(PatternAlternative::DocumentRoot), path_pattern()],
                0..3,
            )
            .prop_map(move |rest| {
                let mut alternatives = first.clone();
                alternatives.extend(rest);
                Pattern { alternatives }
            })
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20_000))]

    #[test]
    fn expressions_round_trip(tree in expr()) {
        let text = tree.to_string();
        let again = parse_expr(&text, &ns()).map_err(|e| TestCaseError::fail(format!("{text}: {e}")))?;
        prop_assert_eq!(again, tree, "{}", text);
    }

    #[test]
    fn patterns_round_trip(tree in pattern()) {
        let text = tree.to_string();
        let again = parse_pattern(&text, &ns()).map_err(|e| TestCaseError::fail(format!("{text}: {e}")))?;
        prop_assert_eq!(again, tree, "{}", text);
    }

    #[test]
    fn sequence_types_round_trip(tree in sequence_type()) {
        let text = tree.to_string();
        let again = parse_sequence_type(&text, &ns()).map_err(|e| TestCaseError::fail(format!("{text}: {e}")))?;
        prop_assert_eq!(again, tree, "{}", text);
    }
}
