//! Properties: on any text the reader never panics and keeps the invariants against
//! `roxmltree` ([`common::invariants`]); generated well-formed documents give
//! `roxmltree`'s stream; hostile shapes take time linear in their size.

mod common;

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use common::{Outcome, exact, invariants, ours};
use einvoice_xml::{Event, Reader};
use proptest::prelude::*;

/// Pieces of XML, well-formed or not, that random sequences of are made of.
const PIECES: &[&str] = &[
    "<a>",
    "</a>",
    "<b>",
    "</b>",
    "<a/>",
    "<p:a>",
    "</p:a>",
    "<q:b/>",
    "<a ",
    "<b ",
    " x='1'",
    " y=\"2\"",
    " x='3'",
    " p:x='4'",
    " xml:lang='en'",
    " xmlns='urn:d'",
    " xmlns=''",
    " xmlns:p='urn:p'",
    " xmlns:q='urn:q'",
    " xmlns:p=''",
    " xmlns:xml='http://www.w3.org/XML/1998/namespace'",
    " p:xmlns='u'",
    ">",
    "/>",
    " ",
    "\n",
    "\r",
    "\r\n",
    "\t",
    "text",
    "&amp;",
    "&lt;",
    "&#65;",
    "&#x20AC;",
    "&#13;",
    "&#xD800;",
    "&#0;",
    "&foo;",
    "&",
    "&#",
    "<!-- c -->",
    "<!--",
    "-->",
    "--",
    "<?pi data?>",
    "<?pi?>",
    "<?xml version='1.0'?>",
    "<?xml?>",
    "<?p:i x?>",
    "<?abc?d?>",
    "<![CDATA[x]]>",
    "<![CDATA[",
    "]]>",
    "<!DOCTYPE a>",
    "'",
    "\"",
    "=",
    "<",
    ">",
    ":",
    "\u{1}",
    "\u{FFFE}",
    "\u{FEFF}",
    "\u{e9}",
    "\u{1F600}",
    "<:a>",
    "</:a>",
    "<xmlns:a>",
    "<xml:a>",
];

fn pieces() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PIECES), 0..40).prop_map(|v| v.concat())
}

/// A generated element tree, serialised.
#[derive(Debug, Clone)]
enum Node {
    Element {
        name: (u8, u8),
        attributes: Vec<(u8, u8, String)>,
        declarations: Vec<(u8, u8)>,
        children: Vec<Node>,
    },
    Text(String),
    Comment(String),
    Pi(u8, String),
    Cdata(String),
}

const PREFIXES: &[&str] = &["", "p", "q", "r"];
const LOCALS: &[&str] = &["a", "b", "\u{e9}l\u{e9}ment", "x-y.z", "_n"];
const URIS: &[&str] = &["urn:1", "urn:2", "", "http://x/&amp;y"];

fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(
            &[
                "x",
                " ",
                "\n",
                "\t",
                "\r\n",
                "&amp;",
                "&lt;",
                "&gt;",
                "&quot;",
                "&apos;",
                "&#65;",
                "&#x10FFFF;",
                "\u{e9}",
                "]]",
                ">",
                "'",
                "\"",
                "\u{85}",
            ][..],
        ),
        0..8,
    )
    .prop_map(|v| v.concat())
}

fn node() -> impl Strategy<Value = Node> {
    let leaf = prop_oneof![
        text().prop_map(Node::Text),
        "[a-z -]{0,8}"
            .prop_map(|s| Node::Comment(s.replace("--", "- ").trim_end_matches('-').to_owned())),
        (0u8..3, "[a-z ?]{0,8}").prop_map(|(t, s)| Node::Pi(t, s.replace("?>", "? "))),
        "[a-z<&\\]>]{0,8}".prop_map(|s| Node::Cdata(s.replace("]]>", "]] >"))),
    ];
    leaf.prop_recursive(4, 40, 6, |inner| {
        (
            (0u8..4, 0u8..5),
            prop::collection::vec((0u8..4, 0u8..5, text()), 0..4),
            prop::collection::vec((0u8..4, 0u8..4), 0..3),
            prop::collection::vec(inner, 0..6),
        )
            .prop_map(|(name, attributes, declarations, children)| Node::Element {
                name,
                attributes,
                declarations,
                children,
            })
    })
}

/// Serialises a tree, declaring every prefix it uses so that the document is
/// namespace-well-formed (an attribute whose expanded name repeats is left out).
fn serialise(node: &Node, out: &mut String, in_scope: &[(&'static str, String)]) {
    match node {
        // `]]>` may not appear in character data, even across two text nodes.
        Node::Text(t) => out.push_str(&t.replace('<', "&lt;").replace('>', "&gt;")),
        Node::Comment(c) => {
            out.push_str("<!--");
            out.push_str(c);
            out.push_str("-->");
        }
        Node::Pi(t, data) => {
            out.push_str(["<?t", "<?pi-x", "<?xml-stylesheet"][usize::from(*t)]);
            if !data.is_empty() {
                out.push(' ');
                out.push_str(data);
            }
            out.push_str("?>");
        }
        Node::Cdata(c) => {
            out.push_str("<![CDATA[");
            out.push_str(c);
            out.push_str("]]>");
        }
        Node::Element {
            name,
            attributes,
            declarations,
            children,
        } => {
            let mut scope: Vec<(&'static str, String)> = in_scope.to_vec();
            let mut declared = String::new();
            let mut seen_prefixes = Vec::new();
            for (prefix, uri) in declarations {
                let prefix = PREFIXES[usize::from(*prefix)];
                let uri = URIS[usize::from(*uri)];
                if seen_prefixes.contains(&prefix) || (!prefix.is_empty() && uri.is_empty()) {
                    continue;
                }
                seen_prefixes.push(prefix);
                if prefix.is_empty() {
                    let _ = write!(declared, " xmlns='{uri}'");
                } else {
                    let _ = write!(declared, " xmlns:{prefix}='{uri}'");
                    scope.retain(|(p, _)| *p != prefix);
                    scope.push((prefix, uri.replace("&amp;", "&")));
                }
            }
            let mut needed = Vec::new();
            let element_prefix = PREFIXES[usize::from(name.0)];
            needed.push(element_prefix);
            for (prefix, _, _) in attributes {
                needed.push(PREFIXES[usize::from(*prefix)]);
            }
            for prefix in needed {
                if !prefix.is_empty() && !scope.iter().any(|(p, _)| *p == prefix) {
                    let _ = write!(declared, " xmlns:{prefix}='urn:{prefix}'");
                    scope.push((prefix, format!("urn:{prefix}")));
                }
            }
            let qname = |prefix: &str, local: &str| {
                if prefix.is_empty() {
                    local.to_owned()
                } else {
                    format!("{prefix}:{local}")
                }
            };
            let tag = qname(element_prefix, LOCALS[usize::from(name.1)]);
            out.push('<');
            out.push_str(&tag);
            out.push_str(&declared);
            // One attribute per expanded name (two prefixes may share a namespace).
            let mut written: Vec<(String, usize)> = Vec::new();
            for (prefix, local, value) in attributes {
                let prefix = PREFIXES[usize::from(*prefix)];
                let uri = scope
                    .iter()
                    .rev()
                    .find(|(p, _)| !prefix.is_empty() && *p == prefix)
                    .map_or(String::new(), |(_, uri)| uri.clone());
                let key = (uri, usize::from(*local));
                if written.contains(&key) {
                    continue;
                }
                out.push(' ');
                out.push_str(&qname(prefix, LOCALS[key.1]));
                written.push(key);
                out.push_str("='");
                out.push_str(&value.replace('<', "&lt;").replace('\'', "&apos;"));
                out.push('\'');
            }
            if children.is_empty() {
                out.push_str("/>");
            } else {
                out.push('>');
                for child in children {
                    serialise(child, out, &scope);
                }
                out.push_str("</");
                out.push_str(&tag);
                out.push('>');
            }
        }
    }
}

fn document() -> impl Strategy<Value = String> {
    (node(), any::<bool>(), any::<bool>()).prop_map(|(node, declaration, bom)| {
        let root = match node {
            element @ Node::Element { .. } => element,
            other => Node::Element {
                name: (0, 0),
                attributes: Vec::new(),
                declarations: Vec::new(),
                children: vec![other],
            },
        };
        let mut out = String::new();
        if bom {
            out.push('\u{FEFF}');
        }
        if declaration {
            out.push_str("<?xml version='1.0' encoding='UTF-8'?>\n");
        }
        serialise(&root, &mut out, &[]);
        out
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4000))]

    #[test]
    fn random_pieces_keep_the_invariants(text in pieces()) {
        prop_assert!(invariants(&text).is_ok(), "{:?}: {:?}", text, invariants(&text));
    }

    #[test]
    fn random_strings_keep_the_invariants(text in "(?s).{0,60}") {
        prop_assert!(invariants(&text).is_ok(), "{:?}: {:?}", text, invariants(&text));
    }

    #[test]
    fn random_markup_strings_keep_the_invariants(text in "[<>/?!=&#;:'\"a-c x\\-\\[\\]\r\n]{0,60}") {
        prop_assert!(invariants(&text).is_ok(), "{:?}: {:?}", text, invariants(&text));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1500))]

    #[test]
    fn generated_documents_give_roxmltree_streams(text in document()) {
        prop_assert!(exact(&text).is_ok(), "{:?}: {:?}", text, exact(&text));
        prop_assert!(matches!(ours(&text), Outcome::Stream(_)), "{:?} refused: {:?}", text, ours(&text));
    }

    /// Every prefix of a generated document is refused (only a whole document is
    /// one), with roxmltree's error.
    #[test]
    fn truncated_documents_are_refused_like_roxmltree(text in document(), cut in 0.0f64..1.0) {
        let boundaries: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss, reason = "an index from a fraction")]
        let at = boundaries[(cut * boundaries.len() as f64) as usize];
        let cut = &text[..at];
        prop_assert!(exact(cut).is_ok(), "{:?}: {:?}", cut, exact(cut));
        prop_assert!(matches!(ours(cut), Outcome::Error(_)), "{:?}", cut);
    }

    /// The events describe the text: ranges are in order, within the text, and the
    /// raw attribute values are slices of it.
    #[test]
    fn ranges_are_in_order(text in document()) {
        let mut reader = Reader::new(&text);
        let mut last = 0;
        while let Some(event) = reader.next_event().unwrap() {
            let range = match &event {
                Event::Start(s) => {
                    for a in s.attributes() {
                        prop_assert!(a.range.start >= s.range().start && a.range.end <= s.range().end);
                        prop_assert!(text[a.range.clone()].contains(a.raw));
                    }
                    s.range()
                }
                Event::End(e) => e.range(),
                Event::Text(t) => t.range.clone(),
                Event::Comment(c) => c.range.clone(),
                Event::ProcessingInstruction(p) => p.range.clone(),
            };
            let empty_end = matches!(&event, Event::End(e) if e.is_empty());
            if !empty_end {
                prop_assert!(range.start >= last && range.end <= text.len(), "{range:?} after {last}");
            }
            last = range.end;
        }
    }
}

/// A hostile document of size `n`.
type Shape = (&'static str, fn(usize) -> String);

/// The concatenation of `f(0)` to `f(n - 1)`.
fn repeated(n: usize, f: impl Fn(usize) -> String) -> String {
    (0..n).map(f).collect::<Vec<_>>().concat()
}

/// The shortest of three runs.
fn time(text: &str) -> Duration {
    (0..3)
        .map(|_| {
            let start = Instant::now();
            let result = Reader::new(text).read_to_end();
            let elapsed = start.elapsed();
            std::hint::black_box(result).ok();
            elapsed
        })
        .min()
        .unwrap_or_default()
}

/// Hostile shapes: four times the size must take well under sixteen times as long
/// (a quadratic cost would), measured against eight times as long.
#[test]
fn hostile_shapes_take_linear_time() {
    let shapes: [Shape; 9] = [
        ("attributes on one element", |n| {
            let attributes: String = repeated(n, |i| format!(" a{i}='{i}'"));
            format!("<e{attributes}/>")
        }),
        ("attributes, the last a duplicate", |n| {
            let attributes: String = repeated(n, |i| format!(" a{i}='{i}'"));
            format!("<e{attributes} a0='x'/>")
        }),
        ("prefixed attributes on one element", |n| {
            let attributes: String = repeated(n, |i| format!(" p:a{i}='{i}'"));
            format!("<e xmlns:p='u'{attributes}/>")
        }),
        ("declarations on one element", |n| {
            let declarations: String = repeated(n, |i| format!(" xmlns:p{i}='u{i}'"));
            format!("<e{declarations}/>")
        }),
        ("declarations then prefixed elements", |n| {
            let declarations: String = repeated(n, |i| format!(" xmlns:p{i}='u{i}'"));
            let children: String = repeated(n, |i| format!("<p{}:c/>", i % 7));
            format!("<e{declarations}>{children}</e>")
        }),
        ("nesting", |n| {
            format!("{}{}", "<a>".repeat(n), "</a>".repeat(n))
        }),
        ("references", |n| {
            format!("<a>{}</a>", "&amp;&#65;".repeat(n))
        }),
        ("a long name", |n| {
            let name = "n".repeat(n * 8);
            format!("<{name}></{name}>")
        }),
        ("a comment of hyphens", |n| {
            format!("<a><!--{}--></a>", "- ".repeat(n * 4))
        }),
    ];
    let n = 8_000;
    let mut report = Vec::new();
    for (name, shape) in shapes {
        let (small, large) = (shape(n), shape(4 * n));
        let (t1, t4) = (time(&small), time(&large));
        let ratio = t4.as_secs_f64() / t1.as_secs_f64().max(1e-9);
        report.push(format!(
            "{name}: {} bytes {t1:?}, {} bytes {t4:?}, ratio {ratio:.1}",
            small.len(),
            large.len()
        ));
        assert!(
            ratio < 8.0,
            "{name}: ratio {ratio:.1}\n{}",
            report.join("\n")
        );
    }
    println!("{}", report.join("\n"));
}
