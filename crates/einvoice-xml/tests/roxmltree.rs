//! The reader against `roxmltree` on hand-written documents: the malformed ones get the
//! same error, the well-formed ones the same stream; the documents where the reader
//! differs by design get the documented result; the limits trigger at the same point.

mod common;

use std::fmt::Write as _;

use common::cases::{DATA, LINE_ENDS, MALFORMED, STRICT, WELL_FORMED};
use common::{Outcome, exact, invariants, ours, ours_with, theirs, theirs_with};
use einvoice_xml::{Error, Event, Limits, Reader};

#[test]
fn malformed_documents_get_roxmltree_errors() {
    let mut failures = Vec::new();
    for (name, text) in MALFORMED {
        match (exact(text), theirs(text)) {
            (Err(e), _) => failures.push(format!("{name}: {e}")),
            (Ok(()), Outcome::Stream(_)) => {
                failures.push(format!("{name}: well-formed for both, expected malformed"));
            }
            (Ok(()), Outcome::Error(_)) => {}
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    println!(
        "{} malformed documents: same error as roxmltree",
        MALFORMED.len()
    );
}

#[test]
fn well_formed_documents_get_roxmltree_streams() {
    let mut failures = Vec::new();
    for (name, text) in WELL_FORMED {
        match (exact(text), ours(text)) {
            (Err(e), _) => failures.push(format!("{name}: {e}")),
            (Ok(()), Outcome::Error(e)) => failures.push(format!("{name}: refused by both: {e}")),
            (Ok(()), Outcome::Stream(_)) => {}
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn documents_roxmltree_accepts_against_the_specification_are_refused() {
    let mut failures = Vec::new();
    for (name, text, expected) in STRICT {
        match ours(text) {
            Outcome::Error(e) if e.message == *expected => {}
            other => failures.push(format!("{name}: expected {expected:?}, got {other:?}")),
        }
        if let Err(e) = invariants(text) {
            failures.push(format!("{name}: {e}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn data_follows_the_specification() {
    let mut failures = Vec::new();
    for (name, text, expected) in DATA {
        match ours(text) {
            Outcome::Stream(lines) if lines == *expected => {}
            other => failures.push(format!("{name}: expected {expected:?}, got {other:?}")),
        }
        if let Err(e) = invariants(text) {
            failures.push(format!("{name}: {e}"));
        }
    }
    for (name, text, expected) in LINE_ENDS {
        let mut reader = Reader::new(text);
        let mut found = None;
        while let Some(event) = reader.next_event().unwrap() {
            match event {
                Event::Comment(c) => found = Some(c.value.into_owned()),
                Event::ProcessingInstruction(pi) => {
                    found = pi.value.map(std::borrow::Cow::into_owned);
                }
                _ => {}
            }
        }
        if found.as_deref() != Some(*expected) {
            failures.push(format!("{name}: expected {expected:?}, got {found:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// What `roxmltree` 0.21.2 gives where the reader differs, pinned so that a change
/// upstream shows here.
#[test]
fn roxmltree_behaviour_where_the_reader_differs() {
    let text_of = |xml: &str| {
        let doc = roxmltree::Document::parse(xml).unwrap();
        doc.root_element().text().unwrap_or_default().to_owned()
    };
    assert_eq!(text_of("<a>&#65;\r</a>"), "A\r");
    assert_eq!(text_of("<a>\r&#65;</a>"), "\rA");
    assert_eq!(text_of("<a>&amp;\r&amp;</a>"), "&\r&");
    assert_eq!(text_of("<a>&#xD800;</a>"), "\u{FFFD}");
    assert_eq!(text_of("<a>&#x110000;</a>"), "\u{FFFD}");
    let doc = roxmltree::Document::parse("<a><!-- x\r\ny --></a>").unwrap();
    let comment = doc.root_element().first_child().unwrap();
    assert_eq!(comment.text(), Some(" x\r\ny "));
    let doc = roxmltree::Document::parse("<a p:xmlns='urn:x'/>").unwrap();
    assert_eq!(doc.root_element().tag_name().namespace(), Some("urn:x"));
    let doc = roxmltree::Document::parse("<?xml\tversion='1.0'?><a/>").unwrap();
    assert_eq!(
        doc.root().first_child().unwrap().pi().unwrap().target,
        "xml"
    );
    let doc = roxmltree::Document::parse("<:a :b='1'/>").unwrap();
    assert_eq!(doc.root_element().tag_name().name(), "a");
    assert_eq!(doc.root_element().attribute("b"), Some("1"));
    for accepted in [
        "<a xmlns:p=''/>",
        "<a xmlns:xmlns='urn:x'/>",
        "<a xmlns='urn:x' xmlns='urn:y'/>",
        "<?xml version='abc' encoding='%' standalone='maybe'?><a/>",
        "<?xml versionx='1.0'?><a/>",
        "<a><?xml?><?a:b x?><?abc?def?></a>",
    ] {
        assert!(roxmltree::Document::parse(accepted).is_ok(), "{accepted}");
    }
    assert!(roxmltree::Document::parse("<xml:a/>").is_err());
}

#[test]
fn nodes_limit_triggers_where_roxmltree_does() {
    let documents = [
        "<a/>",
        "<!-- c --><?p?><a>x<![CDATA[y]]>&amp;z<b/>t<!--c-->u<?q?></a><!-- d -->",
        "<a>\n  <b c='1'>text</b>\n  <d/>\n</a>",
    ];
    for text in documents {
        let nodes = roxmltree::Document::parse(text)
            .unwrap()
            .descendants()
            .count();
        for limit in 0..=u32::try_from(nodes).unwrap() + 1 {
            let ours = ours_with(&mut Reader::with_limits(
                text,
                Limits::default().with_nodes(limit),
            ));
            let options = roxmltree::ParsingOptions {
                nodes_limit: limit,
                ..common::roxmltree_options()
            };
            let theirs = theirs_with(text, options);
            assert_eq!(ours, theirs, "{text}, limit {limit}");
        }
    }
}

#[test]
fn namespaces_limit_triggers_where_roxmltree_does() {
    for count in [65_534, 65_535, 65_536] {
        let mut text = String::from("<r>");
        for i in 0..count {
            let _ = write!(text, "<e xmlns:p{i}='urn:{i}'/>");
        }
        // Repeating a declaration does not count.
        text.push_str("<e xmlns:p0='urn:0'/></r>");
        let ours = ours(&text);
        assert_eq!(ours, theirs(&text), "{count} declarations");
        let refused = matches!(&ours, Outcome::Error(e) if e.message == Error::NamespacesLimitReached.to_string());
        assert_eq!(refused, count >= 65_536, "{count} declarations");
    }
}

#[test]
fn attributes_and_depth_limits() {
    let text = "<a x='1'><b y='2' z='3'/><c/></a>";
    for (limit, refused) in [(0, true), (1, true), (2, true), (3, true), (4, false)] {
        let result =
            Reader::with_limits(text, Limits::default().with_attributes(limit)).read_to_end();
        assert_eq!(result.is_err(), refused, "attributes limit {limit}");
        if refused {
            assert_eq!(result, Err(Error::AttributesLimitReached));
        }
    }
    for (limit, refused) in [(0, true), (1, true), (2, false)] {
        let result = Reader::with_limits(text, Limits::default().with_depth(limit)).read_to_end();
        assert_eq!(
            result,
            if refused {
                Err(Error::DepthLimitReached)
            } else {
                Ok(())
            },
            "depth {limit}"
        );
    }
    let deep = format!("{}{}", "<a>".repeat(1000), "</a>".repeat(1000));
    assert_eq!(
        Reader::with_limits(&deep, Limits::default().with_depth(1000)).read_to_end(),
        Ok(())
    );
    assert_eq!(
        Reader::with_limits(&deep, Limits::default().with_depth(999)).read_to_end(),
        Err(Error::DepthLimitReached)
    );
}

#[test]
fn hundred_thousand_levels_like_roxmltree() {
    let deep = format!("{}{}", "<a>".repeat(100_000), "</a>".repeat(100_000));
    assert_eq!(exact(&deep), Ok(()));
}
