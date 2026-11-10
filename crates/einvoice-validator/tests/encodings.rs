//! Documents in the encodings other than UTF-8 that the validator reads (UTF-16 in either
//! byte order, with and without a byte order mark, US-ASCII, ISO-8859-1, ISO-8859-15 and
//! windows-1252): every document of the CEN EN 16931 UBL and CII 1.3.17 corpora, their
//! cases and mutation cases, re-encoded into each (`common/reencode.rs`, where its
//! characters allow), gives the result of the document in UTF-8: the same report (verdict,
//! schema errors with their lines and columns, findings, SVRL) or the same error. Only the
//! columns on the first line may differ, where the XML declaration names another
//! encoding and so changes length. That the reference engines, Saxon-HE and xmllint, give
//! the results of the UTF-8 documents on the same re-encoded documents is checked by the
//! example `encoding-oracle`, which needs them.

#![cfg(test)]
#![cfg(all(feature = "cen-ubl", feature = "cen-cii"))]

mod common;

use common::reencode::{TARGETS, reencode, with_declaration};
use einvoice_validator::{
    Clock, DEFAULT_MAX_DEPTH, Error, LimitExceeded, Report, ValidateOptions, Validator,
};

/// The corpora: directory, built-in rule set.
const CORPORA: [(&str, &str); 2] = [
    ("corpus/cen-en16931-ubl-1.3.17", "cen-ubl"),
    ("corpus/cen-en16931-cii-1.3.17", "cen-cii"),
];

/// The input documents of `cases.tsv` and `mutations.tsv` of a corpus, relative to the
/// workspace root.
fn inputs(corpus: &str) -> Vec<String> {
    let mut out = Vec::new();
    for file in ["cases.tsv", "mutations.tsv"] {
        let path = common::join(&common::workspace_root(), &format!("{corpus}/{file}"));
        let text = std::fs::read_to_string(&path).unwrap();
        out.extend(
            text.lines()
                .skip(1)
                .map(|line| line.split('\t').nth(1).unwrap().to_owned()),
        );
    }
    out
}

/// The result of a validation, as compared: the report, or the error, with the columns
/// of the first line set to 0.
fn outcome(result: Result<Report, Error>) -> String {
    match result {
        Ok(mut report) => {
            for e in &mut report.schema_errors {
                if e.line == 1 {
                    e.column = 0;
                }
            }
            format!("{report:?}")
        }
        Err(Error::NotXml { line: 1, .. }) => "not XML on line 1".to_owned(),
        Err(Error::Limit {
            limit: LimitExceeded::Depth { max, line: 1, .. },
            ..
        }) => format!("too deep on line 1 (maximum {max})"),
        Err(e) => format!("{e:?}"),
    }
}

/// What one document gave: the number of encodings it was compared in, by target, and
/// the differences.
type Compared = (Vec<usize>, Vec<String>);

/// Compares the document `input` in UTF-8 and in every target encoding its characters
/// allow.
fn compare(validator: &Validator, clock: Clock, input: &str) -> Compared {
    let bytes = std::fs::read(common::join(&common::workspace_root(), input)).unwrap();
    let text = einvoice_xml::decode(&bytes).unwrap().into_owned();
    let deep = common::depth(text.as_bytes()) > DEFAULT_MAX_DEPTH;
    let options = ValidateOptions::new().clock(clock).max_depth(if deep {
        common::RAISED_MAX_DEPTH
    } else {
        DEFAULT_MAX_DEPTH
    });
    let expected = outcome(validator.validate_with(&bytes, &options));
    let mut compared = vec![0; TARGETS.len()];
    let mut differences = Vec::new();
    for (target, count) in TARGETS.iter().zip(&mut compared) {
        let Some(encoded) = reencode(&text, target) else {
            continue;
        };
        *count += 1;
        let actual = outcome(validator.validate_owned_with(encoded, &options));
        if actual != expected {
            differences.push(format!(
                "{input} in {}:\n  UTF-8: {expected:.300}\n  {}: {actual:.300}",
                target.label, target.label
            ));
        }
    }
    (compared, differences)
}

#[test]
fn reencoded_corpus_documents_give_the_utf8_result() {
    let clock = Clock::from_unix_millis(0, 1_789_000_123_456).unwrap();
    let mut documents = 0;
    let mut compared = vec![0usize; TARGETS.len()];
    let mut differences = Vec::new();
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    for (corpus, rule_set) in CORPORA {
        let validator = Validator::builtin(rule_set).unwrap();
        let inputs = inputs(corpus);
        documents += inputs.len();
        let chunks: Vec<&[String]> = inputs.chunks(inputs.len().div_ceil(threads)).collect();
        let results: Vec<Compared> = std::thread::scope(|scope| {
            let handles: Vec<_> = chunks
                .iter()
                .map(|chunk| {
                    let validator = &validator;
                    // As much stack as the deepest corpus case needs at the raised maximum.
                    std::thread::Builder::new()
                        .stack_size(16 << 20)
                        .spawn_scoped(scope, move || {
                            let mut out: Compared = (vec![0; TARGETS.len()], Vec::new());
                            for input in *chunk {
                                let (counts, diffs) = compare(validator, clock, input);
                                for (total, n) in out.0.iter_mut().zip(counts) {
                                    *total += n;
                                }
                                out.1.extend(diffs);
                            }
                            out
                        })
                        .unwrap()
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for (counts, diffs) in results {
            for (total, n) in compared.iter_mut().zip(counts) {
                *total += n;
            }
            differences.extend(diffs);
        }
    }
    println!("{documents} documents in UTF-8, re-encoded:");
    for (target, count) in TARGETS.iter().zip(&compared) {
        println!("  {}: {count}", target.label);
    }
    for d in differences.iter().take(10) {
        eprintln!("{d}");
    }
    assert!(differences.is_empty(), "{} differ", differences.len());
    assert_eq!(documents, 2519);
    // UTF-16 holds every document; the single-byte encodings those whose characters
    // they have: most of them, and for US-ASCII four in five.
    for (target, count) in TARGETS.iter().zip(&compared) {
        if target.form_is_utf16() {
            assert_eq!(*count, documents, "{}", target.label);
        } else {
            assert!(*count > documents * 4 / 5, "{}: {count}", target.label);
        }
    }
}

#[test]
fn rewritten_declarations() {
    let d = |text: &str, name: Option<&str>| with_declaration(text, name).unwrap();
    assert_eq!(
        d(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<a/>",
            Some("latin9")
        ),
        "<?xml version=\"1.0\" encoding=\"latin9\"?>\n<a/>"
    );
    assert_eq!(
        d(
            "<?xml version='1.0'  encoding = 'utf-8' standalone='no' ?><a/>",
            None
        ),
        "<?xml version='1.0' standalone='no' ?><a/>"
    );
    assert_eq!(
        d("<?xml version=\"1.0\"?><a/>", Some("UTF-16")),
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?><a/>"
    );
    assert_eq!(
        d("<a/>", Some("US-ASCII")),
        "<?xml version=\"1.0\" encoding=\"US-ASCII\"?><a/>"
    );
    assert_eq!(d("<a/>", None), "<a/>");
}

/// The encoders against the decoder, on every character they encode.
#[test]
fn encoders_and_decoder_agree() {
    let all: String = (0x20..0x3000_u32)
        .filter_map(char::from_u32)
        .filter(|c| !matches!(c, '<' | '&'))
        .collect();
    for target in &TARGETS {
        let text: String = all
            .chars()
            .filter(|&c| reencode(&c.to_string(), target).is_some())
            .collect();
        let document = format!("<?xml version=\"1.0\"?><a>{text}</a>");
        let bytes = reencode(&document, target).unwrap();
        let decoded = einvoice_xml::decode(&bytes).unwrap();
        let expected = with_declaration(&document, target.declared).unwrap();
        assert_eq!(decoded, expected, "{}", target.label);
    }
}
