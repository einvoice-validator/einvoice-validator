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

## Rule sets fetched on demand

Factur-X (each profile), XRechnung (UBL and CII), the French rules (each set of stylesheets
of the FNFE-MPE package) and Peppol BIS Billing 3.0 may not be redistributed, so the binary
does not embed them: it downloads them from their publishers when, and only when, asked, at
pinned URLs, checks each file against its pinned SHA-256 (SHA-512 for Apache Maven), builds
their bundles in the cache and uses them from there, offline, checking every file on every
load.

```sh
einvoice-validate rules list                        # every rule set, its source, cached or not
einvoice-validate rules fetch xrechnung factur-x    # families, aliases or rule-set names
einvoice-validate --rules xrechnung invoice.xml     # xrechnung-ubl or xrechnung-cii, by document
einvoice-validate --rules factur-x-en16931 --fetch invoice.pdf   # fetch in this run if missing
einvoice-validate rules path factur-x-en16931       # the bundle directory (for --bundle)
einvoice-validate rules clear
```

| Family | Rule sets | Published by, fetched from |
|---|---|---|
| `factur-x` | `factur-x-minimum`, `factur-x-basic-wl`, `factur-x-basic`, `factur-x-en16931`, `factur-x-extended` | the compiled stylesheets and code lists of Factur-X 1.09.2 as the Mustang validator ships them (`org.mustangproject:validator`, Maven Central) |
| `xrechnung` | `xrechnung-ubl`, `xrechnung-cii` (alias `xrechnung`: the one of each document's syntax) | the KoSIT validator configuration for XRechnung 3.0.2 (projekte.kosit.org): the EN 16931 rules, then the XRechnung rules |
| `fr-ctc` | `fr-ctc-cii-en16931`, `fr-ctc-cii-extended-ctc-fr`, `fr-ctc-ubl-en16931`, `fr-ctc-ubl-extended-ctc-fr`, `fr-ctc-cdar`, each also as `...-warning`; `fr-ctc-factur-x-basic-wl`, `fr-ctc-factur-x-en16931`, `fr-ctc-factur-x-extended`; `fr-ctc-flux10` | the FNFE-MPE package "Schematrons FR CTC" 1.4.0.04 (fnfe-mpe.org) |
| `peppol` | `peppol-bis-billing-ubl` (alias `peppol`) | the Peppol BIS Billing 3.0.22 release (OpenPeppol), published as Schematron only: compiled with ph-schematron 10.3.1 as the reference validators do, which needs **Java** and network access to Maven Central (Apache Maven 3.9.16 runs it with strict checksums and a private local repository in the cache) |

A family is fetched as a whole: its rule sets come from the same files. `rules fetch` takes
rule-set names, aliases (`peppol`, `xrechnung`), families or `--all`, and `--from DIR` (any
number of times) to take a published file from a local directory instead of downloading it,
under the file name `rules list` gives (checked all the same). Validating with a rule set that
is not in the cache is an error saying how to fetch it (exit status 2); `--fetch` allows the
download in that run. The bundles are byte for byte those the repository's
`scripts/fetch-*.sh` write: in a clone, `scripts/fetch-peppol.sh` builds the same Peppol bundle
in `target/peppol/peppol-bis-billing-ubl-3.0.22/`, usable with `--bundle`.

The download is HTTPS only (rustls, with the Mozilla root certificates), follows at most five
redirects and never from HTTPS to plain HTTP, tries a file five times with a doubling delay on
an I/O error and on HTTP 408, 429 and 5xx (honouring `Retry-After`), resumes an interrupted
download, and refuses a file over 64 MiB; archives are read in memory, refusing absolute
paths, `..`, links and oversized entries. Requests name the project in their `User-Agent`.

| Variable | Meaning |
|---|---|
| `EINVOICE_VALIDATOR_CACHE` | The cache directory. Default: `$XDG_CACHE_HOME/einvoice-validator`, else `~/.cache/einvoice-validator` (Linux and others), `~/Library/Caches/einvoice-validator` (macOS), `%LOCALAPPDATA%\einvoice-validator\cache` (Windows). `rules clear` removes only what the tool put there. |
| `EINVOICE_VALIDATOR_MIRROR` | A mirror of the publishers' files: `https://HOST/PATH` is fetched as `MIRROR/HOST/PATH` (an `https://` URL, or `http://` to a loopback address). The digests are checked all the same. |
| `JAVA`, `JAVA_HOME` | The Java runtime that compiles the Peppol Schematron (default: `java` on the `PATH`). |

Built without the default feature `fetch` (`cargo install einvoice-cli --no-default-features`),
the binary has no network code: it still lists the rule sets and validates with those already
in the cache.

## Options

| Option | Meaning |
|---|---|
| `FILE...` | Documents to validate, in that order; `-` reads standard input (at most once). After `--`, every argument is a file. A file (or standard input) that starts with `%PDF-` is a PDF: see [PDF files](#pdf-files). |
| `-r`, `--rules NAME` | A rule set fetched on demand, from the cache (see [Rule sets fetched on demand](#rule-sets-fetched-on-demand)), or a built-in rule set: `cen-ubl`, the CEN EN 16931 UBL validation artefacts 1.3.17 with the UBL 2.1 schemas, or `cen-cii`, the CEN EN 16931 CII (UN/CEFACT D16B) validation artefacts 1.3.17, without an XML schema (the CII schemas are not embedded). Without `--rules` and `--bundle`, each document is validated with the rule set of its root element: `cen-ubl` for a UBL `Invoice` or `CreditNote` (namespaces `urn:oasis:names:specification:ubl:schema:xsd:Invoice-2` and `...:CreditNote-2`), `cen-cii` for a `CrossIndustryInvoice` in `urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100`, and `cen-ubl` for any other document, one that is not XML, or a file that cannot be read. Only the root element is read for that, before the document is validated. |
| `--fetch` | With `--rules` naming a rule set fetched on demand that is not in the cache: fetch it first (otherwise an error says how). Nothing is downloaded without it or `rules fetch`. |
| `-b`, `--bundle DIR` | A rule-set bundle: `DIR/bundle.toml` and the stylesheets and resources it names (see the documentation of the `einvoice-validator` crate for the manifest format). Excludes `--rules`. |
| `-f`, `--format FORMAT` | `text` (default), `json` or `svrl`. |
| `-j`, `--jobs N` | Validate N documents at a time, on N threads sharing one loaded rule set (default 1). The output keeps the order of the command line. |
| `--max-depth N` | Refuse a document with an element nested deeper than N levels, the document element being at depth 1 (default 256, libxml2's default; at most 100000). Such a document gets no verdict: an error of kind `unsupported` naming the line and column of the first element too deep, exit status 3. The stylesheets recurse once or twice per element level; each worker thread gets 1 MiB and 4 KiB per allowed level of nesting of templates and functions (twice N, plus 64), at least 8 MiB, and the stylesheets may use all of it but 1 MiB: a larger N, or a stylesheet that recurses, ends with that error rather than a stack overflow. |
| `--fail-closed` | Count as fatal every finding that would not otherwise be fatal or a warning: a flag other than `fatal` and `warning` (`information`, ...), or none, that the rule set gives no severity of its own (see [Severities](#severities)). For rule sets whose flags are not known in advance. |
| `-h`, `--help` | The help, then exit 0. |
| `-V`, `--version` | The version and the built-in rule sets, then exit 0. |
| `--licences`, `--licenses` | The licences of everything embedded in the binary, each with what it covers and its full text, then exit 0: MIT for the code, EUPL-1.2 for the CEN stylesheets of the built-in rule sets, and the OASIS, UN/CEFACT and W3C terms of the UBL 2.1 schemas; then the notices of the third-party crates the binary is compiled from (`THIRD-PARTY-NOTICES`). |

Options take their value as `--format json`, `--format=json`, `-f json` or `-fjson`. An option
given twice is a usage error.

## Severities

Each finding has a severity, `fatal`, `warning` or `other`, and a document with a fatal
finding is invalid. The severity is the one its Schematron flag stands for (`fatal`,
`warning`, anything else or none `other`), as the reference validator of each rule set
reads the reports:

- Factur-X (and the Factur-X profiles of the French rules) and XRechnung: a failed assert
  without a flag is fatal, as the Mustang validator and the KoSIT validator read it (their
  bundles say `unflagged-assert = "fatal"`). Most Factur-X asserts have no flag.
- XRechnung: the scenario of the KoSIT validator configuration that the document matches
  (its `match` expression, evaluated on the document, which reads its root element and
  its `cbc:CustomizationID` or its CII guideline identifier) names the stylesheets that
  run (the CEN rules alone for `urn:cen.eu:en16931:2017`, the CEN and XRechnung rules for
  the XRechnung identifiers) and gives the rule ids it lists their levels, whatever their
  flags (`customLevel`: `error` is fatal, `information` other); for instance `UBL-CR-646`
  is fatal in an XRechnung 3.0 invoice and other in an XRechnung extension. A document
  that matches no scenario (a Peppol BIS invoice, a misspelt identifier) or several is
  invalid, and no stylesheet runs on it, as the KoSIT validator rejects it. The CII
  scenarios also validate against the UN/CEFACT CII D16B XML schema, which is not
  embedded: the text output notes it, the JSON output names it in
  `rule_set.unchecked_schema`.
- CEN EN 16931, Peppol BIS Billing and the other French rules flag every assert `fatal`
  or `warning`.

`--fail-closed` makes every finding that is still `other` fatal.

## Exit status

The highest code that applies to the run:

| Code | Meaning |
|---|---|
| 0 | Every document is valid: no schema error and no finding of severity `fatal` (see [Severities](#severities)). |
| 1 | At least one document is invalid: schema errors, fatal findings, or not well-formed XML. |
| 2 | Usage error; a document that cannot be read, a PDF file included (malformed, without an embedded file named as an invoice or with several, XMP metadata that is not well-formed); a bundle that cannot be read or is not valid (manifest, missing file, SHA-256 mismatch, stylesheet that is not SVRL-producing XSLT). |
| 3 | A construct the engine does not support (in a document: an encoding other than UTF-8, a document type declaration; in a PDF file: encryption, a PDF feature outside the subset read, a limit of the PDF reader; in a rule set: XSLT or XPath outside the supported subset), a document beyond a limit (an element nested deeper than `--max-depth`; a stylesheet whose templates or functions recurse beyond the nesting or the stack allowed), a dynamic error raised by a stylesheet on a document, or an internal error. |

The `rules` commands exit with 0 when they succeed and 2 otherwise (a usage error, a rule set
that cannot be fetched, `rules path` of a rule set not in the cache), after saying why on
standard error.

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
then, for a rule set with scenarios, a line `scenario <name>` or, for a document that matches
none (`<file>: invalid: no scenario`), the reason; for a rule set that names an XML schema it
does not check, a line `note     not checked: ...`; then one line per schema error, one per
stylesheet stopped by an error, and two per finding (severity, rule id and message, then the
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
  "rule_set": {                        the rule set of the first document
    "name": string,                    "cen-ubl"
    "title": string,                   "CEN EN 16931 UBL"
    "version": string,                 "1.3.17"
    "schema": string | null,           "ubl-2.1"
    "unchecked_schema": string | null, an XML schema the reference validator also checks
                                       and this rule set does not: "UN/CEFACT CII D16B (...)"
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
      "scenario": string | null,       the scenario of the rule set the document matched
      "no_scenario": null | {          a rule set that selects by scenario, none applies
        "matching": [ string ],        the scenarios that matched: empty for none, or several
        "message": string
      },
      "stylesheets_run": [ string ],   the stylesheets that ran, in order
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
(`error.kind` is then `io` or `unsupported`); it was added to version 1 as a pure addition, as
were `unchecked_schema`, `scenario`, `no_scenario` and `stylesheets_run`. A document of a rule
set that selects by scenario (XRechnung) that matches no scenario, or several, is `invalid`
with `no_scenario` set and every list empty: no stylesheet ran, and no schema error is reported
(the KoSIT validator validates against the schema of the scenario).

## Licence

`MIT AND EUPL-1.2 AND LicenseRef-OASIS-Copyright-Notice AND LicenseRef-UN-CEFACT-Copyright-Notice AND W3C-19980720`:
the code is MIT; the built-in `cen-ubl` and `cen-cii` rule sets embed the CEN EN 16931
validation stylesheets, which are under the EUPL-1.2 (see `bundles/cen-ubl/` and
`bundles/cen-cii/` of the `einvoice-validator` crate), and the binary embeds the OASIS UBL 2.1
schemas of the `einvoice-xsd` crate, which keep their own terms (OASIS, UN/CEFACT, W3C). The
texts are in `LICENSES/`. `einvoice-validate --licences` prints every one of these licences in
full.

The binary is also compiled from crates of crates.io (`regex`, `num-bigint`, `roxmltree`,
`sha2`, `miniz_oxide` and their dependencies), each used under the MIT licence, which all of
them offer. `THIRD-PARTY-NOTICES` gives, for every one of them on any target, its version, its
licence expression, the licence used, its authors and the text of that licence from its own
package, copyright line included; `einvoice-validate --licences` prints it after the licences
above. It is generated from `Cargo.lock` and the packages of those crates (`cargo run --locked
-p einvoice-testkit --bin third-party-notices -- --write`), and a test of the crate fails when
it is not up to date.
