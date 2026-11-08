//! The built-in datatypes of XML Schema 1.0 (Second Edition, Part 2) that the UBL 2.1
//! schemas use, with their ancestors: whitespace processing and lexical spaces.
//!
//! Each check works on the value after whitespace processing (the `whiteSpace` facet
//! of the type: `preserve` for `string`, `replace` for `normalizedString`, `collapse`
//! for every other type here).

use crate::uri;

/// A built-in simple type this crate implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Builtin {
    /// `xs:anySimpleType`: any string.
    AnySimpleType,
    /// `xs:string`.
    String,
    /// `xs:normalizedString`.
    NormalizedString,
    /// `xs:token`.
    Token,
    /// `xs:language`.
    Language,
    /// `xs:Name`.
    Name,
    /// `xs:NCName`.
    NcName,
    /// `xs:ID`: an `NCName`, unique within the document when it types an attribute.
    Id,
    /// `xs:decimal`, any number of digits.
    Decimal,
    /// `xs:integer`, any number of digits.
    Integer,
    /// `xs:boolean`.
    Boolean,
    /// `xs:date`.
    Date,
    /// `xs:dateTime`.
    DateTime,
    /// `xs:time`.
    Time,
    /// `xs:base64Binary`.
    Base64Binary,
    /// `xs:anyURI`.
    AnyUri,
}

/// The `whiteSpace` facet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WhiteSpace {
    Preserve,
    Replace,
    Collapse,
}

impl Builtin {
    /// Every implemented built-in, base types before the types derived from them.
    pub(crate) const ALL: [Self; 16] = [
        Self::AnySimpleType,
        Self::String,
        Self::NormalizedString,
        Self::Token,
        Self::Language,
        Self::Name,
        Self::NcName,
        Self::Id,
        Self::Decimal,
        Self::Integer,
        Self::Boolean,
        Self::Date,
        Self::DateTime,
        Self::Time,
        Self::Base64Binary,
        Self::AnyUri,
    ];

    /// The local name in the XML Schema namespace.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::AnySimpleType => "anySimpleType",
            Self::String => "string",
            Self::NormalizedString => "normalizedString",
            Self::Token => "token",
            Self::Language => "language",
            Self::Name => "Name",
            Self::NcName => "NCName",
            Self::Id => "ID",
            Self::Decimal => "decimal",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Date => "date",
            Self::DateTime => "dateTime",
            Self::Time => "time",
            Self::Base64Binary => "base64Binary",
            Self::AnyUri => "anyURI",
        }
    }

    /// The built-in this one is derived from by restriction (`None` for
    /// `anySimpleType`, whose base is `anyType`).
    pub(crate) fn base(self) -> Option<Self> {
        match self {
            Self::AnySimpleType => None,
            Self::NormalizedString => Some(Self::String),
            Self::Token => Some(Self::NormalizedString),
            Self::Language | Self::Name => Some(Self::Token),
            Self::NcName => Some(Self::Name),
            Self::Id => Some(Self::NcName),
            Self::Integer => Some(Self::Decimal),
            Self::String
            | Self::Decimal
            | Self::Boolean
            | Self::Date
            | Self::DateTime
            | Self::Time
            | Self::Base64Binary
            | Self::AnyUri => Some(Self::AnySimpleType),
        }
    }

    pub(crate) fn white_space(self) -> WhiteSpace {
        match self {
            Self::AnySimpleType | Self::String => WhiteSpace::Preserve,
            Self::NormalizedString => WhiteSpace::Replace,
            _ => WhiteSpace::Collapse,
        }
    }

    /// Whether `value`, already whitespace-processed, is in the lexical space.
    pub(crate) fn is_valid(self, value: &str) -> bool {
        match self {
            Self::AnySimpleType | Self::String | Self::NormalizedString | Self::Token => true,
            Self::Language => is_language(value),
            Self::Name => is_name(value),
            Self::NcName | Self::Id => is_ncname(value),
            Self::Decimal => is_decimal(value),
            Self::Integer => is_integer(value),
            Self::Boolean => matches!(value, "true" | "false" | "1" | "0"),
            Self::Date => is_date(value),
            Self::DateTime => is_date_time(value),
            Self::Time => is_time(value),
            Self::Base64Binary => is_base64(value),
            Self::AnyUri => uri::is_any_uri(value),
        }
    }

    /// Whether `raw`, before whitespace processing, is in the lexical space once
    /// processed: `self.is_valid(&normalize(raw, self.white_space()))`, without
    /// building the processed value when the check does not need it (the string
    /// types accept every value; base64 skips whitespace as it goes).
    pub(crate) fn is_valid_raw(self, raw: &str) -> bool {
        match self {
            Self::AnySimpleType | Self::String | Self::NormalizedString | Self::Token => true,
            // Collapsing, then dropping the single spaces left between
            // characters, drops every whitespace character.
            Self::Base64Binary => is_base64_skipping(raw, is_xml_space_byte),
            _ => self.is_valid(&normalize(raw, self.white_space())),
        }
    }
}

/// The local names of the XML Schema 1.0 built-in types that this crate does not
/// implement: naming one in `xsi:type` is refused as unsupported, not reported as
/// an unknown type.
pub(crate) const UNIMPLEMENTED_BUILTINS: [&str; 31] = [
    "anyType",
    "IDREF",
    "IDREFS",
    "ENTITY",
    "ENTITIES",
    "NMTOKEN",
    "NMTOKENS",
    "NOTATION",
    "QName",
    "float",
    "double",
    "duration",
    "gYearMonth",
    "gYear",
    "gMonthDay",
    "gDay",
    "gMonth",
    "hexBinary",
    "nonPositiveInteger",
    "negativeInteger",
    "long",
    "int",
    "short",
    "byte",
    "nonNegativeInteger",
    "unsignedLong",
    "unsignedInt",
    "unsignedShort",
    "unsignedByte",
    "positiveInteger",
    "anyAtomicType",
];

/// XML whitespace: space, tab, line feed, carriage return.
pub(crate) fn is_xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// [`is_xml_space`] on a byte of UTF-8 text.
fn is_xml_space_byte(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

/// Applies a `whiteSpace` facet.
pub(crate) fn normalize(value: &str, white_space: WhiteSpace) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    match white_space {
        WhiteSpace::Preserve => Cow::Borrowed(value),
        WhiteSpace::Replace => {
            if value.contains(['\t', '\n', '\r']) {
                Cow::Owned(value.replace(['\t', '\n', '\r'], " "))
            } else {
                Cow::Borrowed(value)
            }
        }
        WhiteSpace::Collapse => {
            // Leading and trailing whitespace only: the value is a slice.
            let trimmed = value.trim_matches(is_xml_space);
            if !trimmed.contains(['\t', '\n', '\r']) && !trimmed.contains("  ") {
                Cow::Borrowed(trimmed)
            } else {
                let mut collapsed = String::with_capacity(trimmed.len());
                for word in trimmed.split(is_xml_space).filter(|s| !s.is_empty()) {
                    if !collapsed.is_empty() {
                        collapsed.push(' ');
                    }
                    collapsed.push_str(word);
                }
                Cow::Owned(collapsed)
            }
        }
    }
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `(\+|-)?([0-9]+(\.[0-9]*)?|\.[0-9]+)`
pub(crate) fn is_decimal(value: &str) -> bool {
    let unsigned = value.strip_prefix(['+', '-']).unwrap_or(value);
    match unsigned.split_once('.') {
        None => is_digits(unsigned),
        Some((int, frac)) => {
            (int.is_empty() || is_digits(int))
                && (frac.is_empty() || is_digits(frac))
                && !(int.is_empty() && frac.is_empty())
        }
    }
}

/// `[\-+]?[0-9]+`
pub(crate) fn is_integer(value: &str) -> bool {
    is_digits(value.strip_prefix(['+', '-']).unwrap_or(value))
}

/// `[a-zA-Z]{1,8}(-[a-zA-Z0-9]{1,8})*` (XML Schema 1.0 Second Edition).
pub(crate) fn is_language(value: &str) -> bool {
    let mut parts = value.split('-');
    let first_ok = parts
        .next()
        .is_some_and(|p| (1..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphabetic()));
    first_ok
        && parts.all(|p| (1..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// `NameStartChar` of XML 1.0 Fifth Edition.
pub(crate) fn is_name_start_char(c: char) -> bool {
    matches!(c,
        ':' | 'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

/// `NameChar` of XML 1.0 Fifth Edition.
pub(crate) fn is_name_char(c: char) -> bool {
    is_name_start_char(c)
        || matches!(c, '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// `Name` of XML 1.0 Fifth Edition.
pub(crate) fn is_name(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(is_name_start_char) && chars.all(is_name_char)
}

/// `NCName` of Namespaces in XML 1.0: a `Name` without colons.
pub(crate) fn is_ncname(value: &str) -> bool {
    !value.contains(':') && is_name(value)
}

/// `QName` of Namespaces in XML 1.0: `(NCName ':')? NCName`.
pub(crate) fn split_qname(value: &str) -> Option<(Option<&str>, &str)> {
    match value.split_once(':') {
        Some((prefix, local)) if is_ncname(prefix) && is_ncname(local) => {
            Some((Some(prefix), local))
        }
        None if is_ncname(value) => Some((None, value)),
        _ => None,
    }
}

/// The base64 alphabet.
fn is_base64_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'+' || b == b'/'
}

/// The lexical space of `base64Binary` (Part 2, 3.2.16, as corrected by the errata of
/// the Second Edition): groups of four characters of the base64 alphabet, the last
/// group optionally padded with one `=` (third character in `AEIMQUYcgkosw048`) or
/// two (second character in `AQgw`), with single spaces allowed between characters.
/// The value is already collapsed, so it has no leading, trailing or double spaces.
pub(crate) fn is_base64(value: &str) -> bool {
    is_base64_skipping(value, |b| b == b' ')
}

/// [`is_base64`] on the characters of `value` that `skip` does not drop, in one
/// pass: the characters left must be a multiple of four, every one of them base64
/// except a final run of at most two `=`, and the padding must fit the last
/// character before it.
fn is_base64_skipping(value: &str, skip: impl Fn(u8) -> bool) -> bool {
    let mut count = 0_usize;
    let mut padding = 0_usize;
    let mut last = None;
    for b in value.bytes().filter(|&b| !skip(b)) {
        count += 1;
        if b == b'=' {
            padding += 1;
        } else if padding > 0 || !is_base64_char(b) {
            // A character after padding, or outside the alphabet.
            return false;
        } else {
            last = Some(b);
        }
    }
    if !count.is_multiple_of(4) || padding > 2 {
        return false;
    }
    match (padding, last) {
        (0, _) => true,
        (1, Some(last)) => b"AEIMQUYcgkosw048".contains(&last),
        (2, Some(last)) => b"AQgw".contains(&last),
        _ => false,
    }
}

fn two_digits(s: &str) -> Option<u32> {
    if s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit()) {
        s.parse().ok()
    } else {
        None
    }
}

/// Whether a year, given by its digits (no sign), is a leap year. Negative years
/// follow the same rule on their absolute value, as libxml2 does: XML Schema 1.0 has
/// no year zero and does not spell out the rule for years before the common era.
fn is_leap_year(digits: &str) -> bool {
    // 10^4 is a multiple of 400: the last four digits decide.
    let tail = &digits[digits.len().saturating_sub(4)..];
    let year: u32 = tail.parse().unwrap_or(1);
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// `-?yyyy-mm-dd` with at least four year digits, no leading zero when there are more,
/// year zero excluded, and a day that exists in that month.
fn date_part(s: &str) -> Option<()> {
    let unsigned = s.strip_prefix('-').unwrap_or(s);
    let mut fields = unsigned.rsplitn(3, '-');
    let day = two_digits(fields.next()?)?;
    let month = two_digits(fields.next()?)?;
    let year = fields.next()?;
    if year.len() < 4 || !is_digits(year) || (year.len() > 4 && year.starts_with('0')) {
        return None;
    }
    if year.bytes().all(|b| b == b'0') {
        return None;
    }
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => return None,
    };
    (1..=days).contains(&day).then_some(())
}

/// `hh:mm:ss(.s+)?`, hours 00 to 23, or 24:00:00 with a zero fraction.
fn time_part(s: &str) -> Option<()> {
    let (whole, fraction) = match s.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (s, None),
    };
    if let Some(fraction) = fraction
        && !is_digits(fraction)
    {
        return None;
    }
    let mut fields = whole.split(':');
    let hour = two_digits(fields.next()?)?;
    let minute = two_digits(fields.next()?)?;
    let second = two_digits(fields.next()?)?;
    if fields.next().is_some() || minute > 59 || second > 59 {
        return None;
    }
    let zero_fraction = fraction.is_none_or(|f| f.bytes().all(|b| b == b'0'));
    (hour < 24 || (hour == 24 && minute == 0 && second == 0 && zero_fraction)).then_some(())
}

/// Splits an optional timezone (`Z` or `(+|-)hh:mm`, at most 14:00 away from UTC)
/// off the end of a date or time; `None` if a timezone is present but invalid.
fn split_timezone(s: &str) -> Option<&str> {
    if let Some(rest) = s.strip_suffix('Z') {
        return Some(rest);
    }
    let bytes = s.as_bytes();
    if bytes.len() >= 6
        && matches!(bytes[bytes.len() - 6], b'+' | b'-')
        && bytes[bytes.len() - 3] == b':'
    {
        let (rest, zone) = s.split_at(s.len() - 6);
        let hours = two_digits(&zone[1..3])?;
        let minutes = two_digits(&zone[4..6])?;
        // A date ending in `-dd` also matches `-hh:mm` only if it has a colon, which
        // dates do not: the shape check above is unambiguous.
        return (minutes <= 59 && (hours < 14 || (hours == 14 && minutes == 0))).then_some(rest);
    }
    Some(s)
}

pub(crate) fn is_date(value: &str) -> bool {
    split_timezone(value).and_then(date_part).is_some()
}

pub(crate) fn is_time(value: &str) -> bool {
    split_timezone(value).and_then(time_part).is_some()
}

pub(crate) fn is_date_time(value: &str) -> bool {
    split_timezone(value)
        .and_then(|rest| rest.split_once('T'))
        .and_then(|(date, time)| date_part(date).and(time_part(time)))
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals() {
        for ok in [
            "0",
            "-1",
            "+1.5",
            ".5",
            "5.",
            "-.5",
            "00.",
            "0001.000",
            "123456789012345678901234567890",
        ] {
            assert!(is_decimal(ok), "{ok}");
        }
        for bad in [
            "", ".", "-", "+", "+.", "1.2.3", "1e3", "1,5", "--1", " 1", "NaN", "INF", "0x10", "１",
        ] {
            assert!(!is_decimal(bad), "{bad}");
        }
    }

    #[test]
    fn integers() {
        for ok in ["0", "-0", "+42", "1234567890123456789012345"] {
            assert!(is_integer(ok), "{ok}");
        }
        for bad in ["", "+", "1.0", "1 2", "a"] {
            assert!(!is_integer(bad), "{bad}");
        }
    }

    #[test]
    fn dates() {
        for ok in [
            "2024-02-29",
            "2000-02-29",
            "12024-01-01",
            "-0001-01-01",
            "-0004-02-29",
            "2024-03-01Z",
            "2024-03-01+14:00",
            "2024-03-01-14:00",
            "2024-03-01+13:59",
        ] {
            assert!(is_date(ok), "{ok}");
        }
        for bad in [
            "2024-02-30",
            "2023-02-29",
            "1900-02-29",
            "-0001-02-29",
            "2024-13-01",
            "2024-00-10",
            "2024-03-00",
            "24-03-01",
            "2024-3-1",
            "0000-01-01",
            "02024-01-01",
            "+2024-03-01",
            "2024-03-01+14:01",
            "2024-03-01+15:00",
            "2024-03-01+01:60",
            "2024-03-01+0100",
            "2024-03-01z",
            "2024-03-01T00:00:00",
            "",
            "2024-03-01 Z",
        ] {
            assert!(!is_date(bad), "{bad}");
        }
    }

    #[test]
    fn times() {
        for ok in [
            "10:15:30",
            "24:00:00",
            "24:00:00.000",
            "24:00:00+01:00",
            "10:15:30.123Z",
            "23:59:59.99999999999999999999",
        ] {
            assert!(is_time(ok), "{ok}");
        }
        for bad in [
            "24:00:01",
            "24:01:00",
            "25:00:00",
            "23:60:00",
            "23:59:60",
            "10:15",
            "10:15:30.",
            "1:15:30",
            "10:15:30+24:00",
            "10:15:30,5",
            "24:00:00.1",
        ] {
            assert!(!is_time(bad), "{bad}");
        }
    }

    #[test]
    fn date_times() {
        for ok in [
            "2024-03-01T10:15:30",
            "2024-12-31T24:00:00Z",
            "2024-03-01T10:15:30.250+01:00",
        ] {
            assert!(is_date_time(ok), "{ok}");
        }
        for bad in [
            "2024-03-01 10:15:30",
            "2024-03-01t10:15:30",
            "2024-03-01",
            "2023-02-29T10:00:00",
            "2024-03-01T10:15Z",
            "2024-03-01T10:15:30Zx",
        ] {
            assert!(!is_date_time(bad), "{bad}");
        }
    }

    #[test]
    fn base64() {
        for ok in [
            "",
            "QQ==",
            "QUI=",
            "QUJD",
            "Q U J D Q Q = =",
            "QUJD QUJD QUI=",
        ] {
            assert!(is_base64(ok), "{ok}");
        }
        for bad in [
            "abc",
            "QR==",
            "QUJ=",
            "Q===",
            "====",
            "QQ=",
            "QQ==QUJD",
            "QU!JD",
            "-_-_",
            "QQ== QQ==",
        ] {
            assert!(!is_base64(bad), "{bad}");
        }
    }

    #[test]
    fn languages() {
        for ok in ["en", "en-GB", "x-a1-B2", "englishx-language"] {
            assert!(is_language(ok), "{ok}");
        }
        for bad in [
            "",
            "en_GB",
            "123",
            "e1",
            "en-123456789",
            "englishxx",
            "en-",
            "en--GB",
            "-en",
        ] {
            assert!(!is_language(bad), "{bad}");
        }
    }

    #[test]
    fn names() {
        assert!(is_ncname("signature-1"));
        assert!(is_ncname("sïgnature"));
        assert!(!is_ncname("1signature"));
        assert!(!is_ncname("a:b"));
        assert!(!is_ncname(""));
        assert_eq!(split_qname("a:b"), Some((Some("a"), "b")));
        assert_eq!(split_qname("b"), Some((None, "b")));
        assert_eq!(split_qname("a:b:c"), None);
        assert_eq!(split_qname("1"), None);
    }

    #[test]
    fn white_space() {
        assert_eq!(normalize(" a \t\n b ", WhiteSpace::Collapse), "a b");
        assert_eq!(normalize("a\tb\n", WhiteSpace::Replace), "a b ");
        assert_eq!(normalize(" a ", WhiteSpace::Preserve), " a ");
        // Only leading and trailing whitespace: a slice of the value, not a copy.
        assert!(matches!(
            normalize("\n  12.50\t", WhiteSpace::Collapse),
            std::borrow::Cow::Borrowed("12.50")
        ));
    }

    /// The collapsed value as XML Schema Part 2 defines it: runs of whitespace
    /// become one space, then leading and trailing spaces go.
    fn reference_collapse(value: &str) -> String {
        value
            .split(is_xml_space)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// `base64Binary` checked on a copy of the value without its spaces.
    fn reference_base64(value: &str) -> bool {
        let bytes: Vec<u8> = value.bytes().filter(|&b| b != b' ').collect();
        if !bytes.len().is_multiple_of(4) {
            return false;
        }
        let padding = bytes.iter().rev().take_while(|&&b| b == b'=').count();
        let data = &bytes[..bytes.len() - padding];
        if padding > 2 || !data.iter().all(|&b| is_base64_char(b)) {
            return false;
        }
        match (padding, data.last()) {
            (0, _) => true,
            (1, Some(last)) => b"AEIMQUYcgkosw048".contains(last),
            (2, Some(last)) => b"AQgw".contains(last),
            _ => false,
        }
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(4000))]

        #[test]
        fn collapse_matches_the_definition(value in "[ \t\n\ra-c.]{0,12}") {
            let collapsed = normalize(&value, WhiteSpace::Collapse);
            proptest::prop_assert_eq!(collapsed.as_ref(), reference_collapse(&value));
        }

        #[test]
        fn base64_in_one_pass(value in "[A-Da-dQgwU0-3+/= !\t\n\r-]{0,16}") {
            proptest::prop_assert_eq!(is_base64(&value), reference_base64(&value));
        }

        #[test]
        fn raw_values_as_processed_values(
            value in "[ \t\n\r0-9a-zA-Z+/=:.T-]{0,16}",
            index in 0..Builtin::ALL.len(),
        ) {
            let builtin = Builtin::ALL[index];
            let processed = normalize(&value, builtin.white_space());
            proptest::prop_assert_eq!(builtin.is_valid_raw(&value), builtin.is_valid(&processed));
        }
    }
}
