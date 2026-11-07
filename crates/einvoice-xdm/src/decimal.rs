//! `xs:decimal`: arbitrary precision, exact arithmetic.
//!
//! A value is an unscaled integer and a scale, `unscaled × 10^-scale`, like the
//! `java.math.BigDecimal` that Saxon-HE 9.9 uses. The scale is part of the observable
//! behaviour: Saxon computes the precision of a decimal division from the scales of
//! its operands (see [`Decimal::checked_div`]). Saxon strips the trailing zeros of
//! every decimal it creates, except when it converts a small `xs:integer` to
//! `xs:decimal`, which keeps scale 0 (see `Integer::to_decimal`). This type keeps the
//! same representation; equality and ordering are numeric and ignore it.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

use num_bigint::{BigInt, Sign};
use num_integer::Integer as _;
use num_traits::{Signed, ToPrimitive, Zero};

use crate::error::{ErrorCode, Result, err};

/// The minimum number of fractional digits of a decimal division (Saxon's
/// `BigDecimalValue.DIVIDE_PRECISION`).
const DIVIDE_PRECISION: i64 = 18;

/// An `xs:decimal` value: `unscaled × 10^-scale`.
#[derive(Debug, Clone)]
pub struct Decimal {
    unscaled: BigInt,
    scale: i64,
}

/// Rounding modes of `java.math.RoundingMode` used by Saxon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rounding {
    /// Nearest, ties away from zero.
    HalfUp,
    /// Nearest, ties towards zero.
    HalfDown,
    /// Towards negative infinity.
    Floor,
    /// Towards positive infinity.
    Ceiling,
}

pub(crate) fn pow10(exponent: u64) -> BigInt {
    num_traits::pow(
        BigInt::from(10_u8),
        usize::try_from(exponent).unwrap_or(usize::MAX),
    )
}

/// `value × 10^exponent` for `exponent >= 0`.
fn scale_up(value: &BigInt, exponent: i64) -> BigInt {
    value * pow10(exponent.unsigned_abs())
}

impl Decimal {
    /// A decimal exactly as given, without stripping trailing zeros.
    #[must_use]
    pub fn from_parts(unscaled: BigInt, scale: i64) -> Self {
        Self { unscaled, scale }
    }

    /// A decimal with its trailing zeros stripped (`BigDecimal.stripTrailingZeros`):
    /// the form of every decimal Saxon creates, except integers converted to decimal.
    #[must_use]
    pub fn stripped(unscaled: BigInt, scale: i64) -> Self {
        if unscaled.is_zero() {
            return Self { unscaled, scale: 0 };
        }
        let mut unscaled = unscaled;
        let mut scale = scale;
        let ten = BigInt::from(10_u8);
        loop {
            let (quotient, remainder) = unscaled.div_rem(&ten);
            if !remainder.is_zero() {
                break;
            }
            unscaled = quotient;
            scale -= 1;
        }
        Self { unscaled, scale }
    }

    /// Zero.
    #[must_use]
    pub fn zero() -> Self {
        Self {
            unscaled: BigInt::zero(),
            scale: 0,
        }
    }

    /// The unscaled value.
    #[must_use]
    pub const fn unscaled(&self) -> &BigInt {
        &self.unscaled
    }

    /// The scale: the value is `unscaled × 10^-scale`.
    #[must_use]
    pub const fn scale(&self) -> i64 {
        self.scale
    }

    /// -1, 0 or 1.
    #[must_use]
    pub fn signum(&self) -> i32 {
        match self.unscaled.sign() {
            Sign::Minus => -1,
            Sign::NoSign => 0,
            Sign::Plus => 1,
        }
    }

    /// Whether the value is zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.unscaled.is_zero()
    }

    /// Parses the lexical form of `xs:decimal` the way Saxon does
    /// (`BigDecimalValue.parse`): optional leading and trailing whitespace
    /// (space, tab, CR, LF), an optional sign, ASCII digits with at most one decimal
    /// point and at least one digit. No exponent. The result has no trailing zeros.
    ///
    /// # Errors
    ///
    /// `FORG0001` when the string is not a decimal.
    pub fn parse(text: &str) -> Result<Self> {
        // States: 0 initial whitespace, 1 after a sign or in the integer part,
        // 3 after the decimal point, 5 in final whitespace.
        let mut state = 0_u8;
        let mut negative = false;
        let mut digits = String::with_capacity(text.len());
        let mut scale = 0_i64;
        let mut found_digit = false;
        for c in text.chars() {
            match c {
                ' ' | '\t' | '\r' | '\n' => {
                    if state != 0 {
                        state = 5;
                    }
                }
                '+' | '-' => {
                    if state != 0 {
                        return bad_decimal(text, "unexpected sign");
                    }
                    state = 1;
                    negative = c == '-';
                }
                '0'..='9' => {
                    if state == 0 {
                        state = 1;
                    } else if state >= 3 {
                        scale += 1;
                    }
                    if state == 5 {
                        return bad_decimal(text, "contains embedded whitespace");
                    }
                    digits.push(c);
                    found_digit = true;
                }
                '.' => {
                    if state == 5 {
                        return bad_decimal(text, "contains embedded whitespace");
                    }
                    if state >= 3 {
                        return bad_decimal(text, "more than one decimal point");
                    }
                    state = 3;
                }
                _ => return bad_decimal(text, &format!("invalid character '{c}'")),
            }
        }
        if !found_digit {
            return bad_decimal(text, "no digits in value");
        }
        let magnitude = digits
            .parse::<BigInt>()
            .or_else(|_| bad_decimal(text, "no digits in value"))?;
        let unscaled = if negative { -magnitude } else { magnitude };
        Ok(Self::stripped(unscaled, scale))
    }

    /// The exact value of a finite double (`new BigDecimal(double)`), stripped.
    ///
    /// # Errors
    ///
    /// `FOCA0002` for NaN and the infinities.
    pub fn from_f64(value: f64) -> Result<Self> {
        if !value.is_finite() {
            return err(
                ErrorCode::Foca0002,
                format!(
                    "Cannot convert double {} to decimal",
                    crate::double::format_double(value)
                ),
            );
        }
        let (mantissa, exponent) = decompose(value);
        let unscaled = if exponent >= 0 {
            BigInt::from(mantissa) << exponent.unsigned_abs()
        } else {
            BigInt::from(mantissa)
                * num_traits::pow(BigInt::from(5_u8), exponent.unsigned_abs() as usize)
        };
        let unscaled = if value.is_sign_negative() {
            -unscaled
        } else {
            unscaled
        };
        let scale = if exponent >= 0 {
            0
        } else {
            i64::from(exponent.unsigned_abs())
        };
        Ok(Self::stripped(unscaled, scale))
    }

    /// The nearest double, ties to even (`BigDecimal.doubleValue`). Overflows to an
    /// infinity, underflows to zero.
    #[must_use]
    pub fn to_f64(&self) -> f64 {
        let text = format!("{}e{}", self.unscaled, -i128::from(self.scale));
        // The string is a valid Rust float literal by construction; the fallback is
        // unreachable.
        text.parse::<f64>().unwrap_or(f64::NAN)
    }

    /// The integer part, truncated towards zero (`BigDecimal.toBigInteger`).
    #[must_use]
    pub fn trunc_to_bigint(&self) -> BigInt {
        if self.scale <= 0 {
            scale_up(&self.unscaled, -self.scale)
        } else {
            // BigInt division truncates towards zero.
            &self.unscaled / pow10(self.scale.unsigned_abs())
        }
    }

    /// Both values with the same scale (the larger of the two): their unscaled values
    /// and the common scale.
    fn aligned(&self, other: &Self) -> (BigInt, BigInt, i64) {
        let scale = self.scale.max(other.scale);
        (
            scale_up(&self.unscaled, scale - self.scale),
            scale_up(&other.unscaled, scale - other.scale),
            scale,
        )
    }

    /// Exact sum, stripped.
    #[must_use]
    pub fn add(&self, other: &Self) -> Self {
        let (a, b, scale) = self.aligned(other);
        Self::stripped(a + b, scale)
    }

    /// Exact difference, stripped.
    #[must_use]
    pub fn sub(&self, other: &Self) -> Self {
        let (a, b, scale) = self.aligned(other);
        Self::stripped(a - b, scale)
    }

    /// Exact product, stripped.
    #[must_use]
    pub fn mul(&self, other: &Self) -> Self {
        Self::stripped(&self.unscaled * &other.unscaled, self.scale + other.scale)
    }

    /// Decimal division as Saxon performs it (`Calculator.decimalDivide`): the
    /// quotient is rounded, ties towards zero, to
    /// `max(18, self.scale - other.scale + 18)` fractional digits, then stripped.
    /// The scales are those of the operands as represented (see the module doc).
    ///
    /// # Errors
    ///
    /// `FOAR0001` when `other` is zero.
    pub fn checked_div(&self, other: &Self) -> Result<Self> {
        if other.is_zero() {
            return err(ErrorCode::Foar0001, "Decimal divide by zero");
        }
        let scale = DIVIDE_PRECISION.max(self.scale - other.scale + DIVIDE_PRECISION);
        // self / other × 10^scale = a × 10^(scale - sa + sb) / b
        let exponent = scale - self.scale + other.scale;
        let (numerator, denominator) = if exponent >= 0 {
            (scale_up(&self.unscaled, exponent), other.unscaled.clone())
        } else {
            (self.unscaled.clone(), scale_up(&other.unscaled, -exponent))
        };
        let quotient = round_quotient(&numerator, &denominator, Rounding::HalfDown);
        Ok(Self::stripped(quotient, scale))
    }

    /// The remainder of the division truncated towards zero (`BigDecimal.remainder`):
    /// its sign is the sign of `self`. Stripped.
    ///
    /// # Errors
    ///
    /// `FOAR0001` when `other` is zero.
    pub fn checked_rem(&self, other: &Self) -> Result<Self> {
        if other.is_zero() {
            return err(ErrorCode::Foar0001, "Decimal modulo zero");
        }
        let (a, b, scale) = self.aligned(other);
        Ok(Self::stripped(a % b, scale))
    }

    /// The quotient truncated towards zero (`BigDecimal.divideToIntegralValue`).
    ///
    /// # Errors
    ///
    /// `FOAR0001` when `other` is zero.
    pub fn checked_div_trunc(&self, other: &Self) -> Result<BigInt> {
        if other.is_zero() {
            return err(ErrorCode::Foar0001, "Integer division by zero");
        }
        let (a, b, _) = self.aligned(other);
        Ok(a / b)
    }

    /// Negation, stripped.
    #[must_use]
    pub fn neg(&self) -> Self {
        Self::stripped(-&self.unscaled, self.scale)
    }

    /// `fn:abs`: the value itself when positive, else its negation (stripped).
    #[must_use]
    pub fn abs(&self) -> Self {
        if self.signum() > 0 {
            self.clone()
        } else {
            self.neg()
        }
    }

    /// `fn:floor`, stripped.
    #[must_use]
    pub fn floor(&self) -> Self {
        self.set_scale(0, Rounding::Floor)
    }

    /// `fn:ceiling`, stripped.
    #[must_use]
    pub fn ceiling(&self) -> Self {
        self.set_scale(0, Rounding::Ceiling)
    }

    /// `fn:round`: nearest integer, ties towards positive infinity. A value without
    /// fractional digits is returned unchanged; otherwise the result is stripped.
    #[must_use]
    pub fn round(&self) -> Self {
        if self.scale <= 0 {
            return self.clone();
        }
        match self.signum() {
            -1 => self.set_scale(0, Rounding::HalfDown),
            1 => self.set_scale(0, Rounding::HalfUp),
            _ => self.clone(),
        }
    }

    /// `BigDecimal.setScale(scale, mode)` followed by stripping.
    pub(crate) fn set_scale(&self, scale: i64, mode: Rounding) -> Self {
        if scale >= self.scale {
            return Self::stripped(scale_up(&self.unscaled, scale - self.scale), scale);
        }
        let divisor = pow10((self.scale - scale).unsigned_abs());
        Self::stripped(round_quotient(&self.unscaled, &divisor, mode), scale)
    }

    /// Whether the value has no fractional part.
    #[must_use]
    pub fn is_whole(&self) -> bool {
        if self.scale <= 0 {
            return true;
        }
        (&self.unscaled % pow10(self.scale.unsigned_abs())).is_zero()
    }
}

/// `numerator / denominator` rounded to an integer with the given mode.
fn round_quotient(numerator: &BigInt, denominator: &BigInt, mode: Rounding) -> BigInt {
    // BigInt division truncates towards zero; the remainder has the sign of the numerator.
    let (quotient, remainder) = numerator.div_rem(denominator);
    if remainder.is_zero() {
        return quotient;
    }
    let negative = (numerator.sign() == Sign::Minus) != (denominator.sign() == Sign::Minus);
    let away = if negative {
        &quotient - 1_u8
    } else {
        &quotient + 1_u8
    };
    let twice = remainder.abs() * 2_u8;
    let half = twice.cmp(&denominator.abs());
    let round_away = match mode {
        Rounding::HalfUp => half != Ordering::Less,
        Rounding::HalfDown => half == Ordering::Greater,
        Rounding::Floor => negative,
        Rounding::Ceiling => !negative,
    };
    if round_away { away } else { quotient }
}

/// A finite double as `mantissa × 2^exponent`, the mantissa non-negative.
pub(crate) fn decompose(value: f64) -> (u64, i32) {
    let bits = value.to_bits();
    let raw_exponent = i32::try_from((bits >> 52) & 0x7ff).unwrap_or(0);
    let fraction = bits & 0x000f_ffff_ffff_ffff;
    if raw_exponent == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1_u64 << 52), raw_exponent - 1075)
    }
}

fn bad_decimal<T>(text: &str, reason: &str) -> Result<T> {
    err(
        ErrorCode::Forg0001,
        format!(
            "Cannot convert string \"{}\" to xs:decimal: {reason}",
            text.trim_matches([' ', '\t', '\r', '\n'])
        ),
    )
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Decimal {}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Ordering {
        let (a, b, _) = self.aligned(other);
        a.cmp(&b)
    }
}

impl Hash for Decimal {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let canonical = Self::stripped(self.unscaled.clone(), self.scale);
        canonical.unscaled.hash(state);
        canonical.scale.hash(state);
    }
}

impl fmt::Display for Decimal {
    /// The XPath string value (Saxon's `BigDecimalValue.decimalToString`): no
    /// exponent, the digits of the representation, `0` for zero.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let negative = self.unscaled.sign() == Sign::Minus;
        let digits = self.unscaled.magnitude().to_string();
        if self.scale == 0 {
            if negative {
                f.write_str("-")?;
            }
            return f.write_str(&digits);
        }
        if digits == "0" {
            return f.write_str("0");
        }
        if negative {
            f.write_str("-")?;
        }
        if self.scale < 0 {
            f.write_str(&digits)?;
            for _ in 0..self.scale.unsigned_abs() {
                f.write_str("0")?;
            }
            return Ok(());
        }
        let len = digits.len();
        let scale = usize::try_from(self.scale).unwrap_or(usize::MAX);
        if scale >= len {
            f.write_str("0.")?;
            for _ in len..scale {
                f.write_str("0")?;
            }
            f.write_str(&digits)
        } else {
            let (integer, fraction) = digits.split_at(len - scale);
            write!(f, "{integer}.{fraction}")
        }
    }
}

impl From<i64> for Decimal {
    /// The stripped decimal with this integer value.
    fn from(value: i64) -> Self {
        Self::stripped(BigInt::from(value), 0)
    }
}

impl ToPrimitive for Decimal {
    fn to_i64(&self) -> Option<i64> {
        if self.is_whole() {
            self.trunc_to_bigint().to_i64()
        } else {
            None
        }
    }

    fn to_u64(&self) -> Option<u64> {
        if self.is_whole() {
            self.trunc_to_bigint().to_u64()
        } else {
            None
        }
    }

    fn to_f64(&self) -> Option<f64> {
        Some(Self::to_f64(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        Decimal::parse(text).unwrap()
    }

    #[test]
    fn parse_strips_and_prints() {
        assert_eq!(d(" 1.50 ").to_string(), "1.5");
        assert_eq!(d("100").to_string(), "100");
        assert_eq!(d("100").scale(), -2);
        assert_eq!(d("-0.0").to_string(), "0");
        assert_eq!(d("-.5").to_string(), "-0.5");
        assert_eq!(d("+1.").to_string(), "1");
        for bad in [
            "1e3", "", " ", ".", "+", "1 .5", "- 1", "1,5", "NaN", "1..2", "1-",
        ] {
            assert_eq!(
                Decimal::parse(bad).unwrap_err().code(),
                ErrorCode::Forg0001,
                "{bad}"
            );
        }
    }

    #[test]
    fn division_precision() {
        assert_eq!(
            d("1").checked_div(&d("3")).unwrap().to_string(),
            "0.333333333333333333"
        );
        assert_eq!(
            d("1.1").checked_div(&d("300")).unwrap().to_string(),
            "0.003666666666666666667"
        );
        let integer_300 = Decimal::from_parts(BigInt::from(300), 0);
        assert_eq!(
            d("1.1").checked_div(&integer_300).unwrap().to_string(),
            "0.0036666666666666667"
        );
    }

    #[test]
    fn from_f64_is_exact() {
        assert_eq!(
            Decimal::from_f64(0.1).unwrap().to_string(),
            "0.1000000000000000055511151231257827021181583404541015625"
        );
        assert_eq!(Decimal::from_f64(-0.0).unwrap().to_string(), "0");
        assert_eq!(
            Decimal::from_f64(1e20).unwrap().to_string(),
            "100000000000000000000"
        );
    }

    #[test]
    fn rounding() {
        assert_eq!(d("2.5").round().to_string(), "3");
        assert_eq!(d("-2.5").round().to_string(), "-2");
        assert_eq!(d("-2.51").round().to_string(), "-3");
        assert_eq!(d("-2.5").floor().to_string(), "-3");
        assert_eq!(d("-2.5").ceiling().to_string(), "-2");
    }
}
