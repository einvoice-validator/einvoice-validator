//! Property tests: exact decimal arithmetic against exact fractions ([`BigRational`]
//! below), ordering, string round trips, Saxon's division rule, and double string
//! round trips.

#![cfg(test)]

use std::cmp::Ordering;
use std::ops::{Add, Div, Mul, Neg, Sub};

use einvoice_bigint::BigInt;
use einvoice_xdm::{Decimal, Integer, format_double, parse_double};
use proptest::prelude::*;

/// An exact fraction of two big integers, the reference the decimals are checked
/// against: always in lowest terms with a positive denominator, so that equal values
/// have equal fields.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BigRational {
    numerator: BigInt,
    denominator: BigInt,
}

impl BigRational {
    /// `numerator / denominator`, reduced; the denominator must not be zero.
    fn new(numerator: BigInt, denominator: BigInt) -> Self {
        assert!(!denominator.is_zero(), "zero denominator");
        let gcd = numerator.gcd(&denominator);
        let reduce = |value: BigInt| value.checked_div(&gcd).unwrap();
        let (mut numerator, mut denominator) = (reduce(numerator), reduce(denominator));
        if denominator.is_negative() {
            numerator = -numerator;
            denominator = -denominator;
        }
        Self {
            numerator,
            denominator,
        }
    }

    fn from_integer(value: BigInt) -> Self {
        Self {
            numerator: value,
            denominator: BigInt::one(),
        }
    }

    fn abs(&self) -> Self {
        Self {
            numerator: self.numerator.abs(),
            denominator: self.denominator.clone(),
        }
    }
}

impl Ord for BigRational {
    /// The denominators are positive: compare the cross products.
    fn cmp(&self, other: &Self) -> Ordering {
        (&self.numerator * &other.denominator).cmp(&(&other.numerator * &self.denominator))
    }
}

impl PartialOrd for BigRational {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Add for BigRational {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(
            self.numerator * &other.denominator + other.numerator * &self.denominator,
            self.denominator * other.denominator,
        )
    }
}

impl Neg for BigRational {
    type Output = Self;
    fn neg(self) -> Self {
        Self {
            numerator: -self.numerator,
            denominator: self.denominator,
        }
    }
}

impl Sub for BigRational {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        self + -other
    }
}

impl Mul for BigRational {
    type Output = Self;
    fn mul(self, other: Self) -> Self {
        Self::new(
            self.numerator * other.numerator,
            self.denominator * other.denominator,
        )
    }
}

impl Div for BigRational {
    type Output = Self;
    fn div(self, other: Self) -> Self {
        Self::new(
            self.numerator * other.denominator,
            self.denominator * other.numerator,
        )
    }
}

#[test]
fn rational_reference_is_exact() {
    let r = |n: i64, d: i64| BigRational::new(BigInt::from(n), BigInt::from(d));
    assert_eq!(r(2, -4), r(-1, 2));
    assert_eq!(r(-2, -4).numerator, BigInt::from(1));
    assert_eq!(r(0, -7), BigRational::from_integer(BigInt::zero()));
    assert_eq!(r(0, -7).denominator, BigInt::one());
    assert_eq!(r(1, 3) + r(1, 6), r(1, 2));
    assert_eq!(r(1, 3) - r(1, 2), r(-1, 6));
    assert_eq!(r(2, 3) * r(-9, 4), r(-3, 2));
    assert_eq!(r(2, 3) / r(-4, 9), r(-3, 2));
    assert_eq!(r(-5, 7).abs(), r(5, 7));
    assert!(r(-1, 3) < r(-1, 4) && r(1, 4) < r(1, 3) && r(2, 6) == r(1, 3));
    assert_eq!(r(7, 3).cmp(&r(14, 6)), Ordering::Equal);
}

/// `10^exponent`; the exponents of these tests are small.
fn pow10(exponent: u64) -> BigInt {
    BigInt::from(10).pow(u32::try_from(exponent).unwrap_or(u32::MAX))
}

fn rational(d: &Decimal) -> BigRational {
    let scale = d.scale();
    if scale >= 0 {
        BigRational::new(d.unscaled().clone(), pow10(scale.unsigned_abs()))
    } else {
        BigRational::from_integer(d.unscaled() * pow10(scale.unsigned_abs()))
    }
}

/// Decimals with up to 38 significant digits and scales from -12 to 30.
fn decimal() -> impl Strategy<Value = Decimal> {
    (any::<i128>(), -12_i64..30)
        .prop_map(|(unscaled, scale)| Decimal::stripped(BigInt::from(unscaled), scale))
}

/// The lexical forms `xs:decimal` accepts.
fn lexical() -> impl Strategy<Value = String> {
    "[ \t\r\n]{0,2}[+-]?([0-9]{1,25}(\\.[0-9]{0,25})?|\\.[0-9]{1,25})[ \t\r\n]{0,2}"
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn add_sub_mul_are_exact(a in decimal(), b in decimal()) {
        prop_assert_eq!(rational(&a.add(&b)), rational(&a) + rational(&b));
        prop_assert_eq!(rational(&a.sub(&b)), rational(&a) - rational(&b));
        prop_assert_eq!(rational(&a.mul(&b)), rational(&a) * rational(&b));
    }

    #[test]
    fn results_are_stripped(a in decimal(), b in decimal()) {
        for r in [a.add(&b), a.sub(&b), a.mul(&b)] {
            let ten = BigInt::from(10);
            prop_assert!(r.is_zero() || !r.unscaled().checked_rem(&ten).unwrap().is_zero());
            prop_assert!(!r.is_zero() || r.scale() == 0);
        }
    }

    #[test]
    fn ordering_is_numeric(a in decimal(), b in decimal()) {
        prop_assert_eq!(a.cmp(&b), rational(&a).cmp(&rational(&b)));
        prop_assert_eq!(a == b, rational(&a) == rational(&b));
        let unstripped = Decimal::from_parts(a.unscaled() * BigInt::from(1000), a.scale() + 3);
        prop_assert_eq!(unstripped.cmp(&a), Ordering::Equal);
    }

    #[test]
    fn string_round_trip(a in decimal()) {
        let text = a.to_string();
        prop_assert!(!text.contains(['e', 'E']));
        let back = Decimal::parse(&text).unwrap();
        prop_assert_eq!(&back, &a);
        prop_assert_eq!(back.to_string(), text);
    }

    #[test]
    fn parse_is_exact(text in lexical()) {
        let d = Decimal::parse(&text).unwrap();
        let trimmed = text.trim_matches([' ', '\t', '\r', '\n']);
        let (negative, body) = match trimmed.as_bytes()[0] {
            b'-' => (true, &trimmed[1..]),
            b'+' => (false, &trimmed[1..]),
            _ => (false, trimmed),
        };
        let (int, frac) = body.split_once('.').unwrap_or((body, ""));
        let digits: BigInt = format!("0{int}{frac}").parse().unwrap();
        let value = BigRational::new(digits, pow10(frac.len() as u64));
        let value = if negative { -value } else { value };
        prop_assert_eq!(rational(&d), value);
    }

    /// Saxon: the quotient rounded half towards zero to
    /// max(18, scale(a) - scale(b) + 18) fractional digits.
    #[test]
    fn division_rule(a in decimal(), b in decimal()) {
        prop_assume!(!b.is_zero());
        let q = a.checked_div(&b).unwrap();
        let scale = 18.max(a.scale() - b.scale() + 18);
        prop_assert!(q.scale() <= scale);
        let exact = rational(&a) / rational(&b);
        let unit = BigRational::new(BigInt::from(1), pow10(scale.unsigned_abs()));
        let error = (rational(&q) - exact.clone()).abs();
        let half = unit / BigRational::from_integer(BigInt::from(2));
        prop_assert!(error <= half);
        if error == half {
            // Ties go towards zero.
            prop_assert!(rational(&q).abs() < exact.abs());
        }
    }

    #[test]
    fn remainder_and_truncated_division(a in decimal(), b in decimal()) {
        prop_assume!(!b.is_zero());
        let q = a.checked_div_trunc(&b).unwrap();
        let r = a.checked_rem(&b).unwrap();
        prop_assert_eq!(
            rational(&a),
            BigRational::from_integer(q) * rational(&b) + rational(&r)
        );
        prop_assert!(rational(&r).abs() < rational(&b).abs());
        prop_assert!(r.is_zero() || r.signum() == a.signum());
    }

    #[test]
    fn rounding_is_half_up_towards_positive_infinity(a in decimal()) {
        let r = a.round();
        prop_assert!(r.is_whole());
        let diff = rational(&a) - rational(&r);
        let half = BigRational::new(BigInt::from(1), BigInt::from(2));
        prop_assert!(diff < half && diff >= -half.clone());
        prop_assert!(rational(&a.floor()) <= rational(&a));
        prop_assert!(rational(&a.ceiling()) >= rational(&a));
        prop_assert!(a.floor().is_whole() && a.ceiling().is_whole());
    }

    #[test]
    fn decimal_from_double_is_exact(bits in any::<u64>()) {
        let x = f64::from_bits(bits);
        prop_assume!(x.is_finite());
        let d = Decimal::from_f64(x).unwrap();
        prop_assert_eq!(d.to_f64().to_bits(), if x == 0.0 { 0.0_f64.to_bits() } else { x.to_bits() });
    }

    /// Saxon's free-format output reads back as the same double, except for
    /// powers of two: Saxon's algorithm treats the gap below a power of two, which is
    /// half the gap above, as equal to it, and may print a decimal that reads back as
    /// the next double towards zero (255 of the 2046 normal powers of two; the
    /// oracle checks all of them against Saxon).
    #[test]
    fn double_string_round_trip(bits in any::<u64>()) {
        let x = f64::from_bits(bits);
        prop_assume!(x.is_finite());
        let text = format_double(x);
        let back = parse_double(&text).unwrap();
        let power_of_two = bits.trailing_zeros() >= 52 && bits & 0x7ff0_0000_0000_0000 != 0;
        if power_of_two {
            let back = back.abs().to_bits();
            prop_assert!(
                back == x.abs().to_bits() || back == x.abs().next_down().to_bits(),
                "{}",
                text
            );
        } else {
            prop_assert_eq!(back.to_bits(), x.to_bits(), "{}", text);
        }
    }

    #[test]
    fn powers_of_two_read_back_within_one_step(k in -1022_i32..=1023) {
        let x = 2_f64.powi(k);
        let back = parse_double(&format_double(x)).unwrap();
        prop_assert!(back.to_bits() == x.to_bits() || back.to_bits() == x.next_down().to_bits());
    }

    #[test]
    fn integer_arithmetic_is_exact(a in any::<i128>(), b in any::<i128>()) {
        let (x, y) = (Integer::from_bigint(BigInt::from(a)), Integer::from_bigint(BigInt::from(b)));
        let sum = x.add(&y);
        let product = x.mul(&y);
        prop_assert_eq!(sum.value(), &(BigInt::from(a) + BigInt::from(b)));
        prop_assert_eq!(product.value(), &(BigInt::from(a) * BigInt::from(b)));
        if b != 0 {
            let q = x.idiv(&y).unwrap();
            let r = x.rem(&y).unwrap();
            prop_assert_eq!(q.value() * BigInt::from(b) + r.value(), BigInt::from(a));
        }
        let parsed = Integer::parse(&x.to_string()).unwrap();
        prop_assert_eq!(parsed.value(), x.value());
    }
}
