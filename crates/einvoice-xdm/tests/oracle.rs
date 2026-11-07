//! Equivalence with Saxon-HE 9.9.1-8: every case of `tests/oracle/cases.tsv` and
//! `tests/oracle/generated.tsv` is evaluated with this crate and compared with the
//! result Saxon produced (`tests/oracle/expected.tsv`, written by the
//! `saxon-oracle` example). Cases whose id starts with `refuse-` must be refused
//! with `ErrorCode::Unsupported`.

#[path = "oracle/format.rs"]
mod format;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use einvoice_xdm::ops::{self, ArithmeticOp, ComparisonOp};
use einvoice_xdm::{AtomicType, AtomicValue, ErrorCode, Integer, XPathRegex, XdmError};

/// The implicit timezone of the Saxon runs (UTC).
const IMPLICIT_TIMEZONE: i16 = 0;

fn operand(arg: &format::Arg) -> Result<AtomicValue, XdmError> {
    let text = AtomicValue::String(arg.lexical.clone());
    let to = |t| text.cast(t);
    match arg.kind.as_str() {
        "string" => Ok(text.clone()),
        "untyped" | "node" => Ok(AtomicValue::UntypedAtomic(arg.lexical.clone())),
        "decimal" => to(AtomicType::Decimal),
        "idecimal" => to(AtomicType::Integer)?.cast(AtomicType::Decimal),
        "integer" => to(AtomicType::Integer),
        "double" => to(AtomicType::Double),
        "boolean" => to(AtomicType::Boolean),
        "date" => to(AtomicType::Date),
        "dateTime" => to(AtomicType::DateTime),
        other => Err(harness(&format!("unknown operand kind {other}"))),
    }
}

fn arithmetic_op(op: &str) -> Option<ArithmeticOp> {
    Some(match op {
        "add" => ArithmeticOp::Add,
        "sub" => ArithmeticOp::Subtract,
        "mul" => ArithmeticOp::Multiply,
        "div" => ArithmeticOp::Divide,
        "idiv" => ArithmeticOp::IntegerDivide,
        "mod" => ArithmeticOp::Modulo,
        _ => return None,
    })
}

fn comparison_op(name: &str) -> Result<ComparisonOp, XdmError> {
    Ok(match name {
        "eq" => ComparisonOp::Equal,
        "ne" => ComparisonOp::NotEqual,
        "lt" => ComparisonOp::Less,
        "le" => ComparisonOp::LessOrEqual,
        "gt" => ComparisonOp::Greater,
        "ge" => ComparisonOp::GreaterOrEqual,
        other => return Err(harness(&format!("unknown comparison {other}"))),
    })
}

/// An error of the test harness itself (unknown operation or operand kind),
/// reported with a code no case may expect.
fn harness(message: &str) -> XdmError {
    XdmError::new(ErrorCode::Unsupported, format!("HARNESS: {message}"))
}

fn cast_target(name: &str) -> Result<AtomicType, XdmError> {
    AtomicType::from_name(&format!("xs:{name}"))
        .ok_or_else(|| harness(&format!("unknown cast target {name}")))
}

/// Every XML 1.0 character.
fn xml_chars() -> impl Iterator<Item = char> {
    [0x9, 0xA, 0xD]
        .into_iter()
        .chain(0x20..=0xD7FF)
        .chain(0xE000..=0xFFFD)
        .chain(0x1_0000..=0x10_FFFF)
        .filter_map(char::from_u32)
}

fn ranges(codes: impl Iterator<Item = u32>) -> String {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for code in codes {
        match runs.last_mut() {
            Some((_, high)) if *high + 1 == code => *high = code,
            _ => runs.push((code, code)),
        }
    }
    runs.iter()
        .map(|(low, high)| format!("{low}-{high}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn string(text: impl Into<String>) -> AtomicValue {
    AtomicValue::String(text.into())
}

fn evaluate(case: &format::Case) -> Result<Vec<AtomicValue>, XdmError> {
    let arg = |i: usize| {
        case.args
            .get(i)
            .ok_or_else(|| harness(&format!("{}: operand {i} missing", case.id)))
            .and_then(operand)
    };
    let op = case.op.as_str();
    if let Some(arithmetic) = arithmetic_op(op) {
        return Ok(vec![ops::arithmetic(arithmetic, &arg(0)?, &arg(1)?)?]);
    }
    if let Some(name) = op.strip_prefix("g-") {
        let (a, b) = (arg(0)?, arg(1)?);
        return Ok(vec![AtomicValue::Boolean(ops::general_compare(
            &a,
            comparison_op(name)?,
            &b,
            IMPLICIT_TIMEZONE,
        )?)]);
    }
    if let Some(name) = op.strip_prefix("v-") {
        let (a, b) = (arg(0)?, arg(1)?);
        return Ok(vec![AtomicValue::Boolean(ops::value_compare(
            &a,
            comparison_op(name)?,
            &b,
            IMPLICIT_TIMEZONE,
        )?)]);
    }
    if let Some(name) = op.strip_prefix("castable-") {
        let target = cast_target(name)?;
        return Ok(vec![AtomicValue::Boolean(arg(0)?.castable(target))]);
    }
    if let Some(name) = op.strip_prefix("cast-") {
        let target = cast_target(name)?;
        return Ok(vec![arg(0)?.cast(target)?]);
    }
    Ok(match op {
        "neg" => vec![ops::negate(&arg(0)?)?],
        "string" => vec![string(arg(0)?.to_string())],
        "number" => vec![AtomicValue::Double(ops::number(&arg(0)?))],
        "boolean" => vec![AtomicValue::Boolean(ops::effective_boolean_value(&arg(
            0,
        )?)?)],
        "round" => vec![ops::round(&arg(0)?)?],
        "floor" => vec![ops::floor(&arg(0)?)?],
        "ceiling" => vec![ops::ceiling(&arg(0)?)?],
        "abs" => vec![ops::abs(&arg(0)?)?],
        "sum" => {
            let values = case
                .args
                .iter()
                .map(operand)
                .collect::<Result<Vec<_>, _>>()?;
            vec![ops::sum(&values)?]
        }
        "matches" => {
            let regex = XPathRegex::new(&arg(1)?.to_string(), "")?;
            vec![AtomicValue::Boolean(regex.is_match(&arg(0)?.to_string()))]
        }
        "replace" => {
            let regex = XPathRegex::new(&arg(1)?.to_string(), "")?;
            vec![string(
                regex.replace(&arg(0)?.to_string(), &arg(2)?.to_string())?,
            )]
        }
        "tokenize" => {
            let regex = XPathRegex::new(&arg(1)?.to_string(), "")?;
            let input = arg(0)?.to_string();
            regex.tokenize(&input)?.into_iter().map(string).collect()
        }
        "cps-regex" => {
            let regex = XPathRegex::new(&arg(0)?.to_string(), "")?;
            let mut buffer = [0_u8; 4];
            let matched = xml_chars()
                .filter(|c| regex.is_match(c.encode_utf8(&mut buffer)))
                .map(u32::from);
            vec![string(ranges(matched))]
        }
        "int-digits" => {
            let mut out = Vec::new();
            for c in xml_chars().filter(|&c| u32::from(c) >= 0x20 && u32::from(c) <= 0xFFFD) {
                if Integer::parse(&format!("1111111111111111{c}")).is_ok() {
                    let value = Integer::parse(&format!("1000000000000000{c}"))?;
                    let digit = value.rem(&Integer::from(10))?;
                    out.push(format!("{}:{digit}", u32::from(c)));
                }
            }
            vec![string(out.join(","))]
        }
        other => return Err(harness(&format!("unknown operation {other}"))),
    })
}

fn render(result: &Result<Vec<AtomicValue>, XdmError>) -> String {
    match result {
        Err(e) => format!("error:{}", e.code().as_str()),
        Ok(items) if items.is_empty() => "empty".to_owned(),
        Ok(items) => items
            .iter()
            .map(|item| {
                format!(
                    "{}:{}",
                    item.atomic_type().name(),
                    format::escape(&item.to_string())
                )
            })
            .collect::<Vec<_>>()
            .join("\t"),
    }
}

fn load(name: &str) -> std::io::Result<String> {
    std::fs::read_to_string(format!(
        "{}/tests/oracle/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
}

#[test]
fn every_case_matches_saxon() {
    let mut cases = format::read_cases(&load("cases.tsv").unwrap()).unwrap();
    cases.extend(format::read_cases(&load("generated.tsv").unwrap()).unwrap());
    let expected: BTreeMap<String, String> = format::read_expected(&load("expected.tsv").unwrap())
        .into_iter()
        .collect();
    assert_eq!(
        expected.len(),
        cases.len(),
        "expected.tsv is stale: rerun the saxon-oracle example"
    );

    let mut failures = String::new();
    let mut refused = 0;
    for case in &cases {
        let saxon = expected
            .get(&case.id)
            .unwrap_or_else(|| panic!("{}: no Saxon result", case.id));
        let result = evaluate(case);
        if let Err(e) = &result {
            assert!(!e.message().starts_with("HARNESS"), "{}: {e}", case.id);
        }
        if case.id.starts_with("refuse-") {
            match &result {
                Err(e) if e.code() == ErrorCode::Unsupported => refused += 1,
                _ => {
                    let _ = writeln!(
                        failures,
                        "{}: must be refused, got {}",
                        case.id,
                        render(&result)
                    );
                }
            }
            continue;
        }
        let ours = render(&result);
        if &ours != saxon {
            let _ = writeln!(failures, "{}: Saxon {saxon:?}, ours {ours:?}", case.id);
        }
    }
    let failed = failures.lines().count();
    assert!(
        failures.is_empty(),
        "{failed} of {} cases differ from Saxon:\n{failures}",
        cases.len()
    );
    println!(
        "{} cases equal to Saxon-HE 9.9.1-8, {refused} refused as unsupported",
        cases.len() - refused
    );
}
