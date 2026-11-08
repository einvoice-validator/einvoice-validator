// Type-checked only (tsc --noEmit, strict), never run: the TypeScript declarations of
// the package, as a program that uses it sees them, by its name.
import init, {
  type EinvoiceValidatorError,
  type Finding,
  type Licence,
  type Report,
  type RuleSetInfo,
  type SvrlReport,
  type ValidateOptions,
  Validator,
  builtinRuleSets,
  licences,
  version,
} from "einvoice-validator";

export async function check(bytes: Uint8Array): Promise<string[]> {
  await init();
  const validator: Validator = new Validator("cen-ubl");
  const same: Validator = Validator.builtin("cen-ubl");
  same.free();
  const ruleSet: RuleSetInfo = validator.ruleSet;
  const cii: Validator = new Validator("cen-cii");
  const ciiSchema: string | null = cii.ruleSet.schema;
  cii.free();
  const options: ValidateOptions = {
    documentUri: "urn:x",
    now: new Date(),
    timezoneMinutes: 60,
    maxDepth: 1000,
    failClosed: true,
  };
  const report: Report = validator.validate(bytes, options);
  const status: "valid" | "invalid" = report.status;
  const formatVersion: 1 = report.format_version;
  const findings: Finding[] = report.findings;
  const severities: ("fatal" | "warning" | "other")[] = findings.map((f) => f.severity);
  const ids: (string | null)[] = findings.map((f) => f.id);
  const lines: number[] = report.schema_errors.map((e) => e.line);
  const notXml: number | undefined = report.error?.column;
  const svrl: SvrlReport[] = validator.validateSvrl(bytes, { now: 0 });
  const texts: string[] = svrl.map((s) => s.svrl);
  const licence: Licence[] = licences();
  try {
    validator.validate(bytes);
  } catch (e) {
    const error = e as EinvoiceValidatorError;
    const kind: "io" | "bundle" | "not-xml" | "unsupported" | "transform" | "internal" = error.kind;
    texts.push(kind, error.message);
    const limit: "depth" | "nesting" | "stack" | "other" | undefined = error.limit;
    const max: number | undefined = error.max;
    texts.push(String(limit), String(max));
  }
  // @ts-expect-error: maxDepth is a number.
  validator.validate(bytes, { maxDepth: "256" });
  // @ts-expect-error: the document is bytes, not a string.
  validator.validate("<Invoice/>");
  // @ts-expect-error: an unknown option.
  validator.validate(bytes, { documentURI: "x" });
  // @ts-expect-error: the status is not a boolean.
  const wrong: boolean = report.status;
  return [
    version(),
    ...builtinRuleSets(),
    ruleSet.version,
    String(ciiSchema),
    status,
    String(formatVersion),
    ...severities,
    ...ids.map((id) => id ?? "-"),
    ...lines.map(String),
    String(notXml),
    ...texts,
    ...licence.map((l) => l.id),
    String(wrong),
  ];
}
