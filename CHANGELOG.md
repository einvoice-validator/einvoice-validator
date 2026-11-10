# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). All crates of the workspace
share one version. Before 1.0.0, a minor version may break the public API; see
[RELEASING.md](RELEASING.md). Each release states the official rule-set versions it was tested
against.

## [Unreleased]

### Fixed

- `scripts/check.sh test` fetches every package first: the licence tests of `einvoice-cli`
  failed on macOS and Windows (#11).
### Changed

- `scripts/fetch-peppol.sh` runs ph-schematron with Apache Maven 3.10.0 (compiled stylesheets
  unchanged; retry options under the Maven Resolver 2 names) (#20).
- `rsa` 0.10.0 and `crypto-bigint` 0.7.11 in the TLS stack of `rules fetch` (#20).
- The privacy scans read `PRIVATE_TERMS` from the deployment environment `privacy`;
  `privacy.yml` can be run by hand; `main` requires the `ci-ok` check (#11).
### Added

- `scripts/privacy-guard.sh` `tree` and `history` report terms of several words split across
  lines or whitespace inside files, like `staged` (#15).
- `upstream.toml` pins the vendored UBL 2.1 and UN/CEFACT CII D16B schemas of `einvoice-xsd`;
  `scripts/freshness.sh` source `unlisted` for upstreams without a version list a script can
  read (#7).
## [0.3.0] - 2026-11-10

Tested against the official rule sets: CEN EN 16931 validation artefacts 1.3.17, Peppol BIS Billing 3.0.22, Factur-X 1.09.2, XRechnung 3.0.2, French e-invoicing rules (FNFE-MPE) 1.4.0.04.

### Security

- A deeply nested stylesheet (an untrusted rule-set bundle) no longer aborts the process with a
  stack overflow when it is loaded, prepared or run (#33). Expressions and patterns nested
  deeper than `einvoice_xpath::MAX_DEPTH` (128) are refused with `ErrorKind::TooDeep`, sequence
  constructors nested deeper than `einvoice_xslt::MAX_NESTING` (64) with
  `LoadErrorKind::TooDeep`; the official stylesheets need a depth of 32 and a nesting of 9.
  `einvoice-validator` and `einvoice-validate` report both as unsupported (exit status 3).
- The stack per nesting level is now 7 KB to parse, 5 KB to compile and 4 KB to evaluate in a
  debug build (was 44, 21 and 21 KB). `StackLimit` and `DynamicContext::with_stack_limit` bound
  an evaluation (`ErrorCode::Limit`); transformations pass their `max_stack` to the evaluator.
- `DEFAULT_MAX_STACK` on WebAssembly is 256 KiB (was 640 KiB): wasmtime's 512 KiB engine stack
  ran out first.

### Added

- `einvoice-validate --fail-closed` (#36): every finding that is neither fatal nor a warning
  (a flag other than `fatal` and `warning`, or none, that the rule set gives no severity of its
  own) is fatal, as `ValidateOptions::fail_closed` makes it; the npm package takes the option
  `failClosed`.
- Rule-set bundles, manifest format 2 (#36): the `[[stylesheet]]` key `unflagged-assert`
  (`fatal`, `warning` or `other`, by default `other`), the severity of a failed assert without a
  flag (`StylesheetEntry::unflagged_assert`), and `[[scenario]]` tables (`Scenario`,
  `RuleSet::scenarios`): the severities a scenario gives rule ids, whatever their flags, in the
  documents whose root element and specification identifier (BT-24) it names, as the
  `customLevel` elements of a KoSIT validator configuration do; `Report::scenario` names the
  one a document matched. Format 1 manifests, and format 2 manifests without them, mean what
  they meant.
- Rule sets that select by scenario (#27), as the KoSIT validator runs its configuration:
  `[[scenario]]` tables with `match` (the XPath expression of a KoSIT scenario, as written),
  `namespaces` (its prefixes, `prefix=uri` separated by spaces) and `stylesheets` (those it runs,
  in order; all by default), the levels optional (`Scenario::match_expression`,
  `Scenario::namespaces`, `Scenario::stylesheets`, `RuleSet::selects_by_scenario`). The
  expressions are compiled with the engine when the bundle is loaded (one outside its subset is
  refused) and evaluated on each document, its document node the context item; one that raises a
  dynamic error is false, as for the KoSIT validator. The one scenario that matches gives the
  stylesheets that run and the severities; a document that matches none, or several, is invalid,
  without schema errors or findings: `Report::no_scenario` (`NoScenario::NoneMatches`,
  `NoScenario::SeveralMatch`) says why. The top-level key `unchecked-schema` names an XML schema
  the reference validator also checks and the rule set does not (`RuleSet::unchecked_schema`,
  `RuleSetVersion::unchecked_schema`). `Scenario::namespace`, `root` and `identifier` return an
  `Option` (`None` for a scenario with `match`).
- The XRechnung bundles have every scenario of the KoSIT configuration for their syntax (7 for
  UBL, 4 for CII), with their `match` expressions: a document with the identifier
  `urn:cen.eu:en16931:2017` runs the CEN rules alone, an XRechnung one the CEN and XRechnung
  rules, and any other (a Peppol BIS invoice, a misspelt identifier, an XRechnung extension
  credit note, which no scenario names) is rejected. Each bundle runs the XML schema its
  scenarios validate against: UBL 2.1, or the UN/CEFACT CII D16B schema (`schema = "cii-d16b"`,
  #32; `cargo xtask xrechnung` checks that the configuration's copy is the file `einvoice-xsd`
  embeds, byte for byte). `fetch-xrechnung.sh` and the catalogue write the same manifests.
- `einvoice-validate`: the text output names the scenario of a document (`scenario <name>`), says
  why a document has none (`invalid: no scenario`), and notes an unchecked schema; the JSON
  output adds `rule_set.unchecked_schema` and, for each document, `scenario`, `no_scenario`
  (`matching`, `message`) and `stylesheets_run` (pure additions to format version 1).
- `cargo xtask <family>` writes `acceptance.tsv` in each fetched corpus: for every case, the KoSIT
  scenario it matches, whether the findings of each stylesheet reject it as the reference
  validator reads them (an unflagged failed assert is an error; for XRechnung, the levels the
  KoSIT report stylesheet gives flags and roles, then those of the scenario), and for XRechnung
  the verdict of the XML schema of the scenario (`schema`: xmllint, run with `--huge` on the
  configuration's copy of the schema, UBL 2.1 or CII D16B; `$XMLLINT`, else `xmllint`) and the
  verdict of the configuration (`verdict`: rejected without a scenario or with several, or when
  the schema does not find the document valid, otherwise from the stylesheets of the scenario);
  documents the test suite accepts must be schema-valid too. It checks the scenarios of the
  XRechnung bundles against the configuration (namespaces, `match` as written, stylesheets,
  levels, XML schema). `tests/rules_equivalence.rs` of `einvoice-cli` compares the verdict of
  every bundle on every corpus document with it, and for XRechnung the scenario, the stylesheets
  that ran and the schema verdict: the bundles agree with xmllint on every document but three
  mutation cases with decimals beyond libxml2's 24 digits
  (`crates/einvoice-cli/tests/schema-disagreements.tsv`). With the CII D16B schema (#32), the
  `xrechnung-cii` bundle finds 183 more of its 1494 corpus documents invalid, as the KoSIT
  configuration does: 132 that the stylesheets accept, and 51 on which a stylesheet stops with
  a dynamic error (the report is returned, invalid); `xrechnung-ubl` already ran UBL 2.1.
- XML Schema validation of CII (#32): `einvoice-xsd` vendors and embeds the UN/CEFACT CII D16B
  schemas with uncoupled code lists (`CrossIndustryInvoice_100pD16B.xsd` and the three modules
  it imports), UNECE's files byte for byte, CRLF line ends included
  (`schemas/uncefact-cii-d16b/`, `-text` in `.gitattributes`, `SHA256SUMS` checked by the tests);
  `Schema::cii_d16b`, `CII_D16B_FILES`, `CII_D16B_ENTRY_POINT`. Loading takes 1.4 ms and keeps
  326 KiB. Compared with xmllint (libxml2 2.9.14) on the 24 cases and 808 mutation cases of
  the CEN CII corpus (`tests/oracle/cii/`): every case agrees but 4 mutation cases, listed with
  their cause (decimals beyond libxml2's 24 digits, a document deeper than its parser accepts, a
  second error on content already invalid); `scripts/check.sh facturx`, `xrechnung` and `fr-ctc`
  run the same comparison on their CII corpora (`EINVOICE_XSD_FETCHED_REQUIRED`). The CII D22B
  schemas are not vendored: they carry no licence notice. In `einvoice-validator`,
  `SchemaId::CiiD16b`, named `cii-d16b` in a manifest.
- Rule sets fetched on demand (#35): `einvoice-validate rules list|fetch|path|clear`, `--rules NAME`
  for Factur-X (`factur-x-minimum` … `factur-x-extended`), XRechnung (`xrechnung-ubl`,
  `xrechnung-cii`, alias `xrechnung`), the French rules (`fr-ctc-*`) and Peppol BIS Billing 3.0
  (`peppol-bis-billing-ubl`, alias `peppol`), read from a cache and downloaded only with
  `rules fetch` or `--fetch`; library `fetched_rule_sets`, `RuleSetCache`, `Validator::from_cache`,
  `Error::NotCached`, `Error::Fetch`, and the feature `fetch` (HTTPS on rustls, retries, resumed
  downloads, size limits, ZIP/tar/gzip readers with path checks; Peppol compiled as
  ph-schematron compiles it, in Rust, without Java). The bundles are byte for byte those of
  `scripts/fetch-*.sh`; with them, every corpus gives the Saxon-HE results.
- `einvoice-validate rules fetch peppol` needs no Java: the Peppol BIS Billing Schematron is
  compiled by `einvoice-schematron` with the ISO Schematron skeleton of the pinned jar of
  ph-schematron-isosch 10.3.1 (Maven Central, SHA-256 checked), and both stylesheets are byte for
  byte those of ph-schematron-maven-plugin 10.3.1, their SHA-256 pinned
  (`PEPPOL-EN16931-UBL` 8c318aad…, `CEN-EN16931-UBL` 06a850a4…); `FetchOptions::java` and
  `FetchedRuleSet::needs_java` are gone. The Peppol corpus (707 cases and 509 mutation cases)
  gives the Saxon-HE results with the bundle it builds.
- `einvoice-schematron` compiles Schematron to XSLT as ph-schematron 10.3.1 does, in Rust:
  `Skeleton` prepares the three stylesheets of the ISO Schematron skeleton (`iso_dsdl_include`,
  `iso_abstract_expand`, `iso_svrl_for_xslt2` importing `iso_schematron_skeleton_for_saxon`, the
  files of ph-schematron-isosch 10.3.1, `SKELETON_FILES`) on the engine; `Skeleton::compile` runs
  them, each result passed to the next step as ph-schematron passes it (a DOM document of the
  JDK read through Saxon's DOM wrapper, `TreeView::Dom`), `document()` restricted to the files
  given (`Sources`), and writes the stylesheet as ph-schematron-maven-plugin writes it. On the
  synthetic test schemas and on the Peppol and CEN Schematron files, each step gives
  ph-schematron's result, byte for byte.
- The XSLT engine runs the ISO Schematron skeleton as Saxon-HE 12.10 does, each construct
  checked against Saxon-HE by probes: `xsl:copy`, `xsl:copy-of`, `xsl:element`,
  `xsl:namespace`, `xsl:processing-instruction`, `xsl:message` (`terminate` as an attribute
  value template, `XTMM9000`), `xsl:attribute` with `namespace`, `xsl:import` with import
  precedence (`einvoice_xslt::load_with_imports`, `ModuleResolver`, `resolve_uri`),
  `xsl:namespace-alias`, `use-when` (`element_available`, `system_property`),
  `extension-element-prefixes`; the `descendant::` axis; `document()` with two arguments and
  base URIs, `id()`, `generate-id()`, `element-available()`, `function-available()` and
  `system-property()`; result trees given to another transformation
  (`Executable::transform_tree`, `TreeTransformation`, `einvoice_xslt_exec::serialize`,
  `Serialization`). A named template that ends with a call of another runs it in place (a tail
  call, as Saxon does), at most `TransformOptions::max_tail_calls` times in a row
  (`DEFAULT_MAX_TAIL_CALLS`, 65 536; beyond, `LimitExceeded::TailCalls`). The `namespace::` and
  `following::` axes are still refused.
- `cargo xtask bench`: the wall time, CPU time and peak memory of `einvoice-validate` and of
  Saxon-HE 12.10 on the whole corpus of each rule set (every document, mutation cases included,
  through every stylesheet) at the given thread counts, a run doing exactly Saxon-HE's work (each
  stylesheet alone, no schema), Saxon-HE's time without the JVM start, and the latency of one
  document. Results in `docs/benchmarks.md` and `docs/benchmarks.json`.
- The engine runs the Factur-X 1.09.2, XRechnung 3.0.2 and French (FNFE-MPE 1.4.0.04) rule
  sets as Saxon-HE 12.10 does (#24, #26, #27, #28): on every case and mutation case of their
  corpora, every stylesheet gives the Saxon-HE result (4250, 6388 and 8483 runs), checked by
  the new equivalence tests `facturx_equivalence`, `xrechnung_equivalence` and
  `fr_ctc_equivalence` of `einvoice-xslt-exec` (required by `scripts/check.sh facturx`,
  `xrechnung` and `fr-ctc`) and by `cargo xtask engine-status` through `einvoice-validate`.
  `upstream.toml` lists the three rule sets as tested (`high`).
- Stylesheets compiled by SchXslt (#30): nodes constructed in a sequence (an `xsl:function`
  body, a variable or parameter with `as`) are new parentless elements, attributes, text nodes
  (zero-length ones included) and comments, as Saxon-HE constructs them; `xsl:call-template`,
  `xsl:apply-templates` and `xsl:next-match` there too; constructed elements keep their
  namespaces when copied; `/` on a tree without document node is `XPDY0050`;
  `system-property()` with a prefixed name.
- XSLT 3.0 parenthesized patterns as Saxon-HE 12.10 reads them (#24): `(A | B)` alone is
  `A | B`, a path ending with `(A | B)` is a union, `(A | B)[p]/c` and `(a | b)/(c | d)/e` match
  as written; a predicate statically non-numeric (a boolean variable) is not positional.
- `fn:empty` (#24).
- Rule-set bundles, manifest format 2 (#24): `[[resource]]` tables list the files the
  stylesheets read with `document()` (path and SHA-256, checked when the bundle is loaded);
  `document()` reads them, by their URI relative to the stylesheet's (`file:` in a directory,
  `bundle:/` in memory), and nothing else (`FODC0002`). `RuleSet::resources`,
  `ResourceEntry`. The fetch scripts write format 2 bundles with the Factur-X code lists.
- Version 2 of the canonical SVRL form (`einvoice-svrl`, `corpus/SVRL-CANONICAL.md`):
  `svrl:metadata` and its foreign elements, the time of the run written `{run-time}`;
  `einvoice_svrl::run_time`. Reports without `svrl:metadata` stay in version 1.
- API ergonomics (#31): `StylesheetReport::fired_rules`, the number of fired rules;
  `ValidateOptions::fail_closed`, under which a finding without flag or with a flag other than
  `fatal` and `warning` is fatal (`Severity::from_flag_fail_closed`); `Validator` is `Clone`,
  sharing the loaded rule set.
- `cargo xtask engine-status` raises the maximum depth for a case deeper than the default, as
  the equivalence tests do, and writes format 2 bundles with the resources of a stylesheet.
- Third-party notices (#22): `einvoice-validate --licences` prints, after the licences of the
  embedded works, the notice of every crate from crates.io compiled into the binary (version,
  licence expression and the licence used, authors, licence text from the crate's package); the
  npm package ships `THIRD-PARTY-NOTICES` for its module. Generated from `Cargo.lock` by
  `cargo run --locked -p einvoice-testkit --bin third-party-notices -- --write`; a test fails
  when they are not up to date.
- Test corpus of Factur-X 1.09.2 (ZUGFeRD 2.5.2), CII (#26), built locally and not committed
  (`corpus/factur-x-1.09.2/README.md`): `scripts/fetch-facturx.sh` takes the stylesheets of
  the five profiles and the code lists they read with `document()` from the Mustang validator
  2.26.0 on Maven Central, and the Factur-X examples of the French standards XP Z12-012 and
  XP Z12-014 (version 1.4) from `fnfe-mpe.org`, every input checked against a pinned SHA-256,
  and writes one rule-set bundle per profile; `cargo xtask facturx` builds 42 cases and 808
  mutation cases with the Saxon-HE results of each profile. The 27 examples are accepted by the
  stylesheet of the profile they declare.
- Test corpora of XRechnung 3.0.2, UBL and CII (#27), built locally and not committed (the
  XRechnung Schematron includes rules derived from Peppol BIS Billing 3.0):
  `scripts/fetch-xrechnung.sh` fetches from KoSIT's GitLab the validator configuration
  2026-08-31 (the CEN rules 1.3.16 and the XRechnung Schematron 2.6.0, compiled with SchXslt),
  the test suite 2026-08-31 and the unit tests of the XRechnung Schematron, which XML Mutate 0.6
  expands into mutants, each with its expectation; `cargo xtask xrechnung` builds 1196 UBL and
  686 CII cases and 1312 mutation cases with the Saxon-HE results of both stylesheets. The
  1733 XML Mutate expectations hold, and the 86 test-suite instances are accepted under the
  scenarios of the configuration (`customLevel` included).
- Test corpora of the French e-invoicing rules, FNFE-MPE package 1.4.0.04 (#28), built locally
  and not committed: `scripts/fetch-fr-ctc.sh` fetches the package (CII, UBL, EXTENDED-CTC-FR,
  the BR-FR rules and their WARNING version, the life-cycle statuses, e-reporting, and the
  package's Factur-X build) and the examples of XP Z12-012 and XP Z12-014 from `fnfe-mpe.org`,
  and writes 14 rule-set bundles; `cargo xtask fr-ctc` builds 53 CII, 75 UBL and 70 CDAR cases
  and 1312 mutation cases with the Saxon-HE results of each stylesheet. The 135 examples are
  accepted by the stylesheets of the profile they declare.
- `cargo xtask engine-status <family>`: runs `einvoice-validate` on the corpus of a fetched
  rule set, one stylesheet at a time, and reports which cases give the Saxon-HE result, which
  differ, which fail and which stylesheets do not load.
- `scripts/check.sh facturx`, `xrechnung` and `fr-ctc`, and the `rulesets` workflow, which
  builds the three families of corpora and runs their cross-checks.
- `upstream.toml`: the pins of Factur-X, the Mustang validator, XRechnung, its Schematron,
  validator configuration and test suite, XML Mutate, the FNFE-MPE package and the examples of
  XP Z12-012 and XP Z12-014.
- `einvoice-pdf`, a new crate (#12): extracts the embedded invoice XML (`factur-x.xml`,
  `zugferd-invoice.xml`, `ZUGFeRD-invoice.xml`, `xrechnung.xml`, `order-x.xml`) of a Factur-X,
  ZUGFeRD or Order-X PDF/A-3 file, byte for byte, with its `/AFRelationship` and the XMP
  metadata of the document (PDF/A identification, `DocumentType`, `DocumentFileName`,
  `Version`, `ConformanceLevel`), in pure Rust (`miniz_oxide`, `roxmltree`), for
  `wasm32-unknown-unknown` too. Reads the part of PDF on the path to embedded files (classic and
  stream cross-references, incremental updates, hybrid files, object streams, `FlateDecode`
  with PNG predictors, name trees), verifies `/Params /Size` and `/CheckSum`, refuses anything
  else as unsupported, broken syntax as malformed and encrypted files; bounds every resource
  against hostile input.
- PDF files in `einvoice-validator` (feature `pdf`, on by default) and `einvoice-validate`
  (#12): `pdf::extract` and `Error::Pdf` in the library; the tool validates a file or standard
  input that starts with `%PDF-` through the invoice XML it embeds, with the rule set of its root
  element unless `--rules` or `--bundle` is given, and reports the embedded file, its
  `/AFRelationship`, the PDF/A identification and the Factur-X / ZUGFeRD properties (a `pdf`
  line in the text output, `documents[].pdf` in the JSON report, an addition to format version
  1). A PDF no invoice can be read from is an error of kind `io` (exit status 2), an encrypted or
  unsupported one of kind `unsupported` (exit status 3).
- The npm package `einvoice-validator` (#9), built from the new crate `einvoice-wasm`
  (`wasm-bindgen`, not published on crates.io): `new Validator("cen-ubl" | "cen-cii")`,
  `validate(bytes, { documentUri, now, timezoneMinutes, maxDepth })` returning the members of a
  document of the JSON format of `einvoice-validate`, `validateSvrl`, `builtinRuleSets()`,
  `licences()`, `version()`, errors thrown with a `kind` (and `limit` for a document beyond a
  limit), TypeScript declarations; one package for Node.js 22, browsers and bundlers. Its tests
  run the CEN UBL and CII corpora (2515 cases) against the Saxon-HE goldens under Node.js and in
  headless Chrome and Firefox. `maxDepth` is at most 400: the stylesheets recurse on the
  JavaScript engine's native stack, which a Web Worker in Chrome exhausts beyond 600 levels.
  Built with `opt-level` 3 (6.2 MB, 0.67 MB with brotli).
- `einvoice-xml`, a new crate (#18): a streaming (pull) XML 1.0 parser with namespaces, which
  builds no tree; it accepts what `roxmltree` 0.21 accepts and reports its errors, except for the
  constructs `roxmltree` accepts against XML 1.0 or Namespaces in XML, which it refuses; linear
  time on hostile input. The validator, `einvoice-xsd` and `einvoice-xpath-eval` read documents
  with it (see Changed).
- `einvoice-xsd`: `Schema::validation`, a `Validation` given the events of an
  `einvoice_xml::Reader`, for a caller that reads the document itself (#18).
- A memory budget (#18): the test `footprint` of `einvoice-validator` fails when the peak heap
  of a validation of the largest CEN corpus document, or of a synthetic invoice of 1000 lines,
  exceeds the recorded budget (64 KiB and 1850 KiB above the document); the `footprint`
  example measures the peak heap and time of a validation and of its stages on that document
  and on an invoice of 5000 lines.
- `einvoice-xslt-exec`: `Executable::transform_compact` (#18) keeps the result as a
  `CompactOutput`, each distinct piece of its text once (a piece runs from one tag written in
  full to the next; the distinct pieces in blocks of up to 64 KiB, never reallocated), and
  gives a `ResultObserver` the result as it is written, as a parser of its text reads it
  (elements with their attributes after namespace fixup, character data with the whitespace of
  indentation, comments). `CompactOutput` compares equal to the same text, `to_text()` makes
  the text, `Display` writes it, `Debug` shows it as a `String`. The equivalence tests of every
  corpus run through it and check, on each case, that the observer was given what
  `einvoice-xml` reads from the text; a property test checks it, and the text, on generated
  documents with mixed content, namespaces and indentation.
- Regression cases for a misordered header reported from real-world use, on synthetic
  documents (mutations of the official examples): a credit note with `cbc:TaxPointDate` after
  `cbc:CreditNoteTypeCode` and `cbc:Note` (the invoice order), an invoice with it before
  `cbc:InvoiceTypeCode` (the credit note order), and both in UBL order as controls
  (`corpus/mutations.toml`, 508 CEN and 509 Peppol mutation cases), with the same edits in the
  xmllint oracle of `einvoice-xsd`. The EN 16931 and Peppol rules report nothing on any of them;
  `einvoice-validate` reports exactly one schema error on the misordered element, as xmllint
  does, and exits with 1 (`einvoice-cli` tests).
- `einvoice-bigint`, a new crate: arbitrary-precision signed integers in safe Rust, without
  dependencies and without assembly (no `unsafe`, no `asm!`, no 128-bit division), `no_std` with
  `alloc`, built for `wasm32-unknown-unknown` too. No division panics: `checked_div_rem`,
  `checked_div`, `checked_rem`, `checked_div_floor`, `checked_mod_floor`, `checked_div_ceil` and
  the others return `None` for a zero divisor, `div_rem_u64` divides by a `NonZeroU64`. The
  tests compare every operation with `num-bigint` 0.5, a dev-dependency only.

### Changed

- `einvoice-xdm` computes `xs:integer` and `xs:decimal` with `einvoice-bigint` instead of
  `num-bigint`, `num-integer` and `num-traits`. `num-bigint` and `num-integer` are no longer
  compiled into the binary, the library or the WebAssembly module, nor the inline assembly of
  `num-bigint` (a `div` instruction on x86 and x86-64); `num-traits` remains only as a dependency
  of `crypto-bigint` (feature `fetch`), and the WebAssembly module has none of them. The results
  are unchanged: the Saxon-HE oracle of `einvoice-xdm` and every equivalence suite give the same
  output, byte for byte. API changes of `einvoice-xdm`: `Decimal::from_parts`, `stripped`,
  `unscaled`, `trunc_to_bigint`, `checked_div_trunc` and `Integer::from_bigint`, `big`, `value`
  take or return an `einvoice_bigint::BigInt`; `Decimal` no longer implements
  `num_traits::ToPrimitive`, and `Decimal::to_i64` and `Decimal::to_u64` are inherent methods
  (`Decimal::to_f64` already was).
- The engine is faster on every rule set: whole job of `einvoice-validate` on each corpus (every
  document, mutation cases included, through every stylesheet), one thread, before and after on
  the same machine, alternating: CEN UBL 2.18 to 1.34 s, CEN CII 0.756 to 0.569 s, Peppol 2.29
  to 1.42 s, Factur-X 24.5 to 7.20 s, XRechnung UBL 6.19 to 4.08 s, XRechnung CII 3.38 to
  2.64 s, French CII 22.0 to 8.67 s, French UBL 9.15 to 5.28 s, French CDAR 0.139 to 0.069 s
  (70.6 to 31.3 s in all, 2.3 times faster); on 8 threads, 9.68 to 4.61 s. With
  `cargo xtask bench` the speed-up over Saxon-HE 12.10, JVM start excluded, is now 1.4 times
  (XRechnung UBL) to 13 times (French CDAR) on one thread, 2.4 times in all. The output of
  every run is unchanged, byte for byte. What changed:
  - a document `document()` loads (the code lists) is parsed once per stylesheet, not once per
    transformation (`einvoice_xpath_eval::DocumentCache`, kept by `Executable`); a regular
    expression compiled at evaluation is kept for every transformation
    (`einvoice_xpath_eval::RegexCache`, `DynamicContext::with_regex_cache`);
  - template rules: per mode and node kind, the outcome of the rule search when it needs no
    pattern to be matched; patterns every node of their rule chain matches (`cac:X`, `text()`,
    `@*|node()`) are not matched; the chains of a name are looked up once per name of the
    source document (`Node::name_id`, `Document::name_count`); a rule with an empty body is not
    set up to run; the built-in rule counts the children only when a rule needs it;
  - XPath: paths and steps are walked on the tree, predicates evaluated in turn, instead of
    being built as lazy sequences, for effective boolean values, `exists`, `empty`, `count`,
    general comparisons, single operands and the value of an expression; `//name` reads the
    elements of the name from an index each document builds when searched twice; a node is
    compared without copying its value (`einvoice_xdm::ops::general_compare_untyped`);
    `xsl:apply-templates` reads its selection (`@*|*`, `.`) from the tree
    (`CompiledExpr::select_in_place`, `NodeSelection`, `SelectionStorage`);
  - peak memory of a process is up to 3.5 MiB higher (the cached code lists: Factur-X 52.1 to
    55.6 MiB on one thread, 65.7 to 69.2 MiB on 8); the peak heap of a validation is unchanged
    (51 KiB for the largest CEN UBL corpus document, 1673 KiB for the invoice of 1000 lines,
    1657 before; `tests/footprint.rs`). The
    stack of a level of a recursion through `xsl:apply-templates` in a debug build is 792 bytes
    for a selection read from the tree (888 before), 1648 for another selection, which no
    official stylesheet recurses through.
- The HTTPS client of the feature `fetch` (#35) runs rustls on a cryptography provider of its own
  on the RustCrypto crates (TLS 1.2 and 1.3, AES-GCM and ChaCha20-Poly1305, ECDHE on P-256 and
  P-384, ECDSA and RSA verification): no C or assembly file is built anywhere. `deny.toml` bans
  the crates that build native code (`cc`, `cmake`, `bindgen`, `pkg-config`, `ring`,
  `aws-lc-sys`, `openssl-sys` and others), and `docs/dependencies.md` audits every third-party
  crate of the binary, the library and the WebAssembly module (`unsafe`, build scripts, inline
  assembly), checked by `tests/supply_chain.rs` of `einvoice-testkit`.
- The built-in rule set `cen-cii` validates documents against the UN/CEFACT CII D16B schemas
  first (`schema = "cii-d16b"`), as `cen-ubl` does with UBL 2.1 (#32): a document invalid for
  the schema is invalid, and when the CEN stylesheet stops with a dynamic error on such a
  document the report is returned, invalid, with the error in `StylesheetReport::failure`. On
  the CEN CII corpus, the 9 VEFA test documents (fragments, already invalid for the CEN rules)
  and 274 of the 808 mutation cases have schema errors (xmllint agrees, but for the documented
  differences: it also rejects the two 400-digit decimals, and refuses the 304-level document
  as not well-formed); 127 of those mutation cases are valid for the CEN rules alone, and in 42
  the stylesheet stops with a dynamic error. The SVRL reports are unchanged.
  `LicenseRef-UN-CEFACT-Copyright-Notice` now carries the 2016 copyright line of the CII schemas
  besides the 2006 one of UBL 2.1. `cargo xtask bench` measures the built-in `cen-cii` with its
  schema, and the run without it.
- The manifests written by `scripts/fetch-*.sh` have neutral comments; `fetch-peppol.sh` pins
  the embedded CEN version (`cen=1.3.16`). `einvoice-cli` depends on rustls, the RustCrypto
  crates and webpki-roots by default (feature `fetch`; licences in THIRD-PARTY-NOTICES; `--no-default-features` leaves them
  out).
- `einvoice-validator` keeps each SVRL report compactly and reads its findings as the
  stylesheet writes it (#18), with `Executable::transform_compact`, instead of writing it to a
  string and parsing that string again with `roxmltree` (no longer a dependency of the
  library). Breaking: `StylesheetReport::svrl` is a method returning `&CompactOutput`
  (re-exported by `einvoice-validator`) instead of a `String` field; `.svrl().to_text()` gives
  the text. A result that is not well-formed XML (no element at its top, two, or a comment
  XML forbids) is still refused with the parser's error. On a synthetic invoice of 5000 lines
  (4.1 MB; its SVRL report 7.1 MB, 40051 fired rules), the peak heap of a validation (document
  given as a buffer) drops from 21719 KiB to 8310 KiB (5.41 to 2.07 times the document; the
  stylesheet's input tree alone is 7514 KiB, 1.87 times), and the report kept from 9225 KiB to
  181 KiB; with a failed assert on each line (5003 findings), from 28108 KiB to 13536 KiB; on
  an invoice of 1000 lines, from 5020 KiB to 1657 KiB. Time on the CEN UBL corpus: 2 % less.
  Every report (each field, the SVRL text and its canonical form) of every case and mutation
  case of the CEN UBL and CII, Peppol, Factur-X, XRechnung and French corpora, the output of
  `einvoice-validate` in each format and that of the npm package are unchanged.

- Documents are read with the streaming parser `einvoice-xml` instead of `roxmltree` (#18). The
  validator checks a document and validates it against the schema in one pass, without a tree
  (a well-formedness error still comes first, wherever it is, then an element nested too deep);
  `einvoice-xsd` validates without a tree (a document with a document type declaration, which
  the reader refuses, is still read with `roxmltree`); `einvoice-xpath-eval` builds the
  stylesheets' input tree, and that of a document read with `document()`, from two passes of
  the reader, and no longer depends on `roxmltree`. On a synthetic invoice of 5000 lines
  (4.1 MB), the peak heap of a validation (document given as a buffer) drops from 37989 KiB to
  21719 KiB, now the stylesheet's input tree (7514 KiB), the SVRL report it writes and the
  reading of that report; the schema stage peaks at 3 KiB instead of 37989 KiB. Time on the
  CEN UBL corpus: 2.6 % less for the whole job. Every report of the xmllint oracle of
  `einvoice-xsd` and every result of the corpora is unchanged. What changes:
  - documents with a construct that XML 1.0 or Namespaces in XML 1.0 forbids, which `roxmltree`
    accepted, are not well-formed (`Error::NotXml`, `DocumentError::Malformed`, `SXXP0003`
    from `document()`): a character reference to a surrogate or above `10FFFF`, a processing
    instruction target `xml` in any case or with a colon, or without whitespace before its
    data, an XML declaration with another pseudo-attribute, version, encoding name or
    `standalone` value, a name starting with a colon, `xmlns:p=""`, a declaration of the
    `xmlns` prefix, the default namespace or the `xml` prefix declared twice on an element, a
    `p:xmlns` attribute (the crate documentation of `einvoice-xml` gives each rule); on a
    document both refuse, the error reported can be such a construct before `roxmltree`'s;
  - in the stylesheets' input tree, line ends are normalised in comments and processing
    instructions, and a carriage return next to a reference in text is a line end, as XML 1.0
    (section 2.11) and Saxon-HE have it.
- The release workflow publishes the crates on crates.io with trusted publishing (OIDC, no
  stored token), in the environment `release`; `scripts/release.sh --publish` skips versions
  already published and retries rate limits, and `--trusted-publishing` configures crates.io
  (#17).
- `num-bigint` 0.5.1 (#19); the property tests of `einvoice-xdm` check decimals against an exact
  fraction type of their own instead of `num-rational`, which has no release for `num-bigint`
  0.5.
- The oracle runs Saxon-HE with a fixed clock (`-now:2000-01-01T00:00:00Z`), so that SchXslt
  reports, which record the time of their run in `svrl:metadata`, are the same on every run;
  their goldens are in version 2 of the canonical form. The goldens of the vendored corpora are
  unchanged.
- The oracle runs again, alone, a mutation case on which the run of all of them stops with a
  dynamic error, and takes that run for its golden: Saxon-HE keeps what its
  `LearningEvaluator` learns of the variables in the compiled stylesheet, over every document
  of a run, so that a later document can fail where alone it would not. The committed goldens
  are unchanged.
- `einvoice-xpath`: `PatternAlternative::Parenthesized` is gone; a path pattern's steps are
  `PathStep`s, axis steps or parenthesized ones (#24). `NamespaceBinding` moves from
  `einvoice-xslt` (which re-exports it) to `einvoice-xpath`.
- `einvoice-xpath-eval`: `Host::call_function` takes its arguments as `Argument`s, evaluated
  when the host asks for them, and the dynamic context; `StaticContext::variable_reference`
  tells a reference in a loop.
- `einvoice-xslt-exec`: local variables, template parameter defaults and the parameters of
  `xsl:call-template` and stylesheet functions are evaluated when Saxon-HE 12.10 evaluates them
  (inlined when referred to once outside a loop, as its `LearningEvaluator` learns otherwise,
  where they are bound when Saxon's count of their references says so): an error in a value
  nothing reads is no longer raised where Saxon does not raise it. A lazy value computed inside
  another counts in the stack limit.
- `einvoice-validator`: `StylesheetReport::canonical_svrl` gives version 2 for a report with
  `svrl:metadata`.
- The oracle accepts the stack frames Saxon-HE prints after an error raised in a stylesheet
  function or a named template (`In function ...`, `In template ...`).
- The licence expressions of `einvoice-validator` and `einvoice-cli` name the terms of the OASIS
  UBL 2.1 schemas, which every build embeds through `einvoice-xsd`: `MIT AND EUPL-1.2 AND
  LicenseRef-OASIS-Copyright-Notice AND LicenseRef-UN-CEFACT-Copyright-Notice AND
  W3C-19980720`, with the three texts in their `LICENSES/`.
- `scripts/release.sh` sets the version of the npm package
  (`crates/einvoice-wasm/npm/package.json`) with the workspace's, and `--verify` checks it.

### Fixed

- A path whose nodes the engine sorts (its left operand not known to give nodes in document
  order: `document('codes.xml')/codedb/cl[@id = 1]`, `(a, b)/c`) is read unsorted and lazily
  where neither its order nor its duplicates matter, an effective boolean value (`xsl:if`,
  `xsl:when`, `if`, `not`, `boolean`, `and`, `or`, a predicate), `exists`, `empty` and an
  operand of a general comparison, as Saxon-HE 12.10 reads it (`Expression.unordered`): its
  left operand in its own order, until a node decides. The engine read every node first, and
  raised the error of a node Saxon-HE never reads: `exists(document('codes.xml')/codedb/cl[@id
  = 1])` was `FORG0001` when a later `cl` has a non-numeric `id`, where Saxon-HE gives `true`.
  New probes `unordered-paths`, `error-unordered-count` and `error-unordered-first` of
  `einvoice-xslt-exec`, their expected results from Saxon-HE 12.10. No result of the corpora
  changes.
- Verdicts of the rule sets with unflagged asserts (#36): a failed assert without a flag made no
  document invalid, so `einvoice-validate --rules factur-x-en16931` and the other Factur-X rule
  sets (and the French bundles that run a Factur-X profile) said `valid` for documents whose
  reports hold failed asserts that the Mustang validator rejects them for. Their bundles now say
  `unflagged-assert = "fatal"`; the XRechnung bundles too (the KoSIT validator reads a missing
  flag as an error), with the levels the scenarios of the configuration give rule ids
  (`customLevel`: for instance `UBL-CR-646` is fatal in an XRechnung 3.0 invoice). On every
  document of the corpora, the verdict of every bundle is now the reference validator's reading
  of the Saxon-HE reports. The manifests change: fetch the Factur-X, XRechnung and French rule
  sets again (`rules fetch factur-x xrechnung fr-ctc`).
- `scripts/fetch-xrechnung.sh` builds the same XML Mutate mutants on every machine: XML Mutate
  numbers the mutants in the order it reads the unit tests, and writes the number into the name
  and the content of each; read from a directory, that order was the file system's. The script
  now gives it the tests one by one, sorted by name; new listing pins for the UBL and CII
  mutants.
- `scripts/fetch-xrechnung.sh` decompresses the XRechnung Schematron archive to a file before
  reading its commit id: no `gzip: stdout: Broken pipe` where SIGPIPE is ignored.

## [0.2.0] - 2026-11-08

Tested against the official rule sets: CEN EN 16931 validation artefacts 1.3.17, Peppol BIS Billing 3.0.22.

### Added

- `einvoice-xsd`: the XML Schema constructs of the CII-family schemas (UN/CEFACT CII D16B and
  D22B, Factur-X profiles) (#25): `fixed` and `default` values on attributes and elements, fixed
  values compared in the value space of their type; anonymous complex types in local element
  declarations; `minLength` and `maxLength` on the string types. A synthetic xmllint oracle suite
  (180 document cases, 69 schema mutations) covers them.
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

### Security

- A deeply nested document no longer aborts the process with a stack overflow (#29). The
  stylesheets apply their template rules one element level inside another, and the location
  paths of the ISO Schematron skeleton recurse over the ancestors of a node, so a document of
  about 20 KB nested 500 deep (on a 2 MiB thread, 1500 on an 8 MiB main thread) overflowed the
  stack in the stylesheet stage, which aborts the whole process rather than returning an error.
  Now:
  - Every parsed document, the one validated and those `document()` loads, may be nested at
    most 256 elements deep (the document element is at depth 1; libxml2's default
    `xmlParserMaxDepth`). Beyond it the document is refused before its schema is checked:
    `Error::Limit` with `LimitExceeded::Depth` and the line and column of the first element
    too deep (`ErrorKind::Unsupported`), a `LIMIT` error in `einvoice-xslt-exec`
    (`TransformError::limit`), `DocumentError::limit` in `einvoice-xpath-eval`. The maximum is
    set with `ValidateOptions::max_depth`, `TransformOptions::max_depth`,
    `Document::parse_with_max_depth`, `Document::parse_loaded_with_max_depth`, and
    `einvoice-validate --max-depth N` (default 256, at most 100000; such a document is an error
    of kind `unsupported`, exit status 3), whose worker threads get a stack that grows with N.
  - Template rules, built-in rules, named templates and stylesheet functions invoked one inside
    another are bounded in nesting (`TransformOptions::max_nesting`, `DEFAULT_MAX_NESTING`:
    `max_nesting_for_depth` of the maximum depth, twice it plus 64, 576 by default) and in the
    stack they use (`TransformOptions::max_stack` and `ValidateOptions::max_stack`,
    `DEFAULT_MAX_STACK`: 1.5 MiB, 640 KiB on WebAssembly): a stylesheet that recurses without
    end fails with `LimitExceeded::Nesting` or `LimitExceeded::Stack`. These errors are never
    recovered as pattern warnings.
  - The stack each level of recursion needs is reduced, from 13.7 KiB to 2.8 KiB in a debug
    build and from 2.9 KiB to 1.3 KiB in a release build: at the default maximum depth, the CEN
    rule sets and both Peppol stylesheets run on a thread with the default 2 MiB stack, debug
    builds included (about 0.8 MiB for CEN UBL, 1.5 MiB for Peppol in a debug build). Copying a
    node to the result or into a temporary tree, the path of a node in a warning, and the
    canonical form of an SVRL report (which refuses elements nested more than 16 deep) no
    longer recurse over the depth of a tree.
  - Breaking: `TransformOptions` has the public fields `max_depth`, `max_nesting` and
    `max_stack` (construct it with `..TransformOptions::default()`); `LimitExceeded` is a new
    `non_exhaustive` enum, `Error::Limit` a new variant.
  - The CII equivalence tests run on the default 2 MiB stack of a test thread: the mutation
    case `long/deep-nesting` (304 deep) is refused under the default maximum, then compared with
    Saxon-HE with the maximum raised.

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

[Unreleased]: https://github.com/einvoice-validator/einvoice-validator/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/einvoice-validator/einvoice-validator/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/einvoice-validator/einvoice-validator/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/einvoice-validator/einvoice-validator/releases/tag/v0.1.0
