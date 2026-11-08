//! The maximum element depth of a parsed document (issue #29): up to it a document is
//! parsed, beyond it [`DocumentError::limit`] is [`LimitExceeded::Depth`], with the
//! position of the first element too deep. Parsing and walking a tree never recurse:
//! a document nested a million deep is refused, or with the maximum raised, parsed and
//! walked, on a thread with the default 2 MiB stack.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

use einvoice_xpath_eval::{DEFAULT_MAX_DEPTH, Document, LimitExceeded, NodeAxis, NodeKind};

fn nested(depth: usize) -> String {
    format!("{}t{}", "<x>".repeat(depth), "</x>".repeat(depth))
}

fn on_a_default_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn the_default_maximum_depth() {
    assert_eq!(DEFAULT_MAX_DEPTH, 256);
    let doc = Document::parse(nested(256), None).unwrap();
    assert_eq!(doc.node_count(), 1 + 256 + 1);
    let error = Document::parse(nested(257), None).unwrap_err();
    // The 257th <x> starts at column 1 + 3 * 256.
    let limit = LimitExceeded::Depth {
        max: 256,
        line: 1,
        column: 769,
    };
    assert_eq!(error.limit(), Some(limit));
    assert_eq!(
        error.message(),
        "1:769: element nested deeper than the maximum depth of 256"
    );
    assert_eq!(limit.max(), 256);
    let error = Document::parse_loaded(nested(257).as_str(), "urn:d").unwrap_err();
    assert_eq!(error.limit(), Some(limit));
    // Not well-formed is not a limit.
    assert_eq!(Document::parse("<a>", None).unwrap_err().limit(), None);
    // Well-formedness is checked first, in the whole text: an element too deep followed
    // by an error is not well-formed.
    let error = Document::parse(format!("{}<", nested(257)), None).unwrap_err();
    assert_eq!(error.limit(), None);
    assert_eq!(error.message(), "XML: unknown token at 1:1801");
}

#[test]
fn other_maximum_depths() {
    let text = "<a>\n  <b>\n    <c/>\n  </b>\n  <d/>\n</a>";
    Document::parse_with_max_depth(text, None, 3).unwrap();
    let error = Document::parse_with_max_depth(text, None, 2).unwrap_err();
    assert_eq!(
        error.limit(),
        Some(LimitExceeded::Depth {
            max: 2,
            line: 3,
            column: 5
        })
    );
    let error = Document::parse_with_max_depth(text, None, 1).unwrap_err();
    assert_eq!(
        error.limit(),
        Some(LimitExceeded::Depth {
            max: 1,
            line: 2,
            column: 3
        })
    );
    // Text, comments and processing instructions are not elements.
    Document::parse_with_max_depth("<a>t<!--c--><?p d?></a>", None, 1).unwrap();
    // Every document has a document element: depth 0 refuses them all.
    assert!(Document::parse_with_max_depth("<a/>", None, 0).is_err());
    let loaded = Document::parse_loaded_with_max_depth(text.to_owned(), "urn:d", 3).unwrap();
    assert_eq!(loaded.document_uri(), Some("urn:d"));
    assert!(Document::parse_loaded_with_max_depth(text, "urn:d", 2).is_err());
}

#[test]
fn a_million_levels_never_overflow() {
    on_a_default_stack(|| {
        const DEPTH: usize = 1_000_000;
        let text = nested(DEPTH);
        let error = Document::parse(text.as_str(), None).unwrap_err();
        assert_eq!(
            error.limit(),
            Some(LimitExceeded::Depth {
                max: 256,
                line: 1,
                column: 769
            })
        );
        let doc = Document::parse_with_max_depth(text, None, u32::MAX).unwrap();
        let root = doc.root();
        assert_eq!(root.string_value(), "t");
        let deepest = root
            .axis(NodeAxis::Descendant)
            .find(|n| n.kind() == NodeKind::Text)
            .unwrap()
            .parent()
            .unwrap();
        assert_eq!(deepest.axis(NodeAxis::Ancestor).count(), DEPTH);
        assert_eq!(root.axis(NodeAxis::Descendant).count(), DEPTH + 1);
        drop(doc);
    });
}
