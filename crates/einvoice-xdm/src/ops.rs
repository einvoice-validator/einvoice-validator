//! Operators and functions on atomic values: arithmetic, comparisons, `fn:number`,
//! the effective boolean value, `fn:round`, `fn:floor`, `fn:ceiling`, `fn:abs` and
//! `fn:sum`, with the type promotions of XPath 2.0 as Saxon-HE 12.10 applies them.

use std::cmp::Ordering;

use crate::atomic::{AtomicType, AtomicValue};
use crate::decimal::Decimal;
use crate::double::{parse_double, trim_xml_whitespace};
use crate::error::{ErrorCode, Result, XdmError, err};
use crate::integer::{Integer, Quotient};

/// An arithmetic operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArithmeticOp {
    /// `+`
    Add,
    /// `-`
    Subtract,
    /// `*`
    Multiply,
    /// `div`
    Divide,
    /// `idiv`
    IntegerDivide,
    /// `mod`
    Modulo,
}

/// A comparison operator, used both as a general (`=`) and a value (`eq`) comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComparisonOp {
    /// `=` / `eq`
    Equal,
    /// `!=` / `ne`
    NotEqual,
    /// `<` / `lt`
    Less,
    /// `<=` / `le`
    LessOrEqual,
    /// `>` / `gt`
    Greater,
    /// `>=` / `ge`
    GreaterOrEqual,
}

impl ComparisonOp {
    /// Whether an ordering satisfies the operator.
    #[must_use]
    pub fn holds(self, ordering: Ordering) -> bool {
        match self {
            Self::Equal => ordering == Ordering::Equal,
            Self::NotEqual => ordering != Ordering::Equal,
            Self::Less => ordering == Ordering::Less,
            Self::LessOrEqual => ordering != Ordering::Greater,
            Self::Greater => ordering == Ordering::Greater,
            Self::GreaterOrEqual => ordering != Ordering::Less,
        }
    }
}

/// A numeric operand after promotion.
enum Number<'a> {
    Integer(&'a Integer),
    Decimal(&'a Decimal),
    Double(f64),
}

/// Converts an `xs:untypedAtomic` operand to `xs:double` (arithmetic and function
/// arguments), leaves numbers alone, refuses the rest.
fn numeric_operand<'a>(value: &'a AtomicValue, what: &str) -> Result<Number<'a>> {
    match value {
        AtomicValue::Integer(i) => Ok(Number::Integer(i)),
        AtomicValue::Decimal(d) => Ok(Number::Decimal(d)),
        AtomicValue::Double(d) => Ok(Number::Double(*d)),
        AtomicValue::UntypedAtomic(s) => parse_double(s).map(Number::Double),
        other => err(
            ErrorCode::Xpty0004,
            format!("{what}: {} is not numeric", other.atomic_type()),
        ),
    }
}

fn as_double(n: &Number<'_>) -> f64 {
    match n {
        Number::Integer(i) => i.to_f64(),
        Number::Decimal(d) => d.to_f64(),
        Number::Double(d) => *d,
    }
}

/// The decimal operand Saxon uses for an integer or decimal in decimal arithmetic.
fn as_decimal(n: &Number<'_>) -> Option<Decimal> {
    match n {
        Number::Integer(i) => Some(i.as_decimal_operand()),
        Number::Decimal(d) => Some((*d).clone()),
        Number::Double(_) => None,
    }
}

/// `a op b` for two atomic values (`xs:untypedAtomic` becomes `xs:double`).
///
/// # Errors
///
/// `XPTY0004` for non-numeric operands, `FORG0001` for an untyped value that is not a
/// number, `FOAR0001` for a division by zero, `FOAR0002` for `idiv` of NaN or
/// infinity, `Unsupported` for date and time arithmetic.
pub fn arithmetic(op: ArithmeticOp, a: &AtomicValue, b: &AtomicValue) -> Result<AtomicValue> {
    let both_dates = matches!(
        (a, b),
        (AtomicValue::Date(_), AtomicValue::Date(_))
            | (AtomicValue::DateTime(_), AtomicValue::DateTime(_))
    );
    if both_dates && op == ArithmeticOp::Subtract {
        return err(
            ErrorCode::Unsupported,
            "date and time subtraction (durations) is not supported",
        );
    }
    let left = numeric_operand(a, "arithmetic")?;
    let right = numeric_operand(b, "arithmetic")?;
    match (&left, &right) {
        (Number::Integer(li), Number::Integer(ri)) => integer_arithmetic(op, li, ri),
        (Number::Double(_), _) | (_, Number::Double(_)) => {
            double_arithmetic(op, as_double(&left), as_double(&right))
        }
        _ => match (as_decimal(&left), as_decimal(&right)) {
            (Some(ld), Some(rd)) => decimal_arithmetic(op, &ld, &rd),
            _ => err(ErrorCode::Xpty0004, "arithmetic on non-numeric values"),
        },
    }
}

fn integer_arithmetic(op: ArithmeticOp, i: &Integer, j: &Integer) -> Result<AtomicValue> {
    Ok(match op {
        ArithmeticOp::Add => AtomicValue::Integer(i.add(j)),
        ArithmeticOp::Subtract => AtomicValue::Integer(i.sub(j)),
        ArithmeticOp::Multiply => AtomicValue::Integer(i.mul(j)),
        ArithmeticOp::Divide => match i.div(j)? {
            Quotient::Integer(q) => AtomicValue::Integer(q),
            Quotient::Decimal(q) => AtomicValue::Decimal(q),
        },
        ArithmeticOp::IntegerDivide => AtomicValue::Integer(i.idiv(j)?),
        ArithmeticOp::Modulo => AtomicValue::Integer(i.rem(j)?),
    })
}

fn decimal_arithmetic(op: ArithmeticOp, p: &Decimal, q: &Decimal) -> Result<AtomicValue> {
    Ok(match op {
        ArithmeticOp::Add => AtomicValue::Decimal(p.add(q)),
        ArithmeticOp::Subtract => AtomicValue::Decimal(p.sub(q)),
        ArithmeticOp::Multiply => AtomicValue::Decimal(p.mul(q)),
        ArithmeticOp::Divide => AtomicValue::Decimal(p.checked_div(q)?),
        ArithmeticOp::IntegerDivide => {
            AtomicValue::Integer(Integer::from_bigint(p.checked_div_trunc(q)?))
        }
        ArithmeticOp::Modulo => AtomicValue::Decimal(p.checked_rem(q)?),
    })
}

fn double_arithmetic(op: ArithmeticOp, x: f64, y: f64) -> Result<AtomicValue> {
    Ok(AtomicValue::Double(match op {
        ArithmeticOp::Add => x + y,
        ArithmeticOp::Subtract => x - y,
        ArithmeticOp::Multiply => x * y,
        ArithmeticOp::Divide => x / y,
        // Java's % on doubles is the IEEE remainder of truncated division, as Rust's.
        ArithmeticOp::Modulo => x % y,
        ArithmeticOp::IntegerDivide => {
            if y == 0.0 {
                return err(ErrorCode::Foar0001, "Integer division by zero");
            }
            if !x.is_finite() {
                return err(
                    ErrorCode::Foar0002,
                    "First operand of idiv is NaN or infinity",
                );
            }
            if y.is_nan() {
                return err(ErrorCode::Foar0002, "Second operand of idiv is NaN");
            }
            return Integer::from_f64(x / y).map(AtomicValue::Integer);
        }
    }))
}

/// Unary minus.
///
/// # Errors
///
/// `XPTY0004` for a non-numeric operand, `FORG0001` for an untyped value that is not
/// a number.
pub fn negate(a: &AtomicValue) -> Result<AtomicValue> {
    Ok(match numeric_operand(a, "unary minus")? {
        Number::Integer(i) => AtomicValue::Integer(i.neg()),
        Number::Decimal(d) => AtomicValue::Decimal(d.neg()),
        Number::Double(d) => AtomicValue::Double(-d),
    })
}

/// Compares two values that are known to be comparable (no untyped value, same
/// family), NaN excluded.
fn compare_same_family(
    a: &AtomicValue,
    b: &AtomicValue,
    implicit_timezone: i16,
) -> Result<Ordering> {
    use AtomicValue as V;
    Ok(match (a, b) {
        (
            V::String(x) | V::UntypedAtomic(x) | V::AnyUri(x),
            V::String(y) | V::UntypedAtomic(y) | V::AnyUri(y),
        ) => {
            // Rust orders strings by code point, as Saxon's codepoint collation.
            x.cmp(y)
        }
        (V::Boolean(x), V::Boolean(y)) => x.cmp(y),
        (V::Integer(x), V::Integer(y)) => x.cmp(y),
        (V::Integer(x), V::Decimal(y)) => x.as_decimal_operand().cmp(y),
        (V::Decimal(x), V::Integer(y)) => x.cmp(&y.as_decimal_operand()),
        (V::Decimal(x), V::Decimal(y)) => x.cmp(y),
        (V::Date(x), V::Date(y)) => x.compare(y, implicit_timezone),
        (V::DateTime(x), V::DateTime(y)) => x.compare(y, implicit_timezone),
        _ => {
            // Any numeric pair involving a double compares as doubles.
            match (a.numeric_as_f64(), b.numeric_as_f64()) {
                (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Greater),
                _ => {
                    return err(
                        ErrorCode::Xpty0004,
                        format!("Cannot compare {} to {}", a.atomic_type(), b.atomic_type()),
                    );
                }
            }
        }
    })
}

/// Saxon's `Type.isGuaranteedComparable` for the types of this crate.
fn comparable(a: AtomicType, b: AtomicType) -> bool {
    let family = |t: AtomicType| match t {
        AtomicType::String | AtomicType::UntypedAtomic | AtomicType::AnyUri => 0,
        AtomicType::Integer | AtomicType::Decimal | AtomicType::Double => 1,
        AtomicType::Boolean => 2,
        AtomicType::Date => 3,
        AtomicType::DateTime => 4,
    };
    family(a) == family(b)
}

/// Saxon's `ValueComparison.compare`: type check, NaN, then the comparison.
fn compare_checked(
    a: &AtomicValue,
    op: ComparisonOp,
    b: &AtomicValue,
    implicit_timezone: i16,
) -> Result<bool> {
    if !comparable(a.atomic_type(), b.atomic_type()) {
        return err(
            ErrorCode::Xpty0004,
            format!("Cannot compare {} to {}", a.atomic_type(), b.atomic_type()),
        );
    }
    if a.is_nan() || b.is_nan() {
        return Ok(op == ComparisonOp::NotEqual);
    }
    Ok(op.holds(compare_same_family(a, b, implicit_timezone)?))
}

/// A value comparison (`eq`, `ne`, `lt`, `le`, `gt`, `ge`) of two atomic values:
/// `xs:untypedAtomic` operands are compared as strings. `implicit_timezone` is the
/// offset in minutes applied to dates and times without timezone.
///
/// # Errors
///
/// `XPTY0004` when the types are not comparable.
pub fn value_compare(
    a: &AtomicValue,
    op: ComparisonOp,
    b: &AtomicValue,
    implicit_timezone: i16,
) -> Result<bool> {
    let untyped_to_string = |v: &AtomicValue| match v {
        AtomicValue::UntypedAtomic(s) => AtomicValue::String(s.clone()),
        other => other.clone(),
    };
    compare_checked(
        &untyped_to_string(a),
        op,
        &untyped_to_string(b),
        implicit_timezone,
    )
}

/// One pair of a general comparison (`=`, `!=`, `<`, `<=`, `>`, `>=`), as Saxon's
/// `GeneralComparison.compare` evaluates it:
///
/// * two untyped values compare as strings; an untyped value and a string too;
/// * an untyped value and a number: when the number is NaN, true for `!=` and false
///   otherwise, without reading the untyped value (Saxon-HE 12.10's
///   `UntypedNumericComparer.quickCompare`); else see [`untyped_numeric_compare`];
/// * an untyped value and another type: the untyped value is cast to that type;
/// * otherwise a value comparison.
///
/// # Errors
///
/// `XPTY0004` when the types are not comparable, `FORG0001` when an untyped value
/// cannot be cast to the type of the other operand (but not when it meets a NaN).
pub fn general_compare(
    a: &AtomicValue,
    op: ComparisonOp,
    b: &AtomicValue,
    implicit_timezone: i16,
) -> Result<bool> {
    use AtomicValue as V;
    match (a, b) {
        (V::UntypedAtomic(_), V::UntypedAtomic(_)) => compare_checked(a, op, b, implicit_timezone),
        (V::UntypedAtomic(u), other) => untyped_against(u, op, other, implicit_timezone, false),
        (other, V::UntypedAtomic(u)) => untyped_against(u, op, other, implicit_timezone, true),
        _ => compare_checked(a, op, b, implicit_timezone),
    }
}

fn untyped_against(
    untyped: &str,
    op: ComparisonOp,
    other: &AtomicValue,
    implicit_timezone: i16,
    untyped_is_right: bool,
) -> Result<bool> {
    let op_for_untyped_left = if untyped_is_right { inverse(op) } else { op };
    if other.is_numeric() {
        // Saxon-HE 12.10 answers before looking at the untyped value, so that even
        // one that is not a number is not an error. (Saxon 9.9 converted it, and
        // Java's Double.compare made NaN equal to NaN and greater than any number.)
        if other.is_nan() {
            return Ok(op == ComparisonOp::NotEqual);
        }
        let ordering = untyped_numeric_compare(untyped, other)?;
        return Ok(op_for_untyped_left.holds(ordering));
    }
    let converted = match other {
        AtomicValue::String(_) | AtomicValue::AnyUri(_) => {
            AtomicValue::UntypedAtomic(untyped.to_owned())
        }
        _ => AtomicValue::UntypedAtomic(untyped.to_owned()).cast(other.atomic_type())?,
    };
    let (left, right) = if untyped_is_right {
        (other, &converted)
    } else {
        (&converted, other)
    };
    if left.is_nan() || right.is_nan() {
        return Ok(op == ComparisonOp::NotEqual);
    }
    Ok(op.holds(compare_same_family(left, right, implicit_timezone)?))
}

const fn inverse(op: ComparisonOp) -> ComparisonOp {
    match op {
        ComparisonOp::Less => ComparisonOp::Greater,
        ComparisonOp::LessOrEqual => ComparisonOp::GreaterOrEqual,
        ComparisonOp::Greater => ComparisonOp::Less,
        ComparisonOp::GreaterOrEqual => ComparisonOp::LessOrEqual,
        other => other,
    }
}

/// An untyped value compared with a number in a general comparison, as Saxon's
/// `UntypedNumericComparer.quickComparison` does it. The number becomes a double; the
/// untyped value is compared as a 64-bit integer when it is an optional `-` and at
/// most 15 ASCII digits and the number an `xs:integer` within 64 bits, and as a
/// double otherwise. Doubles compare with Java's `Double.compare`, in which `-0`
/// is less than `0` and NaN equals NaN and is greater than everything, unlike the
/// IEEE comparison used everywhere else ([`general_compare`] settles a numeric NaN
/// before calling this, so only an untyped `NaN` reaches that case).
///
/// Saxon-HE 12.10 converts the untyped value again for every pair it compares (9.9
/// cached the conversion); this is the comparison of one pair.
///
/// # Errors
///
/// `FORG0001` when the untyped value is not a number.
pub fn untyped_numeric_compare(untyped: &str, number: &AtomicValue) -> Result<Ordering> {
    let d1 = number
        .numeric_as_f64()
        .ok_or_else(|| XdmError::new(ErrorCode::Xpty0004, "not a number"))?;
    let text = trim_xml_whitespace(untyped);
    let mut simple = true;
    let mut whole_part_length = 0_usize;
    let mut first_digit: Option<u32> = None;
    let mut decimal_points = 0;
    let mut negative = false;
    for c in text.chars() {
        match c {
            '0'..='9' => {
                if first_digit.is_none() {
                    first_digit = c.to_digit(10);
                }
                if decimal_points == 0 {
                    whole_part_length += 1;
                }
            }
            '-' => {
                if negative || whole_part_length > 0 || decimal_points > 0 {
                    simple = false;
                    break;
                }
                negative = true;
            }
            '.' => {
                if decimal_points > 0 {
                    simple = false;
                    break;
                }
                decimal_points = 1;
            }
            _ => {
                simple = false;
                break;
            }
        }
    }
    let simple = simple && first_digit.is_some();
    if simple && (1..=10).contains(&whole_part_length) {
        // Quick decision from the magnitude of the integer part.
        let first = f64::from(first_digit.unwrap_or(0));
        let scale = 10_f64.powi(i32::try_from(whole_part_length - 1).unwrap_or(0));
        let (mut lower, mut upper) = (first * scale, (first + 1.0) * scale);
        if negative {
            (lower, upper) = (-upper, -lower);
        }
        if upper < d1 {
            return Ok(Ordering::Less);
        }
        if lower > d1 {
            return Ok(Ordering::Greater);
        }
    }
    if simple
        && decimal_points == 0
        && whole_part_length <= 15
        && let AtomicValue::Integer(i) = number
        && let Some(n) = i.to_i64()
    {
        let l0: i64 = text.parse().unwrap_or(0);
        return Ok(l0.cmp(&n));
    }
    let d0 = parse_double(untyped)?;
    Ok(java_double_compare(d0, d1))
}

/// Java's `Double.compare`: `-0.0 < 0.0`, NaN equal to itself and above everything.
fn java_double_compare(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a.total_cmp(&b),
    }
}

/// `fn:number` of an atomic value: NaN for anything that does not convert.
#[must_use]
pub fn number(value: &AtomicValue) -> f64 {
    match value {
        AtomicValue::Boolean(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        AtomicValue::Integer(_) | AtomicValue::Decimal(_) | AtomicValue::Double(_) => {
            value.numeric_as_f64().unwrap_or(f64::NAN)
        }
        AtomicValue::String(s) | AtomicValue::UntypedAtomic(s) => {
            parse_double(s).unwrap_or(f64::NAN)
        }
        AtomicValue::AnyUri(_) | AtomicValue::Date(_) | AtomicValue::DateTime(_) => f64::NAN,
    }
}

/// The effective boolean value of a single atomic value.
///
/// # Errors
///
/// `FORG0006` for dates and times.
pub fn effective_boolean_value(value: &AtomicValue) -> Result<bool> {
    match value {
        AtomicValue::Boolean(b) => Ok(*b),
        AtomicValue::String(s) | AtomicValue::UntypedAtomic(s) | AtomicValue::AnyUri(s) => {
            Ok(!s.is_empty())
        }
        AtomicValue::Integer(i) => Ok(i.signum() != 0),
        AtomicValue::Decimal(d) => Ok(!d.is_zero()),
        AtomicValue::Double(d) => Ok(*d != 0.0 && !d.is_nan()),
        AtomicValue::Date(_) | AtomicValue::DateTime(_) => err(
            ErrorCode::Forg0006,
            format!(
                "Effective boolean value is not defined for a value of type {}",
                value.atomic_type()
            ),
        ),
    }
}

/// `fn:round`: nearest integer, ties towards positive infinity; the type is kept.
///
/// # Errors
///
/// `XPTY0004` for a non-numeric argument, `FORG0001` for an untyped value that is
/// not a number.
pub fn round(value: &AtomicValue) -> Result<AtomicValue> {
    Ok(match numeric_operand(value, "round")? {
        Number::Integer(i) => AtomicValue::Integer(i.clone()),
        Number::Decimal(d) => AtomicValue::Decimal(d.round()),
        Number::Double(d) => AtomicValue::Double(round_double(d)),
    })
}

/// `2^63`: Java's `(double) Long.MAX_VALUE`.
const LONG_LIMIT: f64 = 9_223_372_036_854_775_808.0;

/// Saxon's `DoubleValue.round(0)`.
fn round_double(value: f64) -> f64 {
    if value.is_nan() || value.is_infinite() || value == 0.0 {
        return value;
    }
    if value > -LONG_LIMIT && value < LONG_LIMIT {
        if (-0.5..0.0).contains(&value) {
            return -0.0;
        }
        // Math.round: floor(x + 1/2) computed exactly.
        let floor = value.floor();
        return if value - floor >= 0.5 {
            floor + 1.0
        } else {
            floor
        };
    }
    // Saxon's general path, reached only beyond the long range, where every double
    // is already whole: scale by 10, drop the last decimal digit, scale back.
    let factor = 10.0;
    let mut d = (value * factor).abs();
    if d.is_infinite() {
        return value;
    }
    let rem = d % 10.0;
    if rem >= 5.0 {
        d += 10.0 - rem;
    } else {
        d -= rem;
    }
    d /= factor;
    if value < 0.0 { -d } else { d }
}

/// `fn:floor`; the type is kept.
///
/// # Errors
///
/// As [`round`].
pub fn floor(value: &AtomicValue) -> Result<AtomicValue> {
    Ok(match numeric_operand(value, "floor")? {
        Number::Integer(i) => AtomicValue::Integer(i.clone()),
        Number::Decimal(d) => AtomicValue::Decimal(d.floor()),
        Number::Double(d) => AtomicValue::Double(d.floor()),
    })
}

/// `fn:ceiling`; the type is kept.
///
/// # Errors
///
/// As [`round`].
pub fn ceiling(value: &AtomicValue) -> Result<AtomicValue> {
    Ok(match numeric_operand(value, "ceiling")? {
        Number::Integer(i) => AtomicValue::Integer(i.clone()),
        Number::Decimal(d) => AtomicValue::Decimal(d.ceiling()),
        Number::Double(d) => AtomicValue::Double(d.ceil()),
    })
}

/// `fn:abs`; the type is kept.
///
/// # Errors
///
/// As [`round`].
pub fn abs(value: &AtomicValue) -> Result<AtomicValue> {
    Ok(match numeric_operand(value, "abs")? {
        Number::Integer(i) => AtomicValue::Integer(i.abs()),
        Number::Decimal(d) => AtomicValue::Decimal(d.abs()),
        Number::Double(d) => AtomicValue::Double(d.abs()),
    })
}

/// `fn:sum` with one argument: the integer 0 for no values, otherwise the values
/// added from left to right, untyped values as doubles. Like Saxon-HE 12.10
/// (`Sum.SumFnElaborator`, which ignores `SumFold.isFinished`), every value is read
/// and checked, also after a running total that is the double NaN.
///
/// # Errors
///
/// `FORG0006` for a value that is neither numeric nor untyped, `FORG0001` for an
/// untyped value that is not a number, and the errors of [`arithmetic`].
pub fn sum<'a>(values: impl IntoIterator<Item = &'a AtomicValue>) -> Result<AtomicValue> {
    sum_lazily(values.into_iter().map(|v| Ok::<_, XdmError>(v.clone())))
}

/// [`sum`] over values that are produced on demand and may fail: the first error of
/// the input, or of the check of a value, is returned. As in [`sum`], every value is
/// read: a running total that is the double NaN does not stop the reading.
///
/// # Errors
///
/// The errors of the input and those of [`sum`].
pub fn sum_lazily<E: From<XdmError>>(
    values: impl IntoIterator<Item = std::result::Result<AtomicValue, E>>,
) -> std::result::Result<AtomicValue, E> {
    let mut total: Option<AtomicValue> = None;
    for value in values {
        let value = match value? {
            AtomicValue::UntypedAtomic(s) => AtomicValue::Double(parse_double(&s)?),
            v if v.is_numeric() => v,
            other => {
                return Err(XdmError::new(
                    ErrorCode::Forg0006,
                    format!(
                        "Input to sum() contains a value of type {} which is neither numeric, nor a duration",
                        other.atomic_type()
                    ),
                )
                .into());
            }
        };
        total = Some(match total {
            None => value,
            Some(t) => arithmetic(ArithmeticOp::Add, &t, &value)?,
        });
    }
    Ok(total.unwrap_or_else(|| AtomicValue::Integer(Integer::from(0))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(s: &str) -> AtomicValue {
        AtomicValue::Decimal(Decimal::parse(s).unwrap())
    }

    fn untyped(s: &str) -> AtomicValue {
        AtomicValue::UntypedAtomic(s.to_owned())
    }

    #[test]
    fn untyped_arithmetic_is_double() {
        let r = arithmetic(ArithmeticOp::Add, &untyped("0.1"), &untyped("0.2")).unwrap();
        assert_eq!(r.to_string(), "0.30000000000000004");
        let r = arithmetic(ArithmeticOp::Add, &dec("0.1"), &dec("0.2")).unwrap();
        assert_eq!(r.to_string(), "0.3");
    }

    #[test]
    fn saxon_untyped_numeric_quirks() {
        let zero = AtomicValue::Integer(Integer::from(0));
        let dzero = dec("0");
        assert!(general_compare(&untyped("-0"), ComparisonOp::Equal, &zero, 0).unwrap());
        assert!(!general_compare(&untyped("-0"), ComparisonOp::Equal, &dzero, 0).unwrap());
        let nan = AtomicValue::Double(f64::NAN);
        assert!(!general_compare(&untyped("NaN"), ComparisonOp::Equal, &nan, 0).unwrap());
        assert!(!general_compare(&untyped("5"), ComparisonOp::Less, &nan, 0).unwrap());
        assert!(!general_compare(&nan, ComparisonOp::Greater, &untyped("5"), 0).unwrap());
        assert!(!general_compare(&untyped("abc"), ComparisonOp::Equal, &nan, 0).unwrap());
        assert!(general_compare(&untyped("abc"), ComparisonOp::NotEqual, &nan, 0).unwrap());
        // An untyped NaN against a number still compares with Double.compare.
        assert!(general_compare(&untyped("NaN"), ComparisonOp::Greater, &zero, 0).unwrap());
        assert_eq!(
            general_compare(&untyped("abc"), ComparisonOp::Equal, &zero, 0)
                .unwrap_err()
                .code(),
            ErrorCode::Forg0001
        );
    }

    #[test]
    fn rounding_doubles() {
        assert_eq!(round_double(2.5), 3.0);
        assert_eq!(round_double(-2.5), -2.0);
        assert!(round_double(-0.4).is_sign_negative());
        assert_eq!(round_double(0.499_999_999_999_999_94), 0.0);
    }
}
