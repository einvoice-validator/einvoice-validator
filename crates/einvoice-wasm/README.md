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
- `js-tests/`: the Node.js tests (`node:test`: the API, and every case of the CEN corpus
  against the Saxon-HE goldens), the TypeScript declarations check, and the browser test
  (the same corpus in headless Chrome and Firefox, served from 127.0.0.1).
- `npm.sh`: builds, tests and packs the package.

```sh
crates/einvoice-wasm/npm.sh tools   # pinned wasm-bindgen-cli (crates.io, --locked) and wasm-opt (binaryen, SHA-256 checked)
crates/einvoice-wasm/npm.sh build   # target/npm/einvoice-validator/: opt-level z, LTO, wasm-opt -Oz
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
the code is MIT; the module embeds the CEN EN 16931 stylesheet of the built-in rule set
`cen-ubl` (EUPL-1.2) and the OASIS UBL 2.1 schemas (their own terms). The package ships the
texts in `LICENSES/`, copied from `einvoice-validator` and `einvoice-xsd`.
