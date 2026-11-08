# einvoice-bigint

Arbitrary-precision signed integers in safe Rust, with no dependency and no assembly: no
`unsafe`, no `asm!`, and no 128-bit division (which the compiler would turn into a call to
its runtime library). Every quotient digit comes from multiplications by a precomputed
reciprocal (Möller and Granlund, "Improved division by invariant integers", 2011); divisors
of two limbs or more use Knuth's algorithm D. The crate is `no_std` (it needs `alloc`) and
builds for `wasm32-unknown-unknown`.

```rust
use einvoice_bigint::BigInt;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ISO 7064 MOD 97-10, the IBAN check: the remainder of a valid IBAN is 1.
    let iban: BigInt = "3214282912345698765432161182".parse()?;
    assert_eq!(iban.checked_rem(&BigInt::from(97)), Some(BigInt::from(1)));

    let big = BigInt::from(10).pow(30) - 7_u8;
    assert_eq!(big.to_string(), "999999999999999999999999999993");
    assert_eq!(big.to_f64(), 1e30);
    Ok(())
}
```

Values that fit in an `i64` are stored inline and computed with machine arithmetic; larger
ones are a sign and 64-bit limbs. The API:

| | |
|---|---|
| arithmetic | `+`, `-`, `*`, unary `-` on values and references, with a `BigInt` or a primitive integer on the right, and `+=`, `-=`, `*=`; `abs`, `pow`, `gcd` |
| division | `checked_div_rem`, `checked_div`, `checked_rem` (truncated: the remainder has the sign of the dividend), `checked_div_rem_floor`, `checked_div_floor`, `checked_mod_floor` (floored: the remainder has the sign of the divisor), `checked_div_ceil`; `None` for a zero divisor, so no division panics, and there is no `/` or `%` operator. `div_rem_u64` divides by a `NonZeroU64` |
| bits | `<<`, `>>` (rounding towards negative infinity) by any unsigned integer type, `bits`, `is_even`, `is_odd` |
| comparison | `Eq`, `Ord`, `Hash` (numeric), `sign`, `signum`, `is_zero`, `is_one`, `is_negative`, `is_positive` |
| decimal strings | `Display` (with the formatter's sign, width and zero padding), `Debug` (the same), `FromStr` |
| conversions | `From` every primitive integer type; `to_i32`, `to_u32`, `to_i64`, `to_u64`, `to_i128`, `to_u128`, `to_usize` and `TryFrom<&BigInt>` for every primitive integer type; `from_f64` (truncation towards zero, exact; `None` for NaN and the infinities); `to_f64` (the nearest double, ties to even, an infinity beyond the range); `from_u64_digits`, `to_u64_digits` |

The results, the accepted strings (an optional sign, ASCII digits, underscores after the
first digit) and the conversions are those of `num-bigint` 0.5, which the tests compare
with on random and edge-case values (`tests/differential.rs`), besides known answers
(`tests/known.rs`). One exception: `to_f64` always rounds correctly, and `num-bigint` 0.5.1
does not (it rounds 2^128 + 2^75 + 2^63 down to 2^128, the nearest double being
2^128 + 2^76), so the tests check it against the standard library's parser.

The `compare` example times both crates on the sizes a decimal arithmetic sees:

```sh
cargo build --release -p einvoice-bigint --example compare
taskset -c 2 target/release/examples/compare
```

Multiplication is schoolbook, the decimal conversions quadratic: the crate is meant for
numbers of up to a few thousand digits.

This crate is part of [einvoice-validator](https://github.com/einvoice-validator/einvoice-validator),
which validates EN 16931 and Peppol BIS Billing 3.0 e-invoices against the official
validation artefacts in pure Rust. Documentation: <https://docs.rs/einvoice-bigint>.

## Licence

MIT (`LICENSES/MIT.txt`).
