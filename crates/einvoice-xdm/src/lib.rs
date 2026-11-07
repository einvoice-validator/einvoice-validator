//! Atomic values of the XPath 2.0 data model, as the official EN 16931 and Peppol
//! BIS Billing 3.0 validation stylesheets use them, with the behaviour of the
//! reference engine, Saxon-HE 12.10.
//!
//! * [`Decimal`]: exact, arbitrary precision `xs:decimal`, including Saxon's
//!   precision for decimal division (at least 18 fractional digits, more when the
//!   dividend has more, ties rounded towards zero) and its string form.
//! * [`Integer`]: arbitrary precision `xs:integer`.
//! * `xs:double` with Saxon's lexical parsing ([`parse_double`]) and string
//!   conversion ([`format_double`]).
//! * [`Date`] and [`DateTime`].
//! * [`AtomicValue`]: casting ([`AtomicValue::cast`]), string value, and in [`ops`]
//!   arithmetic, general and value comparisons, `fn:number`, the effective boolean
//!   value, `fn:round`, `fn:floor`, `fn:ceiling`, `fn:abs` and `fn:sum`.
//! * [`XPathRegex`]: the regular expressions of `fn:matches`, `fn:replace` and
//!   `fn:tokenize`, translated to the `regex` crate with XPath semantics.
//!
//! Behaviour that the reference engine has and this crate does not reproduce is
//! refused with [`ErrorCode::Unsupported`] rather than approximated.
//!
//! ```
//! use einvoice_xdm::{AtomicType, AtomicValue, ops};
//!
//! let price = AtomicValue::UntypedAtomic(" 19.99 ".to_owned()).cast(AtomicType::Decimal)?;
//! let total = ops::arithmetic(ops::ArithmeticOp::Multiply, &price, &AtomicValue::from(3.0))?;
//! assert_eq!(total.to_string(), "59.97");
//! let third = ops::arithmetic(
//!     ops::ArithmeticOp::Divide,
//!     &AtomicValue::from(einvoice_xdm::Integer::from(1)),
//!     &AtomicValue::from(einvoice_xdm::Integer::from(3)),
//! )?;
//! assert_eq!(third.to_string(), "0.333333333333333333");
//! # Ok::<(), einvoice_xdm::XdmError>(())
//! ```

mod atomic;
mod datetime;
mod decimal;
mod double;
mod error;
mod integer;
pub mod ops;
mod regex;
mod tables;

pub use atomic::{AtomicType, AtomicValue, parse_boolean};
pub use datetime::{Date, DateTime};
pub use decimal::Decimal;
pub use double::{format_double, parse_double};
pub use error::{ErrorCode, Result, XdmError};
pub use integer::{Integer, Quotient};
pub use regex::XPathRegex;
