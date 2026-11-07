//! `xs:double`: lexical parsing and string conversion as Saxon-HE 9.9 performs them.
//!
//! * Parsing follows `StringToDouble11` (the XSD 1.1 rules Saxon applies by default),
//!   including its hand-written fast path for short strings.
//! * String conversion follows `FloatingPointConverter.appendDouble`: the
//!   Steele & White free-format algorithm (FPP²), decimal notation for
//!   `1e-6 <= |v| < 1e6`, scientific notation (`1.0E6`) otherwise. Subnormal values
//!   are handed by Saxon to `Double.toString`, whose JDK 19+ algorithm is reproduced
//!   by [`java_shortest`].

use std::cmp::Ordering;

use num_bigint::{BigInt, BigUint};
use num_integer::Integer as _;
use num_traits::{One, Zero};

use crate::decimal::decompose;
use crate::error::{ErrorCode, Result};

/// XML whitespace as trimmed by Saxon's `Whitespace.trimWhitespace`.
pub(crate) const fn is_xml_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

pub(crate) fn trim_xml_whitespace(text: &str) -> &str {
    text.trim_matches(is_xml_whitespace)
}

fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// Parses the lexical form of `xs:double` (also the conversion used by `fn:number`,
/// by arithmetic on `xs:untypedAtomic` values and by comparisons with numbers).
///
/// # Errors
///
/// `FORG0001` when the string is not a double.
pub fn parse_double(text: &str) -> Result<f64> {
    parse_double_inner(text).ok_or_else(|| {
        crate::error::XdmError::new(
            ErrorCode::Forg0001,
            format!("Cannot convert string \"{text}\" to double"),
        )
    })
}

fn parse_double_inner(text: &str) -> Option<f64> {
    let mut contains_disallowed = false;
    let mut contains_whitespace = false;
    if utf16_len(text) < 9 {
        // Saxon's fast path for short unsigned decimal numbers.
        let mut use_library = false;
        let mut number = 0_u32;
        let mut dot: Option<usize> = None;
        let mut last_digit: Option<usize> = None;
        let mut only_space_allowed = false;
        for (index, c) in text.chars().enumerate() {
            match c {
                ' ' | '\n' | '\t' | '\r' => {
                    contains_whitespace = true;
                    if last_digit.is_some() {
                        only_space_allowed = true;
                    }
                }
                '0'..='9' => {
                    if only_space_allowed {
                        return None;
                    }
                    last_digit = Some(index);
                    number = number * 10 + c.to_digit(10).unwrap_or(0);
                }
                '.' => {
                    if only_space_allowed || dot.is_some() {
                        return None;
                    }
                    dot = Some(index);
                }
                'x' | 'X' | 'f' | 'F' | 'd' | 'D' | 'n' | 'N' => {
                    contains_disallowed = true;
                    use_library = true;
                    break;
                }
                _ => use_library = true,
            }
        }
        if !use_library {
            let last_digit = last_digit?;
            return Some(match dot {
                Some(dot) if dot < last_digit => {
                    // Positions, not digits: whitespace between the point and the
                    // first digit counts (Saxon reads ". 5" as 0.05).
                    let after_point = i32::try_from(last_digit - dot).unwrap_or(0);
                    f64::from(number) / 10_f64.powi(after_point)
                }
                _ => f64::from(number),
            });
        }
    } else {
        for c in text.chars() {
            match c {
                ' ' | '\n' | '\t' | '\r' => contains_whitespace = true,
                '0'..='9' | '.' | 'e' | 'E' | '+' | '-' => {}
                _ => {
                    contains_disallowed = true;
                    break;
                }
            }
        }
    }
    let n = if contains_whitespace {
        trim_xml_whitespace(text)
    } else {
        text
    };
    match n {
        "INF" | "+INF" => Some(f64::INFINITY),
        "-INF" => Some(f64::NEG_INFINITY),
        "NaN" => Some(f64::NAN),
        _ if contains_disallowed => None,
        _ => java_parse_double(n),
    }
}

/// `Double.parseDouble` restricted to what can reach it: an optional sign, digits
/// with at most one decimal point (at least one digit), an optional exponent.
fn java_parse_double(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let mut index = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        index += 1;
    }
    let mut mantissa_digits = 0;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
        mantissa_digits += 1;
    }
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
            mantissa_digits += 1;
        }
    }
    if mantissa_digits == 0 {
        return None;
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == start {
            return None;
        }
    }
    if index != bytes.len() {
        return None;
    }
    // Rust's parser is correctly rounded, like Java's, and accepts this grammar.
    text.parse::<f64>().ok()
}

/// The XPath string value of a double, as Saxon writes it.
#[must_use]
pub fn format_double(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value == f64::INFINITY {
        return "INF".to_owned();
    }
    if value == f64::NEG_INFINITY {
        return "-INF".to_owned();
    }
    if value == 0.0 {
        return if value.is_sign_negative() { "-0" } else { "0" }.to_owned();
    }
    let magnitude = value.abs();
    let sign = if value < 0.0 { "-" } else { "" };
    // Saxon writes these four values literally.
    if magnitude.to_bits() == f64::MAX.to_bits() {
        return format!("{sign}1.7976931348623157E308");
    }
    if magnitude.to_bits() == 1 {
        return format!("{sign}4.9E-324");
    }
    let (mantissa, _) = decompose(magnitude);
    if mantissa < (1_u64 << 52) {
        // Subnormal: Saxon delegates to Double.toString.
        let (digits, exponent) = java_shortest(magnitude);
        return format!("{sign}{}", scientific(&digits, exponent));
    }
    let (digits, k) = free_format_digits(magnitude);
    if (1e-6..1e6).contains(&magnitude) {
        format!("{sign}{}", plain(&digits, k))
    } else {
        format!("{sign}{}", scientific(&digits, k - 1))
    }
}

/// `d.ddd` followed by `E` and the exponent; `d.0` when there is a single digit.
fn scientific(digits: &[u8], exponent: i64) -> String {
    let mut out = String::with_capacity(digits.len() + 8);
    let mut iter = digits.iter();
    if let Some(first) = iter.next() {
        out.push(char::from(b'0' + first));
    }
    out.push('.');
    if digits.len() > 1 {
        for d in iter {
            out.push(char::from(b'0' + d));
        }
    } else {
        out.push('0');
    }
    out.push('E');
    out.push_str(&exponent.to_string());
    out
}

/// Decimal notation of `0.d1d2... × 10^k`, without trailing fractional zeros and
/// without a decimal point for integers.
fn plain(digits: &[u8], k: i64) -> String {
    let mut out = String::with_capacity(digits.len() + 8);
    if k <= 0 {
        out.push_str("0.");
        for _ in 0..k.unsigned_abs() {
            out.push('0');
        }
        for d in digits {
            out.push(char::from(b'0' + d));
        }
        return out;
    }
    let k = usize::try_from(k).unwrap_or(usize::MAX);
    for (index, d) in digits.iter().enumerate() {
        if index == k {
            out.push('.');
        }
        out.push(char::from(b'0' + d));
    }
    for _ in digits.len()..k {
        out.push('0');
    }
    out
}

/// The digits of a positive normal double by Steele & White's free-format
/// algorithm as Saxon implements it (`FloatingPointConverter.fppfpp*`), and `k` such
/// that the value is about `0.d1d2... × 10^k`.
///
/// Saxon's fix-up for the narrower gap below a power of two compares the significand
/// with `2^51`, which a normalised significand (`>= 2^52`) never equals: the gaps are
/// always treated as symmetric, and so they are here.
fn free_format_digits(value: f64) -> (Vec<u8>, i64) {
    let (significand, exponent) = decompose(value);
    // Saxon's e and p: value = significand × 2^(e - p) with p = 52.
    let shift = i64::from(exponent);
    let (mut remainder, mut denominator, mut m_minus) = if shift >= 0 {
        (
            BigUint::from(significand) << shift.unsigned_abs(),
            BigUint::one(),
            BigUint::one() << shift.unsigned_abs(),
        )
    } else {
        (
            BigUint::from(significand),
            BigUint::one() << shift.unsigned_abs(),
            BigUint::one(),
        )
    };
    let mut m_plus = m_minus.clone();
    let ten = BigUint::from(10_u8);
    let nine = BigUint::from(9_u8);
    let mut position = 0_i64;
    while remainder < (&denominator + &nine) / &ten {
        position -= 1;
        remainder *= &ten;
        m_minus *= &ten;
        m_plus *= &ten;
    }
    while (&remainder << 1_u8) + &m_plus >= (&denominator << 1_u8) {
        denominator *= &ten;
        position += 1;
    }
    let k0 = position;
    let mut digits = Vec::with_capacity(20);
    loop {
        let r10 = &remainder * &ten;
        let (quotient, rest) = r10.div_rem(&denominator);
        remainder = rest;
        m_minus *= &ten;
        m_plus *= &ten;
        let twice_r = &remainder << 1_u8;
        let low = twice_r < m_minus;
        // Saxon: 2R > 2S - M+, in signed arithmetic (true whenever M+ > 2S).
        let high = &twice_r + &m_plus > (&denominator << 1_u8);
        let mut digit = u8::try_from(&quotient).unwrap_or(0);
        if low || high {
            if high && (!low || twice_r > denominator) {
                digit += 1;
            }
            digits.push(digit);
            break;
        }
        digits.push(digit);
    }
    (digits, k0)
}

/// The decimal `Double.toString` of JDK 19 and later chooses for a positive finite
/// double: among the decimals that round to it, those of minimal length (or of length
/// 1 or 2 when the minimum is 1), the closest one, ties to an even last digit.
/// Returned as significant digits (no trailing zeros) and the decimal exponent of
/// the first digit.
pub(crate) fn java_shortest(value: f64) -> (Vec<u8>, i64) {
    let interval = RoundingInterval::new(value);
    // Start above the largest exponent that can hold a candidate, then walk down:
    // value < 2^bits, so log10(value) < bits × 0.30103.
    let (m, q) = decompose(value);
    let bits = i64::from(q) + 64 - i64::from(m.leading_zeros());
    let mut t = (bits * 30_103).div_euclid(100_000) + 2;
    let (mut t_found, mut low, mut high) = loop {
        if let Some((low, high)) = interval.candidates(t) {
            break (t, low, high);
        }
        t -= 1;
    };
    let mut best = interval.closest(t_found, &low, &high);
    if digit_count(&best) == 1 {
        t_found -= 1;
        if let Some((l, h)) = interval.candidates(t_found) {
            low = l;
            high = h;
            best = interval.closest(t_found, &low, &high);
        }
    }
    // Strip trailing zeros of the chosen significand.
    let ten = BigInt::from(10_u8);
    let mut exponent = t_found;
    while !best.is_zero() && (&best % &ten).is_zero() {
        best /= &ten;
        exponent += 1;
    }
    let text = best.to_string();
    let digits: Vec<u8> = text.bytes().map(|b| b - b'0').collect();
    let first = exponent + i64::try_from(digits.len()).unwrap_or(0) - 1;
    (digits, first)
}

fn digit_count(value: &BigInt) -> usize {
    let ten = BigInt::from(10_u8);
    let mut v = value.clone();
    while !v.is_zero() && (&v % &ten).is_zero() {
        v /= &ten;
    }
    v.to_string().len()
}

/// The set of reals that round to a double, as exact rationals over a power of two.
struct RoundingInterval {
    /// value, lower and upper bound × 2^-q2 (all integers).
    value: BigInt,
    lower: BigInt,
    upper: BigInt,
    /// The common binary exponent: actual = stored × 2^q2.
    q2: i64,
    inclusive: bool,
}

impl RoundingInterval {
    fn new(value: f64) -> Self {
        let (m, q) = decompose(value);
        let normal_power_of_two = m == 1_u64 << 52 && q > -1074;
        // Scale by 4 so that the half gaps are integers.
        let v = BigInt::from(m) * 4_u8;
        let lower_gap = if normal_power_of_two { 1_u8 } else { 2_u8 };
        Self {
            lower: &v - lower_gap,
            upper: &v + 2_u8,
            value: v,
            q2: i64::from(q) - 2,
            inclusive: m % 2 == 0,
        }
    }

    /// `x × 2^q2 / 10^t` as a fraction.
    fn ratio(&self, x: &BigInt, t: i64) -> (BigInt, BigInt) {
        let mut numerator = x.clone();
        let mut denominator = BigInt::one();
        if self.q2 >= 0 {
            numerator <<= self.q2.unsigned_abs();
        } else {
            denominator <<= self.q2.unsigned_abs();
        }
        if t >= 0 {
            denominator *= crate::decimal::pow10(t.unsigned_abs());
        } else {
            numerator *= crate::decimal::pow10(t.unsigned_abs());
        }
        (numerator, denominator)
    }

    /// The smallest and largest integer `c` with `c × 10^t` in the interval.
    fn candidates(&self, t: i64) -> Option<(BigInt, BigInt)> {
        let (ln, ld) = self.ratio(&self.lower, t);
        let (un, ud) = self.ratio(&self.upper, t);
        let mut low = ln.div_ceil(&ld);
        if !self.inclusive && &low * &ld == ln {
            low += 1_u8;
        }
        let mut high = un.div_floor(&ud);
        if !self.inclusive && &high * &ud == un {
            high -= 1_u8;
        }
        (low <= high && high > BigInt::zero()).then_some((low, high))
    }

    /// The candidate in `[low, high]` closest to the value, ties to even.
    fn closest(&self, t: i64, low: &BigInt, high: &BigInt) -> BigInt {
        let (vn, vd) = self.ratio(&self.value, t);
        let floor = vn.div_floor(&vd);
        let ceil = &floor + 1_u8;
        // Compare (v - floor) with (ceil - v), i.e. 2v with floor + ceil.
        let twice = &vn * 2_u8;
        let mid = (&floor + &ceil) * &vd;
        let nearest = match twice.cmp(&mid) {
            Ordering::Less => floor,
            Ordering::Greater => ceil,
            Ordering::Equal => {
                if floor.is_even() {
                    floor
                } else {
                    ceil
                }
            }
        };
        if &nearest < low {
            low.clone()
        } else if &nearest > high {
            high.clone()
        } else {
            nearest
        }
    }
}

/// Java's `Double.toString` for a positive finite double in the scientific form it
/// uses below `1e-3` and from `1e7`: `d.dddE±n`.
#[cfg(test)]
fn java_to_string_scientific(value: f64) -> String {
    let (digits, exponent) = java_shortest(value);
    scientific(&digits, exponent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_saxon() {
        let cases: &[(f64, &str)] = &[
            (1e6, "1.0E6"),
            (999_999.0, "999999"),
            (1e-6, "0.000001"),
            (1e-7, "1.0E-7"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1e21, "1.0E21"),
            (1_234_567.0, "1.234567E6"),
            (100.1, "100.1"),
            (0.5, "0.5"),
            (100.0, "100"),
            (-2.5, "-2.5"),
            (1.0 / 3.0, "0.3333333333333333"),
            (f64::MAX, "1.7976931348623157E308"),
            (4.9e-324, "4.9E-324"),
        ];
        for (value, expected) in cases {
            assert_eq!(format_double(*value), *expected, "{value:e}");
        }
    }

    #[test]
    fn java_subnormal_digits() {
        assert_eq!(java_to_string_scientific(1e-310), "1.0E-310");
        assert_eq!(java_to_string_scientific(4.9e-324), "4.9E-324");
        assert_eq!(java_to_string_scientific(1e23), "1.0E23");
        assert_eq!(java_to_string_scientific(2e-323), "2.0E-323");
    }

    #[test]
    fn parses_like_saxon() {
        assert_eq!(parse_double(". 5").unwrap(), 0.05);
        assert_eq!(parse_double(" 1.5 ").unwrap(), 1.5);
        assert_eq!(parse_double("+INF").unwrap(), f64::INFINITY);
        assert!(parse_double("NaN").unwrap().is_nan());
        assert!(parse_double("INF ").is_err());
        assert!(parse_double(" INF").is_ok());
        assert!(parse_double("1e").is_err());
        assert!(parse_double("Infinity").is_err());
        assert!(parse_double("0x10").is_err());
        assert!(parse_double("1d").is_err());
        assert!(parse_double("").is_err());
        assert_eq!(parse_double("-0").unwrap().to_bits(), (-0.0_f64).to_bits());
    }
}
