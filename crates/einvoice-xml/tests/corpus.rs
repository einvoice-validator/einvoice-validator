//! The reader against `roxmltree` on every XML file of the repository (the vendored
//! artefacts, the corpus with its mutations, the schemas and test documents of the
//! crates) and of the Peppol cache when it is present (`target/peppol/`, built by
//! `scripts/fetch-peppol.sh`): the same stream, or the same error. Then on documents
//! damaged at every position (truncated, a character deleted, markup inserted): the
//! invariants of [`common::invariants`].

mod common;

use std::path::{Path, PathBuf};

use common::{Outcome, exact, invariants_of, ours, root, xml_files};

fn files() -> (Vec<PathBuf>, bool) {
    let root = root();
    let mut files = Vec::new();
    for dir in ["artefacts", "corpus", "crates"] {
        files.extend(xml_files(&root.join(dir)));
    }
    let peppol = root.join("target/peppol");
    let with_peppol = peppol.is_dir();
    if with_peppol {
        files.extend(xml_files(&peppol));
    }
    (files, with_peppol)
}

fn relative(path: &Path) -> String {
    let root = std::path::absolute(root()).unwrap_or_else(|_| root());
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    path.strip_prefix(&root)
        .unwrap_or(&path)
        .display()
        .to_string()
}

#[test]
fn every_xml_file_gives_the_roxmltree_result() {
    let (files, with_peppol) = files();
    let (mut accepted, mut refused, mut not_utf8, mut bytes) = (0, 0, 0, 0);
    let mut failures = Vec::new();
    for path in &files {
        let data = std::fs::read(path).unwrap();
        bytes += data.len();
        let Ok(text) = std::str::from_utf8(&data) else {
            not_utf8 += 1;
            continue;
        };
        match exact(text) {
            Ok(()) => match ours(text) {
                Outcome::Stream(_) => accepted += 1,
                Outcome::Error(e) => {
                    refused += 1;
                    println!("refused by both: {}: {e}", relative(path));
                }
            },
            Err(e) => failures.push(format!("{}: {e}", relative(path))),
        }
    }
    println!(
        "{} files ({bytes} bytes, Peppol cache {}): {accepted} accepted and {refused} refused by both, {not_utf8} not UTF-8, {} differences",
        files.len(),
        if with_peppol { "included" } else { "absent" },
        failures.len()
    );
    assert!(
        files.len() > 1000,
        "corpus not found: {} files",
        files.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Markup inserted by the damage.
const INSERTIONS: &[&str] = &[
    "<",
    ">",
    "&",
    "'",
    "\"",
    "=",
    ":",
    " ",
    "/",
    "!",
    "?",
    "]]>",
    "<!--",
    "-->",
    "--",
    "<?",
    "?>",
    "<![CDATA[",
    "&#0;",
    "&#xD800;",
    "&#65;",
    "&lt",
    "&amp;",
    "&foo;",
    "\u{1}",
    "\r",
    "\t",
    "\u{FFFE}",
    "\u{FEFF}",
    "\u{e9}",
    "xmlns:p=''",
    " xmlns='urn:x'",
    " p:a='1'",
    " xmlns:q='u' q:b='1'",
    "</x>",
    "<x>",
    "<x/>",
    "<:x/>",
    "<!DOCTYPE x>",
    "<?xml version='1.0'?>",
    "<?xml\t?>",
    " a='1' a='2'",
];

/// Damaged copies of `text`: truncated, a character deleted, each insertion, at every
/// `step`-th character boundary.
fn damaged(text: &str, step: usize) -> Vec<String> {
    let boundaries: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .step_by(step)
        .collect();
    let mut out = Vec::new();
    for &i in &boundaries {
        out.push(text[..i].to_owned());
        if let Some(c) = text[i..].chars().next() {
            out.push(format!("{}{}", &text[..i], &text[i + c.len_utf8()..]));
        }
        for insertion in INSERTIONS {
            out.push(format!("{}{insertion}{}", &text[..i], &text[i..]));
        }
    }
    out
}

/// How the damaged documents fared.
#[derive(Default)]
struct Tally {
    total: usize,
    same_error: usize,
    strict: usize,
    declaration: usize,
    accepted: usize,
    failures: Vec<String>,
}

fn tally(texts: &[String]) -> Tally {
    let mut tally = Tally::default();
    for text in texts {
        tally.total += 1;
        let (a, b) = (ours(text), common::theirs(text));
        if let Err(e) = invariants_of(text, &a, &b) {
            tally.failures.push(format!("{e}\n{text:?}"));
            continue;
        }
        match (a, b) {
            (Outcome::Error(a), Outcome::Error(b))
                if a.message == b.message && (a.row, a.col) == (b.row, b.col) =>
            {
                tally.same_error += 1;
            }
            (Outcome::Error(a), _) if a.strict => tally.strict += 1,
            (Outcome::Error(_), _) => tally.declaration += 1,
            (Outcome::Stream(_), _) => tally.accepted += 1,
        }
    }
    tally
}

#[test]
fn damaged_documents_keep_the_invariants() {
    let root = root();
    let documents = [
        (
            "artefacts/cen-en16931-ubl-1.3.17/test/CreditNote-unit-UBL/BR-51.xml",
            1,
        ),
        (
            "corpus/cen-en16931-ubl-1.3.17/mutations/markup/attribute-whitespace-references.xml",
            41,
        ),
        (
            "corpus/cen-en16931-ubl-1.3.17/mutations/markup/crlf-line-ends.xml",
            53,
        ),
        (
            "corpus/cen-en16931-ubl-1.3.17/mutations/markup/comments-around-root.xml",
            67,
        ),
        (
            "corpus/cen-en16931-ubl-1.3.17/mutations/markup/cdata-code.xml",
            71,
        ),
    ];
    let mut texts = Vec::new();
    for (file, step) in documents {
        texts.extend(damaged(
            &std::fs::read_to_string(root.join(file)).unwrap(),
            step,
        ));
    }
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    let chunk = texts.len().div_ceil(threads);
    let tallies: Vec<Tally> = std::thread::scope(|scope| {
        let handles: Vec<_> = texts
            .chunks(chunk)
            .map(|part| scope.spawn(move || tally(part)))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut all = Tally::default();
    for t in tallies {
        all.total += t.total;
        all.same_error += t.same_error;
        all.strict += t.strict;
        all.declaration += t.declaration;
        all.accepted += t.accepted;
        all.failures.extend(t.failures);
    }
    println!(
        "{} damaged documents: {} refused with roxmltree's error, {} refused with an error roxmltree does not report, {} refused after an XML declaration roxmltree reads as a processing instruction, {} accepted with roxmltree's stream",
        all.total, all.same_error, all.strict, all.declaration, all.accepted
    );
    all.failures.truncate(20);
    assert!(all.failures.is_empty(), "{}", all.failures.join("\n\n"));
}
