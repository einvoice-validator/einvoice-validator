//! Atomic values, their types, their string value and casting between them.

use std::fmt;

use crate::datetime::{Date, DateTime};
use crate::decimal::Decimal;
use crate::double::{format_double, parse_double};
use crate::error::{ErrorCode, Result, XdmError, err};
use crate::integer::Integer;

/// An atomic value of one of the types the rule sets use.
#[derive(Debug, Clone)]
pub enum AtomicValue {
    /// `xs:string`.
    String(String),
    /// `xs:untypedAtomic`: the typed value of a node of an untyped document.
    UntypedAtomic(String),
    /// `xs:anyURI` (results of `document-uri()` and `base-uri()`).
    AnyUri(String),
    /// `xs:boolean`.
    Boolean(bool),
    /// `xs:integer`.
    Integer(Integer),
    /// `xs:decimal` (not an integer).
    Decimal(Decimal),
    /// `xs:double`.
    Double(f64),
    /// `xs:date`.
    Date(Date),
    /// `xs:dateTime`.
    DateTime(DateTime),
}

/// The type of an atomic value, also the target of a cast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AtomicType {
    /// `xs:string`.
    String,
    /// `xs:untypedAtomic`.
    UntypedAtomic,
    /// `xs:anyURI`.
    AnyUri,
    /// `xs:boolean`.
    Boolean,
    /// `xs:integer`.
    Integer,
    /// `xs:decimal`.
    Decimal,
    /// `xs:double`.
    Double,
    /// `xs:date`.
    Date,
    /// `xs:dateTime`.
    DateTime,
}

impl AtomicType {
    /// The lexical `QName` of the type, e.g. `xs:decimal`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::String => "xs:string",
            Self::UntypedAtomic => "xs:untypedAtomic",
            Self::AnyUri => "xs:anyURI",
            Self::Boolean => "xs:boolean",
            Self::Integer => "xs:integer",
            Self::Decimal => "xs:decimal",
            Self::Double => "xs:double",
            Self::Date => "xs:date",
            Self::DateTime => "xs:dateTime",
        }
    }

    /// The type named by a lexical `QName` with the `xs` prefix, e.g. `xs:decimal`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "xs:string" => Self::String,
            "xs:untypedAtomic" => Self::UntypedAtomic,
            "xs:anyURI" => Self::AnyUri,
            "xs:boolean" => Self::Boolean,
            "xs:integer" => Self::Integer,
            "xs:decimal" => Self::Decimal,
            "xs:double" => Self::Double,
            "xs:date" => Self::Date,
            "xs:dateTime" => Self::DateTime,
            _ => return None,
        })
    }

    /// Whether the type is numeric.
    #[must_use]
    pub const fn is_numeric(self) -> bool {
        matches!(self, Self::Integer | Self::Decimal | Self::Double)
    }

    /// Whether `self` is `other` or derived from it (`xs:integer` from `xs:decimal`).
    #[must_use]
    pub fn is_subtype_of(self, other: Self) -> bool {
        self == other || (self == Self::Integer && other == Self::Decimal)
    }
}

impl fmt::Display for AtomicType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl AtomicValue {
    /// The type of the value.
    #[must_use]
    pub const fn atomic_type(&self) -> AtomicType {
        match self {
            Self::String(_) => AtomicType::String,
            Self::UntypedAtomic(_) => AtomicType::UntypedAtomic,
            Self::AnyUri(_) => AtomicType::AnyUri,
            Self::Boolean(_) => AtomicType::Boolean,
            Self::Integer(_) => AtomicType::Integer,
            Self::Decimal(_) => AtomicType::Decimal,
            Self::Double(_) => AtomicType::Double,
            Self::Date(_) => AtomicType::Date,
            Self::DateTime(_) => AtomicType::DateTime,
        }
    }

    /// `instance of`: whether the value belongs to the type.
    #[must_use]
    pub fn is_instance_of(&self, target: AtomicType) -> bool {
        self.atomic_type().is_subtype_of(target)
    }

    /// Whether the value is numeric.
    #[must_use]
    pub const fn is_numeric(&self) -> bool {
        self.atomic_type().is_numeric()
    }

    /// Whether the value is the double NaN.
    #[must_use]
    pub fn is_nan(&self) -> bool {
        matches!(self, Self::Double(d) if d.is_nan())
    }

    /// The value of a numeric as a double (decimals and integers rounded to nearest).
    #[must_use]
    pub fn numeric_as_f64(&self) -> Option<f64> {
        match self {
            Self::Integer(i) => Some(i.to_f64()),
            Self::Decimal(d) => Some(d.to_f64()),
            Self::Double(d) => Some(*d),
            _ => None,
        }
    }

    /// Casts the value to the target type, with the XPath 2.0 casting rules and
    /// Saxon's lexical rules.
    ///
    /// # Errors
    ///
    /// `XPTY0004` when the cast is not allowed between the two types, `FORG0001` for
    /// an invalid lexical form, `FOCA0002` for NaN or infinity to decimal or integer,
    /// `Unsupported` for casting a string to `xs:anyURI`.
    pub fn cast(&self, target: AtomicType) -> Result<Self> {
        use AtomicType as T;
        match self {
            Self::String(s) | Self::UntypedAtomic(s) => cast_from_string(s, target),
            Self::AnyUri(s) => match target {
                T::String => Ok(Self::String(s.clone())),
                T::UntypedAtomic => Ok(Self::UntypedAtomic(s.clone())),
                T::AnyUri => Ok(self.clone()),
                _ => not_castable(self, target),
            },
            Self::Boolean(b) => match target {
                T::String | T::UntypedAtomic => Ok(string_like(self.to_string(), target)),
                T::Boolean => Ok(self.clone()),
                T::Integer => Ok(Self::Integer(Integer::from(i64::from(*b)))),
                T::Decimal => Ok(Self::Decimal(Decimal::from(i64::from(*b)))),
                T::Double => Ok(Self::Double(if *b { 1.0 } else { 0.0 })),
                _ => not_castable(self, target),
            },
            Self::Integer(i) => match target {
                T::String | T::UntypedAtomic => Ok(string_like(self.to_string(), target)),
                T::Boolean => Ok(Self::Boolean(i.signum() != 0)),
                T::Integer => Ok(self.clone()),
                T::Decimal => Ok(Self::Decimal(i.to_decimal())),
                T::Double => Ok(Self::Double(i.to_f64())),
                _ => not_castable(self, target),
            },
            Self::Decimal(d) => match target {
                T::String | T::UntypedAtomic => Ok(string_like(self.to_string(), target)),
                T::Boolean => Ok(Self::Boolean(!d.is_zero())),
                T::Integer => Ok(Self::Integer(Integer::from_bigint(d.trunc_to_bigint()))),
                T::Decimal => Ok(self.clone()),
                T::Double => Ok(Self::Double(d.to_f64())),
                _ => not_castable(self, target),
            },
            Self::Double(v) => match target {
                T::String | T::UntypedAtomic => Ok(string_like(self.to_string(), target)),
                T::Boolean => Ok(Self::Boolean(*v != 0.0 && !v.is_nan())),
                T::Integer => Integer::from_f64(*v).map(Self::Integer),
                T::Decimal => Decimal::from_f64(*v).map(Self::Decimal),
                T::Double => Ok(self.clone()),
                _ => not_castable(self, target),
            },
            Self::Date(d) => match target {
                T::String | T::UntypedAtomic => Ok(string_like(self.to_string(), target)),
                T::Date => Ok(self.clone()),
                T::DateTime => Ok(Self::DateTime(d.to_date_time())),
                _ => not_castable(self, target),
            },
            Self::DateTime(d) => match target {
                T::String | T::UntypedAtomic => Ok(string_like(self.to_string(), target)),
                T::Date => Ok(Self::Date(d.date())),
                T::DateTime => Ok(self.clone()),
                _ => not_castable(self, target),
            },
        }
    }

    /// `castable as`: whether [`AtomicValue::cast`] succeeds.
    #[must_use]
    pub fn castable(&self, target: AtomicType) -> bool {
        self.cast(target).is_ok()
    }
}

fn string_like(text: String, target: AtomicType) -> AtomicValue {
    if target == AtomicType::UntypedAtomic {
        AtomicValue::UntypedAtomic(text)
    } else {
        AtomicValue::String(text)
    }
}

fn not_castable<T>(value: &AtomicValue, target: AtomicType) -> Result<T> {
    err(
        ErrorCode::Xpty0004,
        format!("Cannot cast {} to {target}", value.atomic_type()),
    )
}

fn cast_from_string(text: &str, target: AtomicType) -> Result<AtomicValue> {
    use AtomicType as T;
    Ok(match target {
        T::String => AtomicValue::String(text.to_owned()),
        T::UntypedAtomic => AtomicValue::UntypedAtomic(text.to_owned()),
        T::AnyUri => {
            return err(
                ErrorCode::Unsupported,
                "casting a string to xs:anyURI is not supported",
            );
        }
        T::Boolean => AtomicValue::Boolean(parse_boolean(text)?),
        T::Integer => AtomicValue::Integer(Integer::parse(text)?),
        T::Decimal => AtomicValue::Decimal(Decimal::parse(text)?),
        T::Double => AtomicValue::Double(parse_double(text)?),
        T::Date => AtomicValue::Date(Date::parse(text)?),
        T::DateTime => AtomicValue::DateTime(DateTime::parse(text)?),
    })
}

/// The lexical form of `xs:boolean`: `true`, `false`, `1` or `0`, with surrounding
/// whitespace.
///
/// # Errors
///
/// `FORG0001` otherwise.
pub fn parse_boolean(text: &str) -> Result<bool> {
    match crate::double::trim_xml_whitespace(text) {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        other => Err(XdmError::new(
            ErrorCode::Forg0001,
            format!("The string \"{other}\" cannot be cast to a boolean"),
        )),
    }
}

impl fmt::Display for AtomicValue {
    /// The string value (`fn:string`, cast to `xs:string`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(s) | Self::UntypedAtomic(s) | Self::AnyUri(s) => f.write_str(s),
            Self::Boolean(b) => f.write_str(if *b { "true" } else { "false" }),
            Self::Integer(i) => fmt::Display::fmt(i, f),
            Self::Decimal(d) => fmt::Display::fmt(d, f),
            Self::Double(d) => f.write_str(&format_double(*d)),
            Self::Date(d) => fmt::Display::fmt(d, f),
            Self::DateTime(d) => fmt::Display::fmt(d, f),
        }
    }
}

impl From<Integer> for AtomicValue {
    fn from(value: Integer) -> Self {
        Self::Integer(value)
    }
}

impl From<Decimal> for AtomicValue {
    fn from(value: Decimal) -> Self {
        Self::Decimal(value)
    }
}

impl From<f64> for AtomicValue {
    fn from(value: f64) -> Self {
        Self::Double(value)
    }
}

impl From<bool> for AtomicValue {
    fn from(value: bool) -> Self {
        Self::Boolean(value)
    }
}
