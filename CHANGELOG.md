# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). All crates of the workspace
share one version. Before 1.0.0, a minor version may break the public API. Each release states
the official rule-set versions it was tested against.

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
  (templates, modes, priorities, functions, variables and parameters, with source lines). Any
  other element or attribute is refused with an error that names it and its line.
- `einvoice-xpath-eval`: node model and evaluator of that XPath 2.0 subset, with the behaviour
  of Saxon-HE 9.9.1-8. Unknown functions and variables are refused when an expression is
  compiled. Checked against Saxon-HE on every distinct expression of the CEN 1.3.17 stylesheet,
  evaluated at nodes of corpus documents, and on hand-written edge cases.
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
- Pre-commit, commit-message and pre-push checks in `.githooks/`, also run in CI.
- This changelog, and a check on pull requests that the Unreleased section is updated (label
  `no-changelog` to opt out).

[Unreleased]: https://github.com/einvoice-validator/einvoice-validator/commits/main
