// The expected results of the reference engine (Saxon-HE), read from the goldens of the
// corpus (corpus/SVRL-CANONICAL.md), and their comparison with a report of the package.
// Shared by the Node.js tests and the browser test; no dependency on Node.js.

/** Reads a quoted value of a canonical SVRL line, from index `i` (on the opening quote). */
function quoted(line, i) {
  if (line[i] !== '"') throw new Error(`expected a quote at ${i}: ${line}`);
  let out = "";
  for (i += 1; i < line.length; i += 1) {
    const c = line[i];
    if (c === '"') return [out, i + 1];
    if (c !== "\\") {
      out += c;
      continue;
    }
    i += 1;
    const e = line[i];
    if (e === "\\" || e === '"') out += e;
    else if (e === "n") out += "\n";
    else if (e === "r") out += "\r";
    else if (e === "t") out += "\t";
    else if (e === "u") {
      const end = line.indexOf("}", i);
      out += String.fromCodePoint(parseInt(line.slice(i + 2, end), 16));
      i = end;
    } else throw new Error(`unknown escape \\${e}: ${line}`);
  }
  throw new Error(`unterminated quote: ${line}`);
}

/** One canonical SVRL line: its level, kind, attributes and text (or null). */
function parseLine(raw) {
  const level = raw.length - raw.trimStart().length;
  const line = raw.trimStart();
  let i = line.indexOf(" ");
  if (i < 0) return { level: level / 2, kind: line, attributes: {}, text: null };
  const kind = line.slice(0, i);
  const attributes = {};
  let text = null;
  while (i < line.length) {
    if (line[i] === " ") {
      i += 1;
      continue;
    }
    if (line[i] === '"') {
      [text, i] = quoted(line, i);
      continue;
    }
    const eq = line.indexOf("=", i);
    const name = line.slice(i, eq);
    [attributes[name], i] = quoted(line, eq + 1);
  }
  return { level: level / 2, kind, attributes, text };
}

/**
 * The findings of a golden (failed asserts and successful reports, in order), with the
 * pattern and the rule context before each, as the report of the package gives them; or
 * `{ error: code }` for a golden of a case on which the reference engine stops.
 */
export function parseGolden(text, isError) {
  if (isError) {
    const code = text.split("\n").find((l) => l.startsWith("error "));
    return { error: code.slice("error ".length).trim() };
  }
  const lines = text.split("\n").filter((l) => l.length > 0);
  if (lines[0] !== "svrl-canonical 1") throw new Error(`not a canonical SVRL: ${lines[0]}`);
  const findings = [];
  let pattern = null;
  let context = null;
  let current = null;
  for (const raw of lines.slice(1)) {
    const line = parseLine(raw);
    if (line.level === 0) {
      current = null;
      if (line.kind === "active-pattern") {
        pattern = line.attributes.id ?? line.attributes.name ?? null;
        context = null;
      } else if (line.kind === "fired-rule") {
        context = line.attributes.context ?? null;
      } else if (line.kind === "failed-assert" || line.kind === "successful-report") {
        current = {
          kind: line.kind,
          id: line.attributes.id ?? null,
          flag: line.attributes.flag ?? null,
          role: line.attributes.role ?? null,
          location: line.attributes.location ?? "",
          context,
          pattern,
          test: line.attributes.test ?? "",
          text: "",
        };
        findings.push(current);
      }
    } else if (line.level === 1 && current !== null && line.kind === "text") {
      current.text += line.text;
    }
  }
  return { findings };
}

const FIELDS = ["kind", "id", "flag", "role", "location", "context", "pattern", "test", "text"];

/**
 * The differences between a golden and what the package gave for the case: the report of
 * `validate`, or the error it threw. An empty list when they agree.
 */
export function compare(golden, report, thrown) {
  if (golden.error !== undefined) {
    if (thrown !== undefined) {
      return thrown.kind === "transform" && thrown.code === golden.error
        ? []
        : [`expected error ${golden.error}, thrown ${thrown.kind} ${thrown.code}: ${thrown.message}`];
    }
    const codes = report.stylesheet_errors.map((e) => e.code);
    return report.schema_errors.length > 0 && codes.length === 1 && codes[0] === golden.error
      ? []
      : [`expected error ${golden.error}, got a report with stylesheet errors [${codes}]`];
  }
  if (thrown !== undefined) return [`thrown ${thrown.kind}: ${thrown.message}`];
  const out = [];
  if (report.stylesheet_errors.length > 0) out.push("stylesheet errors in the report");
  const got = report.findings;
  const n = Math.max(got.length, golden.findings.length);
  for (let i = 0; i < n && out.length < 5; i += 1) {
    const g = golden.findings[i];
    const f = got[i];
    if (g === undefined || f === undefined) {
      out.push(`finding ${i}: expected ${JSON.stringify(g)}, got ${JSON.stringify(f)}`);
      continue;
    }
    for (const field of FIELDS) {
      if (g[field] !== f[field]) {
        out.push(`finding ${i} (${g.id}): ${field} expected ${JSON.stringify(g[field])}, got ${JSON.stringify(f[field])}`);
      }
    }
  }
  return out;
}

/** The cases of the corpus: `{ id, input, golden, isError }` from cases.tsv and mutations.tsv. */
export function corpusCases(casesTsv, mutationsTsv, ruleSetDir, goldenExists) {
  const out = [];
  for (const tsv of [casesTsv, mutationsTsv]) {
    for (const line of tsv.split("\n").slice(1)) {
      if (line.length === 0) continue;
      const [id, input] = line.split("\t");
      const svrl = `${ruleSetDir}/golden/${id}.svrl.txt`;
      const error = `${ruleSetDir}/golden/${id}.error.txt`;
      const isError = !goldenExists(svrl);
      out.push({ id, input, golden: isError ? error : svrl, isError });
    }
  }
  return out;
}
