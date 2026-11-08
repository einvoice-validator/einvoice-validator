# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). All crates of the workspace
share one version. Before 1.0.0, a minor version may break the public API; see
[RELEASING.md](RELEASING.md). Each release states the official rule-set versions it was tested
against.

## [Unreleased]

### Added

- `einvoice-xpath-eval`: `ValueType` and `BindingValue`, the static type of a host variable or
  function result, inferred as Saxon-HE 12.10 does in a stylesheet; `VariableInfo::value_type`,
  `StaticContext::function_type`, `SimpleStaticContext::declare_typed_variable` and
  `declare_typed_function` (#21).
- CEN EN 16931 for UN/CEFACT CII D16B, release 1.3.17 (#12): the artefacts vendored
  unmodified in `artefacts/cen-en16931-cii-1.3.17/` (EUPL-1.2; the CII D16B schemas of the
  source archive are not vendored), and their corpus in `corpus/cen-en16931-cii-1.3.17/`: the
  15 examples and the 9 VEFA tests of the release (the VEFA cross-check agrees on all 9), 808
  mutation cases from the new catalogue `corpus/mutations-cii.toml` (the rule families of the
  UBL catalogue, the charge indicator in every lexical form, and the element or attribute of
  each `not(X)` rule of CII-SR and CII-DT), with their Saxon-HE 12.10 goldens. 785 of the 810
  assert ids of the stylesheet fail in at least one case (21 in the upstream cases alone); the
  corpus README lists why the 25 others cannot.
- Built-in rule set `cen-cii` (feature `cen-cii`, on by default): the CEN CII stylesheet
  embedded unmodified (`bundles/cen-cii/`), `Validator::cen_cii`. It has no XML schema:
  documents are checked for well-formedness, then by the stylesheet. On the 24 cases and the
  808 mutation cases the engine, the library and `einvoice-validate` give what Saxon-HE gives.
- `einvoice_validator::Syntax`: `Syntax::detect` tells from a document's root element whether it
  is a UBL invoice or credit note or a CII invoice, and `builtin_rule_set` names its built-in
  rule set.
- `einvoice-validate --rules cen-cii`; without `--rules` or `--bundle`, each document is
  validated with the built-in rule set of its root element (`cen-ubl` for UBL, `cen-cii` for
  CII, `cen-ubl` otherwise, as before). The JSON report names the rule set of each document
  (`documents[].rule_set`, an addition to format version 1); its `rule_set` is the first
  document's.
- `einvoice-xpath`, `einvoice-xpath-eval`: the `cast as` expression (with and without `?`),
  which the CII stylesheet uses (`udt:Indicator cast as xs:boolean`), evaluated as Saxon-HE
  12.10 does, checked by probes of `einvoice-xslt-exec` against Saxon-HE.

### Changed

- `einvoice-xpath-eval`: `xs:decimal` of an argument statically `xs:decimal` returns it
  unchanged, an `xs:integer` value included; `div` of operands statically `xs:decimal` (or one
  `xs:integer`) is Saxon's decimal division (#21). Breaking: `VariableInfo` has a new field and
  is no longer `Copy`/`Eq`.
- `einvoice-xslt-exec` passes the static types of variables, parameters and function results to
  the XPath compiler (#21).
- CI and the fetch scripts retry failed downloads (curl, Maven, cargo, rustup, apt) and cache the
  tool archives and Saxon-HE jars, checked against their pinned checksums.
- `einvoice-xsd`: validation copies no values out of the document; on the largest CEN document
  (3.3 MB) its peak above the input drops from 9.4 MiB to 53 KiB and the whole job's peak from
  12.6 MiB to 3.3 MiB (document given as a buffer); the compiled UBL 2.1 schema retains 0.9 MiB
  instead of 1.7 MiB; validating the corpus takes 23 % less time in this stage (#18).
- The mutation catalogue is chosen per rule set (`corpus/mutations.toml` for UBL,
  `corpus/mutations-cii.toml` for CII); `einvoice_validator::licences()` names both CEN
  stylesheets in its EUPL-1.2 entry.

### Fixed

- The release notes no longer repeat the version of a rule set whose name ends with one (Peppol
  BIS Billing 3.0.22): optional `release-name` in `upstream.toml` (#23).

## [0.1.0] - 2026-11-08

Tested against the official rule sets: CEN EN 16931 validation artefacts 1.3.17, Peppol BIS Billing 3.0 3.0.22.

### Added

- Mutation cases: a catalogue of edits of official examples (`corpus/mutations.toml`, applied by
  `cargo xtask corpus`), aimed at totals and rounding, decimal precision and size, lexical forms
  of numbers, booleans and dates, VAT categories, allowances and charges, prices, code lists,
  identifiers with check digits, missing and repeated elements, credit notes, namespaces,
  markup in values, long values, national rules and the elements and attributes EN 16931 does
  not use. 504 cases for CEN 1.3.17 (committed, EUPL-1.2 like the rest of its corpus, with
  their Saxon-HE 12.10 goldens) and 505 for Peppol BIS Billing 3.0.22 (built locally with the
  Peppol corpus). They raise the number of distinct CEN assert ids that fire in the corpus
  from 235 to 973 of 982. A case on which Saxon-HE stops with a dynamic error has the error as
  its golden (`<case>.error.txt`); the oracle runs the mutation cases on one thread with `-t`
  so that every diagnostic is attributed to its document.
- Mutation equivalence tests (#3): on the 504 CEN mutation cases and, with both stylesheets, on
  the 505 Peppol ones, the engine gives what Saxon-HE 12.10 gave: the same canonical SVRL, or
  a dynamic error with the same code (`einvoice-xslt-exec`, `tests/mutation_equivalence.rs`);
  `einvoice-validate` gives the verdicts and findings of the goldens, with the built-in rule set
  and with the Peppol bundle (`einvoice-cli` tests).
- `Validator::validate_owned` and `validate_owned_with`, which take the document as an owned
  buffer; `einvoice-validate` uses them for files.
- `ValidateOptions::clock` and `einvoice_validator::Clock`; on `wasm32-unknown-unknown` a clock
  is required.
- The `wasm-validate` example of `einvoice-validator`; CI runs both wasm examples under wasmtime
  50.0.1 and checks that they import nothing.
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
  embedded, except the two ETSI XAdES schemas (see Changed). Exactly the XML Schema 1.0 constructs those schemas use are implemented; any other
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
  1179 cases of the CEN 1.3.17 corpus, the canonical SVRL equals the Saxon-HE golden; 25
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
- `einvoice-validate --licences` (also `--licenses`) prints the licence of everything the binary
  embeds, each with what it covers and its full text: MIT for the code, EUPL-1.2 for the CEN
  stylesheet of the built-in rule set, and the OASIS, UN/CEFACT and W3C terms of the UBL 2.1
  schemas. The same texts are available to other programs through `einvoice_validator::licences()`
  and `einvoice_xsd::LICENCES` (#17).
- Every published crate has a README and crates.io categories; the `package` job of
  `scripts/check.sh` checks that each package has a description, its README, its licence texts,
  and one to five keywords and categories (#17).

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
- `einvoice-xslt-exec`: `Executable::transform` and `transform_with` accept an owned `String`;
  the tree keeps the text instead of copying it, which roughly halves the transform peak on the
  largest CEN document (6.5 → 3.3 MB).
- The CEN 1.3.17 stylesheet is vendored in the `einvoice-validator` crate (`bundles/cen-ubl/`);
  `einvoice-validator` and `einvoice-cli` are licensed `MIT AND EUPL-1.2`; every crate ships its
  licence texts in `LICENSES/` and has repository, homepage and docs.rs links.
- `einvoice-xslt-exec`: `Transformation::warnings` holds each distinct warning once, in the
  order first reported, as Saxon-HE's error reporter prints them (it held every occurrence).
- `einvoice-xsd` no longer ships the two ETSI XAdES schemas of the OASIS UBL 2.1 package
  (`UBL-XAdESv132-2.1.xsd`, `UBL-XAdESv141-2.1.xsd`), which no licence allows to redistribute;
  its licence is `MIT AND LicenseRef-OASIS-Copyright-Notice AND
  LicenseRef-UN-CEFACT-Copyright-Notice AND W3C-19980720`. The loader leaves out an import of
  one of their two namespaces (`XADES_NAMESPACES`) when its document is not in the set, and
  lists it in `Schema::omitted_imports`; any other missing document is still an error, and so
  is a reference to a component of an omitted namespace. XAdES elements (in UBL extensions
  and in `ds:Object`) are then validated laxly, as `xs:anyType`: their own content is not
  checked, their descendants with a global declaration still are. Given the two files,
  `Schema::load` loads the complete set and validates them strictly. The xmllint oracle uses the
  complete OASIS set (the two files are downloaded, checksums verified, for the tests only),
  checks this crate with both sets, and lists the 47 resulting disagreements, all on mutations
  with XAdES content; every corpus case still agrees (#6).

### Fixed

- `einvoice-validate`: the text output writes each finding's message on one line, without the
  line breaks and trailing spaces the stylesheets leave in some messages (the JSON and SVRL
  outputs keep the text as written).
- `einvoice-xslt-exec` recovers from a dynamic error raised while matching a pattern (a
  template rule's, in `xsl:apply-templates`, `xsl:next-match` and the built-in rules, or the
  `count` pattern of `xsl:number`) as Saxon-HE 12.10 does: the node does not match and the error
  is a warning (`line <L>: <CODE>: An error occurred matching pattern {...}: ...`), instead of
  ending the transformation; a circular global variable (`XTDE0640`) still ends it, and a global
  variable whose evaluation failed is evaluated again when next referred to. An error raised in
  a global variable or a function called from a pattern kept the code `HOST` instead of its own
  (#3).
- `einvoice-xpath-eval` evaluates `and` and `or` as Saxon-HE 12.10 does: when the first operand
  raises a dynamic error, the second is evaluated, and the error is raised only if the second
  does not decide the result (an error of the second operand wins) (#3).
- `einvoice-xslt-exec`: an error in the test of an `xsl:when` names the line of that `xsl:when`,
  as Saxon-HE does, not the line of its `xsl:choose` (#3).
- `einvoice-xsd` declares the licences of the OASIS UBL 2.1 schemas it embeds
  (`MIT AND LicenseRef-OASIS-Copyright-Notice AND LicenseRef-UN-CEFACT-Copyright-Notice AND
  W3C-19980720`) instead of MIT alone, and ships their texts in `LICENSES/` (#6).
- The `package` job of `scripts/check.sh` no longer verifies a crate against stale copies of the
  other workspace crates: cargo reuses the copies it unpacks, and their builds, by name and
  version alone, so a change made without a version change went unseen. The job removes those
  copies and builds in its own target directory (#17).
- `cargo package --workspace` no longer fails on the path dependency of the unpublished `xtask`
  crate, which now carries the workspace version like the others (#17).

[Unreleased]: https://github.com/einvoice-validator/einvoice-validator/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/einvoice-validator/einvoice-validator/releases/tag/v0.1.0
