# einvoice-validate

Command-line validation of e-invoices against the official EN 16931 validation artefacts,
with the [`einvoice-validator`](../einvoice-validator) library: for each document, the XML
check (UTF-8, well-formed, no document type declaration), the XML schema of the rule set
(UBL 2.1), then the official compiled Schematron stylesheets of the rule set, executed with
the behaviour of Saxon-HE.

```sh
cargo install --path crates/einvoice-cli        # installs einvoice-validate
einvoice-validate invoice.xml
einvoice-validate --format json invoices/*.xml > report.json
einvoice-validate --format svrl - < invoice.xml
einvoice-validate --bundle ./my-bundle invoice.xml
```

## Options

| Option | Meaning |
|---|---|
| `FILE...` | Documents to validate, in that order; `-` reads standard input (at most once). After `--`, every argument is a file. |
| `-r`, `--rules NAME` | Built-in rule set. Default and only one today: `cen-ubl`, the CEN EN 16931 UBL validation artefacts 1.3.17 with the UBL 2.1 schemas. |
| `-b`, `--bundle DIR` | A rule-set bundle: `DIR/bundle.toml` and the stylesheets it names (see the documentation of the `einvoice-validator` crate for the manifest format). Excludes `--rules`. |
| `-f`, `--format FORMAT` | `text` (default), `json` or `svrl`. |
| `-j`, `--jobs N` | Validate N documents at a time, on N threads sharing one loaded rule set (default 1). The output keeps the order of the command line. |
| `-h`, `--help` | The help, then exit 0. |
| `-V`, `--version` | The version and the built-in rule sets, then exit 0. |

Options take their value as `--format json`, `--format=json`, `-f json` or `-fjson`. An option
given twice is a usage error.

## Exit status

The highest code that applies to the run:

| Code | Meaning |
|---|---|
| 0 | Every document is valid: no schema error and no finding with flag `fatal`. |
| 1 | At least one document is invalid: schema errors, fatal findings, or not well-formed XML. |
| 2 | Usage error; a document that cannot be read; a bundle that cannot be read or is not valid (manifest, missing file, SHA-256 mismatch, stylesheet that is not SVRL-producing XSLT). |
| 3 | A construct the engine does not support (in a document: an encoding other than UTF-8, a document type declaration; in a rule set: XSLT or XPath outside the supported subset), a dynamic error raised by a stylesheet on a document, or an internal error. |

When the rule set cannot be loaded, nothing is written to standard output; the reason goes
to standard error and the exit status is 2 or 3. Otherwise every document is reported, even
after one fails.

## Output formats

### text

For each document, a line `<file>: valid`, `<file>: invalid: <counts>` or
`<file>: error (<kind>): <message>`, then one line per schema error, one per stylesheet
stopped by an error, and two per finding (severity, rule id and message, then the
location). After several documents, a summary line.
The UBL and XML Schema namespaces in schema errors are written with their usual prefixes
(`cbc:`, `cac:`, `ext:`, `ubl:`, `cn:`, `xs:`). Colours are used only when standard output is a terminal and
`NO_COLOR` is not set to a non-empty value. The text format is meant for people; use `json`
in scripts.

### svrl

The SVRL report of each stylesheet, exactly as the stylesheet wrote it, one XML document after
another (each followed by a newline if it does not end with one), in the order of the
documents, then of the stylesheets. For a single document and a single stylesheet, the output
is that one SVRL document. Problems (unreadable files, documents that are not XML,
stylesheets stopped by an error) go to standard error.

### json

One JSON object for the whole run, UTF-8, pretty-printed with two-space indentation; the
members are always present and in the order below. The format is identified by `format` and
`format_version`; any change that is not a pure addition of members increases
`format_version`. Version 1:

```text
{
  "format": "einvoice-validate",
  "format_version": 1,
  "tool": { "name": "einvoice-validate", "version": string },
  "rule_set": {
    "name": string,                    "cen-ubl"
    "title": string,                   "CEN EN 16931 UBL"
    "version": string,                 "1.3.17"
    "schema": string | null,           "ubl-2.1"
    "stylesheets": [
      { "name": string, "version": string, "path": string, "sha256": string }
    ]
  },
  "documents": [                       one per FILE, in command-line order
    {
      "path": string,                  the FILE as given; "-" for standard input
      "status": "valid" | "invalid" | "error",
      "error": null | {                set when there is no complete report
        "kind": "io" | "bundle" | "not-xml" | "unsupported" | "transform" | "internal",
        "message": string,
        "line": number | null,         not-xml only
        "column": number | null        not-xml only
      },
      "counts": {
        "schema_errors": number, "fatal": number, "warning": number, "other": number,
        "stylesheet_errors": number
      },
      "schema_errors": [
        {
          "kind": string,              stable kinds of einvoice-xsd: "missing-element", ...
          "line": number, "column": number,
          "element": string,           Clark notation: "{namespace}local"
          "attribute": string | null,
          "message": string
        }
      ],
      "findings": [                    in SVRL order, stylesheet after stylesheet
        {
          "stylesheet": string,        its name in the rule set
          "kind": "failed-assert" | "successful-report",
          "id": string | null,         the rule id: "BR-02"
          "flag": string | null,       as the rule set writes it: "fatal", "warning"
          "severity": "fatal" | "warning" | "other",
          "role": string | null,
          "location": string,          XPath of the node concerned
          "context": string | null,    context of the rule that fired
          "pattern": string | null,    id of the pattern
          "test": string,              XPath test of the assert or report
          "text": string               the message, whitespace normalised
        }
      ],
      "stylesheet_errors": [           stylesheets stopped by a dynamic error
        { "stylesheet": string, "code": string, "line": number | null, "message": string }
      ],
      "warnings": [ { "stylesheet": string, "message": string } ]
    }
  ],
  "summary": { "documents": number, "valid": number, "invalid": number, "errors": number },
  "exit_code": number
}
```

`status` is `invalid` for a document with schema errors or fatal findings (`error` is then
`null`) and for a document that is not well-formed XML (`error.kind` is `not-xml`, the other
lists are empty); `error` for any other `error.kind`. A stylesheet that raises a dynamic
error on a document (the official CEN stylesheet does on a non-numeric amount, as it does
under Saxon-HE) is listed in `stylesheet_errors` when the document already has schema
errors: the document is `invalid` and the other stylesheets' findings are kept. Without
schema errors, there is no verdict: `status` is `error`, `error.kind` is `transform`. `warnings` lists the recoverable
conditions a stylesheet reported (ambiguous template rule matches), as Saxon-HE reports them.

## Licence

MIT, like the rest of the project. The built-in `cen-ubl` rule set embeds the CEN EN 16931
validation artefacts, which are under the EUPL-1.2 (see `artefacts/cen-en16931-ubl-1.3.17/`).
