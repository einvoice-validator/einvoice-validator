// Every case of the CEN EN 16931 UBL 1.3.17 corpus (cases.tsv and mutations.tsv) through
// the package under Node.js: the findings of each report (kind, id, flag, role, location,
// rule context, pattern, test and text, in order) must be those of the Saxon-HE golden,
// and where Saxon-HE stops with a dynamic error the package must stop with the same code.
// Prints the time per document. EINVOICE_REPO is the repository root.
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import { Validator } from "einvoice-validator";

import { compare, corpusCases, parseGolden } from "./golden.mjs";

const REPO = process.env.EINVOICE_REPO;
assert.ok(REPO, "EINVOICE_REPO must name the repository root");
const CEN = "corpus/cen-en16931-ubl-1.3.17";

test("every case of the CEN corpus gives the Saxon-HE findings", () => {
  const cases = corpusCases(
    readFileSync(join(REPO, CEN, "cases.tsv"), "utf8"),
    readFileSync(join(REPO, CEN, "mutations.tsv"), "utf8"),
    CEN,
    (path) => existsSync(join(REPO, path)),
  );
  assert.ok(cases.length > 1600, `${cases.length} cases`);
  const started = performance.now();
  const validator = new Validator("cen-ubl");
  const loaded = performance.now();
  const failures = [];
  let errors = 0;
  let bytes = 0;
  let validating = 0;
  for (const c of cases) {
    const document = readFileSync(join(REPO, c.input));
    bytes += document.length;
    const golden = parseGolden(readFileSync(join(REPO, c.golden), "utf8"), c.isError);
    let report;
    let thrown;
    const t0 = performance.now();
    try {
      report = validator.validate(document, { now: 1_789_000_123_456 });
    } catch (e) {
      thrown = e;
    }
    validating += performance.now() - t0;
    if (golden.error !== undefined) errors += 1;
    const differences = compare(golden, report, thrown);
    if (differences.length > 0) failures.push(`${c.id}:\n  ${differences.join("\n  ")}`);
  }
  validator.free();
  console.log(
    `${cases.length} cases (${errors} on which Saxon-HE stops), ${(bytes / 1e6).toFixed(1)} MB: ` +
      `rule set loaded in ${(loaded - started).toFixed(0)} ms, ` +
      `validated in ${(validating / 1000).toFixed(2)} s (validate() calls only), ` +
      `${(validating / cases.length).toFixed(2)} ms per document`,
  );
  assert.deepEqual(failures, [], `${failures.length} cases differ from Saxon-HE`);
});
