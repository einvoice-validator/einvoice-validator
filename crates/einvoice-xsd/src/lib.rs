//! XML Schema validation of UBL 2.1 Invoice and `CreditNote` documents against the
//! official OASIS UBL 2.1 schemas, in pure Rust.
//!
//! The fifteen schema documents reachable from `UBL-Invoice-2.1.xsd` and
//! `UBL-CreditNote-2.1.xsd` are vendored byte for byte in `schemas/oasis-ubl-2.1/`
//! (see its README for their origin and licence) and embedded in the crate
//! ([`UBL_2_1_FILES`]).

mod ubl;

pub use ubl::{CREDIT_NOTE_ENTRY_POINT, INVOICE_ENTRY_POINT, UBL_2_1_FILES};
