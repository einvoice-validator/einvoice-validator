// Every case of the CEN EN 16931 UBL and CII 1.3.17 corpora (cases.tsv and mutations.tsv)
// through the package under Node.js, each with its built-in rule set: the findings of each
// report (kind, id, flag, role, location, rule context, pattern, test and text, in order)
// must be those of the Saxon-HE golden, and where Saxon-HE stops with a dynamic error the
// package must stop with the same code. A case refused under the default maximum depth is
// validated again with the largest (DEEP_CASES lists them). Prints the time per document.
// EINVOICE_REPO is the repository root.
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import { Validator } from "einvoice-validator";

import { CORPORA, DEEP_CASES, compare, corpusCases, parseGolden, validateCase } from "./golden.mjs";

const REPO = process.env.EINVOICE_REPO;
assert.ok(REPO, "EINVOICE_REPO must name the repository root");

const deep = [];

for (const { dir, ruleSet, minimum } of CORPORA) {
  test(`every case of the corpus ${dir} gives the Saxon-HE findings (${ruleSet})`, () => {
    const cases = corpusCases(
      readFileSync(join(REPO, dir, "cases.tsv"), "utf8"),
      readFileSync(join(REPO, dir, "mutations.tsv"), "utf8"),
      dir,
      (path) => existsSync(join(REPO, path)),
    );
    assert.ok(cases.length >= minimum, `${cases.length} cases`);
    const started = performance.now();
    const validator = new Validator(ruleSet);
    const loaded = performance.now();
    const failures = [];
    let errors = 0;
    let bytes = 0;
    let validating = 0;
    for (const c of cases) {
      const document = readFileSync(join(REPO, c.input));
      bytes += document.length;
      const golden = parseGolden(readFileSync(join(REPO, c.golden), "utf8"), c.isError);
      const t0 = performance.now();
      const { report, thrown, raised } = validateCase(validator, document);
      validating += performance.now() - t0;
      if (raised) deep.push(c.id);
      if (golden.error !== undefined) errors += 1;
      const differences = compare(golden, report, thrown);
      if (differences.length > 0) failures.push(`${c.id}:\n  ${differences.join("\n  ")}`);
    }
    validator.free();
    console.log(
      `${dir}: ${cases.length} cases (${errors} on which Saxon-HE stops), ${(bytes / 1e6).toFixed(1)} MB: ` +
        `rule set ${ruleSet} loaded in ${(loaded - started).toFixed(0)} ms, ` +
        `validated in ${(validating / 1000).toFixed(2)} s (validate() calls only), ` +
        `${(validating / cases.length).toFixed(2)} ms per document`,
    );
    assert.deepEqual(failures, [], `${failures.length} cases differ from Saxon-HE`);
  });
}

test("only the known deep cases needed a larger maximum depth", () => {
  assert.deepEqual(deep, DEEP_CASES);
});
