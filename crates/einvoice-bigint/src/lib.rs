//! Arbitrary-precision signed integers in safe Rust, with no dependency and no assembly.
//!
//! [`BigInt`] holds any integer exactly. Values that fit in an `i64` are stored inline
//! and computed with machine arithmetic; larger ones are a sign and a magnitude of
//! 64-bit limbs. The operations:
//!
//! * `+`, `-`, `*` and unary `-`, on values and references, with `BigInt` or a
//!   primitive integer on the right, and their assigning forms;
//! * division: truncated ([`BigInt::checked_div_rem`], the remainder taking the sign of
//!   the dividend), floored ([`BigInt::checked_div_rem_floor`], the remainder taking
//!   the sign of the divisor) and the ceiling quotient ([`BigInt::checked_div_ceil`]).
//!   A zero divisor gives `None`: there is no `/` or `%` operator, so that no division
//!   can panic. [`BigInt::div_rem_u64`] divides by a non-zero constant without that
//!   `Option`;
//! * [`BigInt::pow`], [`BigInt::gcd`], shifts (`<<`, and `>>` rounding towards negative
//!   infinity), comparisons, hashing;
//! * decimal strings: [`core::fmt::Display`] and [`core::str::FromStr`];
//! * conversions from and to every primitive integer type, [`BigInt::from_f64`]
//!   (truncation towards zero, exact) and [`BigInt::to_f64`] (the nearest double, ties
//!   to even).
//!
//! The results, the accepted strings and the conversions are those of the `num-bigint`
//! crate (0.5), which the tests compare with on random and edge-case values, with one
//! exception: [`BigInt::to_f64`] always rounds correctly, which `num-bigint` 0.5.1 does
//! not (it rounds 2^128 + 2^75 + 2^63 down to 2^128, the nearest double being
//! 2^128 + 2^76); its tests check it against the standard library's parser instead.
//!
//! ```
//! use einvoice_bigint::BigInt;
//!
//! let iban: BigInt = "3214282912345698765432161182".parse()?;
//! let (_, remainder) = iban.checked_div_rem(&BigInt::from(97)).ok_or("zero divisor")?;
//! assert_eq!(remainder, BigInt::from(1));
//!
//! let big = BigInt::from(10).pow(30) + BigInt::from(-7);
//! assert_eq!(big.to_string(), "999999999999999999999999999993");
//! assert_eq!(big.to_f64(), 1e30);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Multiplication is schoolbook and the decimal conversions are quadratic: the crate is
//! meant for numbers of up to a few thousand digits. Like any growing collection, an
//! operation whose result cannot be allocated aborts.

#![no_std]

extern crate alloc;

mod convert;
mod div;
mod limbs;
mod ops;
mod text;

use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;
use core::num::NonZeroU64;

pub use convert::TryFromBigIntError;
pub use text::ParseBigIntError;

/// The sign of a [`BigInt`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sign {
    /// Negative.
    Minus,
    /// Zero.
    NoSign,
    /// Positive.
    Plus,
}

/// An integer of any size.
///
/// Equality, ordering and hashing are numeric. `Debug` prints the decimal value, like
/// `Display`.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct BigInt {
    repr: Repr,
}

/// The representation is canonical: a value that fits in an `i64` is always `Small`,
/// so the derived equality and hash are numeric.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Repr {
    Small(i64),
    /// Outside the `i64` range; the magnitude is normalized (no zero limb at the top).
    Large {
        negative: bool,
        magnitude: Vec<u64>,
    },
}

/// A borrowed magnitude: a small value's limb lives inline.
pub(crate) enum Magnitude<'a> {
    Inline(u64),
    Slice(&'a [u64]),
}

impl Magnitude<'_> {
    /// The normalized limbs (empty for zero).
    #[inline]
    pub(crate) fn as_slice(&self) -> &[u64] {
        match self {
            Self::Inline(0) => &[],
            Self::Inline(limb) => core::slice::from_ref(limb),
            Self::Slice(limbs) => limbs,
        }
    }
}

impl BigInt {
    /// Zero.
    pub const ZERO: Self = Self::small(0);

    /// One.
    pub const ONE: Self = Self::small(1);

    #[inline]
    pub(crate) const fn small(value: i64) -> Self {
        Self {
            repr: Repr::Small(value),
        }
    }

    /// Zero.
    #[must_use]
    pub const fn zero() -> Self {
        Self::ZERO
    }

    /// One.
    #[must_use]
    pub const fn one() -> Self {
        Self::ONE
    }

    /// The value `-magnitude` or `magnitude`, from its limbs (not necessarily
    /// normalized), in canonical form.
    pub(crate) fn from_magnitude(negative: bool, mut magnitude: Vec<u64>) -> Self {
        limbs::trim(&mut magnitude);
        if let Some(value) = small_value(negative, &magnitude) {
            return Self::small(value);
        }
        Self {
            repr: Repr::Large {
                negative,
                magnitude,
            },
        }
    }

    /// Like [`Self::from_magnitude`], copying the limbs only for a large value.
    pub(crate) fn from_magnitude_slice(negative: bool, magnitude: &[u64]) -> Self {
        let mut end = magnitude.len();
        while end > 0 && magnitude[end - 1] == 0 {
            end -= 1;
        }
        let magnitude = &magnitude[..end];
        if let Some(value) = small_value(negative, magnitude) {
            return Self::small(value);
        }
        Self {
            repr: Repr::Large {
                negative,
                magnitude: magnitude.to_vec(),
            },
        }
    }

    /// The number of limbs the value's buffer can hold (0 inline).
    #[inline]
    pub(crate) fn capacity(&self) -> usize {
        match &self.repr {
            Repr::Small(_) => 0,
            Repr::Large { magnitude, .. } => magnitude.capacity(),
        }
    }

    /// Whether the value is negative, and its magnitude.
    #[inline]
    pub(crate) fn parts(&self) -> (bool, Magnitude<'_>) {
        match &self.repr {
            Repr::Small(v) => (*v < 0, Magnitude::Inline(v.unsigned_abs())),
            Repr::Large {
                negative,
                magnitude,
            } => (*negative, Magnitude::Slice(magnitude)),
        }
    }

    /// The value from a sign and little-endian 64-bit digits of the magnitude (any
    /// number of zero digits at the top). [`Sign::NoSign`] gives zero.
    #[must_use]
    pub fn from_u64_digits(sign: Sign, digits: &[u64]) -> Self {
        match sign {
            Sign::NoSign => Self::ZERO,
            Sign::Minus => Self::from_magnitude_slice(true, digits),
            Sign::Plus => Self::from_magnitude_slice(false, digits),
        }
    }

    /// The sign and the little-endian 64-bit digits of the magnitude, without zero
    /// digits at the top (none for zero).
    #[must_use]
    pub fn to_u64_digits(&self) -> (Sign, Vec<u64>) {
        let (_, magnitude) = self.parts();
        (self.sign(), magnitude.as_slice().to_vec())
    }

    /// The sign.
    #[must_use]
    #[inline]
    pub fn sign(&self) -> Sign {
        match self.signum() {
            -1 => Sign::Minus,
            0 => Sign::NoSign,
            _ => Sign::Plus,
        }
    }

    /// -1, 0 or 1.
    #[must_use]
    #[inline]
    pub fn signum(&self) -> i32 {
        match &self.repr {
            Repr::Small(v) => match v.cmp(&0) {
                Ordering::Less => -1,
                Ordering::Equal => 0,
                Ordering::Greater => 1,
            },
            Repr::Large { negative: true, .. } => -1,
            Repr::Large { .. } => 1,
        }
    }

    /// Whether the value is zero.
    #[must_use]
    #[inline]
    pub const fn is_zero(&self) -> bool {
        matches!(self.repr, Repr::Small(0))
    }

    /// Whether the value is one.
    #[must_use]
    #[inline]
    pub const fn is_one(&self) -> bool {
        matches!(self.repr, Repr::Small(1))
    }

    /// Whether the value is below zero.
    #[must_use]
    #[inline]
    pub fn is_negative(&self) -> bool {
        self.signum() < 0
    }

    /// Whether the value is above zero.
    #[must_use]
    #[inline]
    pub fn is_positive(&self) -> bool {
        self.signum() > 0
    }

    /// Whether the value is even.
    #[must_use]
    #[inline]
    pub fn is_even(&self) -> bool {
        match &self.repr {
            Repr::Small(v) => v & 1 == 0,
            Repr::Large { magnitude, .. } => magnitude.first().is_some_and(|l| l & 1 == 0),
        }
    }

    /// Whether the value is odd.
    #[must_use]
    #[inline]
    pub fn is_odd(&self) -> bool {
        !self.is_even()
    }

    /// The absolute value.
    #[must_use]
    pub fn abs(&self) -> Self {
        if self.is_negative() {
            -self
        } else {
            self.clone()
        }
    }

    /// The number of bits of the absolute value (0 for zero).
    #[must_use]
    pub fn bits(&self) -> u64 {
        let (_, magnitude) = self.parts();
        limbs::bits(magnitude.as_slice())
    }

    /// The quotient truncated towards zero and the remainder, which has the sign of
    /// `self` (`self = q × other + r`, `|r| < |other|`). `None` when `other` is zero.
    #[must_use]
    #[inline]
    pub fn checked_div_rem(&self, other: &Self) -> Option<(Self, Self)> {
        match (&self.repr, &other.repr) {
            (_, Repr::Small(0)) => None,
            // -1 is the only divisor whose quotient can leave the i64 range.
            (Repr::Small(a), Repr::Small(b)) if *b != -1 => {
                Some((Self::small(a / b), Self::small(a % b)))
            }
            _ => Some(self.div_rem_slow(other)),
        }
    }

    /// The truncated division for a non-zero divisor, when the operands are not both
    /// inline (or the divisor is -1).
    #[inline(never)]
    fn div_rem_slow(&self, other: &Self) -> (Self, Self) {
        let (a_negative, a) = self.parts();
        let (b_negative, b) = other.parts();
        if let [d] = b.as_slice() {
            let (q, r) = div::div_rem_limb(a.as_slice(), *d);
            let r = Self::from(r);
            return (
                Self::from_magnitude(a_negative != b_negative, q),
                if a_negative { -r } else { r },
            );
        }
        let (q, r) = div::div_rem(a.as_slice(), b.as_slice());
        (
            Self::from_magnitude(a_negative != b_negative, q),
            Self::from_magnitude(a_negative, r),
        )
    }

    /// The quotient truncated towards zero; `None` when `other` is zero.
    #[must_use]
    #[inline]
    pub fn checked_div(&self, other: &Self) -> Option<Self> {
        self.checked_div_rem(other).map(|(q, _)| q)
    }

    /// The remainder of the truncated division (sign of `self`); `None` when `other`
    /// is zero.
    #[must_use]
    #[inline]
    pub fn checked_rem(&self, other: &Self) -> Option<Self> {
        match (&self.repr, &other.repr) {
            (_, Repr::Small(0)) => None,
            (Repr::Small(a), Repr::Small(b)) if *b != -1 => Some(Self::small(a % b)),
            _ => Some(self.rem_slow(other)),
        }
    }

    /// The truncated remainder for a non-zero divisor, when the operands are not both
    /// inline (or the divisor is -1): without the quotient when the divisor has one
    /// limb.
    #[inline(never)]
    fn rem_slow(&self, other: &Self) -> Self {
        let (a_negative, a) = self.parts();
        let (_, b) = other.parts();
        if let [d] = b.as_slice() {
            let r = div::rem_limb(a.as_slice(), *d);
            let r = Self::from(r);
            return if a_negative { -r } else { r };
        }
        let (_, r) = div::div_rem(a.as_slice(), b.as_slice());
        Self::from_magnitude(a_negative, r)
    }

    /// The quotient rounded towards negative infinity and the remainder, which has the
    /// sign of `other` (`num_integer::Integer::div_mod_floor`). `None` when `other` is
    /// zero.
    #[must_use]
    #[inline]
    pub fn checked_div_rem_floor(&self, other: &Self) -> Option<(Self, Self)> {
        match (&self.repr, &other.repr) {
            (_, Repr::Small(0)) => None,
            (Repr::Small(a), Repr::Small(b)) if *b != -1 => {
                let (q, r) = floor_small(*a, *b);
                Some((Self::small(q), Self::small(r)))
            }
            _ => Some(self.div_rem_floor_slow(other)),
        }
    }

    /// The floored division for a non-zero divisor, when the operands are not both
    /// inline (or the divisor is -1): the truncated one, adjusted in place.
    #[inline(never)]
    fn div_rem_floor_slow(&self, other: &Self) -> (Self, Self) {
        let (mut q, mut r) = self.div_rem_slow(other);
        if !r.is_zero() && r.is_negative() != other.is_negative() {
            q -= 1_u8;
            r += other;
        }
        (q, r)
    }

    /// The quotient rounded towards negative infinity; `None` when `other` is zero.
    #[must_use]
    #[inline]
    pub fn checked_div_floor(&self, other: &Self) -> Option<Self> {
        match (&self.repr, &other.repr) {
            (_, Repr::Small(0)) => None,
            (Repr::Small(a), Repr::Small(b)) if *b != -1 => {
                Some(Self::small(floor_small(*a, *b).0))
            }
            _ => Some(self.div_rem_floor_slow(other).0),
        }
    }

    /// The remainder of the floored division (sign of `other`); `None` when `other` is
    /// zero.
    #[must_use]
    #[inline]
    pub fn checked_mod_floor(&self, other: &Self) -> Option<Self> {
        match (&self.repr, &other.repr) {
            (_, Repr::Small(0)) => None,
            (Repr::Small(a), Repr::Small(b)) if *b != -1 => {
                Some(Self::small(floor_small(*a, *b).1))
            }
            _ => Some(self.div_rem_floor_slow(other).1),
        }
    }

    /// The quotient rounded towards positive infinity; `None` when `other` is zero.
    #[must_use]
    #[inline]
    pub fn checked_div_ceil(&self, other: &Self) -> Option<Self> {
        match (&self.repr, &other.repr) {
            (_, Repr::Small(0)) => None,
            (Repr::Small(a), Repr::Small(b)) if *b != -1 => {
                let (q, r) = (a / b, a % b);
                // |q| <= 2^62 when |b| >= 2, and r = 0 when |b| = 1.
                Some(Self::small(if r != 0 && (r < 0) == (*b < 0) {
                    q + 1
                } else {
                    q
                }))
            }
            _ => Some(self.div_ceil_slow(other)),
        }
    }

    /// The ceiling quotient for a non-zero divisor, when the operands are not both
    /// inline (or the divisor is -1).
    #[inline(never)]
    fn div_ceil_slow(&self, other: &Self) -> Self {
        let (mut q, r) = self.div_rem_slow(other);
        if !r.is_zero() && r.is_negative() == other.is_negative() {
            q += 1_u8;
        }
        q
    }

    /// The truncated division by a non-zero constant: the quotient and the remainder,
    /// which has the sign of `self`.
    #[must_use]
    pub fn div_rem_u64(&self, divisor: NonZeroU64) -> (Self, Self) {
        let d = divisor.get();
        let (negative, magnitude) = self.parts();
        let (q, r) = match magnitude {
            Magnitude::Inline(x) => (Self::from(x / d), x % d),
            Magnitude::Slice(a) => {
                let (q, r) = div::div_rem_limb(a, d);
                (Self::from_magnitude(false, q), r)
            }
        };
        if negative {
            (-q, -Self::from(r))
        } else {
            (q, Self::from(r))
        }
    }

    /// `self` raised to the power `exponent` (`0^0` is 1).
    #[must_use]
    pub fn pow(&self, exponent: u32) -> Self {
        if let Repr::Small(v) = self.repr
            && let Some(p) = v.checked_pow(exponent)
        {
            return Self::small(p);
        }
        let (negative, base) = self.parts();
        let mut result = vec![1_u64];
        let mut square = base.as_slice().to_vec();
        let mut e = exponent;
        loop {
            if e & 1 == 1 {
                result = limbs::mul(&result, &square);
            }
            e >>= 1;
            if e == 0 {
                break;
            }
            square = limbs::mul(&square, &square);
        }
        Self::from_magnitude(negative && exponent & 1 == 1, result)
    }

    /// The greatest common divisor, never negative (`gcd(0, 0)` is 0).
    #[must_use]
    pub fn gcd(&self, other: &Self) -> Self {
        let (_, a) = self.parts();
        let (_, b) = other.parts();
        let mut a = a.as_slice().to_vec();
        let mut b = b.as_slice().to_vec();
        while !b.is_empty() {
            let (_, r) = div::div_rem(&a, &b);
            a = b;
            b = r;
        }
        Self::from_magnitude(false, a)
    }
}

/// The floored quotient and remainder of inline values, `b` neither 0 nor -1: |q| is at
/// most 2^62 when |b| >= 2, and the remainder is 0 when |b| = 1, so nothing overflows.
#[inline]
const fn floor_small(a: i64, b: i64) -> (i64, i64) {
    let (q, r) = (a / b, a % b);
    if r != 0 && (r < 0) != (b < 0) {
        (q - 1, r + b)
    } else {
        (q, r)
    }
}

/// The `i64` with this sign and normalized magnitude, if there is one.
#[inline]
fn small_value(negative: bool, magnitude: &[u64]) -> Option<i64> {
    match magnitude {
        [] => Some(0),
        [m] if negative => 0_i64.checked_sub_unsigned(*m),
        [m] => i64::try_from(*m).ok(),
        _ => None,
    }
}

impl Default for BigInt {
    fn default() -> Self {
        Self::ZERO
    }
}

impl Ord for BigInt {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.repr, &other.repr) {
            (Repr::Small(a), Repr::Small(b)) => a.cmp(b),
            // A large value is beyond every small one.
            (Repr::Small(_), Repr::Large { negative, .. }) => {
                if *negative {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            (Repr::Large { negative, .. }, Repr::Small(_)) => {
                if *negative {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
            (
                Repr::Large {
                    negative: a_negative,
                    magnitude: a,
                },
                Repr::Large {
                    negative: b_negative,
                    magnitude: b,
                },
            ) => match (a_negative, b_negative) {
                (false, true) => Ordering::Greater,
                (true, false) => Ordering::Less,
                (false, false) => limbs::cmp(a, b),
                (true, true) => limbs::cmp(b, a),
            },
        }
    }
}

impl PartialOrd for BigInt {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
