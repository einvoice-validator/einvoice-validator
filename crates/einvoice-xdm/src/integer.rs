//! `xs:integer`: arbitrary precision.
//!
//! Saxon-HE 12.10 holds an integer either as a 64-bit `Int64Value` or as a
//! `BigIntegerValue`, and the two do not always behave alike: integer division
//! (`div`) and the conversion to `xs:decimal` depend on it (see [`Integer::div`] and
//! [`Integer::to_decimal`]). Saxon normalises the result of most operations to the
//! 64-bit form when it fits, but keeps the big form after parsing a long lexical
//! value (17 characters or more), after `abs` or negation of a big value. This type
//! records the same representation; equality and ordering are numeric.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};

use crate::decimal::Decimal;
use crate::error::{ErrorCode, Result, err};
use crate::tables::java_decimal_digit;

/// `2^63`: Java's `(double) Long.MAX_VALUE` (and minus `(double) Long.MIN_VALUE`).
const LONG_LIMIT: f64 = 9_223_372_036_854_775_808.0;

/// An `xs:integer` value.
#[derive(Debug, Clone)]
pub struct Integer {
    value: BigInt,
    /// Saxon's `BigIntegerValue` representation (always the case outside the i64 range).
    big: bool,
}

/// The result of an integer division: Saxon returns an `xs:integer` when a small
/// non-negative division is exact, an `xs:decimal` otherwise.
#[derive(Debug, Clone)]
pub enum Quotient {
    /// An exact quotient kept as an integer.
    Integer(Integer),
    /// A decimal quotient.
    Decimal(Decimal),
}

impl Integer {
    /// The integer with Saxon's normal representation (`IntegerValue.makeIntegerValue`).
    #[must_use]
    pub fn from_bigint(value: BigInt) -> Self {
        let big = value.to_i64().is_none();
        Self { value, big }
    }

    /// The integer in Saxon's big representation, whatever its magnitude.
    #[must_use]
    pub fn big(value: BigInt) -> Self {
        Self { value, big: true }
    }

    /// The value.
    #[must_use]
    pub const fn value(&self) -> &BigInt {
        &self.value
    }

    /// Whether Saxon would hold this value as a `BigIntegerValue`.
    #[must_use]
    pub const fn is_big_representation(&self) -> bool {
        self.big
    }

    /// The value as an `i64` when it fits.
    #[must_use]
    pub fn to_i64(&self) -> Option<i64> {
        self.value.to_i64()
    }

    /// -1, 0 or 1.
    #[must_use]
    pub fn signum(&self) -> i32 {
        match self.value.sign() {
            num_bigint::Sign::Minus => -1,
            num_bigint::Sign::NoSign => 0,
            num_bigint::Sign::Plus => 1,
        }
    }

    /// Parses the lexical form of `xs:integer` the way Saxon does
    /// (`IntegerValue.stringToInteger`). After trimming characters up to U+0020:
    /// up to 16 characters, an optional sign and ASCII digits; from 17 characters on,
    /// Java's `BigInteger(String)` after removing one leading `+`, which also accepts
    /// a second sign and the decimal digits of any script in the Basic Multilingual
    /// Plane. A long value keeps the big representation.
    ///
    /// # Errors
    ///
    /// `FORG0001` when the string is not an integer.
    pub fn parse(text: &str) -> Result<Self> {
        let trimmed = text.trim_matches(|c: char| c <= '\u{20}');
        if trimmed.is_empty() {
            return bad_integer(text);
        }
        if trimmed.chars().map(char::len_utf16).sum::<usize>() <= 16 {
            let (negative, digits) = match trimmed.as_bytes().first() {
                Some(b'+') => (false, &trimmed[1..]),
                Some(b'-') => (true, &trimmed[1..]),
                _ => (false, trimmed),
            };
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return bad_integer(text);
            }
            let magnitude: i64 = digits.parse().or_else(|_| bad_integer(text))?;
            let value = if negative { -magnitude } else { magnitude };
            return Ok(Self::from(value));
        }
        let t = crate::double::trim_xml_whitespace(text);
        let t = t.strip_prefix('+').unwrap_or(t);
        let parsed = java_big_integer(t)
            .ok_or_else(|| crate::error::XdmError::new(ErrorCode::Forg0001, message(text)))?;
        // Saxon uses Long.parseLong (64-bit form) when what remains is shorter than
        // 16 characters, which only happens when the two trimmings differ.
        if t.chars().map(char::len_utf16).sum::<usize>() < 16 {
            return parsed
                .to_i64()
                .map(Self::from)
                .ok_or_else(|| crate::error::XdmError::new(ErrorCode::Forg0001, message(text)));
        }
        Ok(Self::big(parsed))
    }

    /// The conversion of a double (Saxon-HE 12.10's `IntegerValue.fromDouble`, used by
    /// the cast to `xs:integer` and by `idiv` of doubles): truncation towards zero.
    /// Within the `long` range this is Java's saturating `(long)` cast, so `2^63`
    /// becomes `9223372036854775807`. Beyond it, where every double is whole, the
    /// exact value of the double (its mantissa shifted by its exponent), in the big
    /// representation.
    ///
    /// # Errors
    ///
    /// `FOCA0002` for NaN and the infinities.
    pub fn from_f64(value: f64) -> Result<Self> {
        if value.is_nan() {
            return err(
                ErrorCode::Foca0002,
                "Cannot convert double NaN to an integer",
            );
        }
        if value.is_infinite() {
            return err(
                ErrorCode::Foca0002,
                "Cannot convert double INF to an integer",
            );
        }
        if value > LONG_LIMIT || value < -LONG_LIMIT {
            // Saxon 12.10 (bug 7070) takes the 52 stored bits of the mantissa with
            // the implicit leading 1 and shifts them by the unbiased exponent, which
            // is at least 63 here, so the shift is to the left. (Saxon 9.9 went
            // through the decimal digits of Double.toString instead.)
            let bits = value.to_bits();
            let mantissa = (bits & 0x000f_ffff_ffff_ffff) | 0x0010_0000_0000_0000;
            let exponent = ((bits >> 52) & 0x7ff) - 1023;
            let magnitude = BigInt::from(mantissa) << (exponent - 52);
            let value = if value < 0.0 { -magnitude } else { magnitude };
            return Ok(Self::big(value));
        }
        // Rust's `as` saturates and truncates exactly like Java's (long) cast.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Java's saturating (long) cast is the behaviour reproduced"
        )]
        let truncated = value as i64;
        Ok(Self::from(truncated))
    }

    /// The nearest double, ties to even.
    #[must_use]
    pub fn to_f64(&self) -> f64 {
        if let Some(small) = self.value.to_i64() {
            #[expect(
                clippy::cast_precision_loss,
                reason = "Java's (double) long conversion, round to nearest even"
            )]
            return small as f64;
        }
        self.value.to_string().parse::<f64>().unwrap_or(f64::NAN)
    }

    /// The conversion to `xs:decimal` when the integer's type is only known at run
    /// time (`Converter.IntegerToDecimal`): a 64-bit value keeps scale 0
    /// (`new BigDecimalValue(long)`), a big one is stripped.
    #[must_use]
    pub fn to_decimal(&self) -> Decimal {
        if self.big {
            Decimal::stripped(self.value.clone(), 0)
        } else {
            Decimal::from_parts(self.value.clone(), 0)
        }
    }

    /// The conversion to `xs:decimal` of an integer statically known to be one
    /// (Saxon-HE 12.10's `Converter.UpCastingConverter`, `copyAsSubType`): the
    /// trailing zeros are always stripped, so `300` becomes `3E+2`, of scale -2.
    #[must_use]
    pub fn to_decimal_upcast(&self) -> Decimal {
        Decimal::stripped(self.value.clone(), 0)
    }

    /// The decimal operand Saxon uses when an integer meets a decimal in arithmetic
    /// (`getDecimalValue`): scale 0, whatever the representation.
    #[must_use]
    pub fn as_decimal_operand(&self) -> Decimal {
        Decimal::from_parts(self.value.clone(), 0)
    }

    /// Exact sum.
    #[must_use]
    pub fn add(&self, other: &Self) -> Self {
        Self::from_bigint(&self.value + &other.value)
    }

    /// Exact difference.
    #[must_use]
    pub fn sub(&self, other: &Self) -> Self {
        Self::from_bigint(&self.value - &other.value)
    }

    /// Exact product.
    #[must_use]
    pub fn mul(&self, other: &Self) -> Self {
        Self::from_bigint(&self.value * &other.value)
    }

    /// `div` of two integers as Saxon computes it (`Int64Value.div`,
    /// `BigIntegerValue.div`). When both are 64-bit and in `[0, 2^31)` (Saxon's
    /// `isLong()` is true for every negative value), an exact quotient stays an
    /// `xs:integer` and an inexact one is a decimal division of scale-0 operands.
    /// Otherwise both operands become stripped decimals before the decimal division,
    /// which changes the number of fractional digits (see `Decimal::checked_div`).
    ///
    /// # Errors
    ///
    /// `FOAR0001` when `other` is zero.
    pub fn div(&self, other: &Self) -> Result<Quotient> {
        if other.value.is_zero() {
            return err(ErrorCode::Foar0001, "Integer division by zero");
        }
        let small = |i: &Self| -> Option<i64> {
            if i.big {
                return None;
            }
            let v = i.value.to_i64()?;
            let top = v >> 31;
            (top == 0 || top == 0x1_ffff_ffff).then_some(v)
        };
        if let (Some(a), Some(b)) = (small(self), small(other)) {
            if a % b == 0 {
                return Ok(Quotient::Integer(Self::from(a / b)));
            }
            return Decimal::from_parts(BigInt::from(a), 0)
                .checked_div(&Decimal::from_parts(BigInt::from(b), 0))
                .map(Quotient::Decimal);
        }
        Decimal::stripped(self.value.clone(), 0)
            .checked_div(&Decimal::stripped(other.value.clone(), 0))
            .map(Quotient::Decimal)
    }

    /// `idiv`: the quotient truncated towards zero.
    ///
    /// # Errors
    ///
    /// `FOAR0001` when `other` is zero.
    pub fn idiv(&self, other: &Self) -> Result<Self> {
        if other.value.is_zero() {
            return err(ErrorCode::Foar0001, "Integer division by zero");
        }
        Ok(Self::from_bigint(&self.value / &other.value))
    }

    /// `mod`: the remainder of the truncated division, with the sign of `self`.
    ///
    /// # Errors
    ///
    /// `FOAR0001` when `other` is zero.
    pub fn rem(&self, other: &Self) -> Result<Self> {
        if other.value.is_zero() {
            return err(ErrorCode::Foar0001, "Integer modulo zero");
        }
        Ok(Self::from_bigint(&self.value % &other.value))
    }

    /// Negation. A big value stays big; the 64-bit form normalises.
    ///
    /// Saxon-HE 12.10, like 9.9, recurses without end (stack overflow) when negating
    /// the 64-bit value `-2^63`; this returns `2^63`.
    #[must_use]
    pub fn neg(&self) -> Self {
        if self.big {
            Self::big(-&self.value)
        } else {
            Self::from_bigint(-&self.value)
        }
    }

    /// `fn:abs`. The 64-bit `-2^63` and big negative values give a big value.
    #[must_use]
    pub fn abs(&self) -> Self {
        if !self.value.is_negative() {
            return self.clone();
        }
        if self.big || self.value.to_i64() == Some(i64::MIN) {
            Self::big(self.value.abs())
        } else {
            Self::from_bigint(self.value.abs())
        }
    }
}

/// Java's `new BigInteger(text)`: an optional sign at the start and no other sign,
/// then one or more characters that `Character.digit(c, 10)` accepts.
fn java_big_integer(text: &str) -> Option<BigInt> {
    let (negative, digits) = match text.chars().next() {
        Some('-') => (true, &text[1..]),
        Some('+') => (false, &text[1..]),
        _ => (false, text),
    };
    if digits.is_empty() {
        return None;
    }
    let mut value = BigInt::zero();
    for c in digits.chars() {
        let d = java_decimal_digit(c)?;
        value = value * 10_u8 + d;
    }
    Some(if negative { -value } else { value })
}

fn message(text: &str) -> String {
    format!("Cannot convert string \"{text}\" to an integer")
}

fn bad_integer<T>(text: &str) -> Result<T> {
    err(ErrorCode::Forg0001, message(text))
}

impl From<i64> for Integer {
    fn from(value: i64) -> Self {
        Self {
            value: BigInt::from(value),
            big: false,
        }
    }
}

impl PartialEq for Integer {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl Eq for Integer {}

impl PartialOrd for Integer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Integer {
    fn cmp(&self, other: &Self) -> Ordering {
        self.value.cmp(&other.value)
    }
}

impl Hash for Integer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl fmt::Display for Integer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.value, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_short_and_long() {
        assert_eq!(Integer::parse(" 007 ").unwrap().to_string(), "7");
        assert!(!Integer::parse("-5").unwrap().is_big_representation());
        let long = Integer::parse("00000000000000300").unwrap();
        assert_eq!(long.to_string(), "300");
        assert!(long.is_big_representation());
        assert_eq!(
            Integer::parse("+-1234567890123456").unwrap().to_string(),
            "-1234567890123456"
        );
        assert!(Integer::parse("+-12345678901234").is_err());
        assert!(Integer::parse("1.0").is_err());
        assert!(Integer::parse("").is_err());
        assert!(Integer::parse("+").is_err());
    }

    #[test]
    fn division_representation() {
        let int = |v: i64| Integer::from(v);
        assert!(matches!(int(6).div(&int(2)).unwrap(), Quotient::Integer(_)));
        assert!(matches!(
            int(-6).div(&int(2)).unwrap(),
            Quotient::Decimal(_)
        ));
        let Quotient::Decimal(q) = int(-1).div(&int(300)).unwrap() else {
            panic!("decimal expected");
        };
        assert_eq!(q.to_string(), "-0.00333333333333333333");
    }

    #[test]
    fn from_double() {
        assert_eq!(
            Integer::from_f64(2_f64.powi(63)).unwrap().to_string(),
            "9223372036854775807"
        );
        assert_eq!(
            Integer::from_f64(1e23).unwrap().to_string(),
            "99999999999999991611392"
        );
        assert_eq!(
            Integer::from_f64(-2_f64.powi(64)).unwrap().to_string(),
            "-18446744073709551616"
        );
        assert!(Integer::from_f64(1e23).unwrap().is_big_representation());
        assert_eq!(Integer::from_f64(-2.7).unwrap().to_string(), "-2");
    }
}
