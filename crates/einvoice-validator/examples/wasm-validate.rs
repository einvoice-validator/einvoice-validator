//! An end-to-end smoke test that runs on any target, `wasm32-unknown-unknown` included
//! (no system clock, no file system, no output): loads the built-in rule set `cen-ubl`,
//! validates two documents of the CEN EN 16931 UBL 1.3.17 corpus embedded in the
//! program, under a clock given by the caller (the verdicts, the BR-02 finding, the
//! schema errors), then once without a clock, which fails where there is no system
//! clock, and returns 0 when every result is the expected one, else the number of the
//! first wrong check.
//!
//! ```text
//! cargo run -p einvoice-validator --example wasm-validate; echo $?
//! cargo build --release -p einvoice-validator --example wasm-validate --target wasm32-unknown-unknown
//! wasmtime run --invoke main target/wasm32-unknown-unknown/release/examples/wasm-validate.wasm 0 0
//! ```

use std::process::ExitCode;

use einvoice_validator::{
    Clock, Error, FindingKind, Severity, ValidateOptions, Validator, Verdict,
};

/// A valid invoice: an official example.
const VALID: &[u8] =
    include_bytes!("../../../artefacts/cen-en16931-ubl-1.3.17/ubl/examples/ubl-tc434-example1.xml");

/// An invoice without an invoice number: VEFA test BR-02.002, which must fail BR-02.
const BR_02: &[u8] =
    include_bytes!("../../../corpus/cen-en16931-ubl-1.3.17/vefa/Invoice-unit-UBL/BR-02.002.xml");

fn main() -> ExitCode {
    ExitCode::from(run())
}

/// 0 when every check passes, else the number of the first failing one (from 1).
fn run() -> u8 {
    let Ok(validator) = Validator::cen_ubl() else {
        return 100;
    };
    let Ok(clock) = Clock::from_unix_millis(60, 1_789_000_123_456) else {
        return 101;
    };
    let options = ValidateOptions::new().clock(clock);

    let Ok(report) = validator.validate_with(VALID, &options) else {
        return 1;
    };
    if report.verdict() != Verdict::Valid || !report.schema_errors.is_empty() {
        return 2;
    }
    if report.count(Severity::Fatal) != 0 || report.rule_set.version != "1.3.17" {
        return 3;
    }

    let Ok(report) = validator.validate_owned_with(BR_02.to_vec(), &options) else {
        return 4;
    };
    if report.verdict() != Verdict::Invalid {
        return 5;
    }
    let br02 = report.findings().find(|f| f.id.as_deref() == Some("BR-02"));
    if !br02.is_some_and(|f| {
        f.kind == FindingKind::FailedAssert
            && f.severity == Severity::Fatal
            && f.text == "[BR-02]-An Invoice shall have an Invoice number (BT-1)."
    }) {
        return 6;
    }
    // The schema reports the missing mandatory elements.
    if report.schema_errors.len() != 1 {
        return 7;
    }
    // Without a clock: the system clock where there is one, an error where there is none.
    let without_clock = validator.validate(VALID);
    let expected = if cfg!(all(target_family = "wasm", target_os = "unknown")) {
        matches!(without_clock, Err(Error::Unsupported { .. }))
    } else {
        without_clock.is_ok_and(|r| r.verdict() == Verdict::Valid)
    };
    if !expected {
        return 8;
    }
    0
}
