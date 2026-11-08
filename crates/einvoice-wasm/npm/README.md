# einvoice-validator

Validate EN 16931 e-invoices (UBL 2.1 Invoice and CreditNote, UN/CEFACT CII D16B
CrossIndustryInvoice) against the **official** CEN validation artefacts, in WebAssembly: no
JVM, no native code, nothing fetched at run time. For browsers, bundlers and Node.js, with
TypeScript declarations.

Each validation runs the XML check (UTF-8, well-formed, no document type declaration, no
element nested deeper than 256 levels), the OASIS UBL 2.1 schema for UBL (the CII schemas are
not embedded), then the official compiled CEN EN 16931 Schematron stylesheet of the syntax
(release 1.3.17, embedded unmodified), executed with the behaviour of the reference engine,
Saxon-HE: on every case of the CEN UBL and CII test corpora, the findings are those Saxon-HE
reports. The package is the Rust library
[einvoice-validator](https://crates.io/crates/einvoice-validator) compiled to WebAssembly.

```sh
npm install einvoice-validator
```

## Node.js

The package is ready once imported (ESM, or `require()` on Node.js 22.12 and later).

```js
import { readFileSync } from "node:fs";
import { Validator } from "einvoice-validator";

// Built once (the schema loaded, the stylesheet prepared), then used for any number of
// documents: loading takes far longer than validating one.
const validator = new Validator("cen-ubl");
const report = validator.validate(readFileSync("invoice.xml"));
console.log(report.status); // "valid" or "invalid"
for (const f of report.findings) {
  console.log(`${f.severity} ${f.id}: ${f.text} (at ${f.location})`);
}
for (const e of report.schema_errors) {
  console.log(`schema ${e.line}:${e.column}: ${e.message}`);
}
```

## Browsers and bundlers

The default export `init()` fetches and compiles the WebAssembly module, next to the JavaScript
file (`new URL("einvoice_validator_bg.wasm", import.meta.url)`, which Vite, webpack 5 and other
bundlers resolve); call it once before anything else. It also accepts the module's URL, a
`Response`, its bytes or a compiled `WebAssembly.Module`; `initSync({ module })` instantiates
synchronously from bytes or a module.

```js
import init, { Validator } from "einvoice-validator";

await init();
const validator = new Validator("cen-ubl");
const bytes = new Uint8Array(await file.arrayBuffer());
const report = validator.validate(bytes);
```

The module is about 6.5 MB (1.0 MB with gzip, 0.7 MB with brotli): it embeds the CEN UBL and
CII stylesheets and the UBL 2.1 schemas. In a page, validate in a Web Worker to keep the main
thread free.

## API

- `new Validator(name)`, `Validator.builtin(name)`: the built-in rule set `name`, `"cen-ubl"`
  for UBL invoices and credit notes, `"cen-cii"` for CII invoices (no XML schema: the CII
  rules only); `builtinRuleSets()` lists them (`["cen-ubl", "cen-cii"]`). Each validates
  documents of its syntax only. `validator.ruleSet`: its name, title,
  version, schema and stylesheets (with their SHA-256). `validator.free()` releases its memory
  at once (otherwise it is released when the object is garbage-collected).
- `validator.validate(bytes, options?)`: the report of a document given as a `Uint8Array` (a
  Node.js `Buffer` is one), UTF-8.
- `validator.validateSvrl(bytes, options?)`: the SVRL report of each stylesheet, exactly as it
  wrote it: `[{ stylesheet, version, sha256, svrl }]`.
- `options`: `documentUri` (the URI of the document: the base URI of its nodes, as Saxon-HE
  takes it, copied into the report), `now` (the value of `fn:current-dateTime()`: a `Date` or
  milliseconds from the Unix epoch, by default `Date.now()` when the validation starts),
  `timezoneMinutes` (the implicit timezone, in minutes east of UTC, by default 0) and
  `maxDepth` (the maximum element depth of the document, the document element at depth 1: 1
  to 400, by default 256; the stylesheets recurse once or twice per level on the JavaScript
  engine's stack, and a Web Worker in Chrome runs out of stack at about 600 levels). Other
  members are refused.
- `licences()`: the licence of everything the package embeds, with what it covers and its full
  text, for applications that show them to their users. `version()`: the package's version.

### The report

The members of a document of the JSON format (version 1) of the command-line tool
`einvoice-validate`, with the same names, after a header:

```text
{
  format: "einvoice-validator", format_version: 1,
  tool: { name: "einvoice-validator", version },
  rule_set: { name: "cen-ubl", title, version: "1.3.17", schema: "ubl-2.1", stylesheets: [...] },
  document_uri: string | null,
  status: "valid" | "invalid",        valid: no schema error and no finding of severity fatal
  error: null | { kind: "not-xml", message, line, column },
  counts: { schema_errors, fatal, warning, other, stylesheet_errors },
  schema_errors: [{ kind, line, column, element, attribute, message }],
  findings: [{ stylesheet, kind, id, flag, severity, role, location, context, pattern, test, text }],
  stylesheet_errors: [{ stylesheet, code, line, message }],
  warnings: [{ stylesheet, message }]
}
```

A document that is not well-formed XML is `invalid`, with `error` set (its line and column) and
empty lists. `stylesheet_errors` lists a stylesheet stopped by a dynamic error (the official
CEN stylesheet stops on some malformed amounts, as it does under Saxon-HE) on a document that
already has schema errors.

### Errors

When no complete verdict can be given, the call throws an `Error` named
`EinvoiceValidatorError` with a `kind`: `bundle` (an unknown rule-set name), `unsupported` (an
encoding other than UTF-8, a document type declaration, or a construct the engine refuses
rather than answer differently from Saxon-HE; and a document beyond a limit, with `limit`:
`depth` for an element nested deeper than `maxDepth`, with its `line` and `column`, `nesting`
or `stack` for stylesheets recursing beyond what that depth or the module's stack allows, and
`max`, the limit's value), `transform` (a stylesheet raised a dynamic error
on a document without schema errors; with `stylesheet`, `code` and `line`), `not-xml` (from
`validateSvrl` only, with `line` and `column`) or `internal` (a bug). An argument of the wrong
type or out of range throws a `TypeError` or `RangeError` with the kind `argument`.

## Licence

`MIT AND EUPL-1.2 AND LicenseRef-OASIS-Copyright-Notice AND LicenseRef-UN-CEFACT-Copyright-Notice AND W3C-19980720`:
the code is MIT; the module embeds the CEN EN 16931 UBL and CII validation stylesheets, release
1.3.17, unmodified, which are under the EUPL-1.2, and the OASIS UBL 2.1 schemas, which keep the terms
stated in their headers (OASIS, UN/CEFACT, and the W3C licence of the XML Signature schema,
which asks for its notice to be shown to users: `licences()` returns it). The texts are in
`LICENSES/`.

Source, issues and the Rust crates: <https://github.com/einvoice-validator/einvoice-validator>.
