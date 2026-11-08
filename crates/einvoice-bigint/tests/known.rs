//! Known answers, computed independently of any big-integer library.

#![cfg(test)]

use std::num::NonZeroU64;

use einvoice_bigint::{BigInt, Sign};

fn big(text: &str) -> BigInt {
    text.parse().unwrap()
}

#[test]
fn powers_of_two_and_ten() {
    assert_eq!(
        BigInt::from(2).pow(128).to_string(),
        "340282366920938463463374607431768211456"
    );
    assert_eq!(
        (BigInt::ONE << 128_u32).to_string(),
        "340282366920938463463374607431768211456"
    );
    assert_eq!(
        BigInt::from(-2).pow(127).to_string(),
        "-170141183460469231731687303715884105728"
    );
    assert_eq!(BigInt::from(10).pow(19).to_string(), "10000000000000000000");
    assert_eq!(
        BigInt::from(10).pow(38).to_string(),
        format!("1{}", "0".repeat(38))
    );
    assert_eq!(
        BigInt::from(10).pow(400).to_string(),
        format!("1{}", "0".repeat(400))
    );
    assert_eq!(BigInt::from(0).pow(0), BigInt::ONE);
    assert_eq!(BigInt::from(0).pow(5), BigInt::ZERO);
    assert_eq!(BigInt::from(-1).pow(u32::MAX), BigInt::from(-1));
    // 5^-k expansions: 5^27 = 7450580596923828125.
    assert_eq!(BigInt::from(5).pow(27).to_string(), "7450580596923828125");
    assert_eq!(BigInt::from(5).pow(28).to_string(), "37252902984619140625");
}

#[test]
fn factorial_of_100() {
    let mut f = BigInt::ONE;
    for i in 1_u32..=100 {
        f *= i;
    }
    assert_eq!(
        f.to_string(),
        "93326215443944152681699238856266700490715968264381621468592963895217599993229915\
         608941463976156518286253697920827223758251185210916864000000000000000000000000"
    );
    // 100! / 98! = 9900.
    let mut g = BigInt::ONE;
    for i in 1_u32..=98 {
        g *= i;
    }
    assert_eq!(
        f.checked_div_rem(&g),
        Some((BigInt::from(9900), BigInt::ZERO))
    );
    assert_eq!(f.bits(), 525);
}

#[test]
fn iban_check_digits() {
    // ISO 13616 / ISO 7064 MOD 97-10 on the rearranged numeric form of published
    // example IBANs (letters as 10 to 35, the first four characters moved to the end):
    // the remainder of a valid IBAN is 1.
    let ninety_seven = NonZeroU64::new(97).unwrap();
    for digits in [
        "3214282912345698765432161182",   // GB82 WEST 1234 5698 7654 32
        "370400440532013000131489",       // DE89 3704 0044 0532 0130 00
        "200410100505000132202606152714", // FR14 2004 1010 0505 0001 3M02 606
        "096123456769111471",             // BE71 0961 2345 6769
    ] {
        let n = big(digits);
        let (q, r) = n.div_rem_u64(ninety_seven);
        assert_eq!(r, BigInt::ONE, "{digits}");
        assert_eq!(&(&q * 97_u8) + &r, n);
        assert_eq!(n.checked_rem(&BigInt::from(97)), Some(BigInt::ONE));
        assert_eq!(n.checked_mod_floor(&BigInt::from(97)), Some(BigInt::ONE));
        assert_eq!(
            (-&n).checked_mod_floor(&BigInt::from(97)),
            Some(BigInt::from(96))
        );
    }
}

#[test]
fn truncated_and_floored_signs() {
    let cases: [(i64, i64, i64, i64, i64, i64, i64); 4] = [
        // a, b, trunc q, trunc r, floor q, floor r, ceil q
        (7, 2, 3, 1, 3, 1, 4),
        (-7, 2, -3, -1, -4, 1, -3),
        (7, -2, -3, 1, -4, -1, -3),
        (-7, -2, 3, -1, 3, -1, 4),
    ];
    for (a, b, tq, tr, fq, fr, cq) in cases {
        let (a, b) = (BigInt::from(a), BigInt::from(b));
        assert_eq!(
            a.checked_div_rem(&b),
            Some((BigInt::from(tq), BigInt::from(tr)))
        );
        assert_eq!(
            a.checked_div_rem_floor(&b),
            Some((BigInt::from(fq), BigInt::from(fr)))
        );
        assert_eq!(a.checked_div_ceil(&b), Some(BigInt::from(cq)));
        // The same signs with large operands: a × 2^100 + …
        let scale = BigInt::ONE << 100_u32;
        let (big_a, big_b) = (&a * &scale, &b * &scale);
        assert_eq!(
            big_a.checked_div_rem(&big_b),
            Some((BigInt::from(tq), &BigInt::from(tr) * &scale))
        );
        assert_eq!(
            big_a.checked_div_rem_floor(&big_b),
            Some((BigInt::from(fq), &BigInt::from(fr) * &scale))
        );
    }
    assert_eq!(BigInt::from(5).checked_div_rem(&BigInt::ZERO), None);
    assert_eq!(
        BigInt::ZERO.checked_div_rem(&BigInt::from(5)),
        Some((BigInt::ZERO, BigInt::ZERO))
    );
}

#[test]
fn the_edges_of_i64() {
    let min = BigInt::from(i64::MIN);
    let max = BigInt::from(i64::MAX);
    let two_63 = big("9223372036854775808");
    assert_eq!(-&min, two_63);
    assert_eq!(-two_63.clone(), min);
    assert_eq!(min.abs(), two_63);
    assert_eq!(&max + 1_u8, two_63);
    assert_eq!(&two_63 - 1_u8, max);
    assert_eq!(&min - 1_u8, big("-9223372036854775809"));
    assert_eq!(
        min.checked_div_rem(&BigInt::from(-1)),
        Some((two_63.clone(), BigInt::ZERO))
    );
    assert_eq!(
        min.checked_div_rem(&two_63),
        Some((BigInt::from(-1), BigInt::ZERO))
    );
    assert_eq!(&min * -1_i64, two_63);
    // In place: 2^63 × -1 must come back to the inline -2^63.
    let mut x = two_63.clone();
    x *= -1_i64;
    assert_eq!(x, min);
    assert_eq!(x.to_i64(), Some(i64::MIN));
    assert_eq!(&min * &min, big("85070591730234615865843651857942052864"));
    assert_eq!(two_63.to_i64(), None);
    assert_eq!(two_63.to_u64(), Some(1 << 63));
    assert_eq!(min.to_u64(), None);
    assert_eq!(BigInt::from(u64::MAX).to_string(), "18446744073709551615");
    assert_eq!(
        BigInt::from(i128::MIN).to_string(),
        "-170141183460469231731687303715884105728"
    );
    assert_eq!(
        BigInt::from(u128::MAX).to_string(),
        "340282366920938463463374607431768211455"
    );
    assert_eq!(BigInt::from(i128::MIN).to_i128(), Some(i128::MIN));
    assert_eq!((BigInt::from(i128::MIN) - 1_u8).to_i128(), None);
    assert_eq!(BigInt::from(u128::MAX).to_u128(), Some(u128::MAX));
    assert_eq!((BigInt::from(u128::MAX) + 1_u8).to_u128(), None);
}

#[test]
fn knuth_add_back() {
    // The 64-bit form of the "adding back required" case of Hacker's Delight's
    // divmnu tests: the estimated quotient digit 2^64 - 1 is one too large.
    // u = (2^63 - 1)·2^192 + 2^63·2^128, v = 2^191 + 1.
    let u = BigInt::from_u64_digits(Sign::Plus, &[0, 0, 1 << 63, (1 << 63) - 1]);
    let v = BigInt::from_u64_digits(Sign::Plus, &[1, 0, 1 << 63]);
    let (q, r) = u.checked_div_rem(&v).unwrap();
    assert_eq!(q.to_u64_digits(), (Sign::Plus, vec![u64::MAX - 1]));
    assert_eq!(
        r.to_u64_digits(),
        (Sign::Plus, vec![2, u64::MAX, (1 << 63) - 1])
    );
    // The other add-back case: u = 3 + 2^191, v = 1 + 2^189.
    let u = BigInt::from_u64_digits(Sign::Plus, &[3, 0, 1 << 63]);
    let v = BigInt::from_u64_digits(Sign::Plus, &[1, 0, 1 << 61]);
    assert_eq!(
        u.checked_div_rem(&v),
        Some((
            BigInt::from(3),
            BigInt::from_u64_digits(Sign::Plus, &[0, 0, 1 << 61])
        ))
    );
    // A quotient digit that must not be treated as signed.
    let u = BigInt::from_u64_digits(Sign::Plus, &[0, 0xffff_ffff_ffff_fffe, 1 << 63]);
    let v = BigInt::from_u64_digits(Sign::Plus, &[u64::MAX, 1 << 63]);
    let (q, r) = u.checked_div_rem(&v).unwrap();
    assert_eq!(q.to_u64_digits(), (Sign::Plus, vec![u64::MAX]));
    assert_eq!(
        r.to_u64_digits(),
        (Sign::Plus, vec![u64::MAX, 0x7fff_ffff_ffff_ffff])
    );
}

#[test]
fn strings() {
    assert_eq!(big("0").to_string(), "0");
    assert_eq!(big("-0").to_string(), "0");
    assert_eq!(big("+00012").to_string(), "12");
    assert_eq!(big("1_000_000").to_string(), "1000000");
    assert_eq!(big("-1__2_").to_string(), "-12");
    let long = format!("-{}", "9".repeat(1000));
    assert_eq!(big(&long).to_string(), long);
    assert_eq!(big(&long) - 1_u8, big(&format!("-1{}", "0".repeat(1000))));
    // Long values with runs of zeros, where the split conversion pads its parts.
    for text in [
        format!("1{}", "0".repeat(5000)),
        format!("1{}1", "0".repeat(4000)),
        format!("7{}3{}9", "0".repeat(1500), "0".repeat(1200)),
        format!("{}{}", "9".repeat(800), "0".repeat(3000)),
        "9".repeat(20_000),
    ] {
        let value = big(&text);
        assert_eq!(value.to_string(), text);
        assert_eq!((-&value).to_string(), format!("-{text}"));
    }
    assert_eq!(
        BigInt::from(10).pow(3000).to_string(),
        format!("1{}", "0".repeat(3000))
    );
    for bad in ["", "-", "+"] {
        assert_eq!(
            bad.parse::<BigInt>().unwrap_err().to_string(),
            "cannot parse integer from empty string"
        );
        assert!(bad.parse::<BigInt>().unwrap_err().is_empty());
    }
    for bad in [
        "++1", "-+1", "+-1", "--1", "_1", "-_1", "1a", " 1", "1 ", "1.0", "1e3", "١",
    ] {
        assert_eq!(
            bad.parse::<BigInt>().unwrap_err().to_string(),
            "invalid digit found in string",
            "{bad}"
        );
    }
    assert_eq!(format!("{:+}", BigInt::from(5)), "+5");
    assert_eq!(format!("{:>6}", BigInt::from(-5)), "    -5");
    assert_eq!(format!("{:06}", BigInt::from(-5)), "-00005");
    assert_eq!(
        format!("{:?}", big("123456789012345678901234567890")),
        "123456789012345678901234567890"
    );
}

#[test]
fn doubles() {
    assert_eq!(BigInt::from_f64(0.1), Some(BigInt::ZERO));
    assert_eq!(BigInt::from_f64(-0.0), Some(BigInt::ZERO));
    assert_eq!(BigInt::from_f64(-2.7), Some(BigInt::from(-2)));
    assert_eq!(BigInt::from_f64(f64::NAN), None);
    assert_eq!(BigInt::from_f64(f64::INFINITY), None);
    assert_eq!(BigInt::from_f64(f64::NEG_INFINITY), None);
    assert_eq!(
        BigInt::from_f64(1e23).unwrap().to_string(),
        "99999999999999991611392"
    );
    assert_eq!(
        BigInt::from_f64(-2_f64.powi(64)).unwrap().to_string(),
        "-18446744073709551616"
    );
    assert_eq!(
        BigInt::from_f64(f64::MAX).unwrap(),
        (BigInt::from((1_u64 << 53) - 1)) << 971_u32
    );
    assert_eq!(big("99999999999999991611392").to_f64(), 1e23);
    // Halfway between 2^53 and 2^53 + 2: ties to even.
    assert_eq!(big("9007199254740993").to_f64(), 9_007_199_254_740_992.0);
    assert_eq!(big("9007199254740995").to_f64(), 9_007_199_254_740_996.0);
    // A tie broken by a bit far below: rounds up.
    let tie_plus = (BigInt::from((1_u64 << 53) + 1) << 200_u32) + 1_u8;
    assert_eq!(tie_plus.to_f64(), 4_503_599_627_370_497.0 * 2_f64.powi(201));
    // The top of the range and beyond.
    let max = BigInt::from_f64(f64::MAX).unwrap();
    assert_eq!(max.to_f64(), f64::MAX);
    let halfway = &max + &(BigInt::ONE << 970_u32);
    assert_eq!(halfway.to_f64(), f64::INFINITY);
    assert_eq!((&halfway - 1_u8).to_f64(), f64::MAX);
    assert_eq!((-(BigInt::ONE << 1024_u32)).to_f64(), f64::NEG_INFINITY);
    assert_eq!((BigInt::ONE << 5000_u32).to_f64(), f64::INFINITY);
    assert_eq!(
        BigInt::from(i64::MIN).to_f64(),
        -9_223_372_036_854_775_808.0
    );
}

/// x = 2^128 + 2^75 + 2^63 is above the midpoint 2^128 + 2^75 between the doubles
/// 2^128 and 2^128 + 2^76, so it rounds up. num-bigint 0.5.1 gives 2^128: its scan for
/// the bits below the top 64 counts the bits of each limb from the bits left, which no
/// longer matches the limbs once a limb has been taken in part, and so it looks at bit 0
/// of the lowest limb instead of bit 63.
#[test]
fn to_double_where_num_bigint_is_not_correctly_rounded() {
    let x = BigInt::from_u64_digits(Sign::Plus, &[1 << 63, 1 << 11, 1]);
    assert_eq!(x.to_string(), "340282366920938501251529842425784696832");
    assert_eq!(x.to_f64(), 2_f64.powi(128) + 2_f64.powi(76));
    assert_eq!(
        x.to_f64(),
        "340282366920938501251529842425784696832"
            .parse::<f64>()
            .unwrap()
    );
    assert_eq!((-&x).to_f64(), -(2_f64.powi(128) + 2_f64.powi(76)));
}

#[test]
fn greatest_common_divisors() {
    assert_eq!(BigInt::ZERO.gcd(&BigInt::ZERO), BigInt::ZERO);
    assert_eq!(BigInt::from(-12).gcd(&BigInt::from(18)), BigInt::from(6));
    assert_eq!(
        BigInt::from(i64::MIN).gcd(&BigInt::ZERO),
        big("9223372036854775808")
    );
    let a = BigInt::from(2).pow(200) * 3_u8;
    let b = BigInt::from(2).pow(150) * 9_u8;
    assert_eq!(a.gcd(&b), BigInt::from(2).pow(150) * 3_u8);
}

#[test]
fn shifts_round_towards_negative_infinity() {
    assert_eq!(BigInt::from(-1) >> 1_u8, BigInt::from(-1));
    assert_eq!(BigInt::from(-3) >> 1_u8, BigInt::from(-2));
    assert_eq!(BigInt::from(-4) >> 1_u8, BigInt::from(-2));
    assert_eq!(BigInt::from(-1) >> 10_000_u32, BigInt::from(-1));
    assert_eq!(BigInt::from(5) >> 10_000_u32, BigInt::ZERO);
    let x = -(BigInt::ONE << 200_u32) - 1_u8;
    assert_eq!(&x >> 200_u32, BigInt::from(-2));
    assert_eq!(&x >> 1_000_u32, BigInt::from(-1));
    assert_eq!(BigInt::from(-5) << 100_u32, -(BigInt::from(5) << 100_u32));
}
