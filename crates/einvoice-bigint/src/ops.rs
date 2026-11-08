//! The arithmetic operators.
//!
//! Each operation has an inlined path for two inline (`i64`) values and an out-of-line
//! path for the rest. The forms that consume their left operand (`a + b`, `a += b`,
//! `a * small`) reuse its limbs when it is large.

use core::cmp::Ordering;
use core::mem;
use core::ops::{
    Add, AddAssign, Mul, MulAssign, Neg, Shl, ShlAssign, Shr, ShrAssign, Sub, SubAssign,
};

use crate::{BigInt, Repr, limbs};

/// `±a + ±b` from signs and magnitudes.
fn signed_add(a_negative: bool, a: &[u64], b_negative: bool, b: &[u64]) -> BigInt {
    if a_negative == b_negative {
        return BigInt::from_magnitude(a_negative, limbs::add(a, b));
    }
    match limbs::cmp(a, b) {
        Ordering::Equal => BigInt::ZERO,
        Ordering::Greater => BigInt::from_magnitude(a_negative, limbs::sub(a, b)),
        Ordering::Less => BigInt::from_magnitude(b_negative, limbs::sub(b, a)),
    }
}

/// `a + b`, or `a - b` when `subtract`, for operands that are not both inline.
#[inline(never)]
fn add_slow(a: &BigInt, b: &BigInt, subtract: bool) -> BigInt {
    let (a_negative, x) = a.parts();
    let (b_negative, y) = b.parts();
    // Subtracting b is adding -b; the magnitude of zero is empty, whatever its sign.
    signed_add(
        a_negative,
        x.as_slice(),
        b_negative != subtract,
        y.as_slice(),
    )
}

/// `a += b`, or `a -= b` when `subtract`, in the limbs of `a` when it is large.
#[inline(never)]
fn add_assign_slow(a: &mut BigInt, b: &BigInt, subtract: bool) {
    let Repr::Large {
        negative,
        magnitude,
    } = &mut a.repr
    else {
        *a = add_slow(a, b, subtract);
        return;
    };
    let (b_negative, y) = b.parts();
    let y = y.as_slice();
    if *negative == (b_negative != subtract) {
        // The magnitude grows: the value stays large.
        limbs::add_in_place(magnitude, y);
        return;
    }
    match limbs::cmp(magnitude, y) {
        Ordering::Greater => {
            limbs::sub_in_place(magnitude, y);
            // The difference may fit inline now.
            let negative = *negative;
            *a = BigInt::from_magnitude(negative, mem::take(magnitude));
        }
        Ordering::Equal => *a = BigInt::ZERO,
        Ordering::Less => *a = BigInt::from_magnitude(!*negative, limbs::sub(y, magnitude)),
    }
}

#[inline]
fn add(a: &BigInt, b: &BigInt) -> BigInt {
    if let (Repr::Small(x), Repr::Small(y)) = (&a.repr, &b.repr)
        && let Some(sum) = x.checked_add(*y)
    {
        return BigInt::small(sum);
    }
    add_slow(a, b, false)
}

#[inline]
fn sub(a: &BigInt, b: &BigInt) -> BigInt {
    if let (Repr::Small(x), Repr::Small(y)) = (&a.repr, &b.repr)
        && let Some(difference) = x.checked_sub(*y)
    {
        return BigInt::small(difference);
    }
    add_slow(a, b, true)
}

#[inline]
fn add_assign(a: &mut BigInt, b: &BigInt) {
    if let (Repr::Small(x), Repr::Small(y)) = (&a.repr, &b.repr)
        && let Some(sum) = x.checked_add(*y)
    {
        a.repr = Repr::Small(sum);
        return;
    }
    add_assign_slow(a, b, false);
}

#[inline]
fn sub_assign(a: &mut BigInt, b: &BigInt) {
    if let (Repr::Small(x), Repr::Small(y)) = (&a.repr, &b.repr)
        && let Some(difference) = x.checked_sub(*y)
    {
        a.repr = Repr::Small(difference);
        return;
    }
    add_assign_slow(a, b, true);
}

/// `a × b` for operands that are not both inline.
#[inline(never)]
fn mul_slow(a: &BigInt, b: &BigInt) -> BigInt {
    let (a_negative, x) = a.parts();
    let (b_negative, y) = b.parts();
    BigInt::from_magnitude(
        a_negative != b_negative,
        limbs::mul(x.as_slice(), y.as_slice()),
    )
}

#[inline]
fn mul(a: &BigInt, b: &BigInt) -> BigInt {
    if let (Repr::Small(x), Repr::Small(y)) = (&a.repr, &b.repr) {
        return match x.checked_mul(*y) {
            Some(product) => BigInt::small(product),
            // |x × y| <= 2^126.
            None => BigInt::from(i128::from(*x) * i128::from(*y)),
        };
    }
    mul_slow(a, b)
}

/// `a × b` in place when `a` is large and `b` inline (as in a loop multiplying by ten).
#[inline]
fn mul_assign(a: &mut BigInt, b: &BigInt) {
    if let Repr::Small(m) = b.repr
        && let Repr::Large {
            negative,
            magnitude,
        } = &mut a.repr
        // |b| >= 2 keeps a large value large (beyond 2^64); 2^63 × -1 is the inline
        // -2^63.
        && m.unsigned_abs() >= 2
    {
        limbs::mul_small_add(magnitude, m.unsigned_abs(), 0);
        if m < 0 {
            *negative = !*negative;
        }
        return;
    }
    *a = mul(a, b);
}

#[inline]
fn neg(a: &BigInt) -> BigInt {
    match &a.repr {
        Repr::Small(v) => match v.checked_neg() {
            Some(n) => BigInt::small(n),
            None => BigInt::from(-i128::from(*v)),
        },
        Repr::Large {
            negative,
            magnitude,
        } => BigInt::from_magnitude_slice(!negative, magnitude),
    }
}

impl Add<&BigInt> for &BigInt {
    type Output = BigInt;
    #[inline]
    fn add(self, rhs: &BigInt) -> BigInt {
        add(self, rhs)
    }
}

impl Add<BigInt> for &BigInt {
    type Output = BigInt;
    #[inline]
    fn add(self, mut rhs: BigInt) -> BigInt {
        // Addition commutes: reuse the owned operand's limbs.
        add_assign(&mut rhs, self);
        rhs
    }
}

impl Add<&BigInt> for BigInt {
    type Output = BigInt;
    #[inline]
    fn add(mut self, rhs: &BigInt) -> BigInt {
        add_assign(&mut self, rhs);
        self
    }
}

impl Add<BigInt> for BigInt {
    type Output = BigInt;
    #[inline]
    fn add(self, rhs: BigInt) -> BigInt {
        // Into the operand with the larger buffer.
        let (mut target, other) = if self.capacity() >= rhs.capacity() {
            (self, rhs)
        } else {
            (rhs, self)
        };
        add_assign(&mut target, &other);
        target
    }
}

impl AddAssign<&BigInt> for BigInt {
    #[inline]
    fn add_assign(&mut self, rhs: &BigInt) {
        add_assign(self, rhs);
    }
}

impl AddAssign<BigInt> for BigInt {
    #[inline]
    fn add_assign(&mut self, rhs: BigInt) {
        add_assign(self, &rhs);
    }
}

impl Sub<&BigInt> for &BigInt {
    type Output = BigInt;
    #[inline]
    fn sub(self, rhs: &BigInt) -> BigInt {
        sub(self, rhs)
    }
}

impl Sub<BigInt> for &BigInt {
    type Output = BigInt;
    #[inline]
    fn sub(self, rhs: BigInt) -> BigInt {
        sub(self, &rhs)
    }
}

impl Sub<&BigInt> for BigInt {
    type Output = BigInt;
    #[inline]
    fn sub(mut self, rhs: &BigInt) -> BigInt {
        sub_assign(&mut self, rhs);
        self
    }
}

impl Sub<BigInt> for BigInt {
    type Output = BigInt;
    #[inline]
    fn sub(mut self, rhs: BigInt) -> BigInt {
        sub_assign(&mut self, &rhs);
        self
    }
}

impl SubAssign<&BigInt> for BigInt {
    #[inline]
    fn sub_assign(&mut self, rhs: &BigInt) {
        sub_assign(self, rhs);
    }
}

impl SubAssign<BigInt> for BigInt {
    #[inline]
    fn sub_assign(&mut self, rhs: BigInt) {
        sub_assign(self, &rhs);
    }
}

impl Mul<&BigInt> for &BigInt {
    type Output = BigInt;
    #[inline]
    fn mul(self, rhs: &BigInt) -> BigInt {
        mul(self, rhs)
    }
}

impl Mul<BigInt> for &BigInt {
    type Output = BigInt;
    #[inline]
    fn mul(self, mut rhs: BigInt) -> BigInt {
        mul_assign(&mut rhs, self);
        rhs
    }
}

impl Mul<&BigInt> for BigInt {
    type Output = BigInt;
    #[inline]
    fn mul(mut self, rhs: &BigInt) -> BigInt {
        mul_assign(&mut self, rhs);
        self
    }
}

impl Mul<BigInt> for BigInt {
    type Output = BigInt;
    #[inline]
    fn mul(mut self, rhs: BigInt) -> BigInt {
        mul_assign(&mut self, &rhs);
        self
    }
}

impl MulAssign<&BigInt> for BigInt {
    #[inline]
    fn mul_assign(&mut self, rhs: &BigInt) {
        mul_assign(self, rhs);
    }
}

impl MulAssign<BigInt> for BigInt {
    #[inline]
    fn mul_assign(&mut self, rhs: BigInt) {
        mul_assign(self, &rhs);
    }
}

/// The operators with a primitive integer on the right.
macro_rules! primitive {
    ($($t:ty),*) => {$(
        impl Add<$t> for &BigInt {
            type Output = BigInt;
            #[inline]
            fn add(self, rhs: $t) -> BigInt {
                add(self, &BigInt::from(rhs))
            }
        }
        impl Add<$t> for BigInt {
            type Output = BigInt;
            #[inline]
            fn add(mut self, rhs: $t) -> BigInt {
                add_assign(&mut self, &BigInt::from(rhs));
                self
            }
        }
        impl AddAssign<$t> for BigInt {
            #[inline]
            fn add_assign(&mut self, rhs: $t) {
                add_assign(self, &BigInt::from(rhs));
            }
        }
        impl Sub<$t> for &BigInt {
            type Output = BigInt;
            #[inline]
            fn sub(self, rhs: $t) -> BigInt {
                sub(self, &BigInt::from(rhs))
            }
        }
        impl Sub<$t> for BigInt {
            type Output = BigInt;
            #[inline]
            fn sub(mut self, rhs: $t) -> BigInt {
                sub_assign(&mut self, &BigInt::from(rhs));
                self
            }
        }
        impl SubAssign<$t> for BigInt {
            #[inline]
            fn sub_assign(&mut self, rhs: $t) {
                sub_assign(self, &BigInt::from(rhs));
            }
        }
        impl Mul<$t> for &BigInt {
            type Output = BigInt;
            #[inline]
            fn mul(self, rhs: $t) -> BigInt {
                mul(self, &BigInt::from(rhs))
            }
        }
        impl Mul<$t> for BigInt {
            type Output = BigInt;
            #[inline]
            fn mul(mut self, rhs: $t) -> BigInt {
                mul_assign(&mut self, &BigInt::from(rhs));
                self
            }
        }
        impl MulAssign<$t> for BigInt {
            #[inline]
            fn mul_assign(&mut self, rhs: $t) {
                mul_assign(self, &BigInt::from(rhs));
            }
        }
    )*};
}

primitive!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

impl Neg for &BigInt {
    type Output = BigInt;
    #[inline]
    fn neg(self) -> BigInt {
        neg(self)
    }
}

impl Neg for BigInt {
    type Output = BigInt;
    #[inline]
    fn neg(self) -> BigInt {
        match self.repr {
            Repr::Large {
                negative,
                magnitude,
            } => BigInt::from_magnitude(!negative, magnitude),
            Repr::Small(_) => neg(&self),
        }
    }
}

/// `a << bits`.
fn shl(a: &BigInt, bits: u64) -> BigInt {
    if let Repr::Small(v) = a.repr
        && bits < 64
    {
        return BigInt::from(i128::from(v) << bits);
    }
    let (negative, magnitude) = a.parts();
    // A shift that does not fit in usize limbs could not be allocated either.
    let limbs = usize::try_from(bits / 64).unwrap_or(usize::MAX);
    let shift = shift_bits(bits);
    BigInt::from_magnitude(negative, limbs::shl(magnitude.as_slice(), limbs, shift))
}

/// `a >> bits`, rounding towards negative infinity (an arithmetic shift).
fn shr(a: &BigInt, bits: u64) -> BigInt {
    if let Repr::Small(v) = a.repr {
        return BigInt::small(v >> bits.min(63));
    }
    let (negative, magnitude) = a.parts();
    let limbs = usize::try_from(bits / 64).unwrap_or(usize::MAX);
    let (shifted, lost) = limbs::shr(magnitude.as_slice(), limbs, shift_bits(bits));
    let result = BigInt::from_magnitude(negative, shifted);
    if negative && lost {
        result - 1_u8
    } else {
        result
    }
}

/// `bits % 64`.
fn shift_bits(bits: u64) -> u32 {
    // Below 64: the conversion cannot fail.
    u32::try_from(bits % 64).unwrap_or(0)
}

macro_rules! shifts {
    ($($t:ty),*) => {$(
        impl Shl<$t> for &BigInt {
            type Output = BigInt;
            fn shl(self, bits: $t) -> BigInt {
                shl(self, u64::from(bits))
            }
        }
        impl Shl<$t> for BigInt {
            type Output = BigInt;
            fn shl(self, bits: $t) -> BigInt {
                shl(&self, u64::from(bits))
            }
        }
        impl ShlAssign<$t> for BigInt {
            fn shl_assign(&mut self, bits: $t) {
                *self = shl(self, u64::from(bits));
            }
        }
        impl Shr<$t> for &BigInt {
            type Output = BigInt;
            fn shr(self, bits: $t) -> BigInt {
                shr(self, u64::from(bits))
            }
        }
        impl Shr<$t> for BigInt {
            type Output = BigInt;
            fn shr(self, bits: $t) -> BigInt {
                shr(&self, u64::from(bits))
            }
        }
        impl ShrAssign<$t> for BigInt {
            fn shr_assign(&mut self, bits: $t) {
                *self = shr(self, u64::from(bits));
            }
        }
    )*};
}

shifts!(u8, u16, u32, u64);

/// `usize` shift amounts (the type of `Vec` lengths and of `unsigned_abs` on `isize`).
fn usize_bits(bits: usize) -> u64 {
    // usize has at most 64 bits on every supported target.
    u64::try_from(bits).unwrap_or(u64::MAX)
}

impl Shl<usize> for &BigInt {
    type Output = BigInt;
    fn shl(self, bits: usize) -> BigInt {
        shl(self, usize_bits(bits))
    }
}

impl Shl<usize> for BigInt {
    type Output = BigInt;
    fn shl(self, bits: usize) -> BigInt {
        shl(&self, usize_bits(bits))
    }
}

impl ShlAssign<usize> for BigInt {
    fn shl_assign(&mut self, bits: usize) {
        *self = shl(self, usize_bits(bits));
    }
}

impl Shr<usize> for &BigInt {
    type Output = BigInt;
    fn shr(self, bits: usize) -> BigInt {
        shr(self, usize_bits(bits))
    }
}

impl Shr<usize> for BigInt {
    type Output = BigInt;
    fn shr(self, bits: usize) -> BigInt {
        shr(&self, usize_bits(bits))
    }
}

impl ShrAssign<usize> for BigInt {
    fn shr_assign(&mut self, bits: usize) {
        *self = shr(self, usize_bits(bits));
    }
}
