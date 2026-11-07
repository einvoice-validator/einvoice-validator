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

### Fixed

- `einvoice-xsd` declares the licences of the OASIS UBL 2.1 schemas it embeds
  (`MIT AND LicenseRef-OASIS-Copyright-Notice AND LicenseRef-UN-CEFACT-Copyright-Notice AND
  W3C-19980720 AND LicenseRef-ETSI-XAdES`) instead of MIT alone, and ships their texts in
  `LICENSES/` (#6).

[Unreleased]: https://github.com/einvoice-validator/einvoice-validator/commits/main
