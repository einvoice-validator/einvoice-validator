# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). All crates of the workspace
share one version. Before 1.0.0, a minor version may break the public API; see
[RELEASING.md](RELEASING.md). Each release states the official rule-set versions it was tested
against.

## [Unreleased]

### Added

- Cargo workspace (edition 2024) with the toolchain pinned in `rust-toolchain.toml`, every
  warning denied, clippy `all` and `pedantic` denied, `unsafe` code forbidden, and a
  dependency policy checked by cargo-deny.
- `einvoice-xdm`: atomic values of the XPath 2.0 data model with the behaviour of Saxon-HE
  9.9.1-8: exact arbitrary-precision `xs:decimal` and `xs:integer`, `xs:double` parsing and
  formatting, `xs:date` and `xs:dateTime`, casts, arithmetic, comparisons, rounding, `fn:sum`
  and XSD regular expressions. Checked against Saxon-HE results on 5068 cases.
- `einvoice-xpath`: parser for the XPath 2.0 subset of the official validation stylesheets,
  with XSLT match patterns, sequence types and attribute value templates. Constructs outside
  the subset are refused with an error that names them.
- `einvoice-xslt`: loader for the XSLT 2.0 subset of the official validation stylesheets
  (templates, modes, priorities, functions, variables and parameters, with source lines, and
  the namespaces in scope of literal result elements and `xsl:attribute`, equal sets shared).
  Any other element or attribute is refused with an error that names it and its line.
- `einvoice-xpath-eval`: node model and evaluator of that XPath 2.0 subset, with the behaviour
  of Saxon-HE 9.9.1-8. Unknown functions and variables are refused when an expression is
  compiled. Checked against Saxon-HE on every distinct expression of the CEN 1.3.17 stylesheet,
  evaluated at nodes of corpus documents, and on hand-written edge cases.
- `einvoice-xsd`: XML Schema validation of UBL 2.1 Invoice and CreditNote documents against the
  official OASIS UBL 2.1 schemas, vendored unmodified (under their own notices, not MIT) and
  embedded. Exactly the XML Schema 1.0 constructs those schemas use are implemented; any other
  is refused when a schema set is loaded. Errors carry a stable kind, the line and column.
  Compared with xmllint on the corpus and on mutations; every difference is listed with its
  cause.
- `einvoice-xslt-exec`: executor of the XSLT 2.0 subset of the official validation
  stylesheets, with the behaviour of Saxon-HE 9.9.1-8. A stylesheet is prepared once and run
  on any number of documents; the result is written straight to XML (SVRL for the rule
  sets). Template rules are chosen, and `xsl:next-match` continues, as Saxon does; built-in
  rules, parameters, variables, temporary trees, `as` conversion, `xsl:function`,
  `xsl:number`, simple and complex content and namespace fixup follow XSLT 2.0. Constructs
  whose result would differ from Saxon's are refused when the stylesheet is prepared. On all
  1179 cases of the CEN 1.3.17 corpus, the canonical SVRL equals the Saxon-HE golden; 18
  synthetic probe stylesheets are checked against Saxon-HE results.
- `einvoice-svrl`: the canonical, line-oriented form of SVRL reports (`corpus/SVRL-CANONICAL.md`),
  shared by the oracle tooling and the equivalence tests.
- The CEN EN 16931 UBL validation artefacts 1.3.17, vendored unmodified (EUPL-1.2) with their
  SHA-256 checksums.
- A test corpus of 1179 cases derived from those artefacts (official examples, test files and
  the VEFA unit tests), with the canonical SVRL that Saxon-HE 9.9.1-8 produces for each case and
  a cross-check against the VEFA expectations: `cargo xtask manifest`, `corpus`, `oracle`,
  `crosscheck`, and `scripts/fetch-saxon.sh`.
- `scripts/check.sh`: formatting, lints, documentation, tests, cargo-deny, actionlint, zizmor
  and the oracle, run the same way locally and in CI. CI on Linux, plus macOS and Windows tests
  on pull requests; the oracle workflow re-runs Saxon-HE when the corpus or its tooling changes.
- `upstream.toml` and `scripts/freshness.sh`: every pinned upstream (rule sets, Saxon-HE,
  toolchain, CI tools) with the files that record it, and a weekly workflow that reports newer
  upstream versions in a tracking issue. Dependabot for GitHub Actions and Cargo.
- `scripts/freshness.sh` also checks every direct Cargo dependency on crates.io (read from the
  manifests and `Cargo.lock`, yanked and pre-release versions ignored): a major update, which
  Dependabot leaves to manual migration, keeps the tracking issue open; `scripts/freshness.sh
  crates` runs this check alone.
- Pre-commit, commit-message and pre-push checks in `.githooks/`, also run in CI.
- This changelog, and a check on pull requests that the Unreleased section is updated (label
  `no-changelog` to opt out).
- Release process: `RELEASING.md`, `scripts/release.sh` (version bump, changelog section,
  signed commit and tag, with a dry run) and a workflow that publishes the GitHub release of a
  pushed tag after checking that the tag, the workspace version and the changelog agree.
- README with the project status, design, build and test instructions, and licences;
  `SECURITY.md` for private vulnerability reports.
- `einvoice-xdm`: `AtomicValue::upcast` and `Integer::to_decimal_upcast`; `einvoice-xpath-eval`:
  Saxon-style static type inference, so that `xs:decimal` of an argument Saxon knows to be an
  integer strips trailing zeros as Saxon-HE 12.10 does (#8).
- Peppol BIS Billing 3.0.22 (UBL) test corpus, built locally because the Peppol files carry no
  licence and are not redistributed: `scripts/fetch-peppol.sh` fetches the pinned release tag
  and compiles its Peppol and CEN Schematron files to XSLT with ph-schematron-maven-plugin
  10.3.1 (ISO skeleton), every input and output checked against a pinned checksum, into
  `target/peppol/`; `cargo xtask peppol` extracts 707 cases (examples, snippets, national
  examples and the UBL VEFA unit tests), writes the Saxon-HE 12.10 goldens of both stylesheets
  and cross-checks them with the VEFA expectations (678 of 678 VEFA cases agree) (#4).
- Peppol equivalence test (`einvoice-xslt-exec`, `tests/peppol_equivalence.rs`): on all 707
  cases, the canonical SVRL of both stylesheets equals the Saxon-HE golden. Skipped without the
  corpus, required by `scripts/check.sh peppol` and by the new `peppol` CI workflow, which builds
  the corpus whenever the engine or the tooling changes (#5).
- `bench-peppol` and `heap-peppol` examples of `einvoice-xslt-exec`: time and heap of the whole
  Peppol job (#5).
- `upstream.toml` pins the Peppol release, ph-schematron-maven-plugin and Apache Maven in the
  files that use them.
- `einvoice-validator`: public API. A `Validator` is built once from a rule-set bundle (schema
  loaded, official stylesheets prepared), is `Send + Sync`, and validates any number of
  documents: XML check, UBL 2.1 schema, then each stylesheet. The `Report` gives the schema
  errors with line and column, the findings (id, flag, location, rule context, test, text), the
  raw and canonical SVRL, the rule-set version and the verdict. Bundles are a `bundle.toml`
  manifest with the SHA-256 of each stylesheet, loaded from memory or a directory; the CEN
  EN 16931 UBL 1.3.17 rule set is built in (`cen-ubl`). Findings and verdicts agree with
  Saxon-HE on all 1179 corpus cases (#10).
- `einvoice-validate` (crate `einvoice-cli`): command-line tool. Validates files and standard
  input with a built-in rule set or a bundle directory, on one or more threads; output as text,
  JSON (versioned format, documented) or raw SVRL; documented exit codes (0 valid, 1 invalid,
  2 usage or I/O error, 3 unsupported or internal) (#10).
- Peppol BIS Billing 3.0 as a rule-set bundle: `scripts/fetch-peppol.sh` also writes a
  `bundle.toml` next to the two compiled stylesheets (CEN rules first, with the CEN version
  their Schematron states), so that the directory can be given to `einvoice-validate --bundle`
  or `Validator::from_bundle_dir`. A test of `einvoice-cli` checks the verdicts and findings of
  all 707 Peppol cases against the Saxon-HE goldens; `scripts/check.sh peppol` requires it (#10).
- `scripts/check.sh` jobs `features` (clippy and tests of `einvoice-validator` without its
  default features), `wasm` (`einvoice-validator` built for `wasm32-unknown-unknown`), `pins`
  (`scripts/freshness.sh pins` and the freshness tests) and `guards` (ShellCheck on every
  script and git hook, the tests of the privacy guard and of the release scripts); the `oracle`
  job also runs the xmllint oracle of `einvoice-xsd`, and zizmor also audits
  `.github/dependabot.yml`. CI runs them all (#6, #7, #9, #15).

### Changed

- The oracles use Saxon-HE 12.10, with the XML Resolver 5.3.3 it needs, fetched and
  checksum-checked by `scripts/fetch-saxon.sh`; all goldens regenerated (#8).
- `einvoice-xdm` follows Saxon-HE 12.10: a double cast to an integer beyond ±2^63 is exact; an
  untyped value compared with a numeric NaN is false (true for `!=`) without being read; `sum()`
  checks every item, also after a NaN total (#8).
- `einvoice-xpath-eval`: `document-uri()` is empty for the document given to the
  transformation; `Document::parse` sets only the base URI, the new `Document::parse_loaded`
  sets both for documents loaded by `document()` (#8).
- `einvoice-xslt-exec` follows Saxon-HE 12.10: a document loaded by `document()` has its URI as
  document URI, the transformed document has none, so `active-pattern/@documents` is empty in
  the SVRL; the URI given to `Executable::transform` is the document's base URI; a union pattern
  with an explicit priority is a single template rule, which changes the ambiguity warnings;
  warnings name the node by Saxon's path (`/r/x[2]`, `/r/text()[1]`) and the two rules in
  Saxon's order (#8).

- `scripts/fetch-saxon.sh` exits with status 75 when a download fails, so that
  `scripts/check.sh peppol` can tell a missing network from a real failure.
- The xtask tooling handles rule sets with several stylesheets (one golden directory each) and
  rule sets built outside the tracked tree.
- The oracle workflow installs xmllint and runs on Ubuntu 24.04, whose libxml2 (2.9.14) produced
  the `einvoice-xsd` results; it also runs when the schemas or the oracle files of
  `einvoice-xsd` change (#6).
- `einvoice-xpath-eval`: `DynamicContext::new` takes a `Clock` (implicit timezone and current
  date and time), built with `Clock::new`, `Clock::from_unix_millis` or `Clock::system` (the
  last not on `wasm32-unknown-unknown`); `with_implicit_timezone` and `with_current_date_time`
  are removed. The evaluator reads no clock and runs on `wasm32-unknown-unknown` (#9).
- `einvoice-xpath-eval`: `Document::parse` and `parse_loaded` accept `&str` or `String`; given a
  `String`, the tree keeps it and refers to the texts written verbatim, and its arenas are sized
  exactly. Parsing the largest corpus document peaks at 3.2 MiB from `&str` or 53 KiB from a
  `String` instead of 9.4 MiB; the whole CEN job's peak heap drops by 37 % (#18).
- `einvoice-xslt-exec`: without a system clock (`wasm32-unknown-unknown`),
  `TransformOptions::current_date_time` is required; a current date and time without a timezone
  is refused (#9).

### Fixed

- `einvoice-xsd` declares the licences of the OASIS UBL 2.1 schemas it embeds
  (`MIT AND LicenseRef-OASIS-Copyright-Notice AND LicenseRef-UN-CEFACT-Copyright-Notice AND
  W3C-19980720 AND LicenseRef-ETSI-XAdES`) instead of MIT alone, and ships their texts in
  `LICENSES/` (#6).

[Unreleased]: https://github.com/einvoice-validator/einvoice-validator/commits/main
