//! Times the operations of this crate against `num-bigint` 0.5 on the sizes an XPath
//! data model sees: decimal amounts (one or two limbs), division quotients scaled by
//! 10^18, IBAN remainders, the multi-limb integers of exact double formatting, and
//! numbers of thousands of digits.
//!
//! cargo build --release -p einvoice-bigint --example compare
//! taskset -c 2 target/release/examples/compare
//!
//! Each line gives the best of seven runs, in nanoseconds per operation, for this
//! crate and for num-bigint, and their ratio (above 1: this crate is faster). Pin the
//! process to one core (`taskset`): on a processor with two kinds of cores a migration
//! between two measurements skews the ratio. The loops that pass a value from step to
//! step are functions of their own, compiled the same way for both crates, so that the
//! comparison does not depend on how each closure is inlined into the timing loop.

use std::error::Error;
use std::hint::black_box;
use std::num::NonZeroU64;
use std::time::Instant;

use einvoice_bigint::{BigInt, Sign};
use num_bigint::BigInt as NumBigInt;
use num_integer::Integer as _;
use num_traits::ToPrimitive as _;

/// Nanoseconds per call of `f`, the best of seven runs of at least 30 ms.
fn time<R>(mut f: impl FnMut() -> R) -> f64 {
    let mut iterations = 1_u32;
    loop {
        let start = Instant::now();
        for _ in 0..iterations {
            black_box(f());
        }
        if start.elapsed().as_millis() >= 30 || iterations >= 1 << 28 {
            break;
        }
        iterations *= 2;
    }
    let mut best = f64::INFINITY;
    for _ in 0..7 {
        let start = Instant::now();
        for _ in 0..iterations {
            black_box(f());
        }
        best = best.min(start.elapsed().as_secs_f64() * 1e9 / f64::from(iterations));
    }
    best
}

fn row(name: &str, ours: f64, theirs: f64) {
    println!(
        "| {name:<44} | {ours:>10.1} | {theirs:>10.1} | {:>6.2} |",
        theirs / ours
    );
}

/// The same pseudo-random value of `count` limbs in both crates.
fn both(count: usize, seed: u64) -> (BigInt, NumBigInt) {
    let mut state = seed | 1;
    let limbs: Vec<u64> = (0..count)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        })
        .collect();
    let digits: Vec<u32> = limbs
        .iter()
        .flat_map(|&x| [x, x >> 32])
        .map(|x| u32::try_from(x & 0xffff_ffff).unwrap_or(0))
        .collect();
    (
        BigInt::from_u64_digits(Sign::Plus, &limbs),
        NumBigInt::from_slice(num_bigint::Sign::Plus, &digits),
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    println!("| operation | ours (ns) | num-bigint (ns) | ratio |");
    println!("|---|---:|---:|---:|");
    amounts();
    division()?;
    iban()?;
    double_formatting();
    large()
}

/// Decimal amounts: parse, add, multiply, format, compare, powers of ten.
fn amounts() {
    for text in ["1999", "12345678901234567890123456789"] {
        row(
            &format!("parse {} digits", text.len()),
            time(|| black_box(text).parse::<BigInt>()),
            time(|| black_box(text).parse::<NumBigInt>()),
        );
    }
    let (amount, amount_num) = (BigInt::from(199_999), NumBigInt::from(199_999));
    let (price, price_num) = (BigInt::from(-1_250), NumBigInt::from(-1_250));
    row(
        "add small",
        time(|| black_box(&amount) + black_box(&price)),
        time(|| black_box(&amount_num) + black_box(&price_num)),
    );
    row(
        "multiply small",
        time(|| black_box(&amount) * black_box(&price)),
        time(|| black_box(&amount_num) * black_box(&price_num)),
    );
    row(
        "to_string small",
        time(|| black_box(&amount).to_string()),
        time(|| black_box(&amount_num).to_string()),
    );
    row(
        "clone small",
        time(|| black_box(&amount).clone()),
        time(|| black_box(&amount_num).clone()),
    );
    row(
        "compare small",
        time(|| black_box(&amount).cmp(black_box(&price))),
        time(|| black_box(&amount_num).cmp(black_box(&price_num))),
    );
    let (ten, ten_num) = (BigInt::from(10), NumBigInt::from(10));
    for exponent in [18, 40, 340] {
        row(
            &format!("10^{exponent}"),
            time(|| black_box(&ten).pow(black_box(exponent))),
            time(|| black_box(&ten_num).pow(black_box(exponent))),
        );
    }
}

/// A decimal division: 199999 × 10^20 / -1250 (two limbs by one), then the trailing
/// zeros stripped (division by ten while the remainder is zero).
fn division() -> Result<(), Box<dyn Error>> {
    let (amount, amount_num) = (BigInt::from(199_999), NumBigInt::from(199_999));
    let (price, price_num) = (BigInt::from(-1_250), NumBigInt::from(-1_250));
    let (ten, ten_num) = (BigInt::from(10), NumBigInt::from(10));
    let (power, power_num) = (ten.pow(20), ten_num.pow(20));
    let (scaled, scaled_num) = (&amount * &power, &amount_num * &power_num);
    row(
        "scale up: small × 10^20",
        time(|| black_box(&amount) * black_box(&power)),
        time(|| black_box(&amount_num) * black_box(&power_num)),
    );
    row(
        "div_rem 2 limbs / small",
        time(|| black_box(&scaled).checked_div_rem(black_box(&price))),
        time(|| black_box(&scaled_num).div_rem(black_box(&price_num))),
    );
    let ten_constant = NonZeroU64::new(10).ok_or("ten")?;
    row(
        "strip 20 zeros, div_rem_u64",
        time(|| strip_constant(black_box(&scaled), ten_constant)),
        time(|| strip_num(black_box(&scaled_num), &ten_num)),
    );
    row(
        "strip 20 zeros, checked_div_rem",
        time(|| strip(black_box(&scaled), &ten)),
        time(|| strip_num(black_box(&scaled_num), &ten_num)),
    );
    row(
        "to_string 2 limbs",
        time(|| black_box(&scaled).to_string()),
        time(|| black_box(&scaled_num).to_string()),
    );
    row(
        "to_f64 2 limbs",
        time(|| black_box(&scaled).to_f64()),
        time(|| black_box(&scaled_num).to_f64()),
    );
    Ok(())
}

/// IBAN: a 30-digit number modulo 97.
fn iban() -> Result<(), Box<dyn Error>> {
    let iban = "200410100505000132202606152714";
    let (value, value_num) = (iban.parse::<BigInt>()?, iban.parse::<NumBigInt>()?);
    let (modulus, modulus_num) = (BigInt::from(97), NumBigInt::from(97));
    row(
        "IBAN: mod 97",
        time(|| black_box(&value).checked_rem(black_box(&modulus))),
        time(|| black_box(&value_num) % black_box(&modulus_num)),
    );
    row(
        "IBAN: v × 10 + digit, 30 digits",
        time(|| fold_digits(black_box(iban))),
        time(|| fold_digits_num(black_box(iban))),
    );
    row(
        "IBAN: v *= 10; v += digit, 30 digits",
        time(|| assign_digits(black_box(iban))),
        time(|| assign_digits_num(black_box(iban))),
    );
    Ok(())
}

/// Divides by ten while the remainder is zero.
#[inline(never)]
fn strip(value: &BigInt, ten: &BigInt) -> BigInt {
    let mut v = value.clone();
    while let Some((q, r)) = v.checked_div_rem(ten) {
        if !r.is_zero() {
            break;
        }
        v = q;
    }
    v
}

#[inline(never)]
fn strip_constant(value: &BigInt, ten: NonZeroU64) -> BigInt {
    let mut v = value.clone();
    loop {
        let (q, r) = v.div_rem_u64(ten);
        if !r.is_zero() {
            break v;
        }
        v = q;
    }
}

#[inline(never)]
fn strip_num(value: &NumBigInt, ten: &NumBigInt) -> NumBigInt {
    let mut v = value.clone();
    loop {
        let (q, r) = v.div_rem(ten);
        if r != NumBigInt::ZERO {
            break v;
        }
        v = q;
    }
}

/// The value of ASCII decimal characters, one at a time.
#[inline(never)]
fn fold_digits(text: &str) -> BigInt {
    text.bytes()
        .fold(BigInt::ZERO, |v, d| v * 10_u8 + (d - b'0'))
}

/// The same with the assigning operators.
#[inline(never)]
fn assign_digits(text: &str) -> BigInt {
    let mut v = BigInt::ZERO;
    for d in text.bytes() {
        v *= 10_u8;
        v += d - b'0';
    }
    v
}

#[inline(never)]
fn assign_digits_num(text: &str) -> NumBigInt {
    let mut v = NumBigInt::ZERO;
    for d in text.bytes() {
        v *= 10_u8;
        v += d - b'0';
    }
    v
}

#[inline(never)]
fn fold_digits_num(text: &str) -> NumBigInt {
    text.bytes()
        .fold(NumBigInt::ZERO, |v, d| v * 10_u8 + (d - b'0'))
}

/// Exact double formatting: integers of up to about 1100 bits (18 limbs).
fn double_formatting() {
    let (ten, ten_num) = (BigInt::from(10), NumBigInt::from(10));
    for size in [4, 18] {
        let (wide, wide_num) = both(size, 7);
        let (half, half_num) = both(size / 2, 11);
        row(
            &format!("{size} limbs × 10 in place"),
            time(|| {
                let mut v = black_box(&wide).clone();
                v *= &ten;
                v
            }),
            time(|| {
                let mut v = black_box(&wide_num).clone();
                v *= &ten_num;
                v
            }),
        );
        row(
            &format!("{size} limbs << 1, add, compare"),
            time(|| (black_box(&wide) << 1_u8) + black_box(&half) > wide),
            time(|| (black_box(&wide_num) << 1_u8) + black_box(&half_num) > wide_num),
        );
        row(
            &format!("{size} limbs div_rem {} limbs", size / 2),
            time(|| black_box(&wide).checked_div_rem(black_box(&half))),
            time(|| black_box(&wide_num).div_rem(black_box(&half_num))),
        );
        row(
            &format!("{size} limbs div_floor {} limbs", size / 2),
            time(|| black_box(&wide).checked_div_floor(black_box(&half))),
            time(|| black_box(&wide_num).div_floor(black_box(&half_num))),
        );
    }
}

/// Larger numbers: products, quotients and decimal strings.
fn large() -> Result<(), Box<dyn Error>> {
    for size in [8, 16, 32, 64, 128] {
        let (left, left_num) = both(size, 3);
        let (right, right_num) = both(size, 5);
        row(
            &format!("multiply {size} × {size} limbs"),
            time(|| black_box(&left) * black_box(&right)),
            time(|| black_box(&left_num) * black_box(&right_num)),
        );
        let (product, product_num) = (&left * &right, &left_num * &right_num);
        row(
            &format!("divide {} by {size} limbs", 2 * size),
            time(|| black_box(&product).checked_div_rem(black_box(&right))),
            time(|| black_box(&product_num).div_rem(black_box(&right_num))),
        );
    }
    for digits in [100, 1000, 10_000] {
        let text: String = (0..digits)
            .map(|k| char::from(b'1' + u8::try_from(k % 9).unwrap_or(0)))
            .collect();
        let (value, value_num) = (text.parse::<BigInt>()?, text.parse::<NumBigInt>()?);
        row(
            &format!("parse {digits} digits"),
            time(|| black_box(&text).parse::<BigInt>()),
            time(|| black_box(&text).parse::<NumBigInt>()),
        );
        row(
            &format!("to_string {digits} digits"),
            time(|| black_box(&value).to_string()),
            time(|| black_box(&value_num).to_string()),
        );
    }
    Ok(())
}
