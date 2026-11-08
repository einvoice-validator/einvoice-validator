//! Deeply nested documents (issue #29): a document nested up to the maximum depth
//! ([`ValidateOptions::max_depth`], 256 by default) is validated through every stage, the
//! XML check, the schema and the stylesheets, on a thread with the default 2 MiB stack of
//! a spawned thread, in the build being tested (debug for `cargo test`); a deeper one is
//! refused with [`Error::Limit`], never a stack overflow, which would abort the test
//! binary. The documents are official examples with elements of another namespace
//! nested inside them, where the schemas accept anything (`ext:ExtensionContent` in UBL)
//! or where the CII mutation corpus nests them (the seller).

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]
#![cfg(all(feature = "cen-ubl", feature = "cen-cii"))]

mod common;

use einvoice_validator::{
    DEFAULT_MAX_DEPTH, DEFAULT_MAX_STACK, Error, ErrorKind, LimitExceeded, Report, ValidateOptions,
    Validator, Verdict,
};
use std::sync::atomic::{AtomicUsize, Ordering};

use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};

/// The default stack of a spawned thread.
const STACK: usize = 2 << 20;

/// Runs `f` on a thread with a 2 MiB stack.
fn on_a_default_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

/// The elements nested inside the official example: the outermost declares their
/// namespace.
const OUTER: &str = "<y:x xmlns:y=\"urn:y\">";
const INNER: &str = "<y:x>";
const END: &str = "</y:x>";

fn example(path: &str) -> String {
    std::fs::read_to_string(common::join(&common::workspace_root(), path)).unwrap()
}

/// The text of `example` with `content` inserted after the start tag that `tag` begins.
fn insert_after(example: &str, tag: &str, content: &str) -> String {
    let start = example.find(tag).unwrap();
    let end = start + example[start..].find('>').unwrap() + 1;
    format!("{}{content}{}", &example[..end], &example[end..])
}

/// `n` (at least 1) elements nested inside one another.
fn chain(n: usize) -> String {
    format!("{OUTER}{}{}", INNER.repeat(n - 1), END.repeat(n))
}

/// The CEN UBL example 1, `depth` deep: the invoice, `ext:UBLExtensions`,
/// `ext:UBLExtension` and `ext:ExtensionContent` are depths 1 to 4, the chain inside it
/// depths 5 to `depth`.
fn deep_ubl(depth: usize) -> String {
    insert_after(
        &example("artefacts/cen-en16931-ubl-1.3.17/ubl/examples/ubl-tc434-example1.xml"),
        "<Invoice",
        &format!(
            "<ext:UBLExtensions xmlns:ext=\"urn:oasis:names:specification:ubl:schema:xsd:CommonExtensionComponents-2\"><ext:UBLExtension><ext:ExtensionContent>{}</ext:ExtensionContent></ext:UBLExtension></ext:UBLExtensions>",
            chain(depth - 4)
        ),
    )
}

/// The CEN CII example 3, `depth` deep: the seller is at depth 4, the chain inside it
/// at depths 5 to `depth` (as the mutation case `long/deep-nesting`).
fn deep_cii(depth: usize) -> String {
    insert_after(
        &example("artefacts/cen-en16931-cii-1.3.17/cii/examples/CII_example3.xml"),
        "<ram:SellerTradeParty>",
        &chain(depth - 4),
    )
}

/// The 1-based line and column of the start tag of the element at `depth` (5 or more)
/// of a document of [`deep_ubl`] or [`deep_cii`].
fn position(text: &str, depth: usize) -> (u32, u32) {
    let (offset, _) = text.match_indices("<y:x").nth(depth - 5).unwrap();
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before[before.rfind('\n').map_or(0, |i| i + 1)..]
        .chars()
        .count()
        + 1;
    (u32::try_from(line).unwrap(), u32::try_from(column).unwrap())
}

/// The error for a document whose element at depth `max + 1` is at `at`.
fn assert_too_deep(result: Result<Report, Error>, max: u32, at: (u32, u32)) {
    let error = result.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    let Error::Limit { limit, message } = &error else {
        panic!("{error:?}");
    };
    assert_eq!(
        *limit,
        LimitExceeded::Depth {
            max,
            line: at.0,
            column: at.1
        }
    );
    assert_eq!(
        message,
        &format!(
            "document, {}:{}: element nested deeper than the maximum depth of {max}",
            at.0, at.1
        )
    );
    assert_eq!(error.to_string(), format!("limit exceeded: {message}"));
}

#[test]
fn the_defaults() {
    assert_eq!(DEFAULT_MAX_DEPTH, 256);
    assert_eq!(DEFAULT_MAX_STACK, 1536 << 10);
    assert_eq!(einvoice_validator::max_nesting_for_depth(256), 576);
}

/// Issue #29: the built-in UBL rule set at the maximum depth, then one level deeper and
/// the issue's 1500 levels.
#[test]
fn cen_ubl_at_the_maximum_depth() {
    on_a_default_stack(|| {
        let validator = Validator::cen_ubl().unwrap();
        let report = validator.validate(deep_ubl(256).as_bytes()).unwrap();
        assert_eq!(report.schema_errors, []);
        assert_eq!(report.verdict(), Verdict::Valid);
        assert!(report.stylesheets[0].failure.is_none());
        for depth in [257, 1500] {
            let text = deep_ubl(depth);
            assert_too_deep(
                validator.validate(text.as_bytes()),
                256,
                position(&text, 257),
            );
            // Given as a buffer: the same.
            assert_too_deep(
                validator.validate_owned(text.clone().into_bytes()),
                256,
                position(&text, 257),
            );
        }
    });
}

/// The built-in CII rule set: its location paths recurse over the ancestors of the
/// node that fails a rule.
#[test]
fn cen_cii_at_the_maximum_depth() {
    on_a_default_stack(|| {
        let validator = Validator::cen_cii().unwrap();
        let report = validator.validate(deep_cii(256).as_bytes()).unwrap();
        assert!(report.stylesheets[0].failure.is_none());
        assert_eq!(report.verdict(), Verdict::Valid);
        let text = deep_cii(257);
        assert_too_deep(
            validator.validate(text.as_bytes()),
            256,
            position(&text, 257),
        );
    });
}

/// The Peppol bundle (both its stylesheets) when its cache is built (see
/// `crates/einvoice-xslt-exec/tests/peppol_equivalence.rs`): its rules fire on the
/// innermost, empty, element, and their location paths recurse over its ancestors, so
/// the nesting reaches twice the depth.
#[test]
fn the_peppol_bundle_at_the_maximum_depth() {
    const BUNDLE: &str = "target/peppol/peppol-bis-billing-ubl-3.0.22";
    const REQUIRED: &str = "EINVOICE_PEPPOL_REQUIRED";
    let dir = common::join(&common::workspace_root(), BUNDLE);
    if !dir.join("bundle.toml").exists() {
        assert!(
            std::env::var_os(REQUIRED).is_none_or(|v| v.is_empty()),
            "no Peppol bundle in {BUNDLE}; {REQUIRED} is set"
        );
        eprintln!("SKIPPED: no Peppol bundle in {BUNDLE} (scripts/fetch-peppol.sh)");
        return;
    }
    on_a_default_stack(move || {
        let validator = Validator::from_bundle_dir(&dir).unwrap();
        let report = validator.validate(deep_ubl(256).as_bytes()).unwrap();
        assert_eq!(report.stylesheets.len(), 2);
        assert!(report.stylesheets.iter().all(|s| s.failure.is_none()));
        // The innermost element is empty: a Peppol rule says so.
        assert!(
            report
                .findings()
                .any(|f| f.location.ends_with("/*:x[namespace-uri()='urn:y'][1]"))
        );
        let text = deep_ubl(257);
        assert_too_deep(
            validator.validate(text.as_bytes()),
            256,
            position(&text, 257),
        );
    });
}

/// The maximum depth is the caller's: larger (with the stack it needs), or smaller.
#[test]
fn the_maximum_depth_is_configurable() {
    on_a_default_stack(|| {
        let validator = Validator::cen_ubl().unwrap();
        let deeper = ValidateOptions::new().max_depth(300);
        let report = validator
            .validate_with(deep_ubl(300).as_bytes(), &deeper)
            .unwrap();
        assert_eq!(report.verdict(), Verdict::Valid);
        let text = deep_ubl(301);
        assert_too_deep(
            validator.validate_with(text.as_bytes(), &deeper),
            300,
            position(&text, 301),
        );
        // Example 1 itself is 5 deep (`cac:AccountingSupplierParty/cac:Party/
        // cac:PostalAddress/cac:Country/cbc:IdentificationCode` under the invoice is 6).
        let plain = example("artefacts/cen-en16931-ubl-1.3.17/ubl/examples/ubl-tc434-example1.xml");
        validator
            .validate_with(plain.as_bytes(), &ValidateOptions::new().max_depth(6))
            .unwrap();
        match validator.validate_with(plain.as_bytes(), &ValidateOptions::new().max_depth(5)) {
            Err(Error::Limit {
                limit: LimitExceeded::Depth { max: 5, .. },
                ..
            }) => {}
            other => panic!("{other:?}"),
        }
    });
}

/// The stack the stylesheets may use is the caller's too: below what a document needs,
/// a [`LimitExceeded::Stack`] error names the stylesheet.
#[test]
fn the_stack_is_configurable() {
    on_a_default_stack(|| {
        let validator = Validator::cen_ubl().unwrap();
        let options = ValidateOptions::new().max_stack(32 << 10);
        let error = validator
            .validate_with(deep_ubl(256).as_bytes(), &options)
            .unwrap_err();
        let Error::Limit { limit, message } = &error else {
            panic!("{error:?}");
        };
        assert_eq!(*limit, LimitExceeded::Stack { max: 32 << 10 });
        assert!(
            message.starts_with("stylesheet EN16931-UBL, line "),
            "{message}"
        );
        assert!(
            message.ends_with(
                ": templates and functions invoked one inside another use more than 32768 bytes of stack"
            ),
            "{message}"
        );
    });
}

/// A well-formed fragment nested inside the example: elements opened, closed, empty
/// elements and text, in a random order. `spine` elements are opened first, so that the
/// maximum depth is often reached.
fn fragment(spine: usize, steps: &[u8]) -> String {
    let mut out = String::from(OUTER);
    let mut open = 1;
    for _ in 0..spine {
        out.push_str(INNER);
        open += 1;
    }
    for &step in steps {
        match step {
            0 => {
                out.push_str(INNER);
                open += 1;
            }
            1 if open > 1 => {
                out.push_str(END);
                open -= 1;
            }
            1 | 2 => out.push_str("<y:e/>"),
            _ => out.push_str("t&amp;"),
        }
    }
    out.push_str(&END.repeat(open));
    out
}

/// Random nesting shapes, deep and wide, around the maximum depth: never a stack
/// overflow; a report up to the maximum depth, the depth error beyond it.
#[test]
fn random_nesting_never_aborts() {
    on_a_default_stack(|| {
        let validator = Validator::cen_ubl().unwrap();
        let mut runner = TestRunner::new(Config {
            cases: 48,
            failure_persistence: None,
            ..Config::default()
        });
        let shapes = (200usize..=270, proptest::collection::vec(0u8..4, 0..200));
        let (accepted, refused) = (AtomicUsize::new(0), AtomicUsize::new(0));
        runner
            .run(&shapes, |(spine, steps)| {
                let fragment = fragment(spine, &steps);
                let text = insert_after(
                    &example("artefacts/cen-en16931-ubl-1.3.17/ubl/examples/ubl-tc434-example1.xml"),
                    "<Invoice",
                    &format!(
                        "<ext:UBLExtensions xmlns:ext=\"urn:oasis:names:specification:ubl:schema:xsd:CommonExtensionComponents-2\"><ext:UBLExtension><ext:ExtensionContent>{fragment}</ext:ExtensionContent></ext:UBLExtension></ext:UBLExtensions>"
                    ),
                );
                let depth = common::depth(text.as_bytes());
                match validator.validate(text.as_bytes()) {
                    Ok(report) => {
                        prop_assert!(depth <= 256, "accepted at depth {}", depth);
                        accepted.fetch_add(1, Ordering::Relaxed);
                        prop_assert!(report.stylesheets[0].failure.is_none());
                    }
                    Err(Error::Limit {
                        limit: LimitExceeded::Depth { max: 256, .. },
                        ..
                    }) => {
                        prop_assert!(depth > 256, "refused at depth {}", depth);
                        refused.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => prop_assert!(false, "{}", e),
                }
                Ok(())
            })
            .unwrap();
        let (accepted, refused) = (accepted.into_inner(), refused.into_inner());
        eprintln!("{accepted} documents accepted, {refused} refused");
        assert!(
            accepted > 0 && refused > 0,
            "{accepted} accepted, {refused} refused"
        );
    });
}
