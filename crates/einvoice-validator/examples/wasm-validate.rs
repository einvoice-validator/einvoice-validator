//! An end-to-end smoke test that runs on any target, `wasm32-unknown-unknown` included
//! (no system clock, no file system, no output): loads the built-in rule set `cen-ubl`,
//! validates two documents of the CEN EN 16931 UBL 1.3.17 corpus embedded in the
//! program, under a clock given by the caller (the verdicts, the BR-02 finding, the
//! schema errors), then once without a clock, which fails where there is no system
//! clock, then the valid one with elements nested inside it up to the maximum depth
//! (valid: WebAssembly's 1 MiB stack is enough) and one level deeper (refused with
//! `Error::Limit`), then with a maximum depth and a stack beyond the caps of WebAssembly
//! (refused there with `Error::OptionOutOfRange`, valid elsewhere), then UBL and CII
//! documents as deep as the default maximum depth and as `WASM_MAX_DEPTH`, a rule firing
//! and reporting on every nested element (a report with a finding per level, with the
//! default stack of 256 KiB on WebAssembly), then a stylesheet
//! whose function recurses without end (a `LIMIT` error, not a stack overflow), and
//! returns 0 when every result is the expected one, else the number of the first wrong
//! check.
//!
//! ```text
//! cargo run -p einvoice-validator --example wasm-validate; echo $?
//! cargo build --release -p einvoice-validator --example wasm-validate --target wasm32-unknown-unknown
//! wasmtime run --invoke main target/wasm32-unknown-unknown/release/examples/wasm-validate.wasm 0 0
//! ```

use std::process::ExitCode;

use einvoice_validator::{
    Clock, DEFAULT_MAX_DEPTH, Error, FindingKind, LimitExceeded, Severity, ValidateOptions,
    Validator, Verdict, WASM_MAX_DEPTH, WASM_MAX_STACK,
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
    // Nested up to the maximum depth: validated; one level deeper: refused.
    let Some(deepest) = nested(DEFAULT_MAX_DEPTH) else {
        return 102;
    };
    let Ok(report) = validator.validate_owned_with(deepest.into_bytes(), &options) else {
        return 9;
    };
    if report.verdict() != Verdict::Valid {
        return 10;
    }
    let Some(too_deep) = nested(DEFAULT_MAX_DEPTH + 1) else {
        return 103;
    };
    if !matches!(
        validator.validate_with(too_deep.as_bytes(), &options),
        Err(Error::Limit {
            limit: LimitExceeded::Depth { max: 256, .. },
            ..
        })
    ) {
        return 11;
    }
    if !beyond_the_caps(&validator, &options) {
        return 14;
    }
    // A rule that fires and reports on every element level, up to the default maximum
    // depth and up to the largest one of WebAssembly: a report, never a stack limit.
    let Ok(cii) = Validator::cen_cii() else {
        return 106;
    };
    if let Some(check) = reporting_on_every_level(&validator, &cii, &options) {
        return check;
    }
    // A function calling itself without end: the stack it may use runs out first.
    let recursive = r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
<xsl:function name="f:r" as="xs:integer"><xsl:param name="n" as="xs:integer"/><xsl:sequence select="if ($n lt 0) then 0 else 1 + f:r($n + 1)"/></xsl:function>
<xsl:template match="/"><r><xsl:value-of select="f:r(0)"/></r></xsl:template>
</xsl:stylesheet>"#;
    let Ok(sheet) = einvoice_xslt::load(recursive) else {
        return 104;
    };
    let Ok(executable) = einvoice_xslt_exec::Executable::new(&sheet) else {
        return 105;
    };
    let transform = einvoice_xslt_exec::TransformOptions {
        current_date_time: Some(clock.current_date_time()),
        ..einvoice_xslt_exec::TransformOptions::default()
    };
    let Err(error) = executable.transform_with("<a/>", None, &transform) else {
        return 12;
    };
    if !matches!(
        error.limit(),
        Some(LimitExceeded::Stack { .. } | LimitExceeded::Nesting { .. })
    ) {
        return 13;
    }
    0
}

/// Documents as deep as the default maximum depth and as `WASM_MAX_DEPTH` on whose every
/// nested element a rule fires and reports (UBL-DT-01 with `ubl`, CII-DT-101 with `cii`):
/// `None` when each gets a report with a finding per level, else the number of the
/// failing check.
fn reporting_on_every_level(
    ubl: &Validator,
    cii: &Validator,
    options: &ValidateOptions<'_>,
) -> Option<u8> {
    for (validator, shape, id) in [(ubl, UBL_AMOUNT, "UBL-DT-01"), (cii, CII_ID, "CII-DT-101")] {
        for depth in [DEFAULT_MAX_DEPTH, WASM_MAX_DEPTH] {
            let text = reporting(shape, depth);
            let Ok(report) =
                validator.validate_owned_with(text.into_bytes(), &options.max_depth(depth))
            else {
                return Some(15);
            };
            let found = report
                .findings()
                .filter(|f| f.id.as_deref() == Some(id))
                .count();
            if report.verdict() != Verdict::Invalid || found + 1 != depth as usize {
                return Some(16);
            }
        }
    }
    None
}

/// Options beyond the caps of WebAssembly: refused there, accepted elsewhere.
fn beyond_the_caps(validator: &Validator, options: &ValidateOptions<'_>) -> bool {
    [
        (options.max_depth(WASM_MAX_DEPTH + 1), "max_depth"),
        (options.max_stack(WASM_MAX_STACK + 1), "max_stack"),
    ]
    .iter()
    .all(|(beyond, option)| {
        let result = validator.validate_with(VALID, beyond);
        if cfg!(target_family = "wasm") {
            matches!(result, Err(Error::OptionOutOfRange { option: o, .. }) if o == *option)
        } else {
            result.is_ok_and(|r| r.verdict() == Verdict::Valid)
        }
    })
}

/// The shape of a document on whose every nested element a rule fires and reports: the
/// document element, the nested element's start and end tags, its innermost content, the
/// end of the document element.
type Shape = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

/// UBL amounts with three fraction digits, each failing UBL-DT-01.
const UBL_AMOUNT: Shape = (
    "<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\">",
    "<Amount>",
    "</Amount>",
    "1.123",
    "</Invoice>",
);

/// CII identifiers with a scheme name, each failing CII-DT-101.
const CII_ID: Shape = (
    "<rsm:CrossIndustryInvoice xmlns:rsm=\"urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100\" xmlns:ram=\"urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100\">",
    "<ram:ID schemeName=\"s\">",
    "</ram:ID>",
    "x",
    "</rsm:CrossIndustryInvoice>",
);

/// A document of `shape` `depth` deep: `depth - 1` reporting elements nested in the
/// document element.
fn reporting((open, start, end, content, close): Shape, depth: u32) -> String {
    let n = depth.saturating_sub(1) as usize;
    format!("{open}{}{content}{}{close}", start.repeat(n), end.repeat(n))
}

/// The valid invoice `depth` deep: elements of another namespace nested inside
/// `ext:UBLExtensions/ext:UBLExtension/ext:ExtensionContent` (depths 2 to 4).
fn nested(depth: u32) -> Option<String> {
    let text = std::str::from_utf8(VALID).ok()?;
    let start = text.find("<Invoice")?;
    let end = start + text[start..].find('>')? + 1;
    let n = usize::try_from(depth.checked_sub(4)?).ok()?;
    Some(format!(
        "{}<ext:UBLExtensions xmlns:ext=\"urn:oasis:names:specification:ubl:schema:xsd:CommonExtensionComponents-2\"><ext:UBLExtension><ext:ExtensionContent><y:x xmlns:y=\"urn:y\">{}{}</ext:ExtensionContent></ext:UBLExtension></ext:UBLExtensions>{}",
        &text[..end],
        "<y:x>".repeat(n.checked_sub(1)?),
        "</y:x>".repeat(n),
        &text[end..]
    ))
}
