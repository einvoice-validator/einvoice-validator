//! Division of magnitudes.
//!
//! Every quotient digit comes from a division of a two-limb number by a normalized
//! limb (top bit set), computed with a precomputed reciprocal: Möller and Granlund,
//! "Improved division by invariant integers", IEEE Transactions on Computers 60(2),
//! 2011, algorithm 4. It needs only 64 × 64 → 128-bit multiplications and 64-bit
//! additions, so no 128-bit division (which would call the compiler's runtime
//! library) and no assembly. The reciprocal itself comes from a table of 256 entries
//! and Newton steps, again with multiplications only (the same paper, algorithm 2).
//! Divisors of two limbs or more use Knuth's algorithm D (The Art of Computer
//! Programming, volume 2, 4.3.1).

use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::limbs::{self, high, join, low};

/// A normalized divisor limb and its reciprocal `floor((2^128 - 1) / d) - 2^64`.
#[derive(Clone, Copy)]
pub(crate) struct Divisor {
    d: u64,
    v: u64,
}

impl Divisor {
    /// The divisor `d`, which must have its top bit set.
    #[inline]
    pub(crate) const fn new(d: u64) -> Self {
        debug_assert!(d >> 63 == 1);
        Self {
            d,
            v: reciprocal(d),
        }
    }

    /// `(u1 · 2^64 + u0) / d` and its remainder, for `u1 < d`.
    #[inline]
    pub(crate) fn div_2by1(self, u1: u64, u0: u64) -> (u64, u64) {
        debug_assert!(u1 < self.d);
        // v · u1 + (u1 · 2^64 + u0) = u1 · floor((2^128 - 1) / d) + u0 < 2^128.
        let q = u128::from(self.v) * u128::from(u1) + join(u1, u0);
        let q0 = low(q);
        // The paper's arithmetic is modulo 2^64 from here on.
        let mut q1 = high(q).wrapping_add(1);
        let mut r = u0.wrapping_sub(q1.wrapping_mul(self.d));
        if r > q0 {
            q1 = q1.wrapping_sub(1);
            r = r.wrapping_add(self.d);
        }
        if r >= self.d {
            // The quotient is below 2^64 and so is q1 + 1 here.
            q1 += 1;
            r -= self.d;
        }
        (q1, r)
    }
}

/// `floor((2^19 - 3 · 2^8) / (256 + i))`: the 11-bit first approximation of the
/// reciprocal of a divisor whose top nine bits are `256 + i`.
const APPROXIMATIONS: [u16; 256] = {
    let mut table = [0_u16; 256];
    let mut i = 0;
    while i < 256 {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "i < 256, and the quotient is at most 523520 / 256 = 2045"
        )]
        let value = (523_520 / (256 + i as u32)) as u16;
        table[i] = value;
        i += 1;
    }
    table
};

/// `floor((2^128 - 1) / d) - 2^64` for `d >= 2^63` (Möller and Granlund, algorithm
/// 2, `RECIPROCAL_WORD`): a table lookup, two Newton steps and a final correction,
/// in 64-bit arithmetic modulo 2^64 as the paper specifies.
///
/// A constant function (for the constant divisors), hence the `as` conversions: the
/// widening ones stand for `From`, and the index is below 256.
const fn reciprocal(d: u64) -> u64 {
    let d0 = d & 1;
    let d40 = (d >> 24) + 1;
    let d63 = (d >> 1) + d0;
    // The top nine bits of d are 256 + index, index < 256.
    let index = ((d >> 55) & 0xff) as usize;
    let v0 = APPROXIMATIONS[index] as u64;
    // v0 < 2^11 and d40 <= 2^40: v0^2 · d40 < 2^62.
    let v1 = (v0 << 11)
        .wrapping_sub((v0 * v0 * d40) >> 40)
        .wrapping_sub(1);
    let v2 = (v1 << 13)
        .wrapping_add(v1.wrapping_mul((1_u64 << 60).wrapping_sub(v1.wrapping_mul(d40))) >> 47);
    // e = 2^96 - v2 · d63 + floor(v2 / 2) · d0, which is below 2^64.
    let e = ((v2 >> 1) & 0_u64.wrapping_sub(d0)).wrapping_sub(v2.wrapping_mul(d63));
    let v3 = (v2 << 31).wrapping_add(high(v2 as u128 * e as u128) >> 1);
    // v4 = v3 - floor((v3 + 2^64 + 1) · d / 2^64) = v3 - d - high((v3 + 1) · d), the
    // product (v3 + 1) · d being at most 2^64 · d < 2^128.
    let t = v3 as u128 * d as u128 + d as u128;
    v3.wrapping_sub(high(t)).wrapping_sub(d)
}

/// The divisor `d` (not zero) normalized: its shift and its reciprocal.
pub(crate) const fn normalized(d: u64) -> (u32, Divisor) {
    let shift = d.leading_zeros();
    (shift, Divisor::new(d << shift))
}

/// `a / d` and `a % d` for a normalized `a` and a non-zero `d`.
pub(crate) fn div_rem_limb(a: &[u64], d: u64) -> (Vec<u64>, u64) {
    match a {
        [] => (Vec::new(), 0),
        [x] => {
            let q = x / d;
            (if q == 0 { Vec::new() } else { vec![q] }, x % d)
        }
        _ => {
            let mut q = a.to_vec();
            let (shift, divisor) = normalized(d);
            let r = div_rem_limb_in_place(&mut q, shift, divisor);
            (q, r)
        }
    }
}

/// `a % d` for a normalized `a` and a non-zero `d`, without the quotient.
pub(crate) fn rem_limb(a: &[u64], d: u64) -> u64 {
    match a {
        [] => 0,
        [x] => x % d,
        _ => {
            let (shift, divisor) = normalized(d);
            // As in div_rem_limb_in_place, keeping only the remainder.
            let mut r = a.last().map_or(0, |&top| limbs::funnel_left(0, top, shift));
            for i in (0..a.len()).rev() {
                let below = if i > 0 { a[i - 1] } else { 0 };
                (_, r) = divisor.div_2by1(r, limbs::funnel_left(a[i], below, shift));
            }
            r >> shift
        }
    }
}

/// Divides `a` in place by the divisor `divisor` = `d << shift` and returns `a % d`;
/// `a` is left normalized.
pub(crate) fn div_rem_limb_in_place(a: &mut Vec<u64>, shift: u32, divisor: Divisor) -> u64 {
    // The dividend shifted left by `shift`, one limb at a time from the top; its extra
    // top limb is a[top] >> (64 - shift), below 2^shift <= 2^63 <= the divisor.
    let mut r = a.last().map_or(0, |&top| limbs::funnel_left(0, top, shift));
    for i in (0..a.len()).rev() {
        let below = if i > 0 { a[i - 1] } else { 0 };
        let (q, rest) = divisor.div_2by1(r, limbs::funnel_left(a[i], below, shift));
        a[i] = q;
        r = rest;
    }
    limbs::trim(a);
    r >> shift
}

/// `a / b` and `a % b` for normalized `a` and `b`, `b` not zero.
pub(crate) fn div_rem(a: &[u64], b: &[u64]) -> (Vec<u64>, Vec<u64>) {
    match limbs::cmp(a, b) {
        Ordering::Less => return (Vec::new(), a.to_vec()),
        Ordering::Equal => return (vec![1], Vec::new()),
        Ordering::Greater => {}
    }
    if let [d] = b {
        let (q, r) = div_rem_limb(a, *d);
        return (q, if r == 0 { Vec::new() } else { vec![r] });
    }
    knuth(a, b)
}

/// Knuth's algorithm D for a divisor of two limbs or more and a larger dividend.
fn knuth(dividend: &[u64], divisor: &[u64]) -> (Vec<u64>, Vec<u64>) {
    let size = divisor.len();
    let steps = dividend.len() - size;
    let shift = divisor[size - 1].leading_zeros();
    // The divisor normalized (its top bit set) keeps its size, on the stack when it is
    // short; the dividend gets one more limb.
    let mut stack = [0_u64; 16];
    let heap;
    let den: &[u64] = if shift == 0 {
        divisor
    } else if let Some(buffer) = stack.get_mut(..size) {
        limbs::shl_bits_into(divisor, shift, buffer);
        buffer
    } else {
        heap = limbs::shl_bits(divisor, shift, 0);
        &heap
    };
    let mut num = limbs::shl_bits(dividend, shift, 1);
    let d1 = den[size - 1];
    let d0 = den[size - 2];
    let top = Divisor::new(d1);
    let mut quotient = vec![0_u64; steps + 1];
    for step in (0..=steps).rev() {
        // The window num[step..=step + size] is below den · 2^64, so its top limb
        // n2 <= d1.
        let n2 = num[step + size];
        let n1 = num[step + size - 1];
        let n0 = num[step + size - 2];
        let mut qhat;
        // rhat = (n2 · 2^64 + n1) - qhat · d1, when it is below 2^64.
        let mut rhat;
        if n2 >= d1 {
            debug_assert_eq!(n2, d1);
            qhat = u64::MAX;
            // (d1 · 2^64 + n1) - (2^64 - 1) · d1 = n1 + d1.
            let (sum, overflow) = n1.overflowing_add(d1);
            rhat = (!overflow).then_some(sum);
        } else {
            let (estimate, rest) = top.div_2by1(n2, n1);
            qhat = estimate;
            rhat = Some(rest);
        }
        // At most two corrections (Knuth, theorem B).
        while let Some(rest) = rhat {
            if u128::from(qhat) * u128::from(d0) <= join(rest, n0) {
                break;
            }
            qhat -= 1;
            let (sum, overflow) = rest.overflowing_add(d1);
            rhat = (!overflow).then_some(sum);
        }
        // num[step..=step + size] -= qhat · den, with one carry for the product's high
        // limb and the borrow (as GMP's submul_1): qhat · y + carry is at most
        // 2^128 - 2^64, whose high limb is 2^64 - 1 only with a low limb of 0, which
        // borrows nothing, so high + borrow never exceeds 2^64 - 1.
        let mut carry = 0_u64;
        for (x, &y) in num[step..step + size].iter_mut().zip(den) {
            let (product, product_high) = qhat.carrying_mul(y, carry);
            let (difference, borrow) = x.overflowing_sub(product);
            *x = difference;
            carry = product_high + u64::from(borrow);
        }
        let (limb, below) = num[step + size].overflowing_sub(carry);
        num[step + size] = limb;
        if below {
            // qhat was one too large (probability about 2 / 2^64): add den back. The
            // carry out of the top limb cancels the borrow and is dropped.
            qhat -= 1;
            let mut carry = false;
            for (x, &y) in num[step..step + size].iter_mut().zip(den) {
                let (sum, out) = x.carrying_add(y, carry);
                *x = sum;
                carry = out;
            }
            num[step + size] = num[step + size].wrapping_add(u64::from(carry));
        }
        quotient[step] = qhat;
    }
    limbs::trim(&mut quotient);
    // The remainder, in the dividend's limbs.
    num.truncate(size);
    limbs::shr_bits_in_place(&mut num, shift);
    limbs::trim(&mut num);
    (quotient, num)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reciprocal against the 128-bit division of the test build, on the edges of
    /// every table segment and on a sample of the whole range.
    #[test]
    fn reciprocals_everywhere() {
        let exact = |d: u64| u64::try_from(u128::MAX / u128::from(d) - (1_u128 << 64)).unwrap();
        let mut checked = 0_u64;
        for segment in 256_u64..=512 {
            let start = segment << 55;
            for offset in 0..2048_u64 {
                for d in [start.wrapping_add(offset), start.wrapping_sub(offset + 1)] {
                    if d >> 63 == 1 {
                        assert_eq!(reciprocal(d), exact(d), "{d:#x}");
                        checked += 1;
                    }
                }
            }
        }
        // A pseudo-random sequence over [2^63, 2^64).
        let mut x = 0x9e37_79b9_7f4a_7c15_u64;
        for _ in 0..1_000_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let d = x | (1 << 63);
            assert_eq!(reciprocal(d), exact(d), "{d:#x}");
            checked += 1;
        }
        // Divisors made of a run of ones at the top and a run of ones at the bottom.
        for high_bits in 1..=64_u32 {
            for low_bits in 0..64_u32 {
                let ones = u64::MAX << (64 - high_bits);
                let d = ones | ((1_u64 << low_bits) - 1) | (1 << 63);
                assert_eq!(reciprocal(d), exact(d), "{d:#x}");
                checked += 1;
            }
        }
        assert!(checked > 1_500_000, "{checked}");
    }

    #[test]
    fn reciprocals() {
        for d in [
            1_u64 << 63,
            (1 << 63) + 1,
            u64::MAX,
            u64::MAX - 1,
            0xdead_beef_dead_beef,
            0x8000_0000_ffff_ffff,
            0xffff_ffff_0000_0000,
            0x8000_0001_0000_0000,
        ] {
            let expected = u128::MAX / u128::from(d) - (1_u128 << 64);
            assert_eq!(u128::from(Divisor::new(d).v), expected, "{d:#x}");
        }
    }

    #[test]
    fn two_by_one() {
        let ds = [
            1_u64 << 63,
            u64::MAX,
            0x8000_0000_0000_0001,
            0xc000_0000_0000_0000,
        ];
        let us = [0_u64, 1, u64::MAX, 1 << 63, (1 << 63) - 1, 0xffff_ffff];
        for &d in &ds {
            let divisor = Divisor::new(d);
            for &u1 in &us {
                for &u0 in &us {
                    let u1 = u1 % d;
                    let (q, r) = divisor.div_2by1(u1, u0);
                    let n = join(u1, u0);
                    assert_eq!(u128::from(q), n / u128::from(d));
                    assert_eq!(u128::from(r), n % u128::from(d));
                }
            }
        }
    }
}
