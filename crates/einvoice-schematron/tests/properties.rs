//! Property tests: for any result tree (elements in no namespace, in the default
//! namespace and under prefixes, a prefix bound twice or reused by the writer, namespaced
//! attributes, text with the characters the writer escapes, comments and processing
//! instructions), the stylesheet written is well-formed XML with the same elements, in
//! the same namespaces, the same attributes and values, the same comments and processing
//! instructions, and the same text but for the whitespace of the indentation.

#![cfg(test)]

use std::fmt::Write as _;

use einvoice_xml::{Event, Reader};
use proptest::prelude::*;

/// The prefixes and namespaces of the generated names: `p0`..`p2`, each bound to one
/// namespace only.
const PREFIXED: [(&str, &str); 3] = [("p0", "urn:0"), ("p1", "urn:1"), ("p2", "urn:2")];
/// The default namespace, when an element uses it.
const DEFAULT: &str = "urn:d";
/// Extra declarations of the document element: a second prefix for `urn:1` (the
/// namespace context then binds it twice), `ns0` (which the writer's own prefixes start
/// with), `svrl` for another namespace, and a namespace no name uses.
const EXTRA: [(&str, &str); 4] = [
    ("q", "urn:1"),
    ("ns0", "urn:0"),
    ("svrl", "urn:not-svrl"),
    ("r", "urn:unused"),
];

#[derive(Clone, Debug)]
enum Name {
    Prefixed(usize),
    Default,
    None,
}

#[derive(Clone, Debug)]
enum Gen {
    Element {
        name: Name,
        local: &'static str,
        attributes: Vec<(Option<usize>, &'static str, String)>,
        children: Vec<Gen>,
    },
    Text(String),
    Comment(String),
    Pi(&'static str, String),
}

fn text() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        proptest::sample::select(vec![
            'a', 'b', ' ', '\t', '\n', '\r', '&', '<', '>', '"', '\'', 'é', '😀', '\u{85}',
            '\u{7F}', '\u{A0}', ']',
        ]),
        0..8,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

fn comment() -> impl Strategy<Value = String> {
    "[a-z <>&'\"]{0,6}"
}

fn pi_data() -> impl Strategy<Value = String> {
    "([a-z][a-z ]{0,5})?"
}

fn name() -> impl Strategy<Value = Name> {
    prop_oneof![
        (0..PREFIXED.len()).prop_map(Name::Prefixed),
        Just(Name::Default),
        Just(Name::None),
    ]
}

fn leaf() -> impl Strategy<Value = Gen> {
    prop_oneof![
        3 => text().prop_map(Gen::Text),
        1 => comment().prop_map(Gen::Comment),
        1 => (proptest::sample::select(vec!["pi", "t"]), pi_data())
            .prop_map(|(t, d)| Gen::Pi(t, d)),
    ]
}

fn element(children: impl Strategy<Value = Vec<Gen>>) -> impl Strategy<Value = Gen> {
    (
        name(),
        proptest::sample::select(vec!["a", "b", "item", "x-y", "_z"]),
        proptest::collection::vec(
            (
                proptest::option::of(0..PREFIXED.len()),
                proptest::sample::select(vec!["k", "v", "w"]),
                text(),
            ),
            0..4,
        ),
        children,
    )
        .prop_map(|(name, local, mut attributes, children)| {
            let mut seen = Vec::new();
            attributes.retain(|(p, l, _)| {
                let key = (*p, *l);
                let new = !seen.contains(&key);
                seen.push(key);
                new
            });
            Gen::Element {
                name,
                local,
                attributes,
                children,
            }
        })
}

fn tree() -> impl Strategy<Value = Gen> {
    leaf().prop_recursive(4, 40, 5, |inner| {
        prop_oneof![
            1 => leaf(),
            2 => element(proptest::collection::vec(inner, 0..5)),
        ]
    })
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\r', "&#13;")
}

fn escape_attribute(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
        .replace('\t', "&#9;")
        .replace('\n', "&#10;")
        .replace('\r', "&#13;")
}

/// Writes the tree as the XML of a result tree; `default` is the default namespace in
/// scope.
fn serialize(node: &Gen, default: Option<&str>, root_extra: &[(&str, &str)], out: &mut String) {
    match node {
        Gen::Element {
            name,
            local,
            attributes,
            children,
        } => {
            let qname = match name {
                Name::Prefixed(i) => format!("{}:{local}", PREFIXED[*i].0),
                Name::Default | Name::None => (*local).to_owned(),
            };
            // The declarations of the element, each prefix once.
            let mut declarations: Vec<(&str, &str)> = root_extra.to_vec();
            let mut declare = |prefix: &'static str, uri: &'static str| {
                if !declarations.iter().any(|(p, _)| *p == prefix) {
                    declarations.push((prefix, uri));
                }
            };
            let mut inner_default = default;
            match name {
                Name::Prefixed(i) => declare(PREFIXED[*i].0, PREFIXED[*i].1),
                Name::Default => {
                    declare("", DEFAULT);
                    inner_default = Some(DEFAULT);
                }
                Name::None => {
                    if default.is_some() {
                        declare("", "");
                    }
                    inner_default = None;
                }
            }
            let mut written_attributes = String::new();
            for (prefix, local, value) in attributes {
                match prefix {
                    Some(i) => {
                        let (p, uri) = PREFIXED[*i];
                        declare(p, uri);
                        let _ = write!(
                            written_attributes,
                            " {p}:{local}=\"{}\"",
                            escape_attribute(value)
                        );
                    }
                    None => {
                        let _ = write!(
                            written_attributes,
                            " {local}=\"{}\"",
                            escape_attribute(value)
                        );
                    }
                }
            }
            let _ = write!(out, "<{qname}");
            for (prefix, uri) in declarations {
                if prefix.is_empty() {
                    let _ = write!(out, " xmlns=\"{uri}\"");
                } else {
                    let _ = write!(out, " xmlns:{prefix}=\"{uri}\"");
                }
            }
            out.push_str(&written_attributes);
            out.push('>');
            for child in children {
                serialize(child, inner_default, &[], out);
            }
            let _ = write!(out, "</{qname}>");
        }
        Gen::Text(t) => out.push_str(&escape_text(t)),
        Gen::Comment(c) => {
            let _ = write!(out, "<!--{c}-->");
        }
        Gen::Pi(target, data) => {
            if data.is_empty() {
                let _ = write!(out, "<?{target}?>");
            } else {
                let _ = write!(out, "<?{target} {data}?>");
            }
        }
    }
}

/// What must survive the writer: the events of the document, attributes sorted,
/// namespace declarations left out, text without whitespace (adjacent text merged, empty
/// text dropped), empty comments left out.
fn essence(xml: &str) -> Vec<String> {
    let mut reader = Reader::new(xml);
    let mut events: Vec<String> = Vec::new();
    let mut text = String::new();
    let flush = |text: &mut String, events: &mut Vec<String>| {
        if !text.is_empty() {
            events.push(format!("text {text:?}"));
            text.clear();
        }
    };
    while let Some(event) = reader.next_event().unwrap() {
        match event {
            Event::Start(start) => {
                flush(&mut text, &mut events);
                let name = start.name();
                let mut attributes: Vec<String> = start
                    .attributes()
                    .map(|a| {
                        format!(
                            "{{{}}}{}={:?}",
                            a.name.namespace.unwrap_or(""),
                            a.name.local,
                            a.value.as_ref()
                        )
                    })
                    .collect();
                attributes.sort();
                events.push(format!(
                    "start {{{}}}{} {}",
                    name.namespace.unwrap_or(""),
                    name.local,
                    attributes.join(" ")
                ));
            }
            Event::End(_) => {
                flush(&mut text, &mut events);
                events.push("end".to_owned());
            }
            Event::Text(t) => text.extend(
                t.value
                    .chars()
                    .filter(|c| !matches!(c, ' ' | '\t' | '\n' | '\r')),
            ),
            // ph-commons writes no empty comment.
            Event::Comment(c) if c.value.is_empty() => {}
            Event::Comment(c) => {
                flush(&mut text, &mut events);
                events.push(format!("comment {:?}", c.value.as_ref()));
            }
            Event::ProcessingInstruction(pi) => {
                flush(&mut text, &mut events);
                events.push(format!(
                    "pi {} {:?}",
                    pi.target,
                    pi.value.as_deref().unwrap_or("")
                ));
            }
        }
    }
    flush(&mut text, &mut events);
    events
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn the_stylesheet_written_keeps_the_tree(
        root in element(proptest::collection::vec(tree(), 0..6)),
        extra in proptest::sample::subsequence(EXTRA.to_vec(), 0..=EXTRA.len()),
        before in proptest::collection::vec(leaf().prop_filter("no text", |n| !matches!(n, Gen::Text(_))), 0..2),
        after in proptest::collection::vec(leaf().prop_filter("no text", |n| !matches!(n, Gen::Text(_))), 0..2),
    ) {
        let mut xml = String::new();
        for node in &before {
            serialize(node, None, &[], &mut xml);
        }
        serialize(&root, None, &extra, &mut xml);
        for node in &after {
            serialize(node, None, &[], &mut xml);
        }
        let written = einvoice_schematron::compile_result(&xml).unwrap();
        prop_assert!(
            written.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\n"),
            "{written}"
        );
        prop_assert_eq!(essence(&written), essence(&xml), "{}\n{}", xml, written);
    }
}
