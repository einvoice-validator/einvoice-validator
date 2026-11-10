//! The decoder (`einvoice_xml::decode`) against the two reference parsers, Saxon-HE 12.10
//! and xmllint (libxml2 2.9.14), on every document of `encoding/probes.rs`, as
//! `encoding/expected.tsv` records what they make of each in their reference environment
//! (the runner of the `oracle` workflow, which its header describes: the encoding names
//! xmllint accepts depend on the iconv and ICU of the system, those Saxon-HE accepts on
//! the JDK). The `encoding-oracle` example writes it and checks it against them; this test
//! needs neither. Where both read a document to the same text, so must the decoder and
//! the reader; where both refuse it, so must they; where the references differ, the
//! decoder must refuse the document, and the case must be listed in
//! `encoding/disagreements.tsv` with the reason, as must every document both read that
//! the decoder refuses (an encoding it does not read).

#![cfg(test)]

#[path = "encoding/probes.rs"]
mod probes;

use std::path::Path;

use probes::{Outcome, compare, format_expected, parse_disagreements, parse_expected};

fn read(path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The file is as the example writes it, one line per probe, in order.
#[test]
fn the_expected_results_are_as_written() {
    let text = read(probes::EXPECTED);
    let expected = parse_expected(&text).unwrap();
    assert_eq!(format_expected(&expected), text);
    let ids: Vec<&str> = expected.iter().map(|(id, _, _)| id.as_str()).collect();
    let probes = probes::probes();
    let probe_ids: Vec<&str> = probes.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, probe_ids);
}

#[test]
fn the_decoder_reads_what_both_reference_parsers_read() {
    let probes = probes::probes();
    let expected = parse_expected(&read(probes::EXPECTED)).unwrap();
    let disagreements = parse_disagreements(&read(probes::DISAGREEMENTS)).unwrap();
    let comparison = compare(&probes, &expected, &disagreements);
    println!(
        "{} probes: {} read as both reference parsers read them, {} refused as both refuse them, {} listed",
        probes.len(),
        comparison.read,
        comparison.refused,
        comparison.listed.len()
    );
    assert!(
        comparison.failures.is_empty(),
        "{}",
        comparison.failures.join("\n")
    );
    assert_eq!(
        comparison.read + comparison.refused + comparison.listed.len(),
        probes.len()
    );
}

/// Every name the decoder accepts is a probe that both reference parsers read.
#[test]
fn every_encoding_name_is_checked() {
    let expected = parse_expected(&read(probes::EXPECTED)).unwrap();
    let both_read = |id: &str| {
        expected
            .iter()
            .find(|(e, _, _)| e == id)
            .is_some_and(|(_, saxon, xmllint)| saxon == xmllint && matches!(saxon, Outcome::Ok(_)))
    };
    // The name a probe `<group>/name-<n>-<name>` declares.
    let declares = |id: &str, name: &str| {
        id.split_once('/')
            .and_then(|(_, case)| case.strip_prefix("name-"))
            .and_then(|case| case.split_once('-'))
            .is_some_and(|(_, declared)| declared == name)
    };
    for (name, _) in einvoice_xml::encoding_names() {
        let probes: Vec<&str> = match name {
            "UTF-16" => vec!["utf16le/bom-decl-utf-16", "utf16be/no-bom-decl-utf-16"],
            "UTF-16LE" => vec![
                "utf16le/bom-decl-own-order",
                "utf16le/no-bom-decl-own-order",
            ],
            "UTF-16BE" => vec![
                "utf16be/bom-decl-own-order",
                "utf16be/no-bom-decl-own-order",
            ],
            _ => expected
                .iter()
                .map(|(id, _, _)| id.as_str())
                .filter(|id| declares(id, name))
                .collect(),
        };
        assert!(
            !probes.is_empty() && probes.iter().all(|id| both_read(id)),
            "{name}: no probe that both reference parsers read ({probes:?})"
        );
    }
}
