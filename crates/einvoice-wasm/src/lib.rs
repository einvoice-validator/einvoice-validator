//! JavaScript and TypeScript bindings of [`einvoice_validator`], for WebAssembly
//! (`wasm32-unknown-unknown`, with `wasm-bindgen`): the source of the npm package
//! `einvoice-validator`, for browsers, bundlers and Node.js.
//!
//! The JavaScript API, as its TypeScript declarations give it:
//!
//! ```ts
//! const validator = new Validator("cen-ubl");       // "cen-cii", or Validator.builtin(name)
//! const report = validator.validate(bytes, { documentUri, now, timezoneMinutes, maxDepth, failClosed });
//! report.status;                                     // "valid" | "invalid"
//! report.findings;                                   // [{ id, flag, severity, location, text, ... }]
//! validator.validateSvrl(bytes);                     // [{ stylesheet, version, sha256, svrl }]
//! builtinRuleSets(); licences(); version();
//! ```
//!
//! A report holds the members of a document of the JSON format of `einvoice-validate`,
//! version 1 (status, error, counts, schema errors, findings, stylesheet errors, warnings),
//! after a header: `format` (`einvoice-validator`), `format_version` (1), `tool`,
//! `rule_set` and `document_uri`. A document that is not well-formed XML is `invalid`,
//! with its `error` set (kind `not-xml`, line and column), as in that format; every other
//! error is thrown as a JavaScript `Error` named `EinvoiceValidatorError` with a `kind`
//! (`bundle`, `not-xml`, `unsupported`, `transform`, `internal`), and an argument of the
//! wrong type or out of range as a `TypeError` or `RangeError` with the kind `argument`.
//!
//! The clock that the stylesheets read (`fn:current-dateTime()` and the implicit
//! timezone) is JavaScript's: `Date.now()` when a validation starts, in UTC, unless the
//! options give `now` (a `Date` or milliseconds since the Unix epoch) and
//! `timezoneMinutes` (minutes east of UTC).
//!
//! A document with an element nested deeper than `maxDepth` (by default 256, the
//! library's [`DEFAULT_MAX_DEPTH`](einvoice_validator::DEFAULT_MAX_DEPTH); at most 400,
//! a bound set by the native stacks of JavaScript engines, which the module cannot
//! measure: Chrome's Web Workers overflow on about 600 levels) is refused with an error
//! of kind `unsupported` whose `limit` is `depth`.
//!
//! With `failClosed: true`, every finding that would otherwise be of severity `other` (a
//! flag other than `fatal` and `warning`, or none, that the rule set gives no severity of
//! its own) is `fatal`
//! ([`ValidateOptions::fail_closed`](einvoice_validator::ValidateOptions::fail_closed)).
//!
//! On other targets the crate builds too, so that its conversions are tested natively;
//! its exported functions call JavaScript and only work in WebAssembly.

mod json;
mod render;

use einvoice_validator::{Clock, Error, ValidateOptions};
use js_sys::{Date, Object, Reflect, Uint8Array};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

use crate::json::Json;

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &'static str = include_str!("types.d.ts");

/// A rule set ready to validate documents: its XML schema loaded and its official
/// stylesheets prepared, once. Build one and validate any number of documents with it:
/// loading the rule set takes far longer than validating a document.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Validator {
    inner: einvoice_validator::Validator,
}

#[wasm_bindgen]
impl Validator {
    /// The built-in rule set `name` (see `builtinRuleSets()`): `"cen-ubl"`, the CEN
    /// EN 16931 validation artefacts for UBL, release 1.3.17, with the UBL 2.1 schemas;
    /// `"cen-cii"`, the same release for UN/CEFACT CII D16B, without an XML schema.
    ///
    /// # Errors
    ///
    /// Throws an `EinvoiceValidatorError` of kind `bundle` for an unknown name, a
    /// `TypeError` (kind `argument`) when `name` is not a string.
    #[wasm_bindgen(constructor)]
    pub fn new(
        #[wasm_bindgen(unchecked_param_type = "string")] name: &JsValue,
    ) -> Result<Self, JsValue> {
        install_panic_hook();
        let Some(name) = name.as_string() else {
            return Err(argument_error(
                ArgumentError::Type,
                "the rule-set name must be a string",
            ));
        };
        einvoice_validator::Validator::builtin(&name)
            .map(|inner| Self { inner })
            .map_err(|e| js_error(&e))
    }

    /// The built-in rule set `name`: the same as `new Validator(name)`.
    ///
    /// # Errors
    ///
    /// As the constructor.
    pub fn builtin(
        #[wasm_bindgen(unchecked_param_type = "string")] name: &JsValue,
    ) -> Result<Self, JsValue> {
        Self::new(name)
    }

    /// The rule set: its name, title, version, XML schema, and its stylesheets with their
    /// versions, paths and SHA-256.
    ///
    /// # Errors
    ///
    /// None in practice (an internal error is thrown rather than hidden).
    #[wasm_bindgen(getter, js_name = ruleSet, unchecked_return_type = "RuleSetInfo")]
    pub fn rule_set(&self) -> Result<JsValue, JsValue> {
        parse(&render::rule_set(self.inner.rule_set()))
    }

    /// Validates a document: checks that it is well-formed XML (UTF-8, no document type
    /// declaration), validates it against the rule set's XML schema, then runs every
    /// stylesheet of the rule set on it. The bytes are copied; the array is not changed.
    ///
    /// Returns the report: `status` is `"valid"` when there is no schema error and no
    /// finding with severity `fatal`, else `"invalid"`; a document that is not
    /// well-formed XML is `"invalid"` with `error` set (kind `not-xml`, line and column).
    ///
    /// # Errors
    ///
    /// Throws an `EinvoiceValidatorError` with a `kind`: `unsupported` for an encoding
    /// other than UTF-8, a document type declaration or a construct the engine refuses,
    /// and for a document beyond a limit (with `limit`: `depth`, an element nested deeper
    /// than `maxDepth`, with its `line` and `column`; `nesting` or `stack`, stylesheets
    /// recursing beyond what that depth or the stack allows);
    /// `transform` when a stylesheet raises a dynamic error on a document without schema
    /// errors (with `stylesheet`, `code` and `line`); `bundle` or `internal` for a bug.
    /// Throws a `TypeError` or `RangeError` (kind `argument`) for a document that is not
    /// a `Uint8Array` or for invalid options.
    #[wasm_bindgen(unchecked_return_type = "Report")]
    pub fn validate(
        &self,
        #[wasm_bindgen(unchecked_param_type = "Uint8Array")] document: &JsValue,
        #[wasm_bindgen(unchecked_optional_param_type = "ValidateOptions")] options: &JsValue,
    ) -> Result<JsValue, JsValue> {
        let bytes = document_bytes(document)?;
        let options = Options::read(options)?;
        let result = self
            .inner
            .validate_owned_with(bytes, &options.validate_options());
        let report = render::report(
            self.inner.rule_set(),
            options.document_uri.as_deref(),
            result,
        )
        .map_err(|e| js_error(&e))?;
        parse(&report)
    }

    /// Validates a document as `validate` does, and returns the SVRL report of each
    /// stylesheet of the rule set, in the order they ran, exactly as the stylesheet wrote
    /// it (the schema errors are not part of it).
    ///
    /// # Errors
    ///
    /// As `validate`, and also: an `EinvoiceValidatorError` of kind `not-xml` (with
    /// `line` and `column`) for a document that is not well-formed XML, and of kind
    /// `transform` when a stylesheet raised a dynamic error, even on a document with
    /// schema errors: there is then no SVRL report to return.
    #[wasm_bindgen(js_name = validateSvrl, unchecked_return_type = "SvrlReport[]")]
    pub fn validate_svrl(
        &self,
        #[wasm_bindgen(unchecked_param_type = "Uint8Array")] document: &JsValue,
        #[wasm_bindgen(unchecked_optional_param_type = "ValidateOptions")] options: &JsValue,
    ) -> Result<JsValue, JsValue> {
        let bytes = document_bytes(document)?;
        let options = Options::read(options)?;
        let result = self
            .inner
            .validate_owned_with(bytes, &options.validate_options());
        let reports = render::svrl(result).map_err(|e| js_error(&e))?;
        parse(&reports)
    }
}

/// The names of the built-in rule sets of this build of the package.
///
/// # Errors
///
/// None in practice (an internal error is thrown rather than hidden).
#[wasm_bindgen(js_name = builtinRuleSets, unchecked_return_type = "string[]")]
pub fn builtin_rule_sets() -> Result<JsValue, JsValue> {
    parse(&render::builtin_rule_sets())
}

/// Every licence of what the package embeds, with what it covers and its full text: the
/// MIT licence of the code, the EUPL-1.2 of the CEN stylesheet of the built-in rule set,
/// and the terms of the OASIS UBL 2.1 schemas (OASIS, UN/CEFACT, W3C). The W3C licence of
/// the XML Signature schema asks for its notice "in a location viewable to users": an
/// application can show these texts.
///
/// # Errors
///
/// None in practice (an internal error is thrown rather than hidden).
#[wasm_bindgen(unchecked_return_type = "Licence[]")]
pub fn licences() -> Result<JsValue, JsValue> {
    parse(&render::licences())
}

/// The version of the package.
#[must_use]
#[wasm_bindgen]
pub fn version() -> String {
    render::VERSION.to_owned()
}

#[wasm_bindgen]
extern "C" {
    /// `console.error`.
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn console_error(message: &str);
}

/// Installs, once, when the first `Validator` is built, a panic hook that reports a panic
/// (a bug) on the console with its message and location, before the WebAssembly trap
/// ("unreachable") is thrown as a `WebAssembly.RuntimeError`. No dependency: a few
/// lines of `std` and the `console.error` import.
fn install_panic_hook() {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            console_error(&format!("einvoice-validator: {info}"));
        }));
    });
}

/// The options of one validation, read from the JavaScript object.
#[derive(Debug)]
struct Options {
    document_uri: Option<String>,
    clock: Clock,
    max_depth: u32,
    fail_closed: bool,
}

/// The names of the members of the options object.
const OPTION_NAMES: [&str; 5] = [
    "documentUri",
    "now",
    "timezoneMinutes",
    "maxDepth",
    "failClosed",
];

/// The extreme value of a JavaScript `Date`, in milliseconds from the Unix epoch.
const MAX_DATE_MILLIS: f64 = 8.64e15;

/// The largest `maxDepth`. The stylesheets recurse once or twice per element level, and
/// a WebAssembly engine runs the module's functions on its own native stack, beside the
/// module's stack in linear memory, which is the only one the library measures
/// ([`ValidateOptions::max_stack`]). Some engines' native stack ends first. Measured
/// with the built-in rule sets on documents nested that deep (the package as built by
/// `npm.sh`): in V8, a Chrome Web Worker overflows beyond 600 levels (a `RangeError`,
/// after which the module's memory is no longer consistent), Chrome's main thread beyond
/// 1200 and Node.js beyond 1300; Firefox reaches the library's stack limit beyond 1400,
/// as it should. 400 keeps a margin of half of itself below the smallest, and bounds the
/// nesting of template and function invocations for any document
/// ([`max_nesting_for_depth`](einvoice_validator::max_nesting_for_depth): 864).
const MAX_DEPTH: u32 = 400;

impl Options {
    /// Reads `{ documentUri?, now?, timezoneMinutes?, maxDepth?, failClosed? }`
    /// (`undefined` or `null` for all the defaults); a member set to `undefined` or `null`
    /// takes its default.
    /// Other members are refused, so that a misspelt option is not silently ignored.
    fn read(value: &JsValue) -> Result<Self, JsValue> {
        let mut document_uri = None;
        let mut now = None;
        let mut timezone = 0;
        let mut max_depth = einvoice_validator::DEFAULT_MAX_DEPTH;
        let mut fail_closed = false;
        if !value.is_undefined() && !value.is_null() {
            if !value.is_object() {
                return Err(argument_error(
                    ArgumentError::Type,
                    "the options must be an object",
                ));
            }
            let object: &Object = value.unchecked_ref();
            for key in Object::keys(object).iter() {
                let key = key.as_string().unwrap_or_default();
                if !OPTION_NAMES.contains(&key.as_str()) {
                    return Err(argument_error(
                        ArgumentError::Type,
                        &format!(
                            "unknown option {key:?} (the options are {})",
                            OPTION_NAMES.join(", ")
                        ),
                    ));
                }
            }
            let get = |name: &str| -> Result<Option<JsValue>, JsValue> {
                let v = Reflect::get(object, &JsValue::from_str(name))?;
                Ok((!v.is_undefined() && !v.is_null()).then_some(v))
            };
            if let Some(v) = get("documentUri")? {
                let Some(uri) = v.as_string() else {
                    return Err(argument_error(
                        ArgumentError::Type,
                        "the option documentUri must be a string",
                    ));
                };
                document_uri = Some(uri);
            }
            if let Some(v) = get("now")? {
                now = Some(now_millis(&v)?);
            }
            if let Some(v) = get("timezoneMinutes")? {
                let Some(minutes) = v.as_f64() else {
                    return Err(argument_error(
                        ArgumentError::Type,
                        "the option timezoneMinutes must be a number",
                    ));
                };
                timezone = integer(minutes, -840.0, 840.0)
                    .and_then(|m| i16::try_from(m).ok())
                    .ok_or_else(|| {
                        argument_error(
                            ArgumentError::Range,
                            &format!(
                                "the option timezoneMinutes must be a whole number of minutes from -840 to 840 (-14:00 to +14:00), not {minutes}"
                            ),
                        )
                    })?;
            }
            if let Some(v) = get("maxDepth")? {
                max_depth = max_depth_of(&v)?;
            }
            if let Some(v) = get("failClosed")? {
                fail_closed = v.as_bool().ok_or_else(|| {
                    argument_error(
                        ArgumentError::Type,
                        "the option failClosed must be a boolean",
                    )
                })?;
            }
        }
        let millis = match now {
            Some(millis) => millis,
            None => integer(Date::now(), -MAX_DATE_MILLIS, MAX_DATE_MILLIS).ok_or_else(|| {
                argument_error(ArgumentError::Range, "Date.now() is not a valid time")
            })?,
        };
        let clock = Clock::from_unix_millis(timezone, millis).map_err(|e| {
            argument_error(
                ArgumentError::Range,
                &format!("the option now ({millis} ms from the Unix epoch): {e}"),
            )
        })?;
        Ok(Self {
            document_uri,
            clock,
            max_depth,
            fail_closed,
        })
    }

    fn validate_options(&self) -> ValidateOptions<'_> {
        let options = ValidateOptions::new()
            .clock(self.clock)
            .max_depth(self.max_depth)
            .fail_closed(self.fail_closed);
        match &self.document_uri {
            Some(uri) => options.document_uri(uri),
            None => options,
        }
    }
}

/// The option `maxDepth`: a whole number from 1 to [`MAX_DEPTH`].
fn max_depth_of(value: &JsValue) -> Result<u32, JsValue> {
    let Some(depth) = value.as_f64() else {
        return Err(argument_error(
            ArgumentError::Type,
            "the option maxDepth must be a number",
        ));
    };
    integer(depth, 1.0, f64::from(MAX_DEPTH))
        .and_then(|d| u32::try_from(d).ok())
        .ok_or_else(|| {
            argument_error(
                ArgumentError::Range,
                &format!(
                    "the option maxDepth must be a whole number from 1 to {MAX_DEPTH}, not {depth}"
                ),
            )
        })
}

/// The option `now`: a `Date`, or milliseconds from the Unix epoch.
fn now_millis(value: &JsValue) -> Result<i64, JsValue> {
    let (millis, what) = if let Some(date) = value.dyn_ref::<Date>() {
        (date.get_time(), "a valid Date")
    } else if let Some(millis) = value.as_f64() {
        (
            millis,
            "a whole number of milliseconds within the range of Date",
        )
    } else {
        return Err(argument_error(
            ArgumentError::Type,
            "the option now must be a Date or a number of milliseconds from the Unix epoch",
        ));
    };
    integer(millis, -MAX_DATE_MILLIS, MAX_DATE_MILLIS).ok_or_else(|| {
        argument_error(
            ArgumentError::Range,
            &format!("the option now must be {what}, not {millis}"),
        )
    })
}

/// `x` as an integer, when it is a whole number from `min` to `max` (both within ±2^53,
/// where every integer is exact in a double).
fn integer(x: f64, min: f64, max: f64) -> Option<i64> {
    #[expect(
        clippy::float_cmp,
        reason = "exact on purpose: whether x is a whole number"
    )]
    let whole = x.trunc() == x;
    if !(x.is_finite() && whole && x >= min && x <= max) {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a whole number within ±2^53, checked above, converts exactly"
    )]
    let n = x as i64;
    Some(n)
}

/// The bytes of the document, copied out of its `Uint8Array` (a Node.js `Buffer` is one).
fn document_bytes(document: &JsValue) -> Result<Vec<u8>, JsValue> {
    document.dyn_ref::<Uint8Array>().map_or_else(
        || {
            Err(argument_error(
                ArgumentError::Type,
                "the document must be a Uint8Array (its bytes, UTF-8)",
            ))
        },
        |array| Ok(array.to_vec()),
    )
}

/// The JavaScript value of a JSON value.
fn parse(json: &Json) -> Result<JsValue, JsValue> {
    js_sys::JSON::parse(&json.text())
}

/// An error, thrown as an `Error` named `EinvoiceValidatorError` with its message, its
/// `kind` and its details.
fn js_error(error: &Error) -> JsValue {
    let js = js_sys::Error::new(&error.to_string());
    js.set_name("EinvoiceValidatorError");
    // The details are plain JSON members: an error in them would be a bug, and the
    // error is thrown with its message all the same.
    if let Ok(details) = parse(&Json::Object(render::error_details(error))) {
        Object::assign(&js, &details.unchecked_into());
    }
    js.into()
}

/// The JavaScript class of an argument error.
#[derive(Debug, Clone, Copy)]
enum ArgumentError {
    /// `TypeError`: a value of the wrong type.
    Type,
    /// `RangeError`: a value out of range.
    Range,
}

/// A `TypeError` or `RangeError` with the kind `argument`.
fn argument_error(class: ArgumentError, message: &str) -> JsValue {
    let error: js_sys::Error = match class {
        ArgumentError::Type => js_sys::TypeError::new(message).into(),
        ArgumentError::Range => js_sys::RangeError::new(message).into(),
    };
    // Setting a data property on a fresh error object does not fail.
    let _ = Reflect::set(
        &error,
        &JsValue::from_str("kind"),
        &JsValue::from_str("argument"),
    );
    error.into()
}
