//! Conversions between [`BigInt`] and the primitive numeric types.

use alloc::vec::Vec;
use core::fmt;

use crate::{BigInt, Repr, limbs};

/// The error of a conversion of a [`BigInt`] to a primitive integer type that cannot
/// hold its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TryFromBigIntError;

impl fmt::Display for TryFromBigIntError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("out of range conversion regarding big integer attempted")
    }
}

impl core::error::Error for TryFromBigIntError {}

impl From<i64> for BigInt {
    #[inline]
    fn from(value: i64) -> Self {
        Self::small(value)
    }
}

impl From<u64> for BigInt {
    #[inline]
    fn from(value: u64) -> Self {
        match i64::try_from(value) {
            Ok(small) => Self::small(small),
            Err(_) => Self {
                repr: Repr::Large {
                    negative: false,
                    magnitude: alloc::vec![value],
                },
            },
        }
    }
}

impl From<i128> for BigInt {
    fn from(value: i128) -> Self {
        if let Ok(small) = i64::try_from(value) {
            return Self::small(small);
        }
        let magnitude = value.unsigned_abs();
        Self::from_magnitude(
            value < 0,
            Vec::from([limbs::low(magnitude), limbs::high(magnitude)]),
        )
    }
}

impl From<u128> for BigInt {
    fn from(value: u128) -> Self {
        match i64::try_from(value) {
            Ok(small) => Self::small(small),
            Err(_) => {
                Self::from_magnitude(false, Vec::from([limbs::low(value), limbs::high(value)]))
            }
        }
    }
}

macro_rules! from_small {
    ($($t:ty => $via:ty),*) => {$(
        impl From<$t> for BigInt {
            #[inline]
            fn from(value: $t) -> Self {
                Self::from(<$via>::from(value))
            }
        }
    )*};
}

from_small!(i8 => i64, i16 => i64, i32 => i64, u8 => i64, u16 => i64, u32 => i64);

impl From<isize> for BigInt {
    fn from(value: isize) -> Self {
        // isize has at most 64 bits on every supported target; i128 holds it anyway.
        Self::from(i128::try_from(value).unwrap_or(0))
    }
}

impl From<usize> for BigInt {
    fn from(value: usize) -> Self {
        Self::from(u128::try_from(value).unwrap_or(0))
    }
}

impl BigInt {
    /// The value as an `i128`, when it fits.
    #[must_use]
    pub fn to_i128(&self) -> Option<i128> {
        match &self.repr {
            Repr::Small(v) => Some(i128::from(*v)),
            Repr::Large {
                negative,
                magnitude,
            } => {
                let m = magnitude_u128(magnitude)?;
                if *negative {
                    0_i128.checked_sub_unsigned(m)
                } else {
                    i128::try_from(m).ok()
                }
            }
        }
    }

    /// The value as a `u128`, when it fits.
    #[must_use]
    pub fn to_u128(&self) -> Option<u128> {
        match &self.repr {
            Repr::Small(v) => u128::try_from(*v).ok(),
            Repr::Large { negative: true, .. } => None,
            Repr::Large { magnitude, .. } => magnitude_u128(magnitude),
        }
    }

    /// The value as an `i64`, when it fits.
    #[must_use]
    #[inline]
    pub const fn to_i64(&self) -> Option<i64> {
        match self.repr {
            Repr::Small(v) => Some(v),
            Repr::Large { .. } => None,
        }
    }

    /// The value as a `u64`, when it fits.
    #[must_use]
    pub fn to_u64(&self) -> Option<u64> {
        self.to_u128().and_then(|v| u64::try_from(v).ok())
    }

    /// The value as an `i32`, when it fits.
    #[must_use]
    pub fn to_i32(&self) -> Option<i32> {
        self.to_i64().and_then(|v| i32::try_from(v).ok())
    }

    /// The value as a `u32`, when it fits.
    #[must_use]
    pub fn to_u32(&self) -> Option<u32> {
        self.to_i64().and_then(|v| u32::try_from(v).ok())
    }

    /// The value as a `usize`, when it fits.
    #[must_use]
    pub fn to_usize(&self) -> Option<usize> {
        self.to_u128().and_then(|v| usize::try_from(v).ok())
    }

    /// The nearest double, ties to even; beyond the range of `f64`, an infinity of the
    /// value's sign: the double the standard library's parser gives for the decimal
    /// string. (`num_traits::ToPrimitive::to_f64` of `num-bigint` 0.5.1 is the same but
    /// for the values where it misses a bit below its top 64, see the crate
    /// documentation.)
    #[must_use]
    #[inline]
    pub fn to_f64(&self) -> f64 {
        match &self.repr {
            Repr::Small(v) => {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "the conversion rounds to nearest, ties to even, as intended"
                )]
                let x = *v as f64;
                x
            }
            Repr::Large {
                negative,
                magnitude,
            } => {
                let x = magnitude_to_f64(magnitude);
                if *negative { -x } else { x }
            }
        }
    }

    /// The integer part of a finite double (truncation towards zero), exactly: the
    /// same as `num_traits::FromPrimitive::from_f64` of `num-bigint`. `None` for NaN
    /// and the infinities.
    #[must_use]
    pub fn from_f64(value: f64) -> Option<Self> {
        if !value.is_finite() {
            return None;
        }
        let bits = value.to_bits();
        let negative = bits >> 63 == 1;
        let raw_exponent = (bits >> 52) & 0x7ff;
        if raw_exponent < 1023 {
            // |value| < 1, subnormals included.
            return Some(Self::ZERO);
        }
        let mantissa = (bits & 0x000f_ffff_ffff_ffff) | (1 << 52);
        // value = ± mantissa × 2^(raw_exponent - 1075), raw_exponent in [1023, 2046].
        let magnitude = if raw_exponent >= 1075 {
            Self::from(mantissa) << (raw_exponent - 1075)
        } else {
            Self::from(mantissa >> (1075 - raw_exponent))
        };
        Some(if negative { -magnitude } else { magnitude })
    }
}

macro_rules! try_from {
    ($via:ident: $($t:ty),*) => {$(
        impl TryFrom<&BigInt> for $t {
            type Error = TryFromBigIntError;
            fn try_from(value: &BigInt) -> Result<Self, TryFromBigIntError> {
                value
                    .$via()
                    .and_then(|v| <$t>::try_from(v).ok())
                    .ok_or(TryFromBigIntError)
            }
        }
        impl TryFrom<BigInt> for $t {
            type Error = TryFromBigIntError;
            fn try_from(value: BigInt) -> Result<Self, TryFromBigIntError> {
                <$t>::try_from(&value)
            }
        }
    )*};
}

try_from!(to_i128: i8, i16, i32, i64, i128, isize);
try_from!(to_u128: u8, u16, u32, u64, u128, usize);

/// A normalized magnitude as a `u128`, when it fits.
fn magnitude_u128(magnitude: &[u64]) -> Option<u128> {
    match magnitude {
        [] => Some(0),
        [lo] => Some(u128::from(*lo)),
        [lo, hi] => Some(limbs::join(*hi, *lo)),
        _ => None,
    }
}

/// The nearest double to a normalized, non-empty magnitude.
fn magnitude_to_f64(magnitude: &[u64]) -> f64 {
    let bits = limbs::bits(magnitude);
    let (top, exponent) = if bits <= 64 {
        (magnitude.first().copied().unwrap_or(0), 0)
    } else {
        // The 64 top bits, the lowest of them set when any bit below is (rounding to
        // odd): rounding those 64 bits to 53 then rounds the whole value correctly.
        let exponent = bits - 64;
        let limbs = usize::try_from(exponent / 64).unwrap_or(usize::MAX);
        let shift = u32::try_from(exponent % 64).unwrap_or(0);
        let lo = magnitude.get(limbs).copied().unwrap_or(0);
        let hi = magnitude.get(limbs + 1).copied().unwrap_or(0);
        let top = limbs::funnel_right(hi, lo, shift);
        let below = (lo << (63 - shift) << 1) != 0 || magnitude[..limbs].iter().any(|&l| l != 0);
        (top | u64::from(below), exponent)
    };
    #[expect(
        clippy::cast_precision_loss,
        reason = "the conversion rounds to nearest, ties to even, as intended"
    )]
    let x = top as f64;
    if exponent > 1023 {
        // top >= 2^63, so the value is at least 2^1087.
        return f64::INFINITY;
    }
    // 2^exponent is a normal double; the product is exact or overflows to infinity.
    x * f64::from_bits((1023 + exponent) << 52)
}
