//! Property tests: manifests written from generated values parse back to them, and
//! damaged corpus documents never make the validator panic or report a bug.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

mod common;

use std::fmt::Write as _;

use einvoice_validator::{ErrorKind, RuleSet};
use proptest::prelude::*;

fn name() -> impl Strategy<Value = String> {
    "[A-Za-z0-9][A-Za-z0-9._-]{0,12}"
}

fn version() -> impl Strategy<Value = String> {
    "[0-9A-Za-z.+~-]{1,10}"
}

fn path() -> impl Strategy<Value = String> {
    proptest::collection::vec("[A-Za-z0-9_-][A-Za-z0-9._ -]{0,8}", 1..4).prop_map(|s| s.join("/"))
}

fn sha256() -> impl Strategy<Value = String> {
    "[0-9a-f]{64}"
}

/// Text for a title or a string value: no control characters, quotes and backslashes
/// included.
fn text() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 \"'\\\\#=é€\\[\\]]{0,20}".prop_filter("non-blank", |t| !t.trim().is_empty())
}

/// A TOML basic string.
fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

type Stylesheet = (String, Option<String>, String, String);

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Whatever the values, a manifest that writes them parses back to them.
    #[test]
    fn manifests_round_trip(
        rule_set in name(),
        title in text(),
        bundle_version in version(),
        schema in proptest::bool::ANY,
        stylesheets in proptest::collection::vec(
            (name(), proptest::option::of(version()), path(), sha256()), 1..4),
        comments in proptest::bool::ANY,
    ) {
        let mut seen = std::collections::BTreeSet::new();
        let stylesheets: Vec<Stylesheet> =
            stylesheets.into_iter().filter(|s| seen.insert(s.0.clone())).collect();
        let comment = if comments { "   # a comment" } else { "" };
        let mut manifest = String::new();
        let _ = writeln!(manifest, "format = 1{comment}");
        let _ = writeln!(manifest, "name = {}{comment}", quoted(&rule_set));
        let _ = writeln!(manifest, "title = {}", quoted(&title));
        let _ = writeln!(manifest, "version = '{bundle_version}'");
        if schema {
            let _ = writeln!(manifest, "schema = \"ubl-2.1\"");
        }
        for (name, version, path, sha) in &stylesheets {
            let _ = writeln!(manifest, "\n[[stylesheet]]{comment}\npath = {}", quoted(path));
            let _ = writeln!(manifest, "sha256 = {}\nname = {}", quoted(sha), quoted(name));
            if let Some(version) = version {
                let _ = writeln!(manifest, "version = {}", quoted(version));
            }
        }
        let parsed = RuleSet::parse(&manifest).unwrap();
        prop_assert_eq!(parsed.name(), rule_set.as_str());
        prop_assert_eq!(parsed.title(), title.as_str());
        prop_assert_eq!(parsed.version(), bundle_version.as_str());
        prop_assert_eq!(parsed.schema().is_some(), schema);
        prop_assert_eq!(parsed.stylesheets().len(), stylesheets.len());
        for (entry, (name, version, path, sha)) in parsed.stylesheets().iter().zip(&stylesheets) {
            prop_assert_eq!(entry.name(), name.as_str());
            prop_assert_eq!(entry.version(), version.as_deref().unwrap_or(&bundle_version));
            prop_assert_eq!(entry.path(), path.as_str());
            prop_assert_eq!(entry.sha256(), sha.as_str());
        }
    }

    /// Any text is either a manifest or an error naming the manifest; never a panic.
    #[test]
    fn any_text_is_parsed_or_refused(text in "(format = 1\n)?[a-z\\[\\]=\"' #\n0-9./_-]{0,80}") {
        if let Err(e) = RuleSet::parse(&text) {
            prop_assert_eq!(e.kind(), ErrorKind::Bundle);
            prop_assert!(e.to_string().contains("bundle.toml"), "{}", e);
        }
    }
}

#[cfg(feature = "cen-ubl")]
mod damaged_documents {
    use einvoice_validator::{ErrorKind, Validator};
    use proptest::prelude::*;

    fn validator() -> &'static Validator {
        static CEN: std::sync::OnceLock<Validator> = std::sync::OnceLock::new();
        CEN.get_or_init(|| Validator::cen_ubl().unwrap())
    }

    fn document() -> &'static [u8] {
        static DOCUMENT: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
        DOCUMENT.get_or_init(|| {
            std::fs::read(super::common::case("examples/ubl-tc434-example1.xml").path()).unwrap()
        })
    }

    /// An edit of a document: at a position (a fraction of its length), delete some
    /// bytes and insert others.
    fn edit() -> impl Strategy<Value = (f64, usize, Vec<u8>)> {
        (
            0.0..1.0f64,
            0..12usize,
            proptest::collection::vec(
                prop_oneof![
                    3 => proptest::sample::select(b"<>/&;=\"' \n:x0.-".to_vec()),
                    1 => any::<u8>(),
                ],
                0..6,
            ),
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        /// A damaged invoice gets a report or an error about the document; never a
        /// panic, never an internal error (the parsers of the stages disagreeing) or a
        /// bundle error.
        #[test]
        fn damaged_invoices_are_reported(edits in proptest::collection::vec(edit(), 1..4)) {
            let mut bytes = document().to_vec();
            for (at, delete, insert) in edits {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss,
                    reason = "a position within the document, from a fraction in [0, 1)"
                )]
                let at = ((bytes.len() as f64) * at) as usize;
                let end = (at + delete).min(bytes.len());
                bytes.splice(at..end, insert);
            }
            if let Err(e) = validator().validate(&bytes) {
                prop_assert!(
                    matches!(
                        e.kind(),
                        ErrorKind::NotXml | ErrorKind::Unsupported | ErrorKind::Transform
                    ),
                    "{}",
                    e
                );
            }
        }

        /// An invoice whose element values are replaced by arbitrary text (amounts,
        /// dates, codes and identifiers the rules then compute with) gets a report or an
        /// error about the document; never a panic, an internal or a bundle error. The
        /// result is well-formed, so the schema and the stylesheet run.
        #[test]
        fn invoices_with_other_values_are_reported(
            edits in proptest::collection::vec(
                (0.0..1.0f64, "[0-9a-zA-Z .,:+-]{0,12}|-?[0-9]{0,30}(\\.[0-9]{0,30})?|[ \t\n]{0,3}"),
                1..6,
            )
        ) {
            let mut text = String::from_utf8(document().to_vec()).unwrap();
            for (at, value) in edits {
                // The text nodes between a start tag and the next tag.
                let values: Vec<(usize, usize)> = text
                    .match_indices('>')
                    .filter_map(|(i, _)| {
                        let start = i + 1;
                        let end = start + text[start..].find('<')?;
                        (text[start..end].trim() != "").then_some((start, end))
                    })
                    .collect();
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss,
                    reason = "an index into the values, from a fraction in [0, 1)"
                )]
                let (start, end) = values[((values.len() as f64) * at) as usize];
                text.replace_range(start..end, &value);
            }
            match validator().validate(text.as_bytes()) {
                // A stylesheet only stops without an error being returned when the
                // schema has already made the document invalid.
                Ok(report) => prop_assert!(
                    report.is_complete() || !report.schema_errors.is_empty(),
                    "{:?}",
                    report
                ),
                Err(e) => prop_assert!(
                    matches!(e.kind(), ErrorKind::Unsupported | ErrorKind::Transform),
                    "{}",
                    e
                ),
            }
        }
    }
}
