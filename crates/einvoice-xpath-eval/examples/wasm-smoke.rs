//! A smoke test that runs on any target, `wasm32-unknown-unknown` included (no system
//! clock, no file system, no output): parses a document from borrowed and from owned text,
//! evaluates expressions under a clock given by the caller, and returns 0 when every
//! result is the expected one, else the number of the first wrong check.
//!
//! ```text
//! cargo run -p einvoice-xpath-eval --example wasm-smoke; echo $?
//! cargo build --release -p einvoice-xpath-eval --example wasm-smoke --target wasm32-unknown-unknown
//! wasmtime run --invoke main target/wasm32-unknown-unknown/release/examples/wasm-smoke.wasm 0 0
//! ```

use std::process::ExitCode;

use einvoice_xpath::{NamespaceMap, XS_NAMESPACE, parse_expr};
use einvoice_xpath_eval::{
    Clock, Context, Document, DynamicContext, Item, NoHost, SimpleStaticContext, compile,
};

const XML: &str = "<i xmlns='urn:i'><l a='19.99'>x &amp; y</l><l a='0.01'>z</l></i>";

/// Each expression and its expected value, as strings joined by `|`.
const CHECKS: &[(&str, &str)] = &[
    ("sum(//*:l/xs:decimal(@a))", "20"),
    ("string-join(//*:l, '|')", "x & y|z"),
    ("current-dateTime()", "2026-09-10T02:28:43.456+02:00"),
    (
        "xs:date('2026-09-10') = xs:date('2026-09-10+02:00')",
        "true",
    ),
    ("count(//node())", "5"),
];

fn main() -> ExitCode {
    ExitCode::from(run())
}

/// 0 when every check passes, else the number of the first failing one (from 1).
fn run() -> u8 {
    let Ok(clock) = Clock::from_unix_millis(120, 1_789_000_123_456) else {
        return 100;
    };
    let dc = DynamicContext::new(&NoHost, clock);
    let namespaces = NamespaceMap::new().bind("xs", XS_NAMESPACE);
    let documents = [
        Document::parse(XML, None),
        Document::parse(XML.to_owned(), None),
    ];
    let mut number = 0;
    for document in documents {
        let Ok(document) = document else {
            return 101;
        };
        let context = Context::for_item(Item::Node(document.root()));
        for (text, expected) in CHECKS {
            number += 1;
            let value = parse_expr(text, &namespaces)
                .ok()
                .and_then(|e| compile(&e, &SimpleStaticContext::new()).ok())
                .and_then(|e| e.evaluate(&dc, &context).ok())
                .map(|items| {
                    items
                        .iter()
                        .map(|i| i.string_value().into_owned())
                        .collect::<Vec<_>>()
                        .join("|")
                });
            if value.as_deref() != Some(*expected) {
                return number;
            }
        }
    }
    0
}
