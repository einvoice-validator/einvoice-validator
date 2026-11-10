/// <reference lib="esnext.disposable" />
// The class Validator declares [Symbol.dispose] (for `using`): the line above gives the
// declaration of Symbol.dispose to programs compiled with an older `lib`. It must stay
// the first line of this section, which wasm-bindgen writes first in the declarations,
// after two comments only, where a triple-slash directive is valid.

/** The options of one validation. */
export interface ValidateOptions {
  /**
   * The URI of the document, as the reference engine takes it: the base URI of its nodes,
   * against which the stylesheets resolve relative URIs (as in Saxon-HE,
   * `document-uri(/)` stays empty). Copied into the report as `document_uri`. None by
   * default.
   */
  documentUri?: string | null;
  /**
   * The value of `fn:current-dateTime()` in the stylesheets: a `Date`, or milliseconds
   * from the Unix epoch (a whole number). By default `Date.now()`, read when the
   * validation starts.
   */
  now?: Date | number | null;
  /**
   * The implicit timezone of the stylesheets, in minutes east of UTC, from -840 to 840
   * (note that `Date.prototype.getTimezoneOffset()` counts the other way: pass
   * `-new Date().getTimezoneOffset()` for the local timezone). 0 (UTC) by default.
   */
  timezoneMinutes?: number | null;
  /**
   * The maximum element depth of the document, the document element being at depth 1: a
   * whole number from 1 to 400, by default 256 (libxml2's default). A document with an
   * element nested deeper is refused with an `EinvoiceValidatorError` of kind
   * `unsupported` and `limit` `"depth"`. The stylesheets recurse once or twice per level,
   * within 256 KiB of the module's stack, which keeps the JavaScript engine's own stack
   * from running out (a Web Worker in Chrome has the smallest): a document that needs more
   * is refused with the `limit` `"stack"` (beyond about 240 levels when a rule reports on
   * every level, 480 otherwise).
   */
  maxDepth?: number | null;
  /**
   * Fail closed: every finding that would otherwise be of severity `"other"` (a flag
   * other than `fatal` and `warning`, or none, that the rule set gives no severity of
   * its own) is `"fatal"`, and makes the document invalid. `false` by default.
   */
  failClosed?: boolean | null;
}

/** A stylesheet of a rule set. */
export interface StylesheetInfo {
  /** Its name in the rule set: `"EN16931-UBL"`. */
  name: string;
  /** The version of the rule set it implements: `"1.3.17"`. */
  version: string;
  /** Its path in the bundle. */
  path: string;
  /** The SHA-256 of the stylesheet, lower-case hexadecimal. */
  sha256: string;
}

/** A rule set, as the JSON report of `einvoice-validate` gives it. */
export interface RuleSetInfo {
  /** Its short name: `"cen-ubl"`, `"cen-cii"`. */
  name: string;
  /** Its title: `"CEN EN 16931 UBL"`. */
  title: string;
  /** The version of the official artefacts: `"1.3.17"`. */
  version: string;
  /** The XML schema a document is validated against first, if any: `"ubl-2.1"` or `"cii-d16b"`. */
  schema: string | null;
  /** The stylesheets, in the order they run. */
  stylesheets: StylesheetInfo[];
}

/**
 * The kind of an error: `"argument"` for a `TypeError` or `RangeError` raised by an
 * invalid argument, the others for an `EinvoiceValidatorError`.
 */
export type ErrorKind =
  | "argument"
  | "io"
  | "bundle"
  | "not-xml"
  | "unsupported"
  | "transform"
  | "internal";

/**
 * The `Error` thrown when no complete verdict can be given (its `name` is
 * `"EinvoiceValidatorError"`). Arguments of the wrong type or out of range are thrown as
 * a `TypeError` or `RangeError` with the kind `"argument"` instead.
 */
export interface EinvoiceValidatorError extends Error {
  name: "EinvoiceValidatorError";
  /**
   * `bundle`: an unknown rule-set name, or a rule set that cannot be loaded;
   * `not-xml`: a document that is not well-formed XML (thrown by `validateSvrl` only;
   * `validate` reports it as an invalid document); `unsupported`: an encoding the
   * module does not read, a document type declaration, or a construct the engine refuses rather
   * than answering differently from the reference engine, and a document beyond a limit
   * (`limit` set); `transform`: a dynamic error raised by a stylesheet on the document;
   * `internal`: a bug.
   */
  kind: Exclude<ErrorKind, "argument">;
  /**
   * `unsupported`, for a document beyond a limit: `"depth"`, an element nested deeper
   * than the option `maxDepth`; `"nesting"` or `"stack"`, templates and functions of the
   * stylesheets invoked one inside another beyond the nesting that depth allows, or
   * beyond the stack of the module.
   */
  limit?: "depth" | "nesting" | "stack" | "other";
  /** With `limit`: its value (bytes for `"stack"`). */
  max?: number;
  /**
   * `not-xml` and the `limit` `"depth"`: the 1-based line. `transform`: the line of the
   * stylesheet, or null.
   */
  line?: number | null;
  /** `not-xml` and the `limit` `"depth"`: the 1-based column. */
  column?: number;
  /** `transform`: the stylesheet, by its name in the rule set. */
  stylesheet?: string;
  /** `transform`: the XPath or XSLT error code (`"FORG0001"`, ...). */
  code?: string;
  /** `bundle` (a checksum mismatch) and `io`: the file concerned. */
  path?: string;
}

/** A validity error of the document against the XML schema of the rule set. */
export interface SchemaError {
  /** A stable kind: `"missing-element"`, `"unexpected-element"`, `"invalid-value"`, ... */
  kind: string;
  /** The 1-based line. */
  line: number;
  /** The 1-based column. */
  column: number;
  /** The element concerned, in Clark notation: `"{namespace}local"`. */
  element: string;
  /** The attribute concerned, if any. */
  attribute: string | null;
  /** The explanation. */
  message: string;
}

/** A failed assert or a successful report of a stylesheet, in SVRL order. */
export interface Finding {
  /** The stylesheet, by its name in the rule set. */
  stylesheet: string;
  /** `svrl:failed-assert` or `svrl:successful-report`. */
  kind: "failed-assert" | "successful-report";
  /** The rule id: `"BR-02"`, if the rule has one. */
  id: string | null;
  /** The Schematron flag as the rule set writes it: `"fatal"`, `"warning"`. */
  flag: string | null;
  /** The severity the flag stands for; only `fatal` makes the document invalid. */
  severity: "fatal" | "warning" | "other";
  /** The Schematron role, if any. */
  role: string | null;
  /** The XPath of the node concerned, as the stylesheet writes it. */
  location: string;
  /** The context of the rule that fired. */
  context: string | null;
  /** The id of the pattern the rule belongs to. */
  pattern: string | null;
  /** The XPath test of the assert or report. */
  test: string;
  /** The message, whitespace normalised. */
  text: string;
}

/** A stylesheet stopped by a dynamic error, on a document that has schema errors. */
export interface StylesheetError {
  /** The stylesheet, by its name in the rule set. */
  stylesheet: string;
  /** The XPath or XSLT error code: `"FORG0001"`. */
  code: string;
  /** The line of the stylesheet instruction, when known. */
  line: number | null;
  /** The explanation. */
  message: string;
}

/**
 * The report of one validation. After the header (`format` to `document_uri`), its
 * members are those of a document of the JSON format of `einvoice-validate`, version 1,
 * in the same order, without `path`; the names are the same.
 */
export interface Report {
  /** `"einvoice-validator"`. */
  format: "einvoice-validator";
  /** 1. Any change that is not a pure addition of members increases it. */
  format_version: 1;
  /** The package that wrote the report. */
  tool: { name: "einvoice-validator"; version: string };
  /** The rule set the document was validated with. */
  rule_set: RuleSetInfo;
  /** The `documentUri` option, or null. */
  document_uri: string | null;
  /**
   * The verdict: `"valid"` when there is no schema error and no finding with severity
   * `fatal` (warnings are allowed); `"invalid"` otherwise, and for a document that is not
   * well-formed XML (`error` is then set and the lists are empty).
   */
  status: "valid" | "invalid";
  /** Set only for a document that is not well-formed XML. */
  error: null | {
    kind: "not-xml";
    message: string;
    /** The 1-based line. */
    line: number;
    /** The 1-based column. */
    column: number;
  };
  /** The number of schema errors, of findings by severity, and of stylesheet errors. */
  counts: {
    schema_errors: number;
    fatal: number;
    warning: number;
    other: number;
    stylesheet_errors: number;
  };
  /** The schema validity errors, sorted by position. */
  schema_errors: SchemaError[];
  /** The failed asserts and successful reports, stylesheet after stylesheet. */
  findings: Finding[];
  /**
   * The stylesheets that a dynamic error stopped (the official CEN stylesheet does on a
   * non-numeric amount): only on a document that already has schema errors, which keeps
   * the report and the other stylesheets' findings. Without schema errors, the error is
   * thrown (kind `transform`).
   */
  stylesheet_errors: StylesheetError[];
  /** Recoverable conditions a stylesheet reported, as the reference engine reports them. */
  warnings: { stylesheet: string; message: string }[];
}

/** The SVRL report of one stylesheet, exactly as it wrote it. */
export interface SvrlReport {
  /** The stylesheet, by its name in the rule set. */
  stylesheet: string;
  /** The version of the rule set it implements. */
  version: string;
  /** The SHA-256 of the stylesheet that ran. */
  sha256: string;
  /** The SVRL report (XML). */
  svrl: string;
}

/** A licence of what the package embeds. */
export interface Licence {
  /** The SPDX identifier, or a `LicenseRef-` one. */
  id: string;
  /** What the licence covers. */
  covers: string;
  /** Its full text. */
  text: string;
}
