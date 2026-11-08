// The browser test, run in the page that js-tests/browser.mjs serves: the package as a
// browser loads it (the web entry point, the module fetched by init()), the API checks,
// then every case of the CEN corpus against the Saxon-HE goldens. The result is posted
// back to the server.
import init, { Validator, builtinRuleSets, version } from "/pkg/einvoice_validator.js";

import { compare, corpusCases, parseGolden } from "/tests/golden.mjs";

const CEN = "corpus/cen-en16931-ubl-1.3.17";
const log = [];
const failures = [];

async function fetchBytes(path) {
  const response = await fetch(`/repo/${path}`);
  if (!response.ok) throw new Error(`${path}: HTTP ${response.status}`);
  return new Uint8Array(await response.arrayBuffer());
}

async function fetchText(path) {
  return new TextDecoder().decode(await fetchBytes(path));
}

/** A progress message, printed at once by the runner (to see where a run stops). */
async function progress(message) {
  await fetch("/progress", { method: "POST", body: message });
}

function check(name, condition, detail = "") {
  if (!condition) failures.push(`${name}${detail ? `: ${detail}` : ""}`);
}

async function run() {
  const t0 = performance.now();
  await init();
  const t1 = performance.now();
  const validator = new Validator("cen-ubl");
  const t2 = performance.now();
  log.push(`${navigator.userAgent}`);
  log.push(`init() ${(t1 - t0).toFixed(0)} ms, new Validator("cen-ubl") ${(t2 - t1).toFixed(0)} ms`);
  check("version", typeof version() === "string" && version().length > 0);
  check("builtinRuleSets", JSON.stringify(builtinRuleSets()) === '["cen-ubl"]');

  const example = validator.validate(
    await fetchBytes("artefacts/cen-en16931-ubl-1.3.17/ubl/examples/ubl-tc434-example1.xml"),
  );
  check("official example valid", example.status === "valid", JSON.stringify(example.counts));
  check("rule set version", example.rule_set.version === "1.3.17");

  const br02 = validator.validate(await fetchBytes(`${CEN}/vefa/Invoice-unit-UBL/BR-02.002.xml`));
  check("BR-02.002 invalid", br02.status === "invalid");
  check("BR-02 fatal", br02.findings.some((f) => f.id === "BR-02" && f.severity === "fatal"));
  check("BR-02.002 schema error", br02.schema_errors.length === 1 && br02.schema_errors[0].line > 0);

  const malformed = validator.validate(new TextEncoder().encode("<Invoice>\n<a></Invoice>"));
  check(
    "malformed XML",
    malformed.status === "invalid" && malformed.error.kind === "not-xml" && malformed.error.line === 2,
    JSON.stringify(malformed.error),
  );

  try {
    new Validator("no-such-rule-set");
    check("unknown rule set throws", false);
  } catch (e) {
    check("unknown rule set", e instanceof Error && e.name === "EinvoiceValidatorError" && e.kind === "bundle", `${e}`);
  }
  try {
    validator.validate(new Uint8Array(0), { now: new Date(Number.NaN) });
    check("invalid Date throws", false);
  } catch (e) {
    check("invalid Date", e instanceof RangeError && e.kind === "argument", `${e}`);
  }

  await progress(`API checks done, ${failures.length} failures`);
  const cases = [];
  const golden = new Map();
  const [casesTsv, mutationsTsv, goldenList] = await Promise.all([
    fetchText(`${CEN}/cases.tsv`),
    fetchText(`${CEN}/mutations.tsv`),
    // The list of the golden files, from the server (they are not listed elsewhere).
    fetch(`/goldens/${CEN}`).then((r) => {
      if (!r.ok) throw new Error(`/goldens/${CEN}: HTTP ${r.status}`);
      return r.text();
    }),
  ]);
  const goldens = new Set(goldenList.split("\n"));
  cases.push(...corpusCases(casesTsv, mutationsTsv, CEN, (path) => goldens.has(path)));
  // Sixteen requests at a time.
  for (let i = 0; i < cases.length; i += 16) {
    await Promise.all(
      cases.slice(i, i + 16).map(async (c) => {
        const [document, expected] = await Promise.all([fetchBytes(c.input), fetchText(c.golden)]);
        golden.set(c.id, { document, expected: parseGolden(expected, c.isError) });
      }),
    );
  }
  await progress(`${cases.length} corpus cases fetched`);
  let validating = 0;
  let differing = 0;
  for (const c of cases) {
    const { document, expected } = golden.get(c.id);
    let report;
    let thrown;
    const start = performance.now();
    try {
      report = validator.validate(document, { now: 1_789_000_123_456 });
    } catch (e) {
      thrown = e;
    }
    validating += performance.now() - start;
    const differences = compare(expected, report, thrown);
    if (differences.length > 0) {
      differing += 1;
      if (differing <= 10) failures.push(`${c.id}: ${differences.join("; ")}`);
    }
  }
  if (differing > 10) failures.push(`... ${differing} cases differ from Saxon-HE in all`);
  log.push(
    `${cases.length} corpus cases, ${cases.length - differing} as Saxon-HE: ` +
      `${(validating / 1000).toFixed(2)} s, ${(validating / cases.length).toFixed(2)} ms per document`,
  );
  validator.free();
}

try {
  await run();
} catch (e) {
  failures.push(`exception: ${e}\n${e.stack}`);
}
await fetch("/result", {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ log, failures }),
});
document.body.textContent = failures.length === 0 ? "ok" : "failed";
