# einvoice-validate

Command-line validation of e-invoices against the official EN 16931 validation artefacts,
with the [`einvoice-validator`](https://crates.io/crates/einvoice-validator) library: for each
document, the XML check (UTF-8, well-formed, no document type declaration), the XML schema of
the rule set (UBL 2.1; none for CII), then the official compiled Schematron stylesheets of the rule set,
executed with the behaviour of Saxon-HE. XML files and Factur-X / ZUGFeRD / Order-X PDF files
(the invoice XML they embed). It is part of
[einvoice-validator](https://github.com/einvoice-validator/einvoice-validator).

```sh
cargo install --locked einvoice-cli             # installs einvoice-validate, from crates.io
```

Or from a clone of the repository:

```sh
cargo install --locked --path crates/einvoice-cli
```

```sh
einvoice-validate invoice.xml                       # cen-ubl or cen-cii, from the root element
einvoice-validate --rules cen-cii invoice.xml
einvoice-validate --format json invoices/*.xml > report.json
einvoice-validate --format svrl - < invoice.xml
einvoice-validate --bundle ./my-bundle invoice.xml
```

Peppol BIS Billing 3.0 is used as a bundle: its files may not be redistributed, so
`scripts/fetch-peppol.sh` (in a clone of the repository) downloads the pinned release, compiles
its two stylesheets and writes the `bundle.toml` that names them:

```sh
scripts/fetch-peppol.sh
einvoice-validate --bundle target/peppol/peppol-bis-billing-ubl-3.0.22 invoice.xml
```

## Options

| Option | Meaning |
|---|---|
| `FILE...` | Documents to validate, in that order; `-` reads standard input (at most once). After `--`, every argument is a file. A file (or standard input) that starts with `%PDF-` is a PDF: see [PDF files](#pdf-files). |
| `-r`, `--rules NAME` | Built-in rule set: `cen-ubl`, the CEN EN 16931 UBL validation artefacts 1.3.17 with the UBL 2.1 schemas, or `cen-cii`, the CEN EN 16931 CII (UN/CEFACT D16B) validation artefacts 1.3.17, without an XML schema (the CII schemas are not embedded). Without `--rules` and `--bundle`, each document is validated with the rule set of its root element: `cen-ubl` for a UBL `Invoice` or `CreditNote` (namespaces `urn:oasis:names:specification:ubl:schema:xsd:Invoice-2` and `...:CreditNote-2`), `cen-cii` for a `CrossIndustryInvoice` in `urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100`, and `cen-ubl` for any other document, one that is not XML, or a file that cannot be read. Only the root element is read for that, before the document is validated. |
| `-b`, `--bundle DIR` | A rule-set bundle: `DIR/bundle.toml` and the stylesheets it names (see the documentation of the `einvoice-validator` crate for the manifest format). Excludes `--rules`. |
| `-f`, `--format FORMAT` | `text` (default), `json` or `svrl`. |
| `-j`, `--jobs N` | Validate N documents at a time, on N threads sharing one loaded rule set (default 1). The output keeps the order of the command line. |
| `--max-depth N` | Refuse a document with an element nested deeper than N levels, the document element being at depth 1 (default 256, libxml2's default; at most 100000). Such a document gets no verdict: an error of kind `unsupported` naming the line and column of the first element too deep, exit status 3. The stylesheets recurse once or twice per element level; each worker thread gets 1 MiB and 4 KiB per allowed level of nesting of templates and functions (twice N, plus 64), at least 8 MiB, and the stylesheets may use all of it but 1 MiB: a larger N, or a stylesheet that recurses, ends with that error rather than a stack overflow. |
| `-h`, `--help` | The help, then exit 0. |
| `-V`, `--version` | The version and the built-in rule sets, then exit 0. |
| `--licences`, `--licenses` | The licences of everything embedded in the binary, each with what it covers and its full text, then exit 0: MIT for the code, EUPL-1.2 for the CEN stylesheets of the built-in rule sets, and the OASIS, UN/CEFACT and W3C terms of the UBL 2.1 schemas. |

Options take their value as `--format json`, `--format=json`, `-f json` or `-fjson`. An option
given twice is a usage error.

## Exit status

The highest code that applies to the run:

| Code | Meaning |
|---|---|
| 0 | Every document is valid: no schema error and no finding with flag `fatal`. |
| 1 | At least one document is invalid: schema errors, fatal findings, or not well-formed XML. |
| 2 | Usage error; a document that cannot be read, a PDF file included (malformed, without an embedded file named as an invoice or with several, XMP metadata that is not well-formed); a bundle that cannot be read or is not valid (manifest, missing file, SHA-256 mismatch, stylesheet that is not SVRL-producing XSLT). |
| 3 | A construct the engine does not support (in a document: an encoding other than UTF-8, a document type declaration; in a PDF file: encryption, a PDF feature outside the subset read, a limit of the PDF reader; in a rule set: XSLT or XPath outside the supported subset), a document beyond a limit (an element nested deeper than `--max-depth`; a stylesheet whose templates or functions recurse beyond the nesting or the stack allowed), a dynamic error raised by a stylesheet on a document, or an internal error. |

When the rule set cannot be loaded, nothing is written to standard output; the reason goes
to standard error and the exit status is 2 or 3. Otherwise every document is reported, even
after one fails.

## PDF files

A file that starts with `%PDF-` is read as a Factur-X, ZUGFeRD or Order-X PDF/A-3 file, with
the [`einvoice-pdf`](https://crates.io/crates/einvoice-pdf) crate: the document validated is
the embedded file named `factur-x.xml`, `zugferd-invoice.xml`, `ZUGFeRD-invoice.xml`,
`xrechnung.xml` or `order-x.xml`, byte for byte, exactly as if it had been given as an XML file
(with the PDF file's URI as its document URI), and the rule set is chosen from its root element
unless `--rules` or `--bundle` is given. The text and JSON outputs add what the PDF states
about it: the name of the embedded file, its `/AFRelationship`, and from the XMP metadata of
the document the PDF/A identification (`pdfaid:part`, `pdfaid:conformance`) and the Factur-X /
ZUGFeRD / Order-X properties (`DocumentType`, `DocumentFileName`, `Version`,
`ConformanceLevel`). These are reported, not checked: the verdict is that of the XML.

```sh
einvoice-validate invoice.pdf                       # cen-cii for a Factur-X invoice
einvoice-validate --format json invoices/*.pdf invoices/*.xml
```

A PDF file no invoice can be read from is an error of kind `io` (exit status 2): a malformed
file, no embedded file with one of those names or several, XMP metadata that is not
well-formed. An encrypted file (PDF/A forbids encryption), a PDF feature outside the subset
read on the path to the embedded files (another filter than `FlateDecode`, ...) or a limit of
the reader exceeded is an error of kind `unsupported` (exit status 3).

## Output formats

### text

For each document, a line `<file>: valid`, `<file>: invalid: <counts>` or
`<file>: error (<kind>): <message>`; for a PDF file whose invoice was extracted, a line
`pdf      <name> embedded in a PDF <version>: AFRelationship <relationship>; PDF/A-<part><conformance>; <properties>`;
then one line per schema error, one per stylesheet stopped by an error, and two per finding
(severity, rule id and message, then the location). After several documents, a summary line.
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
  "rule_set": {                        the rule set of the first document
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
      "rule_set": string,              the name of the rule set it was validated with
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
      "warnings": [ { "stylesheet": string, "message": string } ],
      "pdf": null | {                  a PDF file whose invoice XML was extracted
        "version": string,             of the PDF header: "1.7"
        "attachment": {                the embedded invoice XML
          "file_name": string,         "factur-x.xml"
          "relationship": string | null,   its /AFRelationship: "Data", "Alternative", ...
          "in_associated_files": boolean,  listed in the catalog's /AF
          "description": string | null,    /Desc
          "mime_type": string | null       /Subtype of the stream: "text/xml"
        },
        "xmp": null | {                the XMP metadata of the document
          "pdfa_part": string | null,         "3"
          "pdfa_conformance": string | null,  "B"
          "invoice": null | {          the Factur-X / ZUGFeRD / Order-X properties
            "namespace": string,       "urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#"
            "document_type": string | null,       "INVOICE"
            "document_file_name": string | null,  "factur-x.xml"
            "version": string | null,             "1.0"
            "conformance_level": string | null    "EN 16931"
          }
        }
      }
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
`pdf` is `null` for an XML document and for a PDF file no invoice could be extracted from
(`error.kind` is then `io` or `unsupported`); it was added to version 1 as a pure addition.

## Licence

`MIT AND EUPL-1.2 AND LicenseRef-OASIS-Copyright-Notice AND LicenseRef-UN-CEFACT-Copyright-Notice AND W3C-19980720`:
the code is MIT; the built-in `cen-ubl` and `cen-cii` rule sets embed the CEN EN 16931
validation stylesheets, which are under the EUPL-1.2 (see `bundles/cen-ubl/` and
`bundles/cen-cii/` of the `einvoice-validator` crate), and the binary embeds the OASIS UBL 2.1
schemas of the `einvoice-xsd` crate, which keep their own terms (OASIS, UN/CEFACT, W3C). The
texts are in `LICENSES/`. `einvoice-validate --licences` prints every one of these licences in
full.
