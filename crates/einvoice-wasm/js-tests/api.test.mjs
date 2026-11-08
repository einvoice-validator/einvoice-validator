// The API of the package under Node.js, imported by its name from an installed copy of
// the packed package (crates/einvoice-wasm/npm.sh test). No network. EINVOICE_REPO is the
// repository root, where the documents are read.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import init, {
  Validator,
  builtinRuleSets,
  licences,
  version,
} from "einvoice-validator";

import { compare, parseGolden } from "./golden.mjs";

const REPO = process.env.EINVOICE_REPO;
assert.ok(REPO, "EINVOICE_REPO must name the repository root");
const CEN = "corpus/cen-en16931-ubl-1.3.17";
const read = (path) => readFileSync(join(REPO, path));
const EXAMPLE = "artefacts/cen-en16931-ubl-1.3.17/ubl/examples/ubl-tc434-example1.xml";
const BR_02 = `${CEN}/vefa/Invoice-unit-UBL/BR-02.002.xml`;
const BR_CO_10 = `${CEN}/vefa/Invoice-unit-UBL/BR-CO-10.008.xml`;
const DUPLICATE = `${CEN}/mutations/duplicates/invoice-number.xml`;
const MALFORMED = new TextEncoder().encode(
  '<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2">\n  <a></Invoice>',
);

const PACKAGE_JSON = fileURLToPath(import.meta.resolve("einvoice-validator/package.json"));
const PACKAGE_DIR = dirname(PACKAGE_JSON);
const packageJson = JSON.parse(readFileSync(PACKAGE_JSON, "utf8"));

// One validator for the whole file: loading the rule set is the slow part.
const validator = new Validator("cen-ubl");

/** Runs f, which must throw; returns what it threw. */
function thrown(f) {
  try {
    f();
  } catch (e) {
    return e;
  }
  assert.fail("did not throw");
}

test("the package is ready on import, init() resolves at once", async () => {
  await init();
  assert.equal(version(), packageJson.version);
  assert.deepEqual(builtinRuleSets(), ["cen-ubl"]);
});

test("an official CEN example is valid", () => {
  const report = validator.validate(read(EXAMPLE));
  assert.equal(report.format, "einvoice-validator");
  assert.equal(report.format_version, 1);
  assert.deepEqual(report.tool, { name: "einvoice-validator", version: packageJson.version });
  assert.equal(report.status, "valid");
  assert.equal(report.error, null);
  assert.equal(report.document_uri, null);
  assert.deepEqual(report.counts, {
    schema_errors: 0,
    fatal: 0,
    warning: 0,
    other: 0,
    stylesheet_errors: 0,
  });
  assert.deepEqual(report.schema_errors, []);
  assert.deepEqual(report.findings, []);
  assert.deepEqual(report.stylesheet_errors, []);
  assert.deepEqual(report.warnings, []);
  assert.equal(report.rule_set.name, "cen-ubl");
  assert.equal(report.rule_set.version, "1.3.17");
  assert.equal(report.rule_set.schema, "ubl-2.1");
  assert.equal(report.rule_set.stylesheets.length, 1);
  assert.equal(report.rule_set.stylesheets[0].name, "EN16931-UBL");
  assert.match(report.rule_set.stylesheets[0].sha256, /^[0-9a-f]{64}$/);
  assert.deepEqual(validator.ruleSet, report.rule_set);
  // The members, in the order of the JSON format of einvoice-validate.
  assert.deepEqual(Object.keys(report), [
    "format",
    "format_version",
    "tool",
    "rule_set",
    "document_uri",
    "status",
    "error",
    "counts",
    "schema_errors",
    "findings",
    "stylesheet_errors",
    "warnings",
  ]);
});

test("VEFA BR-02.002 fails BR-02 and the schema, as Saxon-HE reports", () => {
  const report = validator.validate(read(BR_02));
  assert.equal(report.status, "invalid");
  const br02 = report.findings.filter((f) => f.id === "BR-02");
  assert.equal(br02.length, 1);
  assert.deepEqual(Object.keys(br02[0]), [
    "stylesheet",
    "kind",
    "id",
    "flag",
    "severity",
    "role",
    "location",
    "context",
    "pattern",
    "test",
    "text",
  ]);
  assert.equal(br02[0].kind, "failed-assert");
  assert.equal(br02[0].flag, "fatal");
  assert.equal(br02[0].severity, "fatal");
  assert.equal(br02[0].stylesheet, "EN16931-UBL");
  assert.equal(br02[0].text, "[BR-02]-An Invoice shall have an Invoice number (BT-1).");
  assert.equal(
    br02[0].location,
    "/*:Invoice[namespace-uri()='urn:oasis:names:specification:ubl:schema:xsd:Invoice-2'][1]",
  );
  assert.equal(report.counts.fatal, report.findings.filter((f) => f.severity === "fatal").length);
  assert.equal(report.schema_errors.length, 1);
  const [schema] = report.schema_errors;
  assert.equal(schema.kind, "missing-element");
  assert.ok(Number.isInteger(schema.line) && schema.line > 0);
  assert.ok(Number.isInteger(schema.column) && schema.column > 0);
  assert.deepEqual(
    compare(parseGolden(read(`${CEN}/golden/vefa/Invoice-unit-UBL/BR-02.002.xml.svrl.txt`).toString()), report),
    [],
  );
});

test("VEFA BR-CO-10.008 fails BR-CO-10 (exact decimal sums)", () => {
  const report = validator.validate(read(BR_CO_10));
  assert.equal(report.status, "invalid");
  assert.ok(report.findings.some((f) => f.id === "BR-CO-10" && f.severity === "fatal"));
  assert.deepEqual(
    compare(parseGolden(read(`${CEN}/golden/vefa/Invoice-unit-UBL/BR-CO-10.008.xml.svrl.txt`).toString()), report),
    [],
  );
});

test("a document that is not well-formed XML is invalid, with the line and column", () => {
  const report = validator.validate(MALFORMED);
  assert.equal(report.status, "invalid");
  assert.equal(report.error.kind, "not-xml");
  assert.equal(report.error.line, 2);
  assert.equal(report.error.column, 6);
  assert.match(report.error.message, /^2:6: not well-formed XML: /);
  assert.deepEqual(report.findings, []);
  assert.deepEqual(report.schema_errors, []);
  // validateSvrl has no SVRL to give: it throws.
  const e = thrown(() => validator.validateSvrl(MALFORMED));
  assert.ok(e instanceof Error);
  assert.equal(e.name, "EinvoiceValidatorError");
  assert.equal(e.kind, "not-xml");
  assert.equal(e.line, 2);
  assert.equal(e.column, 6);
});

test("an unknown rule-set name is a bundle error", () => {
  for (const make of [() => new Validator("no-such-rule-set"), () => Validator.builtin("cen-cii-0")]) {
    const e = thrown(make);
    assert.ok(e instanceof Error);
    assert.equal(e.name, "EinvoiceValidatorError");
    assert.equal(e.kind, "bundle");
    assert.match(e.message, /no built-in rule set named .*\(built in: cen-ubl\)/);
  }
  const e = thrown(() => new Validator(42));
  assert.ok(e instanceof TypeError);
  assert.equal(e.kind, "argument");
  assert.ok(Validator.builtin("cen-ubl") instanceof Validator);
});

test("raw SVRL", () => {
  const reports = validator.validateSvrl(read(BR_02), { documentUri: "urn:example:br-02" });
  assert.equal(reports.length, 1);
  const [svrl] = reports;
  assert.deepEqual(Object.keys(svrl), ["stylesheet", "version", "sha256", "svrl"]);
  assert.equal(svrl.stylesheet, "EN16931-UBL");
  assert.equal(svrl.version, "1.3.17");
  assert.equal(svrl.sha256, validator.ruleSet.stylesheets[0].sha256);
  assert.match(svrl.svrl, /<svrl:schematron-output /);
  assert.match(svrl.svrl, /<svrl:failed-assert [^>]*id="BR-02"/);
  // As under Saxon-HE, the CEN stylesheet leaves @documents empty (corpus/SVRL-CANONICAL.md).
  assert.match(svrl.svrl, /<svrl:active-pattern documents="" id="UBL-model"/);
});

test("the document URI", () => {
  const report = validator.validate(read(EXAMPLE), { documentUri: "file:///invoices/a.xml" });
  assert.equal(report.document_uri, "file:///invoices/a.xml");
  assert.equal(report.status, "valid");
});

test("a stylesheet stopped by a dynamic error on a document with schema errors", () => {
  const golden = read(`${CEN}/golden/mutations/duplicates/invoice-number.xml.error.txt`).toString();
  const code = golden.match(/^error (\S+)$/m)[1];
  const report = validator.validate(read(DUPLICATE));
  assert.equal(report.status, "invalid");
  assert.ok(report.schema_errors.length > 0);
  assert.equal(report.stylesheet_errors.length, 1);
  assert.equal(report.stylesheet_errors[0].code, code);
  assert.equal(report.counts.stylesheet_errors, 1);
  const e = thrown(() => validator.validateSvrl(read(DUPLICATE)));
  assert.equal(e.name, "EinvoiceValidatorError");
  assert.equal(e.kind, "transform");
  assert.equal(e.code, code);
  assert.equal(e.stylesheet, "EN16931-UBL");
});

test("an unsupported encoding is an unsupported error", () => {
  const e = thrown(() =>
    validator.validate(new TextEncoder().encode("<?xml version='1.0' encoding='ISO-8859-1'?><a/>")),
  );
  assert.equal(e.name, "EinvoiceValidatorError");
  assert.equal(e.kind, "unsupported");
  assert.match(e.message, /ISO-8859-1/);
});

test("the options and the clock", () => {
  const bytes = read(EXAMPLE);
  for (const options of [
    undefined,
    null,
    {},
    { now: new Date("2026-01-02T03:04:05Z") },
    { now: 0 },
    { now: Date.now(), timezoneMinutes: -300 },
    { timezoneMinutes: 840 },
    { documentUri: null, now: undefined, timezoneMinutes: null },
  ]) {
    assert.equal(validator.validate(bytes, options).status, "valid", JSON.stringify(options));
  }
  for (const [options, Class] of [
    [5, TypeError],
    ["x", TypeError],
    [{ documentURI: "x" }, TypeError],
    [{ documentUri: 5 }, TypeError],
    [{ now: "2026-01-01" }, TypeError],
    [{ now: new Date("not a date") }, RangeError],
    [{ now: 1.5 }, RangeError],
    [{ now: 8.64e15 + 1 }, RangeError],
    [{ now: Number.NaN }, RangeError],
    [{ timezoneMinutes: "60" }, TypeError],
    [{ timezoneMinutes: 841 }, RangeError],
    [{ timezoneMinutes: 0.5 }, RangeError],
  ]) {
    const e = thrown(() => validator.validate(bytes, options));
    assert.ok(e instanceof Class, `${JSON.stringify(options)}: ${e}`);
    assert.equal(e.kind, "argument", JSON.stringify(options));
  }
});

test("the document must be a Uint8Array", () => {
  for (const document of ["<Invoice/>", [60, 97, 47, 62], new ArrayBuffer(4), undefined]) {
    const e = thrown(() => validator.validate(document));
    assert.ok(e instanceof TypeError);
    assert.equal(e.kind, "argument");
  }
  // A Buffer is a Uint8Array; a view on part of a buffer is read as such.
  const bytes = read(EXAMPLE);
  const padded = new Uint8Array(bytes.length + 8);
  padded.set(bytes, 4);
  assert.equal(validator.validate(padded.subarray(4, 4 + bytes.length)).status, "valid");
  assert.deepEqual(padded.subarray(4, 4 + bytes.length), new Uint8Array(bytes));
});

test("the licences of what the package embeds, with the texts it ships", () => {
  const list = licences();
  assert.deepEqual(
    list.map((l) => l.id),
    [
      "MIT",
      "EUPL-1.2",
      "LicenseRef-OASIS-Copyright-Notice",
      "LicenseRef-UN-CEFACT-Copyright-Notice",
      "W3C-19980720",
    ],
  );
  // Every term of the package's licence has its text in LICENSES/, the same text.
  assert.deepEqual(packageJson.license.split(" AND "), list.map((l) => l.id));
  for (const l of list) {
    assert.ok(l.covers.length > 0);
    const shipped = readFileSync(join(PACKAGE_DIR, "LICENSES", `${l.id}.txt`), "utf8");
    assert.equal(l.text, shipped, l.id);
  }
});

test("require() from CommonJS gets the same API", () => {
  const require = createRequire(import.meta.url);
  const cjs = require("einvoice-validator");
  assert.equal(cjs.version(), version());
  assert.equal(cjs.Validator, Validator);
});
