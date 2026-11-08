//! Template priorities: exact decimals, and the default priority of a pattern alternative.

use std::fmt;

use einvoice_xpath::{
    KindTest, NameOrWildcard, NameTest, NodeTest, PathStep, PatternAlternative, PatternAnchor,
};

/// Fractional digits kept exactly.
const SCALE_DIGITS: u32 = 18;
const SCALE: i128 = 10_i128.pow(SCALE_DIGITS);

/// A template priority: an `xs:decimal` (XSLT 2.0 section 6.4), held exactly as a count of
/// 10⁻¹⁸ units. Priorities compare as numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Priority(i128);

impl Priority {
    /// `-0.5`
    pub const MINUS_HALF: Self = Self(-SCALE / 2);
    /// `-0.25`
    pub const MINUS_QUARTER: Self = Self(-SCALE / 4);
    /// `0`
    pub const ZERO: Self = Self(0);
    /// `0.5`
    pub const HALF: Self = Self(SCALE / 2);

    /// Parses the lexical form of an `xs:decimal` (surrounding whitespace allowed, optional
    /// sign, digits with at most one `.`). `None` when the text is not a decimal, has more
    /// than 18 fractional digits or is too large.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim_matches(|c| matches!(c, ' ' | '\t' | '\r' | '\n'));
        let (negative, digits) = match text.as_bytes().first() {
            Some(b'-') => (true, text.get(1..)?),
            Some(b'+') => (false, text.get(1..)?),
            _ => (false, text),
        };
        let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
        let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
        if (whole.is_empty() && fraction.is_empty()) || !all_digits(whole) || !all_digits(fraction)
        {
            return None;
        }
        let fraction = fraction.trim_end_matches('0');
        let fraction_len = u32::try_from(fraction.len())
            .ok()
            .filter(|&n| n <= SCALE_DIGITS)?;
        let mut units: i128 = 0;
        for b in whole.bytes().chain(fraction.bytes()) {
            units = units.checked_mul(10)?.checked_add(i128::from(b - b'0'))?;
        }
        units = units.checked_mul(10_i128.pow(SCALE_DIGITS - fraction_len))?;
        Some(Self(if negative { -units } else { units }))
    }
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let magnitude = self.0.unsigned_abs();
        let scale = SCALE.unsigned_abs();
        let (whole, fraction) = (magnitude / scale, magnitude % scale);
        if fraction == 0 {
            return write!(f, "{sign}{whole}");
        }
        let width = usize::try_from(SCALE_DIGITS).map_err(|_| fmt::Error)?;
        let digits = format!("{fraction:0width$}");
        write!(f, "{sign}{whole}.{}", digits.trim_end_matches('0'))
    }
}

/// The default priority of one alternative of a pattern (XSLT 2.0 section 6.4; the XSLT
/// 3.0 forms `root()` and paths with a parenthesized step fall under its last rule):
///
/// - `/`: −0.5;
/// - a single step without predicate, on the child or attribute axis, whose test is a
///   `QName` or `element(QName)`/`attribute(QName)`: 0;
/// - the same with `prefix:*` or `*:local`: −0.25;
/// - the same with any other node test (`*`, `node()`, `text()`, `element()`, ...): −0.5;
/// - anything else: 0.5.
#[must_use]
pub fn default_priority(alternative: &PatternAlternative) -> Priority {
    let PatternAlternative::Path(path) = alternative else {
        return match alternative {
            PatternAlternative::DocumentRoot => Priority::MINUS_HALF,
            _ => Priority::HALF,
        };
    };
    let PathStep::Axis(first) = &path.first else {
        return Priority::HALF;
    };
    if path.anchor != PatternAnchor::Relative
        || !path.rest.is_empty()
        || !first.predicates.is_empty()
    {
        return Priority::HALF;
    }
    match &first.test {
        NodeTest::Name(NameTest::Name(_))
        | NodeTest::Kind(
            KindTest::Element(Some(NameOrWildcard::Name(_)))
            | KindTest::Attribute(Some(NameOrWildcard::Name(_))),
        ) => Priority::ZERO,
        NodeTest::Name(NameTest::AnyLocal { .. } | NameTest::AnyNamespace { .. }) => {
            Priority::MINUS_QUARTER
        }
        NodeTest::Name(NameTest::Any) | NodeTest::Kind(_) => Priority::MINUS_HALF,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use einvoice_xpath::{NamespaceMap, parse_pattern};

    #[test]
    fn parses_and_prints_decimals_exactly() {
        for (text, printed) in [
            ("0", "0"),
            ("1065", "1065"),
            ("-0.5", "-0.5"),
            (" +2.50 ", "2.5"),
            ("-10", "-10"),
            (".25", "0.25"),
            ("3.", "3"),
            ("0.000000000000000001", "0.000000000000000001"),
            ("-0", "0"),
        ] {
            assert_eq!(
                Priority::parse(text).map(|p| p.to_string()).as_deref(),
                Some(printed),
                "{text}"
            );
        }
        for bad in [
            "",
            "-",
            ".",
            "1e3",
            "1.2.3",
            "a",
            "0.0000000000000000001",
            "1 2",
            "--1",
        ] {
            assert_eq!(Priority::parse(bad), None, "{bad}");
        }
        assert!(Priority::parse("-0.5") < Priority::parse("-0.25"));
        assert!(Priority::parse("2") > Priority::parse("1.999999"));
        assert_eq!(Priority::parse("-0.5"), Some(Priority::MINUS_HALF));
    }

    #[test]
    fn default_priorities() {
        let ns = NamespaceMap::new().bind("p", "urn:p");
        for (pattern, expected) in [
            ("/", "-0.5"),
            ("p:a", "0"),
            ("@a", "0"),
            ("attribute::p:a", "0"),
            ("element(p:a)", "0"),
            ("attribute(a)", "0"),
            ("p:*", "-0.25"),
            ("@*:a", "-0.25"),
            ("*", "-0.5"),
            ("@*", "-0.5"),
            ("node()", "-0.5"),
            ("text()", "-0.5"),
            ("comment()", "-0.5"),
            ("processing-instruction()", "-0.5"),
            ("element()", "-0.5"),
            ("element(*)", "-0.5"),
            ("p:a[1]", "0.5"),
            ("p:a/p:b", "0.5"),
            ("/p:a", "0.5"),
            ("//p:a", "0.5"),
            ("root()", "0.5"),
            ("(/p:a | /p:b)[1]", "0.5"),
            ("(p:a | p:b)/p:c", "0.5"),
            ("(p:a)/(p:b | p:c)/text()", "0.5"),
            ("(p:a)", "0"),
            ("(*)", "-0.5"),
        ] {
            let parsed = parse_pattern(pattern, &ns).unwrap();
            let [alternative] = parsed.alternatives.as_slice() else {
                panic!("{pattern}")
            };
            assert_eq!(
                default_priority(alternative).to_string(),
                expected,
                "{pattern}"
            );
        }
    }
}
