//! Property tests through the public API: the lexical spaces of the built-in types
//! against independent references (the regular expressions of XML Schema Part 2, a
//! calendar computed differently), and robustness of validation on damaged
//! documents.
#![cfg(test)]

use std::fmt::Write as _;
use std::sync::OnceLock;

use einvoice_xsd::{ErrorKind, Schema};
use proptest::prelude::*;
use regex::Regex;

/// One global element per built-in type, in the namespace `urn:p`.
fn types_schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let mut text = String::from(
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="urn:p" elementFormDefault="qualified">"#,
        );
        for name in ["decimal", "integer", "boolean", "date", "dateTime", "time", "base64Binary", "language", "NCName"] {
            write!(text, r#"<xs:element name="{name}" type="xs:{name}"/>"#).unwrap();
        }
        text.push_str("</xs:schema>");
        Schema::load(&[("p.xsd", text.as_str())], &["p.xsd"]).unwrap()
    })
}

fn ubl() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| Schema::ubl_2_1().unwrap())
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Whether `value` is valid for the built-in type `name`, according to this crate.
fn accepted(name: &str, value: &str) -> bool {
    let xml = format!("<p:{name} xmlns:p=\"urn:p\">{}</p:{name}>", escape(value));
    let report = types_schema().validate(&xml).unwrap();
    assert!(
        report
            .errors
            .iter()
            .all(|e| e.kind == ErrorKind::InvalidValue),
        "{report:?}"
    );
    report.is_valid()
}

fn collapse(value: &str) -> String {
    value
        .split([' ', '\t', '\n', '\r'])
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn matches(pattern: &str, value: &str) -> bool {
    Regex::new(&format!("^(?:{pattern})$"))
        .unwrap()
        .is_match(value)
}

/// Days in a month of the proleptic Gregorian calendar, as the difference of two
/// day numbers (H. Hinnant's `days_from_civil`), not by a month table. Years before
/// the common era use the rule of their absolute value, as the crate documents.
fn days_in_month(year: i64, month: i64) -> i64 {
    fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe
    }
    let year = year.abs();
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    days_from_civil(next_year, next_month, 1) - days_from_civil(year, month, 1)
}

const SPACE: &str = "[ \t\n]{0,2}";

fn date_parts() -> impl Strategy<Value = (String, i64, i64, String)> {
    (
        prop_oneof![
            "-?[0-9]{4}",
            "-?[1-9][0-9]{4,6}",
            "-?0[0-9]{4}",
            "[0-9]{1,3}"
        ],
        0_i64..14,
        0_i64..33,
        prop_oneof![
            Just(String::new()),
            Just("Z".to_owned()),
            "[+-](0[0-9]|1[0-5]):[0-6][0-9]"
        ],
    )
}

fn timezone_ok(tz: &str) -> bool {
    if tz.is_empty() || tz == "Z" {
        return true;
    }
    let hours: i64 = tz[1..3].parse().unwrap();
    let minutes: i64 = tz[4..6].parse().unwrap();
    minutes < 60 && hours * 60 + minutes <= 14 * 60
}

fn date_ok(year: &str, month: i64, day: i64) -> bool {
    let digits = year.trim_start_matches('-');
    let Ok(number) = digits.parse::<i64>() else {
        return false;
    };
    digits.len() >= 4
        && !(digits.len() > 4 && digits.starts_with('0'))
        && number != 0
        && (1..=12).contains(&month)
        && day >= 1
        && day <= days_in_month(number, month)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn decimal_lexical_space(core in "[0-9.+-]{0,6}", before in SPACE, after in SPACE) {
        let value = format!("{before}{core}{after}");
        prop_assert_eq!(accepted("decimal", &value), matches(r"(\+|-)?([0-9]+(\.[0-9]*)?|\.[0-9]+)", &collapse(&value)));
    }

    #[test]
    fn integer_lexical_space(core in "[0-9.+ -]{0,6}", before in SPACE, after in SPACE) {
        let value = format!("{before}{core}{after}");
        prop_assert_eq!(accepted("integer", &value), matches(r"[\-+]?[0-9]+", &collapse(&value)));
    }

    #[test]
    fn boolean_lexical_space(core in "(true|false|0|1|TRUE|2|t)?", before in SPACE, after in SPACE) {
        let value = format!("{before}{core}{after}");
        prop_assert_eq!(accepted("boolean", &value), matches("true|false|1|0", &collapse(&value)));
    }

    #[test]
    fn language_lexical_space(core in "[a-zA-Z0-9_-]{0,20}", before in SPACE, after in SPACE) {
        let value = format!("{before}{core}{after}");
        prop_assert_eq!(accepted("language", &value), matches("[a-zA-Z]{1,8}(-[a-zA-Z0-9]{1,8})*", &collapse(&value)));
    }

    #[test]
    fn base64_lexical_space(core in "[A-Da-dQgwU0-3+/= !-]{0,12}", before in SPACE, after in SPACE) {
        let value = format!("{before}{core}{after}");
        // The lexical space of xs:base64Binary as given (for the collapsed value) in
        // XML Schema 1.1 Part 2, 3.3.16, identical to the corrected 1.0 grammar.
        let b04 = "[AQgw]";
        let b16 = "[AEIMQUYcgkosw048]";
        let b64 = "[A-Za-z0-9+/]";
        let pattern = format!(
            "(({b64} ?{b64} ?{b64} ?{b64} ?)*(({b64} ?{b64} ?{b64} ?{b64})|({b64} ?{b64} ?{b16} ?=)|({b64} ?{b04} ?= ?=)))?"
        );
        prop_assert_eq!(accepted("base64Binary", &value), matches(&pattern, &collapse(&value)));
    }

    #[test]
    fn date_lexical_space((year, month, day, tz) in date_parts(), before in SPACE, after in SPACE) {
        let value = format!("{before}{year}-{month:02}-{day:02}{tz}{after}");
        prop_assert_eq!(accepted("date", &value), date_ok(&year, month, day) && timezone_ok(&tz));
    }

    #[test]
    fn time_lexical_space(
        hour in 0_i64..26,
        minute in 0_i64..62,
        second in 0_i64..62,
        fraction in prop_oneof![Just(String::new()), Just(".".to_owned()), "\\.[0-9]{1,6}"],
        tz in prop_oneof![Just(String::new()), Just("Z".to_owned()), "[+-](0[0-9]|1[0-5]):[0-6][0-9]"],
    ) {
        let value = format!("{hour:02}:{minute:02}:{second:02}{fraction}{tz}");
        let fraction_ok = fraction != ".";
        let zero_fraction = fraction.chars().all(|c| c == '.' || c == '0');
        let clock_ok = (hour < 24 && minute < 60 && second < 60) || (hour == 24 && minute == 0 && second == 0 && zero_fraction);
        let expected = fraction_ok && clock_ok && timezone_ok(&tz);
        prop_assert_eq!(accepted("time", &value), expected);
        let date_time = format!("2024-02-29T{value}");
        prop_assert_eq!(accepted("dateTime", &date_time), expected);
    }

    #[test]
    fn ncname_lexical_space(core in "[a-z0-9:._é-]{0,6}", before in SPACE, after in SPACE) {
        let value = format!("{before}{core}{after}");
        prop_assert_eq!(accepted("NCName", &value), matches(r"[_\p{L}][-._0-9\p{L}]*", &collapse(&value)) );
    }

    #[test]
    fn damaged_documents_never_panic(start in any::<prop::sample::Index>(), length in 0_usize..400, insert in "[<>/a-z:=\" ]{0,12}") {
        let base = include_str!("oracle/documents/invoice-full.xml");
        let mut at = start.index(base.len());
        while !base.is_char_boundary(at) {
            at -= 1;
        }
        let mut end = (at + length).min(base.len());
        while !base.is_char_boundary(end) {
            end -= 1;
        }
        let damaged = format!("{}{insert}{}", &base[..at], &base[end..]);
        let lines = u32::try_from(damaged.lines().count()).unwrap();
        if let Ok(report) = ubl().validate(&damaged) {
            for error in &report.errors {
                prop_assert!(error.line >= 1 && error.line <= lines.max(1));
            }
            prop_assert_eq!(ubl().validate(&damaged).unwrap(), report);
        }
    }
}
