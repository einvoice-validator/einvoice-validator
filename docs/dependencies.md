# Dependencies: native code, build scripts, `unsafe`

What the third-party crates compiled into the distributed artefacts contain beyond safe
Rust. The artefacts are the `einvoice-validate` binary (`einvoice-cli`, with its default
features, which include `fetch`), the library (`einvoice-validator`, with any of its
features) and the WebAssembly module of the npm package (`einvoice-wasm`,
`wasm32-unknown-unknown`).

## Policy

- **No C or C++ is compiled and no assembly file is assembled or linked**, on any target.
  No crate of the graphs ships a C, C++ or assembly source, header, object file or
  library; no build script compiles or links native code. Rust inline assembly (`asm!`)
  is in a few crates, listed with what it does and where it is compiled in the table
  "Inline assembly" below; the WebAssembly module has none. The crates that build native
  code (`cc`, `cmake`, `bindgen`, `pkg-config`, `vcpkg`, `ring`, `aws-lc-sys`,
  `openssl-sys`, `libz-sys` and others) are banned in `deny.toml` for the whole
  workspace, dev-dependencies and the repository tooling (`xtask`) included.
- **The project's own crates contain no `unsafe` code**: `unsafe_code = "forbid"` for every
  member of the workspace.
- **Third-party `unsafe`** is listed below, crate by crate: SIMD, hardware cryptography
  instructions, operating system calls, the FFI of the WebAssembly module, data
  structures.
- **The HTTPS downloads** of the rule sets that are not redistributed (the feature `fetch`
  of `einvoice-validator`, on by default in `einvoice-cli`, never built for WebAssembly)
  use rustls with a cryptography provider of the project
  (`crates/einvoice-validator/src/rulesets/fetch/tls.rs`) on the RustCrypto crates: pure
  Rust, without the C and assembly of `ring` or `aws-lc-rs`. What is downloaded is checked
  against the SHA-256 or SHA-512 pinned for it; TLS is a second line of defence. Without
  the feature, nothing is downloaded and none of those crates is built. `rsa` (0.10.0) only
  verifies signatures with public keys: the advisory every
  release of it carries, RUSTSEC-2023-0071 (timing leaks in private-key operations), is
  ignored in `deny.toml` for that reason.

`crates/einvoice-testkit/tests/supply_chain.rs` checks all of it against the packages
themselves (`cargo test -p einvoice-testkit --test supply_chain`, part of
`scripts/check.sh test` and `deny`): it fails when a crate of the graphs ships a native
file, has a build dependency or a build script that builds native code, or when the
tables below do not list exactly the crates of the graphs, those with a build script and
those with inline assembly. A new crate therefore fails the check until it is audited
here.

The graphs: normal and build dependencies (`cargo tree -e normal,build`), procedural
macros included since they run on the build machine, on every target. In the column
"In", CLI is `einvoice-cli` with its default features, library is `einvoice-validator`
with all its features, wasm is the WebAssembly module (its procedural macros run on the
build machine and are not part of the module).

## Third-party crates

| Crate | In | Role | `unsafe` |
|---|---|---|---|
| `adler2` | CLI, library | Adler-32 checksum of zlib streams (miniz_oxide) | none (`forbid(unsafe_code)`) |
| `aead` | CLI, library | AEAD traits (RustCrypto) | none |
| `aes` | CLI, library | AES block cipher (RustCrypto) | AES-NI (x86) and ARMv8 AES intrinsics, chosen at run time; constant-time software fallback |
| `aes-gcm` | CLI, library | AES-GCM (RustCrypto) | none |
| `aho-corasick` | CLI, library, wasm | multi-pattern search, for regex | SIMD prefilters (SSE2, AVX2, NEON, simd128), unchecked indexing in the automata |
| `autocfg` | CLI, library | build-time probes of rustc (build dependency of num-traits) | none |
| `base16ct` | CLI, library | constant-time hexadecimal (RustCrypto) | `from_utf8_unchecked` on the ASCII it writes |
| `base64ct` | CLI, library | constant-time Base64 (RustCrypto, PEM) | `from_utf8_unchecked` on the ASCII it writes |
| `block-buffer` | CLI, library, wasm | block buffering of the hashes (RustCrypto) | casts between blocks and byte arrays of the same layout |
| `bumpalo` | wasm | arena allocator of the wasm-bindgen macros (build machine only) | the allocator itself |
| `cfg-if` | CLI, library, wasm | `cfg` macro | none |
| `chacha20` | CLI, library | ChaCha20 stream cipher (RustCrypto) | SSE2, AVX2 and NEON intrinsics, chosen at run time |
| `chacha20poly1305` | CLI, library | ChaCha20-Poly1305 (RustCrypto) | none |
| `cipher` | CLI, library | cipher traits (RustCrypto) | none |
| `cmov` | CLI, library | constant-time conditional moves (RustCrypto, through ctutils) | inline assembly (see below), so that the compiler cannot turn a selection into a branch |
| `const-oid` | CLI, library, wasm | ASN.1 object identifiers (RustCrypto) | a `repr(transparent)` reference cast |
| `cpubits` | CLI, library | pointer width selection (RustCrypto) | none |
| `cpufeatures` | CLI, library | CPU feature detection (RustCrypto) | `cpuid` (x86), `getauxval`/`sysctlbyname` through libc (aarch64) |
| `crypto-bigint` | CLI, library | constant-time big integers (RSA, field arithmetic) | `repr(transparent)` casts between limbs and words |
| `crypto-common` | CLI, library, wasm | common traits (RustCrypto) | none |
| `crypto-primes` | CLI, library | prime generation (dependency of rsa, for key generation, not used here) | none |
| `ctr` | CLI, library | CTR mode (RustCrypto) | none |
| `ctutils` | CLI, library | constant-time utilities (RustCrypto) | none (`forbid(unsafe_code)`: it delegates to cmov) |
| `der` | CLI, library | ASN.1 DER (RustCrypto: keys, signatures) | `repr(transparent)` reference casts of validated input |
| `digest` | CLI, library, wasm | hash traits (RustCrypto) | none |
| `ecdsa` | CLI, library | ECDSA signature verification (RustCrypto) | none |
| `elliptic-curve` | CLI, library | elliptic curve traits and ECDH (RustCrypto) | `repr(transparent)` casts of non-zero scalars and non-identity points |
| `ff` | CLI, library | finite field traits | none |
| `fiat-crypto` | CLI, library | formally verified field arithmetic, generated (P-384) | none |
| `getrandom` | CLI, library | the operating system's random source | system calls and FFI to the OS (`getrandom(2)`, `getentropy`, `BCryptGenRandom`...) |
| `ghash` | CLI, library | GHASH of AES-GCM (RustCrypto) | none |
| `group` | CLI, library | elliptic curve group traits | none |
| `hkdf` | CLI, library | HKDF (RustCrypto, through elliptic-curve) | none |
| `hmac` | CLI, library | HMAC (RustCrypto) | none |
| `hybrid-array` | CLI, library, wasm | arrays of generic length (RustCrypto) | `MaybeUninit` initialisation and slice casts |
| `inout` | CLI, library | in-place and in/out buffers (RustCrypto) | raw pointers for in-place operations |
| `js-sys` | wasm | JavaScript bindings (WebAssembly module) | FFI to the JavaScript imports |
| `libc` | CLI, library | bindings to the C library that std links anyway (no C compiled) | FFI declarations |
| `memchr` | CLI, library, wasm | byte search (XML parser, regex) | SIMD search (SSE2, AVX2, NEON, simd128) and raw pointers |
| `miniz_oxide` | CLI, library | Deflate decompression (PDF, ZIP, gzip) | none (`forbid(unsafe_code)`) |
| `num-traits` | CLI, library | numeric traits (crypto-bigint) | `to_int_unchecked` after a range check |
| `once_cell` | CLI, library | lazy values (rustls) | the cells themselves (atomics, `UnsafeCell`) |
| `p256` | CLI, library | NIST P-256: ECDHE and ECDSA (RustCrypto) | none |
| `p384` | CLI, library | NIST P-384: ECDHE and ECDSA (RustCrypto) | none |
| `pem-rfc7468` | CLI, library | PEM (RustCrypto) | `from_utf8_unchecked` on the ASCII it writes |
| `pkcs1` | CLI, library | PKCS #1 RSA keys (RustCrypto) | none |
| `pkcs8` | CLI, library | PKCS #8 keys (RustCrypto) | none |
| `poly1305` | CLI, library | Poly1305 (RustCrypto) | AVX2 intrinsics, chosen at run time |
| `polyval` | CLI, library | POLYVAL of GHASH (RustCrypto) | CLMUL (x86) and PMULL (aarch64) intrinsics, chosen at run time |
| `primefield` | CLI, library | prime field arithmetic (RustCrypto) | none |
| `primeorder` | CLI, library | prime order curve arithmetic (RustCrypto) | none |
| `proc-macro2` | wasm | procedural macros of wasm-bindgen (build machine only) | a few unchecked string constructors |
| `quote` | wasm | procedural macros of wasm-bindgen (build machine only) | none |
| `r-efi` | CLI, library | UEFI bindings of getrandom, only for the `uefi` targets (never built here) | FFI |
| `rand_core` | CLI, library | random number traits (RustCrypto) | none |
| `regex` | CLI, library, wasm | regular expressions of the XPath engine | an `unsafe impl` of the `Pattern` searcher |
| `regex-automata` | CLI, library, wasm | regex engines | unchecked transitions and layouts of the automata |
| `regex-syntax` | CLI, library, wasm | regex parser | none (`forbid(unsafe_code)`) |
| `rfc6979` | CLI, library | deterministic ECDSA nonces (signing, not used here) | none |
| `roxmltree` | CLI, library, wasm | XML tree of the schema and stylesheet loaders | none (`forbid(unsafe_code)`) |
| `rsa` | CLI, library | RSA signature verification (RustCrypto; public keys only) | none |
| `rustls` | CLI, library | TLS 1.2 and 1.3 of the downloads | none (`forbid(unsafe_code)`) |
| `rustls-pki-types` | CLI, library | certificate and key types of rustls | one `transmute` of an IPv6 address (`[u16; 8]` to `[u8; 16]`) |
| `rustls-webpki` | CLI, library | certificate path validation of rustls | none |
| `rustversion` | wasm | compiler version macros of wasm-bindgen (build machine only) | none outside its tests |
| `sec1` | CLI, library | SEC 1 point encodings (RustCrypto) | none |
| `sha2` | CLI, library, wasm | SHA-256, SHA-384, SHA-512 (digests of the pins, TLS) | SHA-NI (x86) and ARMv8 SHA intrinsics, chosen at run time; inline assembly only on loongarch64 and riscv (see below) |
| `signature` | CLI, library | signature traits (RustCrypto) | none |
| `spki` | CLI, library | SubjectPublicKeyInfo (RustCrypto) | none |
| `subtle` | CLI, library | constant-time comparisons (rustls, RustCrypto) | an optimization barrier (volatile read) and one `Ordering` cast |
| `syn` | wasm | Rust parser of the wasm-bindgen macros (build machine only) | raw pointers in its token buffer (the other occurrences parse the `unsafe` keyword) |
| `typenum` | CLI, library, wasm | type-level numbers (RustCrypto) | none |
| `unicode-ident` | wasm | identifier tables of the wasm-bindgen macros (build machine only) | `get_unchecked` in its tables |
| `universal-hash` | CLI, library | universal hash traits (RustCrypto) | none |
| `untrusted` | CLI, library | input parsing of rustls-webpki | none |
| `wasm-bindgen` | wasm | JavaScript glue of the WebAssembly module | FFI to the JavaScript imports |
| `wasm-bindgen-macro` | wasm | its macro (build machine only) | none |
| `wasm-bindgen-macro-support` | wasm | its macro code generator (build machine only) | in the code it generates (the FFI glue of the module) |
| `wasm-bindgen-shared` | wasm | shared definitions of wasm-bindgen | none |
| `webpki-roots` | CLI, library | Mozilla root certificates (data) | none (`forbid(unsafe_code)`) |
| `wnaf` | CLI, library | windowed scalar multiplication (RustCrypto) | none |
| `zeroize` | CLI, library | wiping secrets from memory (rustls, RustCrypto) | volatile writes; an assembly barrier (see below) |

## Build scripts

None compiles or links native code; most probe the Rust compiler.

| Crate | What it does |
|---|---|
| `getrandom` | Sets a `cfg` when built with the memory sanitizer. |
| `libc` | Reads the target and the rustc version (`rustc --version`), sets `cfg`s (FreeBSD version, musl version, ...). |
| `num-traits` | Probes the compiler with `autocfg` (compiles one Rust expression) and sets a `cfg`. |
| `proc-macro2` | Reads the rustc version, compiles a Rust probe to detect proc-macro features, sets `cfg`s (build machine, WebAssembly module only). |
| `quote` | Reads the rustc version, sets a `cfg`. |
| `rustls` | Sets the `cfg` `read_buf` on a nightly compiler with the feature `read_buf` (not used here). |
| `rustversion` | Reads the rustc version and writes it for its macros. |
| `wasm-bindgen` | Sets `cfg`s; for Emscripten only, a linker argument exporting a symbol. |
| `wasm-bindgen-shared` | Records the git revision of its source when built from a checkout, and a hash of its schema file. |

## Inline assembly

No crate of the graphs ships an assembly file. These contain `asm!` (Rust inline
assembly); the column "Compiled" says on which targets the code is part of a build.

| Crate | Where | Compiled | What |
|---|---|---|---|
| `cmov` | `src/backends/x86.rs`, `src/backends/aarch64.rs`, `src/backends/soft.rs` | x86 and x86-64 (`cmov`), aarch64 (`csel`); `soft.rs` only on arm, riscv32 and riscv64 | Constant-time conditional moves, which a compiler could otherwise turn into branches. Used by the RustCrypto crates of the feature `fetch` (through `ctutils`). Not in the WebAssembly module. |
| `cpufeatures` | `src/loongarch64.rs` | loongarch64 only | Reads the CPU configuration. |
| `getrandom` | `src/backends/linux_raw.rs`, `src/backends/rndr.rs` | only with `--cfg getrandom_backend="linux_raw"` or `"rndr"`, never set here | Raw Linux system calls; the aarch64 `RNDR` instruction. |
| `sha2` | `src/sha256/loongarch64_asm.rs`, `src/sha512/loongarch64_asm.rs`, `src/sha256/riscv_zknh/unroll.rs`, `src/sha512/riscv_zknh/unroll.rs` | loongarch64; riscv with `--cfg sha2_backend="riscv-zknh"` | SHA-2 rounds; on x86-64 and aarch64 `sha2` uses intrinsics and portable Rust, no assembly. |
| `zeroize` | `src/barrier.rs` | aarch64, arm, x86, x86-64 and the other targets where `asm!` is stable | An empty assembly block (`"# {}"`, an assembler comment: no instruction) taking a pointer to wiped memory, so that the compiler keeps the writes that wipe it. Used by rustls and the RustCrypto crates of the feature `fetch`. Not in the WebAssembly module. |
