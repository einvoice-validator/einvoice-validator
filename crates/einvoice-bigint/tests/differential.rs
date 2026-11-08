//! Every operation against `num-bigint` 0.5 (with `num-integer` and `num-traits`) on
//! random values: small and large, positive and negative, edge limbs, and values of
//! thousands of digits.

#![cfg(test)]

mod common;

use std::num::NonZeroU64;

use common::{check, limb, nonzero, num_from_limbs, our_sign, ours_of, pair, value};
use einvoice_bigint::BigInt;
use num_bigint::BigInt as NumBigInt;
use num_integer::Integer as _;
use num_traits::{FromPrimitive as _, Pow as _, Signed as _, ToPrimitive as _, Zero as _};
use proptest::prelude::*;

/// Up to 64 limbs (about 1200 digits) in the common tests.
const MAX: usize = 64;

/// `cases` cases, or the number in `PROPTEST_CASES` (for longer runs).
fn config(cases: u32) -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(cases);
    ProptestConfig::with_cases(cases)
}

proptest! {
    #![proptest_config(config(2048))]

    #[test]
    fn addition(a in value(MAX), b in value(MAX)) {
        let expected = &a.theirs + &b.theirs;
        check(&(&a.ours + &b.ours), &expected)?;
        check(&(a.ours.clone() + &b.ours), &expected)?;
        check(&(&a.ours + b.ours.clone()), &expected)?;
        check(&(a.ours.clone() + b.ours.clone()), &expected)?;
        let mut x = a.ours.clone();
        x += &b.ours;
        check(&x, &expected)?;
        let mut x = a.ours.clone();
        x += b.ours.clone();
        check(&x, &expected)?;
    }

    #[test]
    fn subtraction(a in value(MAX), b in value(MAX)) {
        let expected = &a.theirs - &b.theirs;
        check(&(&a.ours - &b.ours), &expected)?;
        check(&(a.ours.clone() - b.ours.clone()), &expected)?;
        let mut x = a.ours.clone();
        x -= &b.ours;
        check(&x, &expected)?;
        check(&(&a.ours - &a.ours), &NumBigInt::zero())?;
    }

    #[test]
    fn multiplication(a in value(MAX), b in value(MAX)) {
        let expected = &a.theirs * &b.theirs;
        check(&(&a.ours * &b.ours), &expected)?;
        check(&(a.ours.clone() * &b.ours), &expected)?;
        check(&(&a.ours * b.ours.clone()), &expected)?;
        check(&(a.ours.clone() * b.ours.clone()), &expected)?;
        let mut x = a.ours.clone();
        x *= &b.ours;
        check(&x, &expected)?;
        let mut x = a.ours.clone();
        x *= b.ours.clone();
        check(&x, &expected)?;
    }

    #[test]
    fn negation_and_absolute_value(a in value(MAX)) {
        check(&-&a.ours, &-&a.theirs)?;
        check(&-a.ours.clone(), &-&a.theirs)?;
        check(&a.ours.abs(), &a.theirs.abs())?;
    }

    #[test]
    fn comparisons_and_predicates(a in value(MAX), b in value(MAX)) {
        prop_assert_eq!(a.ours.cmp(&b.ours), a.theirs.cmp(&b.theirs));
        prop_assert_eq!(a.ours.partial_cmp(&b.ours), a.theirs.partial_cmp(&b.theirs));
        prop_assert_eq!(a.ours == b.ours, a.theirs == b.theirs);
        prop_assert_eq!(a.ours.sign(), our_sign(a.theirs.sign()));
        prop_assert_eq!(a.ours.is_zero(), a.theirs.is_zero());
        prop_assert_eq!(a.ours.is_negative(), a.theirs.is_negative());
        prop_assert_eq!(a.ours.is_positive(), a.theirs.is_positive());
        prop_assert_eq!(a.ours.is_even(), a.theirs.is_even());
        prop_assert_eq!(a.ours.is_odd(), a.theirs.is_odd());
        prop_assert_eq!(a.ours.bits(), a.theirs.bits());
        prop_assert_eq!(i64::from(a.ours.signum()), a.theirs.signum().to_i64().unwrap());
    }

    #[test]
    fn truncated_division(a in value(MAX), b in value(MAX)) {
        if b.theirs.is_zero() {
            prop_assert!(a.ours.checked_div_rem(&b.ours).is_none());
            prop_assert!(a.ours.checked_div(&b.ours).is_none());
            prop_assert!(a.ours.checked_rem(&b.ours).is_none());
        } else {
            let (q, r) = a.theirs.div_rem(&b.theirs);
            let (our_q, our_r) = a.ours.checked_div_rem(&b.ours).unwrap();
            check(&our_q, &q)?;
            check(&our_r, &r)?;
            check(&a.ours.checked_div(&b.ours).unwrap(), &(&a.theirs / &b.theirs))?;
            check(&a.ours.checked_rem(&b.ours).unwrap(), &(&a.theirs % &b.theirs))?;
        }
    }

    #[test]
    fn floored_and_ceiling_division(a in value(MAX), b in value(MAX)) {
        if b.theirs.is_zero() {
            prop_assert!(a.ours.checked_div_rem_floor(&b.ours).is_none());
            prop_assert!(a.ours.checked_div_floor(&b.ours).is_none());
            prop_assert!(a.ours.checked_mod_floor(&b.ours).is_none());
            prop_assert!(a.ours.checked_div_ceil(&b.ours).is_none());
        } else {
            let (q, r) = a.theirs.div_mod_floor(&b.theirs);
            let (our_q, our_r) = a.ours.checked_div_rem_floor(&b.ours).unwrap();
            check(&our_q, &q)?;
            check(&our_r, &r)?;
            check(&a.ours.checked_div_floor(&b.ours).unwrap(), &a.theirs.div_floor(&b.theirs))?;
            check(&a.ours.checked_mod_floor(&b.ours).unwrap(), &a.theirs.mod_floor(&b.theirs))?;
            check(&a.ours.checked_div_ceil(&b.ours).unwrap(), &a.theirs.div_ceil(&b.theirs))?;
        }
    }

    #[test]
    fn division_by_a_constant(a in value(MAX), d in limb()) {
        if let Some(divisor) = NonZeroU64::new(d) {
            let (q, r) = a.theirs.div_rem(&NumBigInt::from(d));
            let (our_q, our_r) = a.ours.div_rem_u64(divisor);
            check(&our_q, &q)?;
            check(&our_r, &r)?;
        }
    }

    /// Divisors that share their high limbs with the dividend, the cases that
    /// exercise the corrections of Knuth's algorithm D.
    #[test]
    fn division_sharing_high_limbs(
        divisor in prop::collection::vec(limb(), 2..10),
        low in prop::collection::vec(limb(), 0..8),
        high in prop::collection::vec(limb(), 0..3),
        bump in 0_u64..4,
        negative in any::<(bool, bool)>(),
    ) {
        let mut dividend = low.clone();
        dividend.extend_from_slice(&divisor);
        dividend.extend_from_slice(&high);
        // A dividend just above or below a multiple of the divisor's top limbs.
        if let Some(first) = dividend.first_mut() {
            *first = first.wrapping_add(bump);
        }
        let a = pair(negative.0, &dividend);
        let b = pair(negative.1, &divisor);
        if !b.theirs.is_zero() {
            let (q, r) = a.theirs.div_rem(&b.theirs);
            let (our_q, our_r) = a.ours.checked_div_rem(&b.ours).unwrap();
            check(&our_q, &q)?;
            check(&our_r, &r)?;
        }
    }

    /// q × b + r with a random remainder below b: every quotient digit is known.
    #[test]
    fn division_reconstructs(q in value(MAX), b in nonzero(MAX), r in value(MAX)) {
        let r_theirs = r.theirs.abs() % b.theirs.abs();
        let a = &q.theirs.abs() * &b.theirs.abs() + &r_theirs;
        let (our_q, our_r) = ours_of(&a).checked_div_rem(&ours_of(&b.theirs.abs())).unwrap();
        check(&our_q, &q.theirs.abs())?;
        check(&our_r, &r_theirs)?;
    }

    #[test]
    fn powers(a in value(4), exponent in 0_u32..48) {
        check(&a.ours.pow(exponent), &a.theirs.pow(exponent))?;
    }

    #[test]
    fn powers_of_small_bases(base in -40_i64..40, exponent in 0_u32..400) {
        check(&BigInt::from(base).pow(exponent), &NumBigInt::from(base).pow(exponent))?;
    }

    #[test]
    fn greatest_common_divisor(a in value(MAX), b in value(MAX)) {
        check(&a.ours.gcd(&b.ours), &a.theirs.gcd(&b.theirs))?;
    }

    #[test]
    fn shifts(a in value(MAX), bits in prop_oneof![0_u64..200, 0_u64..5000]) {
        let shift = usize::try_from(bits).unwrap();
        check(&(&a.ours << bits), &(&a.theirs << shift))?;
        check(&(&a.ours >> bits), &(&a.theirs >> shift))?;
        check(&(a.ours.clone() << shift), &(&a.theirs << shift))?;
        check(&(a.ours.clone() >> shift), &(&a.theirs >> shift))?;
        let mut x = a.ours.clone();
        x <<= bits;
        check(&x, &(&a.theirs << shift))?;
        let mut x = a.ours.clone();
        x >>= bits;
        check(&x, &(&a.theirs >> shift))?;
        if let Ok(small) = u8::try_from(bits) {
            check(&(&a.ours << small), &(&a.theirs << small))?;
            check(&(&a.ours >> small), &(&a.theirs >> small))?;
        }
        if let Ok(small) = u32::try_from(bits) {
            check(&(&a.ours << small), &(&a.theirs << small))?;
            check(&(&a.ours >> small), &(&a.theirs >> small))?;
        }
    }

    #[test]
    fn decimal_strings(a in value(MAX)) {
        let text = a.ours.to_string();
        prop_assert_eq!(&text, &a.theirs.to_string());
        prop_assert_eq!(format!("{:?}", a.ours), a.theirs.to_string());
        prop_assert_eq!(format!("{:+}", a.ours), format!("{:+}", a.theirs));
        prop_assert_eq!(format!("{:>40}", a.ours), format!("{:>40}", a.theirs));
        prop_assert_eq!(format!("{:<40}", a.ours), format!("{:<40}", a.theirs));
        prop_assert_eq!(format!("{:040}", a.ours), format!("{:040}", a.theirs));
        prop_assert_eq!(&text.parse::<BigInt>().unwrap(), &a.ours);
        // Leading zeros and separators.
        let padded = if let Some(digits) = text.strip_prefix('-') {
            format!("-000{digits}")
        } else {
            format!("+000{text}")
        };
        prop_assert_eq!(&padded.parse::<BigInt>().unwrap(), &a.ours);
    }

    /// Arbitrary strings over the characters that matter: the same verdict, value and
    /// error message as num-bigint.
    #[test]
    fn parsing(text in "[-+_0-9a ]{0,30}|[-+]?[0-9_]{0,60}|[0-9]{1,400}") {
        let ours = text.parse::<BigInt>();
        let theirs = text.parse::<NumBigInt>();
        match (ours, theirs) {
            (Ok(o), Ok(t)) => check(&o, &t)?,
            (Err(o), Err(t)) => prop_assert_eq!(o.to_string(), t.to_string()),
            (o, t) => prop_assert!(false, "{:?}: ours {:?}, num-bigint {:?}", text, o, t),
        }
    }

    /// The nearest double, ties to even, as the standard library's parser rounds the
    /// decimal string (num-bigint 0.5.1 is not always correctly rounded: see
    /// `to_double_where_num_bigint_is_not_correctly_rounded` in known.rs).
    #[test]
    fn to_double(a in value(MAX)) {
        prop_assert_eq!(a.ours.to_f64().to_bits(), correctly_rounded(&a.theirs).to_bits());
    }

    #[test]
    fn from_double(bits in any::<u64>()) {
        let x = f64::from_bits(bits);
        match (BigInt::from_f64(x), NumBigInt::from_f64(x)) {
            (Some(o), Some(t)) => check(&o, &t)?,
            (None, None) => {}
            (o, t) => prop_assert!(false, "{:e}: ours {:?}, num-bigint {:?}", x, o, t),
        }
    }

    #[test]
    fn integer_conversions(a in value(4)) {
        prop_assert_eq!(a.ours.to_i64(), a.theirs.to_i64());
        prop_assert_eq!(a.ours.to_u64(), a.theirs.to_u64());
        prop_assert_eq!(a.ours.to_i128(), a.theirs.to_i128());
        prop_assert_eq!(a.ours.to_u128(), a.theirs.to_u128());
        prop_assert_eq!(a.ours.to_i32(), a.theirs.to_i32());
        prop_assert_eq!(a.ours.to_u32(), a.theirs.to_u32());
        prop_assert_eq!(a.ours.to_usize(), a.theirs.to_usize());
        prop_assert_eq!(u8::try_from(&a.ours).ok(), u8::try_from(&a.theirs).ok());
        prop_assert_eq!(i8::try_from(&a.ours).ok(), i8::try_from(&a.theirs).ok());
        prop_assert_eq!(u16::try_from(&a.ours).ok(), u16::try_from(&a.theirs).ok());
        prop_assert_eq!(i16::try_from(&a.ours).ok(), i16::try_from(&a.theirs).ok());
        prop_assert_eq!(isize::try_from(&a.ours).ok(), isize::try_from(&a.theirs).ok());
        prop_assert_eq!(u128::try_from(a.ours.clone()).ok(), u128::try_from(&a.theirs).ok());
        prop_assert_eq!(i64::try_from(a.ours.clone()).ok(), i64::try_from(&a.theirs).ok());
    }

    #[test]
    fn from_primitives(
        wide in any::<(i128, u128)>(),
        word in any::<(i64, u64, isize, usize)>(),
        narrow in any::<(i32, u32, i16, u16, i8, u8)>(),
    ) {
        check(&BigInt::from(wide.0), &NumBigInt::from(wide.0))?;
        check(&BigInt::from(wide.1), &NumBigInt::from(wide.1))?;
        check(&BigInt::from(word.0), &NumBigInt::from(word.0))?;
        check(&BigInt::from(word.1), &NumBigInt::from(word.1))?;
        check(&BigInt::from(word.2), &NumBigInt::from(word.2))?;
        check(&BigInt::from(word.3), &NumBigInt::from(word.3))?;
        check(&BigInt::from(narrow.0), &NumBigInt::from(narrow.0))?;
        check(&BigInt::from(narrow.1), &NumBigInt::from(narrow.1))?;
        check(&BigInt::from(narrow.2), &NumBigInt::from(narrow.2))?;
        check(&BigInt::from(narrow.3), &NumBigInt::from(narrow.3))?;
        check(&BigInt::from(narrow.4), &NumBigInt::from(narrow.4))?;
        check(&BigInt::from(narrow.5), &NumBigInt::from(narrow.5))?;
    }

    #[test]
    fn primitive_operands(a in value(MAX), v in any::<i64>(), u in any::<u64>(), small in any::<u8>()) {
        check(&(&a.ours + v), &(&a.theirs + v))?;
        check(&(&a.ours - v), &(&a.theirs - v))?;
        check(&(&a.ours * v), &(&a.theirs * v))?;
        check(&(a.ours.clone() + u), &(&a.theirs + u))?;
        check(&(a.ours.clone() - u), &(&a.theirs - u))?;
        check(&(a.ours.clone() * u), &(&a.theirs * u))?;
        check(&(&a.ours + small), &(&a.theirs + small))?;
        check(&(&a.ours - small), &(&a.theirs - small))?;
        check(&(&a.ours * small), &(&a.theirs * small))?;
        let mut x = a.ours.clone();
        x += v;
        x *= u;
        x -= small;
        check(&x, &((&a.theirs + v) * u - small))?;
        let mut x = a.ours.clone();
        x *= v;
        check(&x, &(&a.theirs * v))?;
    }

    #[test]
    fn hashing_is_numeric(a in value(MAX)) {
        use std::hash::{BuildHasher as _, RandomState};
        let state = RandomState::new();
        // The same value reached by different routes hashes the same.
        let routed = (&a.ours + &BigInt::from(u64::MAX)) - &BigInt::from(u64::MAX);
        prop_assert_eq!(&routed, &a.ours);
        prop_assert_eq!(state.hash_one(&routed), state.hash_one(&a.ours));
    }
}

proptest! {
    #![proptest_config(config(64))]

    /// Values of thousands of digits (up to 700 limbs, about 13 500 digits).
    #[test]
    fn huge_values(
        a in (any::<bool>(), prop::collection::vec(limb(), 100..700)),
        b in (any::<bool>(), prop::collection::vec(limb(), 1..400)),
    ) {
        let a = pair(a.0, &a.1);
        let b = pair(b.0, &b.1);
        check(&(&a.ours + &b.ours), &(&a.theirs + &b.theirs))?;
        check(&(&a.ours - &b.ours), &(&a.theirs - &b.theirs))?;
        check(&(&a.ours * &b.ours), &(&a.theirs * &b.theirs))?;
        if !b.theirs.is_zero() {
            let (q, r) = a.theirs.div_rem(&b.theirs);
            let (our_q, our_r) = a.ours.checked_div_rem(&b.ours).unwrap();
            check(&our_q, &q)?;
            check(&our_r, &r)?;
        }
        let text = a.ours.to_string();
        prop_assert_eq!(&text, &a.theirs.to_string());
        prop_assert_eq!(&text.parse::<BigInt>().unwrap(), &a.ours);
        prop_assert_eq!(a.ours.to_f64().to_bits(), correctly_rounded(&a.theirs).to_bits());
    }
}

/// Doubles: every exponent, with mantissas at the edges and in between, both signs.
#[test]
fn from_double_every_exponent() {
    let mantissas = mantissa_patterns();
    let mut count = 0_u32;
    for exponent in 0_u64..=2047 {
        for &mantissa in &mantissas {
            for sign in [0, 1_u64 << 63] {
                let x = f64::from_bits(sign | exponent << 52 | mantissa);
                let ours = BigInt::from_f64(x);
                let theirs = NumBigInt::from_f64(x);
                assert_eq!(
                    ours.as_ref().map(BigInt::to_u64_digits),
                    theirs.map(|t| {
                        let (sign, digits) = t.to_u64_digits();
                        (our_sign(sign), digits)
                    }),
                    "{x:e}"
                );
                count += 1;
            }
        }
    }
    assert!(count > 100_000, "{count}");
}

/// The nearest double to an integer, ties to even: the standard library's parser on
/// its decimal string, which rounds correctly.
fn correctly_rounded(value: &NumBigInt) -> f64 {
    value.to_string().parse::<f64>().unwrap()
}

/// Integers near every rounding boundary of every binary exponent up to beyond the
/// range of f64: the nearest double, ties to even.
#[test]
fn to_double_rounding_boundaries() {
    let mut count = 0_u32;
    for top in [
        (1_u64 << 52) | 1,     // odd mantissa
        (1_u64 << 52) | 2,     // even mantissa
        (1_u64 << 53) - 1,     // all ones: rounding up carries to the next power
        1_u64 << 52,           // a power of two
        0x001a_bcde_f012_3457, // an arbitrary odd one
    ] {
        for exponent in 0_u64..1100 {
            // mantissa × 2^(exponent + 1) + {half, half ± 1, 1, 2^exponent ± …}
            let base = num_from_limbs(false, &[top]) << (exponent + 1);
            let half = NumBigInt::from(1) << exponent;
            let mut deltas = vec![
                NumBigInt::zero(),
                half.clone(),
                &half + 1,
                NumBigInt::from(1),
                &half << 1_u8,
                (&half << 1_u8) - 1,
            ];
            if exponent > 0 {
                deltas.push(&half - 1);
            }
            for delta in deltas {
                for value in [&base + &delta, -(&base + &delta)] {
                    let ours = ours_of(&value).to_f64();
                    assert_eq!(
                        ours.to_bits(),
                        correctly_rounded(&value).to_bits(),
                        "{value}"
                    );
                    count += 1;
                }
            }
        }
    }
    assert!(count > 60_000, "{count}");
}

/// Exact ties (a 53-bit mantissa, then a one, then zeros) broken by a single bit at
/// every position below, with the value's top at every bit of a limb: the bit that
/// decides the rounding may be anywhere in any lower limb.
#[test]
fn to_double_ties_broken_far_below() {
    let mut count = 0_u32;
    for top_bit in 53_u64..330 {
        for mantissa in [1_u64 << 52, (1_u64 << 52) | 1, (1_u64 << 53) - 1] {
            // mantissa · 2^(top_bit - 52) + 2^(top_bit - 53): halfway between two doubles.
            let tie = (num_from_limbs(false, &[mantissa]) << (top_bit - 52))
                + (NumBigInt::from(1) << (top_bit - 53));
            for below in 0..top_bit - 53 {
                for value in [
                    &tie + (NumBigInt::from(1) << below),
                    &tie - (NumBigInt::from(1) << below),
                ] {
                    let ours = ours_of(&value).to_f64();
                    assert_eq!(
                        ours.to_bits(),
                        correctly_rounded(&value).to_bits(),
                        "{value}"
                    );
                    count += 1;
                }
            }
            let ours = ours_of(&tie).to_f64();
            assert_eq!(ours.to_bits(), correctly_rounded(&tie).to_bits(), "{tie}");
        }
    }
    assert!(count > 200_000, "{count}");
}

fn mantissa_patterns() -> Vec<u64> {
    let mut patterns = vec![
        0,
        1,
        2,
        3,
        (1 << 52) - 1,
        1 << 51,
        (1 << 51) + 1,
        0x000a_aaaa_aaaa_aaaa,
    ];
    for s in 0..52 {
        patterns.push(1 << s);
        patterns.push((1 << s) - 1);
    }
    patterns
}
