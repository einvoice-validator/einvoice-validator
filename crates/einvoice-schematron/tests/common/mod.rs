//! Shared by the tests: the `.dom` files the `ph-schematron-oracle` example writes (the DOM
//! documents ph-schematron builds, node by node), and the comparison of a document with
//! one.

#![allow(dead_code, reason = "each test uses part of the helpers")]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use einvoice_schematron::{Document, Node};

/// The root of the workspace.
pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// The data directory of the tests.
pub(crate) fn data() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
}

pub(crate) fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Undoes the escapes of the oracle (`\\`, `\t`, `\n`, `\r`, `\x{XX}`).
pub(crate) fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('x') => {
                assert_eq!(chars.next(), Some('{'), "bad escape in {s:?}");
                let hex: String = chars.by_ref().take_while(|&c| c != '}').collect();
                let code = u32::from_str_radix(&hex, 16).unwrap();
                out.push(char::from_u32(code).unwrap());
            }
            other => panic!("bad escape \\{other:?} in {s:?}"),
        }
    }
    out
}

/// The escapes of the oracle.
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\x{{{:x}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

/// `document` in the `.dom` format of the oracle.
pub(crate) fn dom_text(document: &Document) -> String {
    let mut out = String::from("dom 1\n");
    for node in &document.children {
        dump(node, 0, &mut out);
    }
    out
}

fn dump(node: &Node, depth: usize, out: &mut String) {
    match node {
        Node::Element(element) => {
            let _ = writeln!(
                out,
                "E\t{depth}\t{}\t{}",
                escape(element.namespace().unwrap_or("")),
                escape(element.name())
            );
            for attribute in element.attributes() {
                let _ = writeln!(
                    out,
                    "A\t{depth}\t{}\t{}\t{}",
                    escape(attribute.namespace.as_deref().unwrap_or("")),
                    escape(&attribute.name),
                    escape(&attribute.value)
                );
            }
            for child in &element.children {
                dump(child, depth + 1, out);
            }
        }
        Node::Text(data) => {
            let _ = writeln!(out, "T\t{depth}\t{}", escape(data));
        }
        Node::CData(data) => {
            let _ = writeln!(out, "D\t{depth}\t{}", escape(data));
        }
        Node::Comment(data) => {
            let _ = writeln!(out, "C\t{depth}\t{}", escape(data));
        }
        Node::ProcessingInstruction { target, data } => {
            let _ = writeln!(out, "P\t{depth}\t{}\t{}", escape(target), escape(data));
        }
    }
}

/// The first line where two texts differ, with its neighbours.
pub(crate) fn first_difference(expected: &str, actual: &str) -> String {
    let e: Vec<&str> = expected.split('\n').collect();
    let a: Vec<&str> = actual.split('\n').collect();
    let line = e
        .iter()
        .zip(&a)
        .position(|(x, y)| x != y)
        .unwrap_or_else(|| e.len().min(a.len()));
    let from = line.saturating_sub(2);
    let show = |lines: &[&str]| {
        lines
            .iter()
            .enumerate()
            .skip(from)
            .take(5)
            .map(|(i, l)| format!("  {:>6} {l:?}", i + 1))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "first difference at line {}\nexpected:\n{}\nactual:\n{}",
        line + 1,
        show(&e),
        show(&a)
    )
}
