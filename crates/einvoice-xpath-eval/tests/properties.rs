//! Property tests of the node model and of path evaluation, on random trees, against a
//! naive reference: every axis is defined from parent links and document order only, and
//! paths are evaluated step by step with explicit sorting.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]
// Proptest is not a dependency on wasm targets.
#![cfg(not(target_family = "wasm"))]

use std::fmt::Write as _;
use std::rc::Rc;

use einvoice_xpath::{NamespaceMap, QName, parse_expr};
use einvoice_xpath_eval::{
    Clock, Context, Document, DynamicContext, Item, NoHost, Node, NodeAxis, NodeKind,
    SimpleStaticContext, TreeBuilder, compile,
};
use proptest::prelude::*;

/// A random tree: elements named `a`, `b` or `c` with attributes `x`/`y`, text and
/// comments.
#[derive(Debug, Clone)]
enum Tree {
    Element(&'static str, Vec<&'static str>, Vec<Tree>),
    Text(String),
    Comment,
}

fn tree() -> impl Strategy<Value = Tree> {
    let leaf = prop_oneof![
        "[a-z ]{1,3}".prop_map(Tree::Text),
        Just(Tree::Comment),
        (prop::sample::select(vec!["a", "b", "c"]), attributes())
            .prop_map(|(n, attrs)| Tree::Element(n, attrs, Vec::new())),
    ];
    leaf.prop_recursive(4, 40, 4, |inner| {
        (
            prop::sample::select(vec!["a", "b", "c"]),
            attributes(),
            prop::collection::vec(inner, 0..4),
        )
            .prop_map(|(n, attrs, children)| Tree::Element(n, attrs, children))
    })
}

fn attributes() -> impl Strategy<Value = Vec<&'static str>> {
    prop::sample::select(vec![vec![], vec!["x"], vec!["y"], vec!["x", "y"]])
}

fn xml(tree: &Tree, out: &mut String) {
    match tree {
        Tree::Element(name, attrs, children) => {
            let _ = write!(out, "<{name}");
            for (i, a) in attrs.iter().enumerate() {
                let _ = write!(out, " {a}='{i}'");
            }
            out.push('>');
            for child in children {
                xml(child, out);
            }
            let _ = write!(out, "</{name}>");
        }
        Tree::Text(text) => out.push_str(text),
        Tree::Comment => out.push_str("<!--c-->"),
    }
}

fn document(tree: &Tree) -> Rc<Document> {
    let mut text = String::new();
    // The document element is always an element.
    let root = match tree {
        Tree::Element(..) => tree.clone(),
        other => Tree::Element("a", Vec::new(), vec![other.clone()]),
    };
    xml(&root, &mut text);
    Document::parse(&text, None).unwrap()
}

/// Every node in document order (attributes after their element), built without the
/// axes under test: recursion over children and attributes only.
fn all_nodes(doc: &Rc<Document>) -> Vec<Node> {
    fn walk(node: &Node, out: &mut Vec<Node>) {
        out.push(node.clone());
        out.extend(node.attributes());
        for child in node.children() {
            walk(&child, out);
        }
    }
    let mut out = Vec::new();
    walk(&doc.root(), &mut out);
    out
}

fn ancestors(node: &Node) -> Vec<Node> {
    let mut out = Vec::new();
    let mut current = node.parent();
    while let Some(n) = current {
        current = n.parent();
        out.push(n);
    }
    out
}

/// The reference definition of each axis, in axis order.
fn reference_axis(all: &[Node], node: &Node, axis: NodeAxis) -> Vec<Node> {
    let position = |n: &Node| all.iter().position(|m| m == n).unwrap();
    let is_attribute = |n: &Node| n.kind() == NodeKind::Attribute;
    let here = position(node);
    let ancestors_of_node = ancestors(node);
    let mut out: Vec<Node> = match axis {
        NodeAxis::SelfAxis => vec![node.clone()],
        NodeAxis::Parent => node.parent().into_iter().collect(),
        NodeAxis::Child => all
            .iter()
            .filter(|n| !is_attribute(n) && n.parent().as_ref() == Some(node))
            .cloned()
            .collect(),
        NodeAxis::Attribute => all
            .iter()
            .filter(|n| is_attribute(n) && n.parent().as_ref() == Some(node))
            .cloned()
            .collect(),
        NodeAxis::Descendant | NodeAxis::DescendantOrSelf => all
            .iter()
            .filter(|n| {
                (!is_attribute(n) && ancestors(n).contains(node))
                    || (axis == NodeAxis::DescendantOrSelf && *n == node)
            })
            .cloned()
            .collect(),
        NodeAxis::FollowingSibling | NodeAxis::PrecedingSibling => {
            if is_attribute(node) {
                Vec::new()
            } else {
                all.iter()
                    .filter(|n| {
                        !is_attribute(n)
                            && n.parent().is_some()
                            && n.parent() == node.parent()
                            && if axis == NodeAxis::FollowingSibling {
                                position(n) > here
                            } else {
                                position(n) < here
                            }
                    })
                    .cloned()
                    .collect()
            }
        }
        NodeAxis::Ancestor => ancestors_of_node.iter().rev().cloned().collect(),
        NodeAxis::AncestorOrSelf => ancestors_of_node
            .iter()
            .rev()
            .cloned()
            .chain(std::iter::once(node.clone()))
            .collect(),
        NodeAxis::Preceding => all
            .iter()
            .filter(|n| !is_attribute(n) && position(n) < here && !ancestors_of_node.contains(n))
            .cloned()
            .collect(),
    };
    if axis.is_reverse() {
        out.reverse();
    }
    out
}

const AXES: [NodeAxis; 11] = [
    NodeAxis::Child,
    NodeAxis::Attribute,
    NodeAxis::SelfAxis,
    NodeAxis::Descendant,
    NodeAxis::DescendantOrSelf,
    NodeAxis::FollowingSibling,
    NodeAxis::Parent,
    NodeAxis::Ancestor,
    NodeAxis::AncestorOrSelf,
    NodeAxis::PrecedingSibling,
    NodeAxis::Preceding,
];

/// The XPath axes of the supported subset (no `descendant`).
const XPATH_AXES: [(&str, NodeAxis); 10] = [
    ("child", NodeAxis::Child),
    ("attribute", NodeAxis::Attribute),
    ("self", NodeAxis::SelfAxis),
    ("descendant-or-self", NodeAxis::DescendantOrSelf),
    ("following-sibling", NodeAxis::FollowingSibling),
    ("parent", NodeAxis::Parent),
    ("ancestor", NodeAxis::Ancestor),
    ("ancestor-or-self", NodeAxis::AncestorOrSelf),
    ("preceding-sibling", NodeAxis::PrecedingSibling),
    ("preceding", NodeAxis::Preceding),
];

const TESTS: [&str; 5] = ["node()", "*", "a", "text()", "x"];

fn test_matches(test: &str, axis: NodeAxis, node: &Node) -> bool {
    let principal = if axis == NodeAxis::Attribute {
        NodeKind::Attribute
    } else {
        NodeKind::Element
    };
    match test {
        "node()" => true,
        "text()" => node.kind() == NodeKind::Text,
        "*" => node.kind() == principal,
        name => node.kind() == principal && node.name().is_some_and(|q| q.local == name),
    }
}

/// One step: axis, node test, predicate (`""`, `[1]` or `[last()]`).
type PathStep = (usize, usize, &'static str);

/// The path: from every node (`/descendant-or-self::node()`), then the steps.
fn path_text(steps: &[PathStep]) -> String {
    let mut text = String::from("/descendant-or-self::node()");
    for &(axis, test, predicate) in steps {
        let _ = write!(text, "/{}::{}{predicate}", XPATH_AXES[axis].0, TESTS[test]);
    }
    text
}

fn reference_path(all: &[Node], steps: &[PathStep]) -> Vec<Node> {
    let mut current = reference_axis(all, &all[0], NodeAxis::DescendantOrSelf);
    for &(axis, test, predicate) in steps {
        let (_, node_axis) = XPATH_AXES[axis];
        let mut next = Vec::new();
        for node in &current {
            let matching: Vec<Node> = reference_axis(all, node, node_axis)
                .into_iter()
                .filter(|n| test_matches(TESTS[test], node_axis, n))
                .collect();
            match predicate {
                "[1]" => next.extend(matching.first().cloned()),
                "[last()]" => next.extend(matching.last().cloned()),
                _ => next.extend(matching),
            }
        }
        next.sort_by(Node::cmp_document_order);
        next.dedup();
        current = next;
    }
    current
}

fn evaluate(doc: &Rc<Document>, text: &str) -> Vec<Node> {
    let expr = compile(
        &parse_expr(text, &NamespaceMap::new()).unwrap(),
        &SimpleStaticContext::new(),
    )
    .unwrap();
    let dc = DynamicContext::new(&NoHost, Clock::from_unix_millis(0, 0).unwrap());
    expr.evaluate(&dc, &Context::for_item(Item::Node(doc.root())))
        .unwrap()
        .into_iter()
        .map(|item| item.as_node().unwrap().clone())
        .collect()
}

fn single(doc: &Rc<Document>, text: &str) -> String {
    let expr = compile(
        &parse_expr(text, &NamespaceMap::new()).unwrap(),
        &SimpleStaticContext::new(),
    )
    .unwrap();
    let dc = DynamicContext::new(&NoHost, Clock::from_unix_millis(0, 0).unwrap());
    let items = expr
        .evaluate(&dc, &Context::for_item(Item::Node(doc.root())))
        .unwrap();
    items[0].string_value().into_owned()
}

fn path_steps() -> impl Strategy<Value = Vec<PathStep>> {
    prop::collection::vec(
        (
            0..XPATH_AXES.len(),
            0..TESTS.len(),
            prop::sample::select(vec!["", "", "[1]", "[last()]"]),
        ),
        1..4,
    )
}

/// XML text with everything that makes a parsed text differ from its source: character
/// and entity references, line ends, CDATA sections, whitespace in attribute values,
/// comments and processing instructions, prefixed and default-namespace names.
fn source_text() -> impl Strategy<Value = String> {
    let piece = prop::sample::select(vec![
        "a",
        "é",
        " ",
        "\t",
        "\n",
        "\r\n",
        "\r",
        "&amp;",
        "&lt;",
        "&#65;",
        "&#x1F600;",
        "<![CDATA[<&]]>",
        "<!--c-->",
        "<!--\r\n-->",
        "<?t d\r\ne?>",
        "<e/>",
        "<p:e p:x='1'/>",
    ]);
    let content = prop::collection::vec(piece, 0..8).prop_map(|p| p.concat());
    let value = prop::collection::vec(
        prop::sample::select(vec![
            "v", " ", "\t", "\n", "\r\n", "&#9;", "&#10;", "&amp;", "&quot;", "é",
        ]),
        0..5,
    )
    .prop_map(|p| p.concat());
    (content.clone(), value, content).prop_map(|(before, value, inner)| {
        format!(
            "<?xml version='1.0'?><!--top-->\r\n<p:r xmlns:p='urn:p' xmlns='urn:d' a='{value}' \
             p:b='{value}'>{before}<e c='{value}'>{inner}</e>{before}</p:r>\r\n<?t top?>"
        )
    })
}

/// The tree of a parsed document, built through the public [`TreeBuilder`] from the
/// parser's values: the reference for [`Document::parse`].
fn reference_tree(text: &str) -> Rc<Document> {
    fn name(prefix: Option<&str>, namespace: Option<&str>, local: &str) -> QName {
        QName {
            prefix: prefix.map(str::to_owned),
            namespace: namespace.filter(|n| !n.is_empty()).map(str::to_owned),
            local: local.to_owned(),
        }
    }
    fn walk(text: &str, node: roxmltree::Node<'_, '_>, builder: &mut TreeBuilder) {
        match node.node_type() {
            roxmltree::NodeType::Root => {}
            roxmltree::NodeType::Element => {
                let start = node.range().start + 1;
                let written = &text[start..start + text[start..].find([' ', '/', '>']).unwrap()];
                let prefix = written.split_once(':').map(|(p, _)| p);
                builder
                    .start_element(name(
                        prefix,
                        node.tag_name().namespace(),
                        node.tag_name().name(),
                    ))
                    .unwrap();
                for a in node.attributes() {
                    let prefix = text[a.range_qname()].split_once(':').map(|(p, _)| p);
                    builder
                        .attribute(name(prefix, a.namespace(), a.name()), a.value())
                        .unwrap();
                }
                for child in node.children() {
                    walk(text, child, builder);
                }
                builder.end_element().unwrap();
            }
            roxmltree::NodeType::Text => {
                if node.parent().is_some_and(|p| !p.is_root()) {
                    builder.text(node.text().unwrap()).unwrap();
                }
            }
            roxmltree::NodeType::Comment => builder.comment(node.text().unwrap()).unwrap(),
            roxmltree::NodeType::PI => {
                let pi = node.pi().unwrap();
                builder
                    .processing_instruction(pi.target, pi.value.unwrap_or(""))
                    .unwrap();
            }
        }
    }
    let parsed = roxmltree::Document::parse(text).unwrap();
    let mut builder = TreeBuilder::new();
    for child in parsed.root().children() {
        walk(text, child, &mut builder);
    }
    builder.finish(None, None).unwrap()
}

/// Kind, name (with its prefix and namespace), string value and parent position of every
/// node, in document order.
fn dump(doc: &Rc<Document>) -> Vec<String> {
    let all = all_nodes(doc);
    all.iter()
        .map(|n| {
            let name = n
                .name()
                .map(|q| format!("{q} {{{}}}", q.namespace.as_deref().unwrap_or("")));
            let parent = n.parent().map(|p| all.iter().position(|m| *m == p));
            format!("{:?} {name:?} {:?} {parent:?}", n.kind(), n.string_value())
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn parsed_trees_match_the_built_reference(text in source_text()) {
        let expected = dump(&reference_tree(&text));
        prop_assert_eq!(dump(&Document::parse(text.as_str(), None).unwrap()), expected.clone());
        prop_assert_eq!(dump(&Document::parse(text, None).unwrap()), expected);
    }

    #[test]
    fn any_clock_reading_is_a_value_or_an_error(millis in any::<i64>(), offset in -900i16..=900) {
        // Never a panic (overflow checks are on in every profile).
        let clock = Clock::from_unix_millis(offset, millis);
        prop_assert_eq!(clock.is_ok(), (-840..=840).contains(&offset)
            && millis.checked_add(i64::from(offset) * 60_000).is_some());
        if let Ok(clock) = clock {
            prop_assert_eq!(clock.current_date_time().timezone(), Some(offset));
        }
    }

    #[test]
    fn axes_match_their_definition(tree in tree()) {
        let doc = document(&tree);
        let all = all_nodes(&doc);
        for node in &all {
            for axis in AXES {
                let actual: Vec<Node> = node.axis(axis).collect();
                prop_assert_eq!(actual, reference_axis(&all, node, axis), "{:?} {:?}", node, axis);
            }
        }
        // The arena order is document order.
        prop_assert!(all.windows(2).all(|w| w[0].cmp_document_order(&w[1]).is_lt()));
    }

    #[test]
    fn paths_match_the_reference(tree in tree(), steps in path_steps()) {
        let doc = document(&tree);
        let all = all_nodes(&doc);
        let text = path_text(&steps);
        let expected = reference_path(&all, &steps);
        prop_assert_eq!(&evaluate(&doc, &text), &expected, "{}", text);
        // Lazy and counting consumers agree with the full sequence.
        prop_assert_eq!(single(&doc, &format!("exists({text})")), (!expected.is_empty()).to_string());
        prop_assert_eq!(single(&doc, &format!("count({text})")), expected.len().to_string());
        prop_assert_eq!(evaluate(&doc, &format!("({text})[1]")), expected.first().cloned().into_iter().collect::<Vec<_>>());
        prop_assert_eq!(evaluate(&doc, &format!("({text})[last()]")), expected.last().cloned().into_iter().collect::<Vec<_>>());
        // A union with itself and with a reversed copy is the same sequence.
        prop_assert_eq!(&evaluate(&doc, &format!("{text} | {text}")), &expected);
        prop_assert_eq!(&evaluate(&doc, &format!("reverse({text})/.")), &expected);
    }
}
