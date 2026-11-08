//! Decimal strings: [`fmt::Display`] and [`FromStr`].

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::str::FromStr;

use crate::{BigInt, Magnitude, div, limbs};

/// 10^19, the largest power of ten below 2^64: the decimal conversions work on chunks
/// of 19 digits.
const CHUNK: u64 = 10_000_000_000_000_000_000;
const CHUNK_DIGITS: usize = 19;
/// 10^19 normalized, with its reciprocal.
const CHUNK_DIVISOR: (u32, div::Divisor) = div::normalized(CHUNK);

/// The error of parsing a [`BigInt`]. The messages are those of `num-bigint`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseBigIntError {
    empty: bool,
}

impl ParseBigIntError {
    /// Whether the string held no digit at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.empty
    }
}

impl fmt::Display for ParseBigIntError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.empty {
            "cannot parse integer from empty string"
        } else {
            "invalid digit found in string"
        })
    }
}

impl core::error::Error for ParseBigIntError {}

const EMPTY: ParseBigIntError = ParseBigIntError { empty: true };
const INVALID: ParseBigIntError = ParseBigIntError { empty: false };

impl FromStr for BigInt {
    type Err = ParseBigIntError;

    /// A decimal integer, with the syntax of `num-bigint`: an optional `-` or `+`
    /// (not both), then ASCII digits, which may be separated by underscores after the
    /// first one (`1_000`).
    fn from_str(s: &str) -> Result<Self, ParseBigIntError> {
        let (negative, rest) = match s.strip_prefix('-') {
            Some(tail) if tail.starts_with('+') => return Err(INVALID),
            Some(tail) => (true, tail),
            None => (false, s),
        };
        let rest = match rest.strip_prefix('+') {
            Some(tail) if !tail.starts_with('+') => tail,
            _ => rest,
        };
        if rest.is_empty() {
            return Err(EMPTY);
        }
        if rest.starts_with('_') {
            return Err(INVALID);
        }
        let mut digits = Vec::with_capacity(rest.len());
        for b in rest.bytes() {
            match b {
                b'0'..=b'9' => digits.push(b - b'0'),
                b'_' => {}
                _ => return Err(INVALID),
            }
        }
        Ok(Self::from_magnitude(negative, parse_digits(&digits)))
    }
}

/// The magnitude of decimal digit values, most significant first.
fn parse_digits(digits: &[u8]) -> Vec<u64> {
    let chunk_value = |chunk: &[u8]| chunk.iter().fold(0_u64, |acc, &d| acc * 10 + u64::from(d));
    // The first chunk takes the digits beyond a multiple of 19.
    let first = match digits.len() % CHUNK_DIGITS {
        0 => CHUNK_DIGITS.min(digits.len()),
        n => n,
    };
    let (head, tail) = digits.split_at(first);
    // 19 digits are a little under 64 bits.
    let mut magnitude = Vec::with_capacity(digits.len() / CHUNK_DIGITS + 1);
    magnitude.push(chunk_value(head));
    for chunk in tail.chunks(CHUNK_DIGITS) {
        limbs::mul_small_add(&mut magnitude, CHUNK, chunk_value(chunk));
    }
    magnitude
}

impl fmt::Display for BigInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (negative, magnitude) = self.parts();
        match magnitude {
            Magnitude::Inline(v) => f.pad_integral(!negative, "", &u64_digits(v)),
            Magnitude::Slice(limbs) => f.pad_integral(!negative, "", &magnitude_digits(limbs)),
        }
    }
}

impl fmt::Debug for BigInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// The decimal digits of a `u64`.
fn u64_digits(v: u64) -> String {
    let mut out = String::with_capacity(20);
    push_digits(&mut out, v, 0);
    out
}

/// The number of decimal digits of `v` (1 for 0).
fn decimal_length(v: u64) -> usize {
    // At most 20.
    v.checked_ilog10()
        .map_or(1, |l| usize::try_from(l).unwrap_or(0) + 1)
}

/// Appends the decimal digits of `v`, left-padded with zeros to `width`.
fn push_digits(out: &mut String, mut v: u64, width: usize) {
    let mut buffer = [b'0'; 20];
    let mut start = buffer.len();
    while v > 0 {
        start -= 1;
        // v % 10 < 10.
        buffer[start] = b'0' + u8::try_from(v % 10).unwrap_or(0);
        v /= 10;
    }
    let start = start.min(buffer.len() - width.max(1));
    for &b in &buffer[start..] {
        out.push(char::from(b));
    }
}

/// From this many limbs on, a value is split in two by a power of ten and each part
/// converted on its own (measured with the `compare` example); below, its digits come
/// from repeated division by 10^19.
const SPLIT_LIMBS: usize = 16;

/// The decimal digits of a normalized, non-zero magnitude.
fn magnitude_digits(magnitude: &[u64]) -> String {
    // A limb holds less than 19.3 decimal digits.
    let mut out = String::with_capacity(magnitude.len() * 20);
    if magnitude.len() < SPLIT_LIMBS {
        push_chunked(&mut out, magnitude.to_vec(), 0);
        return out;
    }
    // powers[k] = 10^(19 · 2^k), up to half the size of the value.
    let mut powers = Vec::from([Vec::from([CHUNK])]);
    while let Some(last) = powers.last()
        && last.len() * 2 <= magnitude.len()
    {
        let square = limbs::mul(last, last);
        powers.push(square);
    }
    push_split(&mut out, magnitude.to_vec(), &powers, powers.len() - 1, 0);
    out
}

/// Appends the digits of `value`, left-padded with zeros to `width` digits: the
/// quotient and the remainder of its division by the power of ten of about half its
/// size, each converted the same way. The divisions are throughput-bound, where the
/// repeated division by 10^19 is a chain of dependent steps.
fn push_split(out: &mut String, value: Vec<u64>, powers: &[Vec<u64>], level: usize, width: usize) {
    if value.len() < SPLIT_LIMBS {
        push_chunked(out, value, width);
        return;
    }
    let mut level = level;
    while level > 0 && powers[level].len() * 2 > value.len() {
        level -= 1;
    }
    // value has at least twice the limbs of the power: the quotient is not zero.
    let (high, low) = div::div_rem(&value, &powers[level]);
    let low_width = CHUNK_DIGITS << level;
    push_split(out, high, powers, level, width.saturating_sub(low_width));
    push_split(out, low, powers, level, low_width);
}

/// Appends the digits of `value` (normalized), left-padded with zeros to `width`
/// digits, by repeated division by 10^19.
fn push_chunked(out: &mut String, value: Vec<u64>, width: usize) {
    let mut rest = value;
    let mut chunks = Vec::with_capacity(rest.len() * 64 / 63 + 1);
    let (shift, divisor) = CHUNK_DIVISOR;
    while !rest.is_empty() {
        chunks.push(div::div_rem_limb_in_place(&mut rest, shift, divisor));
    }
    let mut chunks = chunks.iter().rev();
    let Some(&top) = chunks.next() else {
        out.extend(core::iter::repeat_n('0', width));
        return;
    };
    let length = decimal_length(top) + chunks.len() * CHUNK_DIGITS;
    out.extend(core::iter::repeat_n('0', width.saturating_sub(length)));
    push_digits(out, top, 0);
    for &chunk in chunks {
        push_digits(out, chunk, CHUNK_DIGITS);
    }
}
