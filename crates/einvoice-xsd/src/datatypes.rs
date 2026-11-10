//! The built-in simple types of XML Schema 1.0 (Second Edition, Part 2): whitespace
//! processing and lexical spaces. Every primitive and derived built-in datatype is here;
//! the schemas use some of them, and a document can name any of them in `xsi:type`.
//!
//! Each check works on the value after whitespace processing (the `whiteSpace` facet
//! of the type: `preserve` for `string`, `replace` for `normalizedString`, `collapse`
//! for every other type here). Where a value is valid only in a context (an `xs:QName`
//! whose prefix must be declared, an `xs:IDREF` that must match an `xs:ID` of the
//! document, an `xs:ENTITY` that must name an unparsed entity, an `xs:NOTATION` that
//! must name a notation of the schema), the lexical check is here and the validation
//! checks the rest.

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
    /// `xs:IDREF`: an `NCName`, which must be the value of an `xs:ID` of the document.
    IdRef,
    /// `xs:ENTITY`: an `NCName`, which must be the name of an unparsed entity of the
    /// document type declaration.
    Entity,
    /// `xs:NMTOKEN`.
    NmToken,
    /// `xs:nonPositiveInteger`.
    NonPositiveInteger,
    /// `xs:negativeInteger`.
    NegativeInteger,
    /// `xs:long`: from -2^63 to 2^63 - 1.
    Long,
    /// `xs:int`: from -2^31 to 2^31 - 1.
    Int,
    /// `xs:short`: from -2^15 to 2^15 - 1.
    Short,
    /// `xs:byte`: from -128 to 127.
    Byte,
    /// `xs:nonNegativeInteger`.
    NonNegativeInteger,
    /// `xs:unsignedLong`: from 0 to 2^64 - 1.
    UnsignedLong,
    /// `xs:unsignedInt`: from 0 to 2^32 - 1.
    UnsignedInt,
    /// `xs:unsignedShort`: from 0 to 65535.
    UnsignedShort,
    /// `xs:unsignedByte`: from 0 to 255.
    UnsignedByte,
    /// `xs:positiveInteger`.
    PositiveInteger,
    /// `xs:float`.
    Float,
    /// `xs:double`.
    Double,
    /// `xs:duration`.
    Duration,
    /// `xs:gYearMonth`.
    GYearMonth,
    /// `xs:gYear`.
    GYear,
    /// `xs:gMonthDay`.
    GMonthDay,
    /// `xs:gDay`.
    GDay,
    /// `xs:gMonth`.
    GMonth,
    /// `xs:hexBinary`.
    HexBinary,
    /// `xs:QName`: its prefix, if any, must be declared where the value is.
    QName,
    /// `xs:NOTATION`: a `QName` naming a notation declared in the schema.
    Notation,
    /// `xs:NMTOKENS`: a list of one or more `NMTOKEN`.
    NmTokens,
    /// `xs:IDREFS`: a list of one or more `IDREF`.
    IdRefs,
    /// `xs:ENTITIES`: a list of one or more `ENTITY`.
    Entities,
}

/// The `whiteSpace` facet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WhiteSpace {
    Preserve,
    Replace,
    Collapse,
}

impl Builtin {
    /// Every built-in simple type, base types before the types derived from them.
    pub(crate) const ALL: [Self; 45] = [
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
        Self::IdRef,
        Self::Entity,
        Self::NmToken,
        Self::NonPositiveInteger,
        Self::NegativeInteger,
        Self::Long,
        Self::Int,
        Self::Short,
        Self::Byte,
        Self::NonNegativeInteger,
        Self::UnsignedLong,
        Self::UnsignedInt,
        Self::UnsignedShort,
        Self::UnsignedByte,
        Self::PositiveInteger,
        Self::Float,
        Self::Double,
        Self::Duration,
        Self::GYearMonth,
        Self::GYear,
        Self::GMonthDay,
        Self::GDay,
        Self::GMonth,
        Self::HexBinary,
        Self::QName,
        Self::Notation,
        Self::NmTokens,
        Self::IdRefs,
        Self::Entities,
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
            Self::IdRef => "IDREF",
            Self::Entity => "ENTITY",
            Self::NmToken => "NMTOKEN",
            Self::NonPositiveInteger => "nonPositiveInteger",
            Self::NegativeInteger => "negativeInteger",
            Self::Long => "long",
            Self::Int => "int",
            Self::Short => "short",
            Self::Byte => "byte",
            Self::NonNegativeInteger => "nonNegativeInteger",
            Self::UnsignedLong => "unsignedLong",
            Self::UnsignedInt => "unsignedInt",
            Self::UnsignedShort => "unsignedShort",
            Self::UnsignedByte => "unsignedByte",
            Self::PositiveInteger => "positiveInteger",
            Self::Float => "float",
            Self::Double => "double",
            Self::Duration => "duration",
            Self::GYearMonth => "gYearMonth",
            Self::GYear => "gYear",
            Self::GMonthDay => "gMonthDay",
            Self::GDay => "gDay",
            Self::GMonth => "gMonth",
            Self::HexBinary => "hexBinary",
            Self::QName => "QName",
            Self::Notation => "NOTATION",
            Self::NmTokens => "NMTOKENS",
            Self::IdRefs => "IDREFS",
            Self::Entities => "ENTITIES",
        }
    }

    /// The base type definition: the built-in this one is derived from by restriction,
    /// or `anySimpleType` for the primitive types and the list types (`None` for
    /// `anySimpleType`, whose base is `anyType`).
    pub(crate) fn base(self) -> Option<Self> {
        match self {
            Self::AnySimpleType => None,
            Self::NormalizedString => Some(Self::String),
            Self::Token => Some(Self::NormalizedString),
            Self::Language | Self::Name | Self::NmToken => Some(Self::Token),
            Self::NcName => Some(Self::Name),
            Self::Id | Self::IdRef | Self::Entity => Some(Self::NcName),
            Self::Integer => Some(Self::Decimal),
            Self::NonPositiveInteger | Self::Long | Self::NonNegativeInteger => Some(Self::Integer),
            Self::NegativeInteger => Some(Self::NonPositiveInteger),
            Self::Int => Some(Self::Long),
            Self::Short => Some(Self::Int),
            Self::Byte => Some(Self::Short),
            Self::UnsignedLong | Self::PositiveInteger => Some(Self::NonNegativeInteger),
            Self::UnsignedInt => Some(Self::UnsignedLong),
            Self::UnsignedShort => Some(Self::UnsignedInt),
            Self::UnsignedByte => Some(Self::UnsignedShort),
            Self::String
            | Self::Decimal
            | Self::Boolean
            | Self::Date
            | Self::DateTime
            | Self::Time
            | Self::Base64Binary
            | Self::AnyUri
            | Self::Float
            | Self::Double
            | Self::Duration
            | Self::GYearMonth
            | Self::GYear
            | Self::GMonthDay
            | Self::GDay
            | Self::GMonth
            | Self::HexBinary
            | Self::QName
            | Self::Notation
            | Self::NmTokens
            | Self::IdRefs
            | Self::Entities => Some(Self::AnySimpleType),
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
            Self::NcName | Self::Id | Self::IdRef | Self::Entity => is_ncname(value),
            Self::Decimal => is_decimal(value),
            Self::Integer => is_integer(value),
            Self::Boolean => matches!(value, "true" | "false" | "1" | "0"),
            Self::Date => is_date(value),
            Self::DateTime => is_date_time(value),
            Self::Time => is_time(value),
            Self::Base64Binary => is_base64(value),
            Self::AnyUri => uri::is_any_uri(value),
            Self::NmToken => is_nmtoken(value),
            Self::NonPositiveInteger
            | Self::NegativeInteger
            | Self::Long
            | Self::Int
            | Self::Short
            | Self::Byte
            | Self::NonNegativeInteger
            | Self::UnsignedLong
            | Self::UnsignedInt
            | Self::UnsignedShort
            | Self::UnsignedByte
            | Self::PositiveInteger => self.is_in_range(value),
            Self::Float | Self::Double => is_float(value),
            Self::Duration => is_duration(value),
            Self::GYearMonth => is_g_year_month(value),
            Self::GYear => is_g_year(value),
            Self::GMonthDay => is_g_month_day(value),
            Self::GDay => is_g_day(value),
            Self::GMonth => is_g_month(value),
            Self::HexBinary => is_hex_binary(value),
            Self::QName | Self::Notation => split_qname(value).is_some(),
            Self::NmTokens => is_list(value, is_nmtoken),
            Self::IdRefs | Self::Entities => is_list(value, is_ncname),
        }
    }

    /// The bounds of an integer type derived from `xs:integer`, as signed decimal
    /// strings without leading zeros: `None` for no bound.
    fn integer_bounds(self) -> (Option<&'static str>, Option<&'static str>) {
        match self {
            Self::NonPositiveInteger => (None, Some("0")),
            Self::NegativeInteger => (None, Some("-1")),
            Self::Long => (Some("-9223372036854775808"), Some("9223372036854775807")),
            Self::Int => (Some("-2147483648"), Some("2147483647")),
            Self::Short => (Some("-32768"), Some("32767")),
            Self::Byte => (Some("-128"), Some("127")),
            Self::NonNegativeInteger => (Some("0"), None),
            Self::UnsignedLong => (Some("0"), Some("18446744073709551615")),
            Self::UnsignedInt => (Some("0"), Some("4294967295")),
            Self::UnsignedShort => (Some("0"), Some("65535")),
            Self::UnsignedByte => (Some("0"), Some("255")),
            Self::PositiveInteger => (Some("1"), None),
            _ => (None, None),
        }
    }

    /// Whether `value` is an `xs:integer` within the bounds of the type (Part 2,
    /// 3.3.14 to 3.3.25: each is derived from `xs:integer` by `minInclusive` and
    /// `maxInclusive` alone, so that its lexical space is that of `xs:integer` for the
    /// values in range, any sign and leading zeros included).
    fn is_in_range(self, value: &str) -> bool {
        if !is_integer(value) {
            return false;
        }
        let (min, max) = self.integer_bounds();
        min.is_none_or(|min| compare_integers(value, min).is_ge())
            && max.is_none_or(|max| compare_integers(value, max).is_le())
    }

    /// Whether the length facets (`minLength`, `maxLength`) apply to the type and
    /// this crate implements them: the types derived from `xs:string`, whose length
    /// is counted in characters (Part 2, 4.3.1). They also apply to `xs:anyURI` and
    /// `xs:base64Binary`, which the supported schemas never restrict that way; they
    /// do not apply to the other types here.
    pub(crate) fn has_length(self) -> bool {
        matches!(
            self,
            Self::String
                | Self::NormalizedString
                | Self::Token
                | Self::Language
                | Self::Name
                | Self::NcName
                | Self::Id
                | Self::IdRef
                | Self::Entity
                | Self::NmToken
        )
    }

    /// Whether the length facets apply to the type (Part 2, 4.1.5, Applicable Facets):
    /// the string types, `anyURI`, the binary types, `QName`, `NOTATION` and the list
    /// types (whose length is counted in items).
    pub(crate) fn length_applies(self) -> bool {
        self.has_length()
            || matches!(
                self,
                Self::AnyUri
                    | Self::Base64Binary
                    | Self::HexBinary
                    | Self::QName
                    | Self::Notation
                    | Self::NmTokens
                    | Self::IdRefs
                    | Self::Entities
            )
    }

    /// Whether a value of the type is valid only in a context the lexical check does
    /// not see: an `xs:QName` (its prefix), an `xs:NOTATION` (the notations of the
    /// schema), an `xs:IDREF` (the `xs:ID` of the document), an `xs:ENTITY` (the
    /// unparsed entities of the document type declaration), and their lists.
    pub(crate) fn needs_context(self) -> bool {
        matches!(
            self,
            Self::QName
                | Self::Notation
                | Self::IdRef
                | Self::IdRefs
                | Self::Entity
                | Self::Entities
        )
    }

    /// The value of a lexical form, already whitespace-processed and valid, as a
    /// key: two valid lexical forms of the type denote the same value (Part 2,
    /// 2.2.1: equality in the value space) exactly when their keys are equal.
    /// `None` for the date and time types, whose values (instants with an optional
    /// timezone) this crate does not compare: a value constraint on them is
    /// refused when the schema is loaded.
    pub(crate) fn value_key(self, value: &str) -> Option<std::borrow::Cow<'_, str>> {
        use std::borrow::Cow;
        match self {
            // The string types and anyURI: the value is the string itself (anyURI
            // values are not URI-normalised: Part 2, 3.2.17 defines no equality
            // beyond identity of the strings).
            Self::AnySimpleType
            | Self::String
            | Self::NormalizedString
            | Self::Token
            | Self::Language
            | Self::Name
            | Self::NcName
            | Self::Id
            | Self::IdRef
            | Self::Entity
            | Self::NmToken
            | Self::AnyUri => Some(Cow::Borrowed(value)),
            Self::Decimal
            | Self::Integer
            | Self::NonPositiveInteger
            | Self::NegativeInteger
            | Self::Long
            | Self::Int
            | Self::Short
            | Self::Byte
            | Self::NonNegativeInteger
            | Self::UnsignedLong
            | Self::UnsignedInt
            | Self::UnsignedShort
            | Self::UnsignedByte
            | Self::PositiveInteger => Some(Cow::Owned(canonical_decimal(value))),
            // Two hexadecimal digits per octet, in either case.
            Self::HexBinary => Some(Cow::Owned(value.to_ascii_uppercase())),
            Self::Boolean => Some(Cow::Borrowed(match value {
                "true" | "1" => "true",
                _ => "false",
            })),
            // The lexical space admits one encoding per octet sequence (the bits
            // a padded group leaves unused must be zero), up to the spaces between
            // characters.
            Self::Base64Binary => Some(if value.contains(' ') {
                Cow::Owned(value.replace(' ', ""))
            } else {
                Cow::Borrowed(value)
            }),
            // Instants and durations with an optional timezone, floating-point values
            // (NaN, zeros of either sign), qualified names (resolved in a context) and
            // lists: not compared.
            Self::Date
            | Self::DateTime
            | Self::Time
            | Self::Float
            | Self::Double
            | Self::Duration
            | Self::GYearMonth
            | Self::GYear
            | Self::GMonthDay
            | Self::GDay
            | Self::GMonth
            | Self::QName
            | Self::Notation
            | Self::NmTokens
            | Self::IdRefs
            | Self::Entities => None,
        }
    }

    /// Whether [`Self::value_key`] compares the values of the type.
    pub(crate) fn has_value_key(self) -> bool {
        !matches!(
            self,
            Self::Date
                | Self::DateTime
                | Self::Time
                | Self::Float
                | Self::Double
                | Self::Duration
                | Self::GYearMonth
                | Self::GYear
                | Self::GMonthDay
                | Self::GDay
                | Self::GMonth
                | Self::QName
                | Self::Notation
                | Self::NmTokens
                | Self::IdRefs
                | Self::Entities
        )
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

/// The canonical representation of a valid `xs:decimal` lexical form (Part 2,
/// 3.2.3.2), with no digit limit: no `+`, no leading zeros before the point, no
/// trailing zeros after it, no point for an integral value, and `0` for zero
/// (whatever its sign).
pub(crate) fn canonical_decimal(value: &str) -> String {
    let (negative, unsigned) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let (int, frac) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let int = int.trim_start_matches('0');
    let frac = frac.trim_end_matches('0');
    if int.is_empty() && frac.is_empty() {
        return "0".to_owned();
    }
    let mut out = String::with_capacity(int.len() + frac.len() + 3);
    if negative {
        out.push('-');
    }
    out.push_str(if int.is_empty() { "0" } else { int });
    if !frac.is_empty() {
        out.push('.');
        out.push_str(frac);
    }
    out
}

/// `[\-+]?[0-9]+`
pub(crate) fn is_integer(value: &str) -> bool {
    is_digits(value.strip_prefix(['+', '-']).unwrap_or(value))
}

/// The sign of an `xs:integer` lexical form (zero is never negative) and its digits
/// without leading zeros.
fn integer_parts(s: &str) -> (bool, &str) {
    let (negative, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let digits = digits.trim_start_matches('0');
    (negative && !digits.is_empty(), digits)
}

/// Compares two `xs:integer` lexical forms (any sign, leading zeros) by value.
fn compare_integers(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let ((a_negative, a), (b_negative, b)) = (integer_parts(a), integer_parts(b));
    let magnitude = a.len().cmp(&b.len()).then_with(|| a.cmp(b));
    match (a_negative, b_negative) {
        (false, false) => magnitude,
        (true, true) => magnitude.reverse(),
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
    }
}

/// `xs:float` and `xs:double` (Part 2, 3.2.4.1 and 3.2.5.1): a decimal mantissa
/// optionally followed by `E` or `e` and an integer exponent, or `INF`, `-INF`, `NaN`
/// (XML Schema 1.0 has no `+INF`).
pub(crate) fn is_float(value: &str) -> bool {
    if matches!(value, "INF" | "-INF" | "NaN") {
        return true;
    }
    match value.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => is_decimal(mantissa) && is_integer(exponent),
        None => is_decimal(value),
    }
}

/// `xs:duration` (Part 2, 3.2.6.1): an optional `-`, `P`, then years, months and days
/// (`nY`, `nM`, `nD`, in that order, each optional), then optionally `T` and hours,
/// minutes and seconds (`nH`, `nM`, `nS`, at least one), with at least one component
/// in all; the numbers are unsigned integers of any length, the seconds a decimal
/// (`1.`, `.5` and `1.5` alike, as XML Schema 1.1 writes the grammar).
pub(crate) fn is_duration(value: &str) -> bool {
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let Some(rest) = unsigned.strip_prefix('P') else {
        return false;
    };
    let (date, time) = match rest.split_once('T') {
        Some((date, time)) => (date, Some(time)),
        None => (rest, None),
    };
    // Components `digits designator`, in the order of `designators`, each at most once.
    let components = |s: &str, designators: &[u8], seconds: bool| -> Option<usize> {
        let mut count = 0;
        let mut next = 0;
        let mut rest = s;
        while !rest.is_empty() {
            let end = rest.find(|c: char| !(c.is_ascii_digit() || (seconds && c == '.')))?;
            let (number, tail) = rest.split_at(end);
            let designator = *tail.as_bytes().first()?;
            let position = designators[next..].iter().position(|&d| d == designator)? + next;
            let is_seconds = seconds && designator == b'S';
            let valid = if is_seconds {
                is_decimal(number) && !number.starts_with(['+', '-'])
            } else {
                is_digits(number)
            };
            if !valid {
                return None;
            }
            count += 1;
            next = position + 1;
            rest = &tail[1..];
        }
        Some(count)
    };
    let Some(date_count) = components(date, b"YMD", false) else {
        return false;
    };
    match time {
        None => date_count > 0,
        Some(time) => components(time, b"HMS", true).is_some_and(|n| n > 0),
    }
}

/// A year of a date or `g*` type: an optional `-`, at least four digits, no leading
/// zero when there are more, and not zero (XML Schema 1.0 has no year zero).
fn is_year(s: &str) -> bool {
    let digits = s.strip_prefix('-').unwrap_or(s);
    digits.len() >= 4
        && is_digits(digits)
        && !(digits.len() > 4 && digits.starts_with('0'))
        && !digits.bytes().all(|b| b == b'0')
}

/// A month, `01` to `12`.
fn month(s: &str) -> Option<u32> {
    two_digits(s).filter(|m| (1..=12).contains(m))
}

/// `xs:gYearMonth`: `year-mm`, then an optional timezone.
pub(crate) fn is_g_year_month(value: &str) -> bool {
    split_timezone(value)
        .and_then(|rest| rest.rsplit_once('-'))
        .is_some_and(|(year, mm)| is_year(year) && month(mm).is_some())
}

/// `xs:gYear`: a year, then an optional timezone.
pub(crate) fn is_g_year(value: &str) -> bool {
    split_timezone(value).is_some_and(is_year)
}

/// `xs:gMonthDay`: `--mm-dd`, a day that exists in that month in some year (February
/// 29 included), then an optional timezone.
pub(crate) fn is_g_month_day(value: &str) -> bool {
    let Some((mm, dd)) = split_timezone(value)
        .and_then(|rest| rest.strip_prefix("--"))
        .and_then(|rest| rest.split_once('-'))
    else {
        return false;
    };
    let days = match month(mm) {
        Some(2) => 29,
        Some(4 | 6 | 9 | 11) => 30,
        Some(_) => 31,
        None => return false,
    };
    two_digits(dd).is_some_and(|d| (1..=days).contains(&d))
}

/// `xs:gDay`: `---dd`, 01 to 31, then an optional timezone.
pub(crate) fn is_g_day(value: &str) -> bool {
    split_timezone(value)
        .and_then(|rest| rest.strip_prefix("---"))
        .and_then(two_digits)
        .is_some_and(|d| (1..=31).contains(&d))
}

/// `xs:gMonth`: `--mm` (as corrected by the errata of the Second Edition), then an
/// optional timezone.
pub(crate) fn is_g_month(value: &str) -> bool {
    split_timezone(value)
        .and_then(|rest| rest.strip_prefix("--"))
        .and_then(month)
        .is_some()
}

/// `xs:hexBinary`: pairs of hexadecimal digits, in either case.
pub(crate) fn is_hex_binary(value: &str) -> bool {
    value.len().is_multiple_of(2) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `Nmtoken` of XML 1.0 Fifth Edition: one or more name characters.
pub(crate) fn is_nmtoken(value: &str) -> bool {
    !value.is_empty() && value.chars().all(is_name_char)
}

/// A list type (Part 2, 2.5.1.2): items separated by single spaces (the value is
/// collapsed), each valid, at least one (the built-in list types have a `minLength` of
/// 1).
pub(crate) fn is_list(value: &str, item: fn(&str) -> bool) -> bool {
    !value.is_empty() && value.split(' ').all(item)
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
    fn canonical_decimals() {
        for (value, canonical) in [
            ("0", "0"),
            ("-0", "0"),
            ("+0.000", "0"),
            (".0", "0"),
            ("0.", "0"),
            ("1.50", "1.5"),
            ("+01.500", "1.5"),
            ("-001", "-1"),
            ("-.5", "-0.5"),
            ("5.", "5"),
            ("100", "100"),
            ("10.010", "10.01"),
            (
                "000123456789012345678901234567890.000",
                "123456789012345678901234567890",
            ),
        ] {
            assert_eq!(canonical_decimal(value), canonical, "{value}");
        }
    }

    #[test]
    fn value_keys() {
        let key = |builtin: Builtin, value: &str| {
            builtin.value_key(value).map(std::borrow::Cow::into_owned)
        };
        assert_eq!(key(Builtin::Boolean, "1"), key(Builtin::Boolean, "true"));
        assert_eq!(key(Builtin::Boolean, "0"), key(Builtin::Boolean, "false"));
        assert_ne!(key(Builtin::Boolean, "0"), key(Builtin::Boolean, "true"));
        assert_eq!(key(Builtin::Integer, "+002"), key(Builtin::Integer, "2"));
        assert_eq!(key(Builtin::Decimal, "1.5"), key(Builtin::Decimal, "1.50"));
        assert_eq!(
            key(Builtin::Base64Binary, "Q U J D"),
            key(Builtin::Base64Binary, "QUJD")
        );
        // Strings are compared as they are: whitespace processing comes first.
        assert_ne!(key(Builtin::String, " a"), key(Builtin::String, "a"));
        assert_ne!(key(Builtin::Language, "EN"), key(Builtin::Language, "en"));
        assert_ne!(key(Builtin::AnyUri, "a%20b"), key(Builtin::AnyUri, "a b"));
        for builtin in [Builtin::Date, Builtin::DateTime, Builtin::Time] {
            assert!(!builtin.has_value_key());
            assert_eq!(builtin.value_key("2024-01-01"), None);
        }
        for builtin in Builtin::ALL {
            assert_eq!(
                builtin.has_value_key(),
                builtin.value_key("").is_some(),
                "{builtin:?}"
            );
        }
    }

    #[test]
    fn length_facets_apply_to_the_string_types() {
        let with_length: Vec<Builtin> = Builtin::ALL
            .into_iter()
            .filter(|b| b.has_length())
            .collect();
        assert_eq!(
            with_length,
            [
                Builtin::String,
                Builtin::NormalizedString,
                Builtin::Token,
                Builtin::Language,
                Builtin::Name,
                Builtin::NcName,
                Builtin::Id,
                Builtin::IdRef,
                Builtin::Entity,
                Builtin::NmToken
            ]
        );
        let length_applies: Vec<Builtin> = Builtin::ALL
            .into_iter()
            .filter(|b| b.length_applies() && !b.has_length())
            .collect();
        assert_eq!(
            length_applies,
            [
                Builtin::Base64Binary,
                Builtin::AnyUri,
                Builtin::HexBinary,
                Builtin::QName,
                Builtin::Notation,
                Builtin::NmTokens,
                Builtin::IdRefs,
                Builtin::Entities
            ]
        );
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

    /// The lexical spaces of the built-in types a document can name in `xsi:type`,
    /// after whitespace processing (Part 2 of XML Schema 1.0, Second Edition).
    #[test]
    #[expect(clippy::too_many_lines, reason = "one table of cases")]
    fn other_built_in_types() {
        use Builtin as B;
        let cases: &[(Builtin, &[&str], &[&str])] = &[
            (
                B::Float,
                &[
                    "1", "1.5", "-1.5E3", "1e3", ".5", "5.", "+1", "INF", "-INF", "NaN", "1.5e+3",
                    "1e1000", "-0", "+.5e-1",
                ],
                &[
                    "", "+INF", "inf", "nan", "1E", "1E+", "E1", "1 2", "0x10", ".", "-", "1.2.3",
                    "1d", "1f", ".e1", "1e1.5",
                ],
            ),
            (B::Double, &["1e400", "-0.0e-0", "NaN"], &["+INF", ""]),
            (
                B::Duration,
                &[
                    "P1Y",
                    "P1Y2M3DT4H5M6.7S",
                    "-P1D",
                    "PT1H",
                    "PT1.5S",
                    "PT.5S",
                    "PT1.S",
                    "P0Y",
                    "PT1M",
                    "P1M",
                    "-PT0S",
                    "P99999999999999999999Y",
                ],
                &[
                    "P", "PT", "P1DT", "P1H", "PT1D", "P1.5Y", "+P1D", "P-1D", "1Y", "P1M1Y",
                    "P1Y1Y", "pt1h", "P1D T1H", "P1Y2MT", "P0.5D", "PT1H2H", "P1W", "P1Y-1M",
                    "PT1S1S", "P1DT1.5H",
                ],
            ),
            (
                B::GYearMonth,
                &[
                    "2024-02",
                    "-0001-01",
                    "12024-01",
                    "2024-01Z",
                    "2024-01+14:00",
                    "0001-01",
                ],
                &[
                    "2024-13",
                    "2024-00",
                    "0000-01",
                    "-0000-01",
                    "024-01",
                    "2024-01+14:01",
                    "2024-1",
                    "02024-01",
                    "2024-01-01",
                ],
            ),
            (
                B::GYear,
                &[
                    "2024",
                    "-2024",
                    "12345",
                    "2024Z",
                    "2024-05:00",
                    "2024+00:00",
                    "99999999999999999999",
                ],
                &[
                    "0000",
                    "-0000",
                    "012345",
                    "024",
                    "+2024",
                    "2024-14:30",
                    "2024+1:00",
                ],
            ),
            (
                B::GMonthDay,
                &["--02-29", "--12-31", "--01-01Z", "--01-01+01:00"],
                &[
                    "--02-30", "--04-31", "--13-01", "--01-00", "-01-01", "--1-01",
                ],
            ),
            (
                B::GDay,
                &["---01", "---31", "---01Z", "---15-12:00", "---31+14:00"],
                &["---32", "---00", "---1", "--01"],
            ),
            (
                B::GMonth,
                &["--01", "--12", "--01Z", "--05+10:00"],
                &["--13", "--00", "--01--", "--1", "-01"],
            ),
            (
                B::HexBinary,
                &["", "0F", "0f", "0FB7"],
                &["F", "0G", "0F B7", "0Fb"],
            ),
            (
                B::NonPositiveInteger,
                &["0", "-0", "+0", "-1", "-00001"],
                &["1", "+1", "", "-1.0"],
            ),
            (
                B::NegativeInteger,
                &["-1", "-99999999999999999999999"],
                &["0", "-0", "1"],
            ),
            (
                B::Long,
                &[
                    "9223372036854775807",
                    "-9223372036854775808",
                    "+1",
                    "00009223372036854775807",
                ],
                &["9223372036854775808", "-9223372036854775809", "1.0", "-"],
            ),
            (
                B::Int,
                &["2147483647", "-2147483648", "+0"],
                &["2147483648", "-2147483649"],
            ),
            (B::Short, &["32767", "-32768"], &["32768", "-32769"]),
            (B::Byte, &["127", "-128", "+127", "0127"], &["128", "-129"]),
            (
                B::NonNegativeInteger,
                &["0", "-0", "+1", "99999999999999999999999"],
                &["-1"],
            ),
            (
                B::UnsignedLong,
                &["18446744073709551615", "0", "-0", "+1"],
                &["18446744073709551616", "-1"],
            ),
            (B::UnsignedInt, &["4294967295", "-0", "+5"], &["4294967296"]),
            (B::UnsignedShort, &["65535"], &["65536"]),
            (B::UnsignedByte, &["255", "+255", "-0"], &["256", "-1"]),
            (B::PositiveInteger, &["1", "+1", "01"], &["0", "-0", "-1"]),
            (
                B::NmToken,
                &["abc", "1a", "-.:_", "\u{e9}"],
                &["a b", "", "a@b"],
            ),
            (B::NmTokens, &["a b c", "a"], &["", "a @"]),
            (B::IdRef, &["a1"], &["1a", "a:b", ""]),
            (B::IdRefs, &["a b", "a"], &["", "a 1"]),
            (B::Entity, &["a"], &["1", ""]),
            (B::Entities, &["a b"], &[""]),
            (
                B::QName,
                &["a", "xs:a", "xmlns:a"],
                &["1a", ":a", "a:", "xs:a:b", ""],
            ),
            (B::Notation, &["a", "xs:a"], &["1"]),
        ];
        for (builtin, valid, invalid) in cases {
            for value in *valid {
                assert!(builtin.is_valid(value), "{builtin:?} {value:?}");
            }
            for value in *invalid {
                assert!(!builtin.is_valid(value), "{builtin:?} {value:?}");
            }
        }
        // Whitespace is collapsed first.
        assert!(B::Duration.is_valid_raw(" P1D "));
        assert!(B::NmTokens.is_valid_raw("  a \t b "));
        assert!(!B::HexBinary.is_valid_raw("0F B7"));
        // Every type derives from anySimpleType, through its base types.
        for builtin in Builtin::ALL {
            let mut current = builtin;
            let mut steps = 0;
            while let Some(base) = current.base() {
                assert!(
                    Builtin::ALL.iter().position(|b| *b == base)
                        < Builtin::ALL.iter().position(|b| *b == current),
                    "{builtin:?}: base {base:?} after it in ALL"
                );
                current = base;
                steps += 1;
            }
            assert_eq!(current, B::AnySimpleType, "{builtin:?}");
            assert!(steps <= 7, "{builtin:?}");
        }
        assert_eq!(compare_integers("+0010", "10"), std::cmp::Ordering::Equal);
        assert_eq!(compare_integers("-0", "0"), std::cmp::Ordering::Equal);
        assert_eq!(compare_integers("-11", "-2"), std::cmp::Ordering::Less);
        assert_eq!(compare_integers("9", "10"), std::cmp::Ordering::Less);
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
        fn decimal_keys_are_numeric_equality(
            a in "[+-]?[0-9]{0,6}(\\.[0-9]{0,6})?",
            b in "[+-]?[0-9]{0,6}(\\.[0-9]{0,6})?",
        ) {
            proptest::prop_assume!(is_decimal(&a) && is_decimal(&b));
            // Reference: the value scaled by 10^6, exact in an i64.
            let scaled = |value: &str| -> i64 {
                let negative = value.starts_with('-');
                let unsigned = value.trim_start_matches(['+', '-']);
                let (int, frac) = unsigned.split_once('.').unwrap_or((unsigned, ""));
                let int: i64 = if int.is_empty() { 0 } else { int.parse().unwrap() };
                let frac: i64 = format!("{frac:0<6}").parse().unwrap();
                let magnitude = int * 1_000_000 + frac;
                if negative { -magnitude } else { magnitude }
            };
            proptest::prop_assert_eq!(
                canonical_decimal(&a) == canonical_decimal(&b),
                scaled(&a) == scaled(&b),
                "{} {}", a, b
            );
            // The canonical form is itself a valid decimal with the same value.
            proptest::prop_assert!(is_decimal(&canonical_decimal(&a)));
            proptest::prop_assert_eq!(scaled(&canonical_decimal(&a)), scaled(&a));
        }

        #[test]
        fn decimal_keys_ignore_zeros_and_plus(
            negative: bool,
            int in "[0-9]{0,4}",
            frac in "[0-9]{0,4}",
            lead in 0_usize..3,
            trail in 0_usize..3,
            plus: bool,
        ) {
            proptest::prop_assume!(!int.is_empty() || !frac.is_empty());
            let sign = if negative { "-" } else { "" };
            let a = format!("{sign}{int}.{frac}");
            let other_sign = if negative { "-" } else if plus { "+" } else { "" };
            let b = format!("{other_sign}{}{int}.{frac}{}", "0".repeat(lead), "0".repeat(trail));
            proptest::prop_assert_eq!(canonical_decimal(&a), canonical_decimal(&b));
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
