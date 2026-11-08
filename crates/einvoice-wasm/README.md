# einvoice-wasm

JavaScript and TypeScript bindings of [einvoice-validator](../einvoice-validator), compiled to
WebAssembly with `wasm-bindgen`: the source of the npm package `einvoice-validator`, for
browsers, bundlers and Node.js. Not published on crates.io (`publish = false`); the npm
package's own README is [`npm/README.md`](npm/README.md).

- `src/lib.rs`: the exported API (`Validator`, `builtinRuleSets`, `licences`, `version`), the
  options and the errors thrown; `src/render.rs`: the report, built as JSON (the members of a
  document of the JSON format of `einvoice-validate`, version 1) and read back with
  `JSON.parse`; `src/types.d.ts`: the TypeScript interfaces, added to the generated
  declarations.
- `npm/`: what the package adds to the output of `wasm-bindgen`: `package.json`, the README,
  and the Node.js entry point (`node.mjs`, which instantiates the module synchronously).
- `js-tests/`: the Node.js tests (`node:test`: the API, and every case of the CEN UBL and CII corpora
  against the Saxon-HE goldens), the TypeScript declarations check, and the browser test
  (the same corpora in headless Chrome and Firefox, served from 127.0.0.1, and the largest
  `maxDepth` on the main thread and in a Web Worker).
- `npm.sh`: builds, tests and packs the package.

```sh
crates/einvoice-wasm/npm.sh tools   # pinned wasm-bindgen-cli (crates.io, --locked) and wasm-opt (binaryen, SHA-256 checked)
crates/einvoice-wasm/npm.sh build   # target/npm/einvoice-validator/: opt-level 3, LTO, wasm-opt -O3
crates/einvoice-wasm/npm.sh test    # Node.js tests, TypeScript check, browser test, on the packed package
crates/einvoice-wasm/npm.sh pack    # npm pack --dry-run
```

The tools are installed outside the repository (`EINVOICE_WASM_TOOLS`, by default
`~/.cache/einvoice-validator/wasm-tools`); `wasm-bindgen-cli` must be the version of the
`wasm-bindgen` crate in `Cargo.lock`, which the build checks. The native tests of the crate
(`cargo test -p einvoice-wasm`) cover the report and the errors; the exported functions call
JavaScript and only run in WebAssembly.

## Licence

`MIT AND EUPL-1.2 AND LicenseRef-OASIS-Copyright-Notice AND LicenseRef-UN-CEFACT-Copyright-Notice AND W3C-19980720`:
the code is MIT; the module embeds the CEN EN 16931 stylesheets of the built-in rule sets
`cen-ubl` and `cen-cii` (EUPL-1.2, the features of those names, on by default) and the OASIS
UBL 2.1 and UN/CEFACT CII D16B schemas (their own terms). The package ships the texts in
`LICENSES/`, copied from `einvoice-validator`, and `THIRD-PARTY-NOTICES`: the notices of the
crates of crates.io compiled into the module (`wasm-bindgen`, `js-sys`, `regex`, `roxmltree` and
their dependencies, on `wasm32-unknown-unknown`), each with the text of the licences it is
used under: MIT, and the Unicode licence v3 too for `unicode-ident`. The file is generated from `Cargo.lock` and the packages of those crates
(`cargo run --locked -p einvoice-testkit --bin third-party-notices -- --write`);
`tests/notices.rs` fails when it is not up to date.
