//! `--max-depth` (issue #29): a document nested deeper than the maximum depth (256 by
//! default) is an error of kind `unsupported`, exit status 3, never a crash; with the
//! maximum raised, it is validated, the worker threads' stacks growing with it.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

mod common;

/// The CEN UBL example 1, `depth` deep, with elements of another namespace nested in
/// `ext:ExtensionContent` (depth 4).
fn deep_ubl(depth: usize) -> String {
    let path = common::join(
        &common::workspace_root(),
        "artefacts/cen-en16931-ubl-1.3.17/ubl/examples/ubl-tc434-example1.xml",
    );
    let text = std::fs::read_to_string(path).unwrap();
    let start = text.find("<Invoice").unwrap();
    let end = start + text[start..].find('>').unwrap() + 1;
    let n = depth - 4;
    format!(
        "{}<ext:UBLExtensions xmlns:ext=\"urn:oasis:names:specification:ubl:schema:xsd:CommonExtensionComponents-2\"><ext:UBLExtension><ext:ExtensionContent><y:x xmlns:y=\"urn:y\">{}{}</ext:ExtensionContent></ext:UBLExtension></ext:UBLExtensions>{}",
        &text[..end],
        "<y:x>".repeat(n - 1),
        "</y:x>".repeat(n),
        &text[end..]
    )
}

/// Valid: the example only gets the CEN warning on extensions.
fn assert_valid(run: &common::Run) {
    assert_eq!(run.code, 0, "{run:?}");
    assert!(
        run.stdout
            .starts_with("-: valid: 1 warning\n  warning  UBL-CR-001:"),
        "{run:?}"
    );
}

#[test]
fn documents_deeper_than_the_maximum_are_refused() {
    let ok = common::run(&["-"], Some(deep_ubl(256).as_bytes()));
    assert_valid(&ok);
    // The issue's 1500 levels, which aborted the process before.
    for depth in [257, 1500] {
        let run = common::run(&["-"], Some(deep_ubl(depth).as_bytes()));
        assert_eq!(run.code, 3, "{run:?}");
        // The 257th element: on the line of the invoice's start tag (14), after the
        // extension wrappers and 252 nested elements.
        assert_eq!(
            run.stdout,
            "-: error (unsupported): limit exceeded: document, 14:1572: element nested deeper than the maximum depth of 256\n"
        );
    }
    let json = common::run(&["--format", "json", "-"], Some(deep_ubl(257).as_bytes()));
    assert_eq!(json.code, 3);
    let report = common::json::parse(&json.stdout).unwrap();
    let document = &report.get("documents").array()[0];
    assert_eq!(document.get("status").str(), Some("error"));
    assert_eq!(document.get("error").get("kind").str(), Some("unsupported"));
    assert!(
        document
            .get("error")
            .get("message")
            .str()
            .unwrap()
            .contains("element nested deeper than the maximum depth of 256")
    );
}

/// A larger maximum: the document is validated, on worker threads whose stack grows
/// with it (here 6064 levels of nesting: about 25 MiB).
#[test]
fn the_maximum_depth_can_be_raised() {
    let run = common::run(&["--max-depth", "300", "-"], Some(deep_ubl(300).as_bytes()));
    assert_valid(&run);
    let run = common::run(&["--max-depth=300", "-"], Some(deep_ubl(301).as_bytes()));
    assert_eq!(run.code, 3, "{run:?}");
    assert!(
        run.stdout.contains("maximum depth of 300"),
        "{}",
        run.stdout
    );
    let run = common::run(
        &["--max-depth", "3000", "--jobs", "2", "-"],
        Some(deep_ubl(3000).as_bytes()),
    );
    assert_valid(&run);
}

#[test]
fn bad_maximum_depths_are_usage_errors() {
    for bad in ["0", "x", "100001"] {
        let run = common::run(&["--max-depth", bad, "-"], Some(b"<a/>"));
        assert_eq!(run.code, 2, "{run:?}");
        assert!(
            run.stderr
                .contains("--max-depth: expected a positive integer of at most 100000"),
            "{}",
            run.stderr
        );
    }
}

#[test]
fn the_help_documents_the_option() {
    let run = common::run(&["--help"], None);
    assert!(run.stdout.contains("--max-depth N"), "{}", run.stdout);
    assert!(
        run.stdout.contains("(default: 256, as libxml2"),
        "{}",
        run.stdout
    );
}
