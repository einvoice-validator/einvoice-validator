//! Shared helpers of the tests: the same value as an `einvoice_bigint::BigInt` and as a
//! `num_bigint::BigInt` (the reference), and strategies for random values.

#![allow(
    dead_code,
    reason = "each test binary uses a different subset of the helpers"
)]

use einvoice_bigint::{BigInt, Sign};
use num_bigint::{BigInt as NumBigInt, Sign as NumSign};
use proptest::prelude::*;

/// A value as both types, built from the same limbs.
#[derive(Clone, Debug)]
pub(crate) struct Pair {
    pub(crate) ours: BigInt,
    pub(crate) theirs: NumBigInt,
}

pub(crate) fn num_sign(sign: Sign) -> NumSign {
    match sign {
        Sign::Minus => NumSign::Minus,
        Sign::NoSign => NumSign::NoSign,
        Sign::Plus => NumSign::Plus,
    }
}

pub(crate) fn our_sign(sign: NumSign) -> Sign {
    match sign {
        NumSign::Minus => Sign::Minus,
        NumSign::NoSign => Sign::NoSign,
        NumSign::Plus => Sign::Plus,
    }
}

/// The reference value with these 64-bit limbs.
pub(crate) fn num_from_limbs(negative: bool, limbs: &[u64]) -> NumBigInt {
    let digits: Vec<u32> = limbs
        .iter()
        .flat_map(|&l| {
            [
                u32::try_from(l & 0xffff_ffff).unwrap(),
                u32::try_from(l >> 32).unwrap(),
            ]
        })
        .collect();
    let sign = if negative {
        NumSign::Minus
    } else {
        NumSign::Plus
    };
    NumBigInt::from_slice(sign, &digits)
}

pub(crate) fn pair(negative: bool, limbs: &[u64]) -> Pair {
    let ours = BigInt::from_u64_digits(if negative { Sign::Minus } else { Sign::Plus }, limbs);
    Pair {
        ours,
        theirs: num_from_limbs(negative, limbs),
    }
}

/// Our value of the reference value, through limbs only (no strings).
pub(crate) fn ours_of(theirs: &NumBigInt) -> BigInt {
    let (sign, digits) = theirs.to_u64_digits();
    BigInt::from_u64_digits(our_sign(sign), &digits)
}

/// Asserts that our result is the reference result, as a value (limbs and sign) and
/// in canonical form (equal to the value built from the reference's limbs).
pub(crate) fn check(ours: &BigInt, theirs: &NumBigInt) -> Result<(), TestCaseError> {
    let (sign, digits) = theirs.to_u64_digits();
    prop_assert_eq!(ours.to_u64_digits(), (our_sign(sign), digits));
    prop_assert_eq!(ours, &ours_of(theirs));
    Ok(())
}

/// Limbs with many edge values: zero, all ones, single bits, masks, small numbers.
pub(crate) fn limb() -> impl Strategy<Value = u64> {
    prop_oneof![
        6 => any::<u64>(),
        2 => Just(0_u64),
        2 => Just(u64::MAX),
        1 => Just(1_u64 << 63),
        1 => Just((1_u64 << 63) - 1),
        1 => (0_u32..64).prop_map(|s| 1_u64 << s),
        1 => (1_u32..64).prop_map(|s| (1_u64 << s) - 1),
        1 => 0_u64..16,
        1 => Just(10_000_000_000_000_000_000_u64),
    ]
}

/// Magnitudes of up to `max` limbs, mostly short ones.
pub(crate) fn limbs(max: usize) -> impl Strategy<Value = Vec<u64>> {
    prop_oneof![
        6 => prop::collection::vec(limb(), 0..=3),
        3 => prop::collection::vec(limb(), 0..=12),
        1 => prop::collection::vec(limb(), 0..=max),
    ]
}

/// Values around the edges of the inline representation and of the limbs.
pub(crate) fn special() -> impl Strategy<Value = Pair> {
    let values: Vec<i128> = vec![
        0,
        1,
        -1,
        2,
        -2,
        10,
        -10,
        97,
        i128::from(i64::MAX),
        i128::from(i64::MIN),
        i128::from(i64::MAX) + 1,
        i128::from(i64::MIN) - 1,
        i128::from(u64::MAX),
        -i128::from(u64::MAX),
        i128::from(u64::MAX) + 1,
        -i128::from(u64::MAX) - 1,
        i128::MAX,
        i128::MIN,
        i128::from(i32::MAX),
        i128::from(i32::MIN),
    ];
    prop::sample::select(values).prop_map(|v| Pair {
        ours: BigInt::from(v),
        theirs: NumBigInt::from(v),
    })
}

/// A random value: small primitives, edge values or limbs of up to `max` limbs.
pub(crate) fn value(max: usize) -> impl Strategy<Value = Pair> {
    prop_oneof![
        2 => any::<i64>().prop_map(|v| Pair { ours: BigInt::from(v), theirs: NumBigInt::from(v) }),
        1 => any::<i128>().prop_map(|v| Pair { ours: BigInt::from(v), theirs: NumBigInt::from(v) }),
        2 => special(),
        6 => (any::<bool>(), limbs(max)).prop_map(|(negative, l)| pair(negative, &l)),
    ]
}

/// Like [`value`], with 1 in place of 0.
pub(crate) fn nonzero(max: usize) -> impl Strategy<Value = Pair> {
    value(max).prop_map(|p| {
        if p.ours.is_zero() {
            pair(false, &[1])
        } else {
            p
        }
    })
}
