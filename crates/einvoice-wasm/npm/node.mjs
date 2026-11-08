// The entry point for Node.js: the WebAssembly module is read from the package and
// instantiated synchronously, so the API is ready as soon as the package is imported (or
// required: the module has no top-level await). The default export, init(), resolves at
// once, so code written for browsers (await init()) runs unchanged.
import { readFileSync } from "node:fs";
import { initSync } from "./einvoice_validator.js";

initSync({
  module: readFileSync(new URL("./einvoice_validator_bg.wasm", import.meta.url)),
});

export * from "./einvoice_validator.js";
export { default } from "./einvoice_validator.js";
