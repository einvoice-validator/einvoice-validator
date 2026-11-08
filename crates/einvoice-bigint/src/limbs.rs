//! Magnitudes: unsigned integers as little-endian slices of 64-bit limbs.
//!
//! A magnitude is *normalized* when its last (most significant) limb is not zero; zero
//! is the empty slice. The functions below take normalized inputs unless stated
//! otherwise and return vectors that the caller normalizes (`trim`).

use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;

/// The low 64 bits.
#[expect(
    clippy::cast_possible_truncation,
    reason = "taking the low half of a 128-bit value is the intent"
)]
pub(crate) const fn low(x: u128) -> u64 {
    x as u64
}

/// The high 64 bits.
pub(crate) const fn high(x: u128) -> u64 {
    low(x >> 64)
}

/// `hi × 2^64 + lo`.
pub(crate) fn join(hi: u64, lo: u64) -> u128 {
    (u128::from(hi) << 64) | u128::from(lo)
}

/// The high limb of `(hi:lo) << shift` for `shift < 64`: `hi << shift` filled from the
/// top of `lo`.
pub(crate) fn funnel_left(hi: u64, lo: u64, shift: u32) -> u64 {
    high(join(hi, lo) << shift)
}

/// The low limb of `(hi:lo) >> shift` for `shift < 64`.
pub(crate) fn funnel_right(hi: u64, lo: u64, shift: u32) -> u64 {
    low(join(hi, lo) >> shift)
}

/// Removes the zero limbs at the top.
pub(crate) fn trim(v: &mut Vec<u64>) {
    while v.last() == Some(&0) {
        v.pop();
    }
}

/// Compares two normalized magnitudes.
pub(crate) fn cmp(a: &[u64], b: &[u64]) -> Ordering {
    a.len()
        .cmp(&b.len())
        .then_with(|| a.iter().rev().cmp(b.iter().rev()))
}

/// The number of significant bits.
pub(crate) fn bits(a: &[u64]) -> u64 {
    match a.last() {
        None => 0,
        Some(top) => {
            // A slice holds fewer than 2^61 limbs: the product cannot overflow.
            (a.len() as u64) * 64 - u64::from(top.leading_zeros())
        }
    }
}

/// `a + b`.
pub(crate) fn add(a: &[u64], b: &[u64]) -> Vec<u64> {
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    let mut out = Vec::with_capacity(long.len() + 1);
    let mut carry = false;
    for (i, &x) in long.iter().enumerate() {
        let y = short.get(i).copied().unwrap_or(0);
        let (sum, c) = x.carrying_add(y, carry);
        out.push(sum);
        carry = c;
    }
    if carry {
        out.push(1);
    }
    out
}

/// `a - b` for `a >= b`.
pub(crate) fn sub(a: &[u64], b: &[u64]) -> Vec<u64> {
    debug_assert_ne!(cmp(a, b), Ordering::Less);
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = false;
    for (i, &x) in a.iter().enumerate() {
        let y = b.get(i).copied().unwrap_or(0);
        let (diff, c) = x.borrowing_sub(y, borrow);
        out.push(diff);
        borrow = c;
    }
    debug_assert!(!borrow);
    trim(&mut out);
    out
}

/// `v += b`, in place.
pub(crate) fn add_in_place(v: &mut Vec<u64>, b: &[u64]) {
    if v.len() < b.len() {
        v.resize(b.len(), 0);
    }
    let (low, rest) = v.split_at_mut(b.len());
    let mut carry = false;
    for (x, &y) in low.iter_mut().zip(b) {
        let (sum, c) = x.carrying_add(y, carry);
        *x = sum;
        carry = c;
    }
    for x in rest {
        if !carry {
            return;
        }
        let (sum, c) = x.overflowing_add(1);
        *x = sum;
        carry = c;
    }
    if carry {
        v.push(1);
    }
}

/// `v -= b` for `v >= b`, in place; `v` is left normalized.
pub(crate) fn sub_in_place(v: &mut Vec<u64>, b: &[u64]) {
    debug_assert_ne!(cmp(v, b), Ordering::Less);
    let (low, rest) = v.split_at_mut(b.len());
    let mut borrow = false;
    for (x, &y) in low.iter_mut().zip(b) {
        let (difference, c) = x.borrowing_sub(y, borrow);
        *x = difference;
        borrow = c;
    }
    for x in rest {
        if !borrow {
            break;
        }
        let (difference, c) = x.overflowing_sub(1);
        *x = difference;
        borrow = c;
    }
    debug_assert!(!borrow);
    trim(v);
}

/// `a × b` (schoolbook; see the crate documentation for the sizes this is meant for).
pub(crate) fn mul(a: &[u64], b: &[u64]) -> Vec<u64> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    let mut out = vec![0_u64; long.len() + short.len()];
    for (i, &s) in short.iter().enumerate() {
        if s == 0 {
            continue;
        }
        let mut carry = 0_u64;
        // out[i..] += long × s, the carry ending in out[i + long.len()], still zero.
        let row = &mut out[i..];
        for (o, &l) in row.iter_mut().zip(long) {
            // l × s + o + carry <= (2^64 - 1)^2 + 2 (2^64 - 1) = 2^128 - 1.
            let (lo, hi) = l.carrying_mul_add(s, *o, carry);
            *o = lo;
            carry = hi;
        }
        row[long.len()] = carry;
    }
    trim(&mut out);
    out
}

/// `v × m + add`, in place.
pub(crate) fn mul_small_add(v: &mut Vec<u64>, m: u64, add: u64) {
    let mut carry = add;
    for limb in v.iter_mut() {
        let (lo, hi) = limb.carrying_mul(m, carry);
        *limb = lo;
        carry = hi;
    }
    if carry != 0 {
        v.push(carry);
    }
    trim(v);
}

/// `a << shift`.
pub(crate) fn shl(a: &[u64], limbs: usize, shift: u32) -> Vec<u64> {
    if a.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(limbs.saturating_add(a.len()).saturating_add(1));
    out.resize(limbs, 0);
    if shift == 0 {
        out.extend_from_slice(a);
    } else {
        let mut previous = 0_u64;
        for &limb in a {
            out.push(funnel_left(limb, previous, shift));
            previous = limb;
        }
        out.push(funnel_left(0, previous, shift));
    }
    trim(&mut out);
    out
}

/// `a >> (64 × limbs + shift)`, and whether a non-zero bit was shifted out.
pub(crate) fn shr(a: &[u64], limbs: usize, shift: u32) -> (Vec<u64>, bool) {
    let Some(kept) = a.get(limbs..) else {
        return (Vec::new(), !a.is_empty());
    };
    let dropped = a[..limbs].iter().any(|&l| l != 0);
    if shift == 0 {
        return (kept.to_vec(), dropped);
    }
    let lost = kept.first().is_some_and(|&l| l << (64 - shift) != 0);
    let mut out = Vec::with_capacity(kept.len());
    for (i, &limb) in kept.iter().enumerate() {
        let next = kept.get(i + 1).copied().unwrap_or(0);
        out.push(funnel_right(next, limb, shift));
    }
    trim(&mut out);
    (out, dropped || lost)
}

/// `a << shift` for `shift < 64`, into exactly `a.len() + extra` limbs, `extra` being 0
/// when the shift cannot carry out of the top limb and 1 otherwise. Not normalized.
pub(crate) fn shl_bits(a: &[u64], shift: u32, extra: usize) -> Vec<u64> {
    let mut out = Vec::with_capacity(a.len() + extra);
    let mut previous = 0_u64;
    for &limb in a {
        out.push(funnel_left(limb, previous, shift));
        previous = limb;
    }
    if extra > 0 {
        out.push(funnel_left(0, previous, shift));
    }
    out
}

/// `a << shift` for `shift < 64` into `out`, of the same length as `a`, when the shift
/// cannot carry out of the top limb. Not normalized.
pub(crate) fn shl_bits_into(a: &[u64], shift: u32, out: &mut [u64]) {
    let mut previous = 0_u64;
    for (o, &limb) in out.iter_mut().zip(a) {
        *o = funnel_left(limb, previous, shift);
        previous = limb;
    }
}

/// `a >>= shift` for `shift < 64`, in place. Not normalized.
pub(crate) fn shr_bits_in_place(a: &mut [u64], shift: u32) {
    if shift == 0 {
        return;
    }
    for i in 0..a.len() {
        let next = a.get(i + 1).copied().unwrap_or(0);
        a[i] = funnel_right(next, a[i], shift);
    }
}
