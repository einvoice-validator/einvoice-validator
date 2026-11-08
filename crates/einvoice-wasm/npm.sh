#!/usr/bin/env bash
# The npm package einvoice-validator, built from this crate (README.md): the WebAssembly
# module and its JavaScript bindings (wasm-bindgen, target web), optimised with wasm-opt,
# with the entry point for Node.js, the TypeScript declarations, the README and the
# licence texts.
#
#   crates/einvoice-wasm/npm.sh tools   # install the pinned wasm-bindgen-cli and wasm-opt
#   crates/einvoice-wasm/npm.sh build   # the package, in target/npm/einvoice-validator/
#   crates/einvoice-wasm/npm.sh test    # Node.js tests, then the browser test, on the packed package
#   crates/einvoice-wasm/npm.sh pack    # npm pack --dry-run: what would be published
#   crates/einvoice-wasm/npm.sh         # build, test, pack
#
# Tools, each from its variable, else the tools directory, else PATH:
# - WASM_BINDGEN: wasm-bindgen, the version of the wasm-bindgen crate in Cargo.lock;
# - WASM_OPT: wasm-opt of binaryen, version BINARYEN_VERSION below.
# `tools` installs both in EINVOICE_WASM_TOOLS (default
# ${XDG_CACHE_HOME:-~/.cache}/einvoice-validator/wasm-tools), outside the repository:
# wasm-bindgen-cli built from crates.io with --locked (kept when that version is there
# already), and binaryen's release archive for x86_64 Linux (or the archive
# BINARYEN_ARCHIVE names), checked against its SHA-256 below on every run and kept in
# EINVOICE_WASM_TOOLS/downloads/, so that a cache of that directory saves both.
# Also needed: the wasm32-unknown-unknown target of the pinned toolchain, jq, node and npm.
#
# The test runs the Node.js tests (node:test) in a throw-away project where the packed
# package is installed from its tarball, offline; then type-checks a TypeScript program
# against its declarations with typescript TYPESCRIPT_VERSION (npm exec, which needs the
# network the first time); then the browser test in each headless browser found: CHROME
# (default google-chrome, chromium or chromium-browser on PATH) and FIREFOX (default
# firefox on PATH). Without typescript or without any browser it prints SKIPPED, except in
# CI (CI set), where it fails.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

BINARYEN_VERSION=133
# The release asset binaryen-version_133-x86_64-linux.tar.gz, by its id in the GitHub
# REST API (the download endpoint of a release asset), and its SHA-256 (the digest
# GitHub records for the asset).
BINARYEN_ASSET=https://api.github.com/repos/WebAssembly/binaryen/releases/assets/580050094
BINARYEN_SHA256=2dc9c7813f5375db93d96ead4b78222fcc3e2677bbb832297af4797782a37489
# The WebAssembly features the pinned toolchain enables by default for
# wasm32-unknown-unknown (rustc --print cfg): wasm-opt may use these and no others.
WASM_FEATURES=(--enable-bulk-memory --enable-multivalue --enable-mutable-globals
    --enable-nontrapping-float-to-int --enable-reference-types --enable-sign-ext)
TYPESCRIPT_VERSION=7.0.2

CRATE=crates/einvoice-wasm
OUT=target/npm/einvoice-validator
TARGET=wasm32-unknown-unknown
TOOLS=${EINVOICE_WASM_TOOLS:-${XDG_CACHE_HOME:-$HOME/.cache}/einvoice-validator/wasm-tools}

say() { printf '\n\033[1m== %s\033[0m\n' "$*"; }

# missing WHAT: SKIPPED locally (status 0), a failure in CI (status 1).
missing() {
    if [ -n "${CI:-}" ]; then
        echo "$1: required in CI" >&2
        return 1
    fi
    echo "SKIPPED: $1"
}

# The version of the wasm-bindgen crate in Cargo.lock: wasm-bindgen-cli must be the same.
bindgen_version() {
    cargo metadata --locked --format-version 1 |
        jq -r '[.packages[] | select(.name == "wasm-bindgen") | .version] | if length == 1 then .[0] else error("wasm-bindgen: \(length) versions") end'
}

# tool NAME VARIABLE-VALUE: the program to run.
tool() {
    if [ -n "$2" ]; then
        echo "$2"
    elif [ -x "$TOOLS/bin/$1" ]; then
        echo "$TOOLS/bin/$1"
    else
        command -v "$1" || { echo "$1 not found: run $0 tools, or set its variable" >&2; return 1; }
    fi
}

cmd_tools() {
    local version archive dir
    version=$(bindgen_version)
    mkdir -p "$TOOLS/bin"
    say "wasm-bindgen-cli $version (crates.io, --locked)"
    cargo install --locked wasm-bindgen-cli --version "$version" --root "$TOOLS"
    "$TOOLS/bin/wasm-bindgen" --version
    say "binaryen $BINARYEN_VERSION (wasm-opt)"
    if [ "$(uname -sm)" != "Linux x86_64" ]; then
        echo "binaryen: only the x86_64 Linux archive is pinned; set WASM_OPT to wasm-opt version $BINARYEN_VERSION" >&2
        return 1
    fi
    dir=$(mktemp -d)
    if [ -n "${BINARYEN_ARCHIVE:-}" ]; then
        # An archive downloaded before: checked all the same.
        archive=$BINARYEN_ARCHIVE
        echo "binaryen: $archive"
    else
        # Kept in the tools directory (a CI cache), downloaded again unless it has the
        # pinned SHA-256: a kept archive is never trusted.
        archive=$TOOLS/downloads/binaryen-version_$BINARYEN_VERSION-x86_64-linux.tar.gz
        mkdir -p "$TOOLS/downloads"
        if [ -f "$archive" ] && [ "$(sha256sum "$archive" | cut -d' ' -f1)" = "$BINARYEN_SHA256" ]; then
            echo "binaryen: $archive already present"
        else
            rm -f "$archive"
            curl --fail --silent --show-error --location --retry 3 -H 'Accept: application/octet-stream' \
                -o "$archive" "$BINARYEN_ASSET"
        fi
    fi
    if [ "$(sha256sum "$archive" | cut -d' ' -f1)" != "$BINARYEN_SHA256" ]; then
        echo "binaryen: the archive does not have the pinned SHA-256 $BINARYEN_SHA256" >&2
        [ -n "${BINARYEN_ARCHIVE:-}" ] || rm -f "$archive"
        rm -rf "$dir"
        return 1
    fi
    echo "binaryen: SHA-256 $BINARYEN_SHA256 verified"
    tar -xzf "$archive" -C "$dir" "binaryen-version_$BINARYEN_VERSION/bin/wasm-opt"
    mv "$dir/binaryen-version_$BINARYEN_VERSION/bin/wasm-opt" "$TOOLS/bin/wasm-opt"
    rm -rf "$dir"
    "$TOOLS/bin/wasm-opt" --version
}

# sizes FILE: its size raw, gzip -9 and brotli (quality 11), with Node.js's zlib.
sizes() {
    # shellcheck disable=SC2016 # JavaScript template literals, not shell expansions
    node -e '
        const { readFileSync } = require("node:fs");
        const zlib = require("node:zlib");
        const data = readFileSync(process.argv[1]);
        const gzip = zlib.gzipSync(data, { level: 9 }).length;
        const brotli = zlib.brotliCompressSync(data, {
            params: { [zlib.constants.BROTLI_PARAM_QUALITY]: 11, [zlib.constants.BROTLI_PARAM_SIZE_HINT]: data.length },
        }).length;
        const kb = (n) => `${n} bytes (${(n / 1024 / 1024).toFixed(2)} MiB)`;
        console.log(`${process.argv[1]}: raw ${kb(data.length)}, gzip ${kb(gzip)}, brotli ${kb(brotli)}`);
    ' "$1"
}

cmd_build() {
    local bindgen opt version expected crate_version work term status=0
    bindgen=$(tool wasm-bindgen "${WASM_BINDGEN:-}")
    opt=$(tool wasm-opt "${WASM_OPT:-}")
    expected=$(bindgen_version)
    version=$("$bindgen" --version)
    if [ "$version" != "wasm-bindgen $expected" ]; then
        echo "$bindgen: $version, Cargo.lock has wasm-bindgen $expected" >&2
        return 1
    fi
    version=$("$opt" --version)
    if [[ $version != "wasm-opt version $BINARYEN_VERSION "* ]]; then
        echo "$opt: $version, expected binaryen $BINARYEN_VERSION" >&2
        return 1
    fi
    echo "$("$bindgen" --version), $version"
    crate_version=$(cargo metadata --locked --no-deps --format-version 1 |
        jq -r '.packages[] | select(.name == "einvoice-wasm") | .version')
    if [ "$(jq -r .version "$CRATE/npm/package.json")" != "$crate_version" ]; then
        echo "$CRATE/npm/package.json: version $(jq -r .version "$CRATE/npm/package.json"), the crate is $crate_version" >&2
        return 1
    fi

    say "cargo build ($TARGET, opt-level 3, LTO)"
    # A profile of its own, so that the release builds of the other jobs are not changed.
    # opt-level 3 rather than z: measured on the CEN corpora under Node.js, 1.8 times as
    # fast for about 95 kB more with brotli (0.67 MiB instead of 0.58 MiB).
    cargo build --locked --target "$TARGET" -p einvoice-wasm --profile npm \
        --config 'profile.npm.inherits="release"' \
        --config 'profile.npm.opt-level=3' \
        --config 'profile.npm.lto=true' \
        --config 'profile.npm.codegen-units=1'

    say "wasm-bindgen --target web, wasm-opt -O3"
    work=target/npm/work
    rm -rf "$OUT" "$work"
    mkdir -p "$OUT/LICENSES" "$work"
    "$bindgen" --target web --out-name einvoice_validator --out-dir "$work" \
        "target/$TARGET/npm/einvoice_wasm.wasm"
    "$opt" -O3 "${WASM_FEATURES[@]}" --strip-debug --strip-producers \
        "$work/einvoice_validator_bg.wasm" -o "$OUT/einvoice_validator_bg.wasm"
    cp "$work/einvoice_validator.js" "$work/einvoice_validator.d.ts" "$OUT/"
    cp "$CRATE/npm/package.json" "$CRATE/npm/README.md" "$CRATE/npm/node.mjs" "$CRATE/npm/node.d.mts" "$OUT/"
    # einvoice-validator ships the texts of every term of the module (its own licence
    # expression is the package's; its tests check the texts).
    cp crates/einvoice-validator/LICENSES/*.txt "$OUT/LICENSES/"
    # Every term of the licence expression has its text; MIT's is the repository's LICENSE.
    for term in $(jq -r .license "$OUT/package.json" | sed 's/ AND / /g'); do
        if [ ! -f "$OUT/LICENSES/$term.txt" ]; then
            echo "$OUT: LICENSES/$term.txt is missing (licence $(jq -r .license "$OUT/package.json"))" >&2
            status=1
        fi
    done
    cmp LICENSE "$OUT/LICENSES/MIT.txt" || status=1
    [ "$status" -eq 0 ] || return 1

    say "size"
    sizes "$work/einvoice_validator_bg.wasm"
    sizes "$OUT/einvoice_validator_bg.wasm"
    echo "package: $OUT"
}

# The browsers of the browser test, as kind=path.
browsers() {
    local chrome=${CHROME:-} firefox=${FIREFOX:-} name
    if [ -z "$chrome" ]; then
        for name in google-chrome chromium chromium-browser; do
            chrome=$(command -v "$name") && break
        done
    fi
    [ -z "$firefox" ] && firefox=$(command -v firefox || true)
    [ -n "$chrome" ] && echo "chrome=$chrome"
    [ -n "$firefox" ] && echo "firefox=$firefox"
    return 0
}

cmd_test() {
    local tmp tarball project repo=$PWD status=0 list=()
    [ -f "$OUT/package.json" ] || { echo "$OUT: not built (run $0 build)" >&2; return 1; }
    node --version
    npm --version
    tmp=$(mktemp -d)
    # shellcheck disable=SC2064 # the directory is known now
    trap "rm -rf '$tmp'" RETURN
    say "npm pack, npm install --offline in a throw-away project"
    tarball=$(cd "$OUT" && npm pack --silent --pack-destination "$tmp")
    project=$tmp/project
    mkdir "$project"
    echo '{ "private": true, "type": "module" }' >"$project/package.json"
    (cd "$project" && npm install --offline --no-audit --no-fund --ignore-scripts --silent "$tmp/$tarball")
    cp "$CRATE"/js-tests/*.mjs "$CRATE"/js-tests/*.ts "$project/"

    say "Node.js tests"
    (cd "$project" && EINVOICE_REPO=$repo node --test --test-reporter=spec api.test.mjs corpus.test.mjs) || status=1

    say "TypeScript declarations (typescript $TYPESCRIPT_VERSION, strict)"
    if ! (cd "$project" && npm exec --yes --package="typescript@$TYPESCRIPT_VERSION" -- tsc --version); then
        missing "TypeScript check (npm could not get typescript@$TYPESCRIPT_VERSION: no network access?)" || status=1
    else
        # As Node.js resolves the package (condition "node"), then as bundlers do.
        local resolution
        for resolution in nodenext:nodenext bundler:esnext; do
            echo "moduleResolution ${resolution%%:*}"
            (cd "$project" && npm exec --yes --package="typescript@$TYPESCRIPT_VERSION" -- tsc --noEmit \
                --strict --target es2022 --lib es2022,dom --types "" \
                --module "${resolution#*:}" --moduleResolution "${resolution%%:*}" types.test.ts) || status=1
        done
    fi

    say "browser test"
    mapfile -t list < <(browsers)
    if [ ${#list[@]} -eq 0 ]; then
        missing "browser test (no Chrome, Chromium or Firefox: set CHROME or FIREFOX)" || status=1
    else
        node "$project/browser.mjs" "$project/node_modules/einvoice-validator" "$repo" "${list[@]}" || status=1
    fi
    return "$status"
}

cmd_pack() {
    [ -f "$OUT/package.json" ] || { echo "$OUT: not built (run $0 build)" >&2; return 1; }
    (cd "$OUT" && npm pack --dry-run)
}

commands=("$@")
[ ${#commands[@]} -eq 0 ] && commands=(build test pack)
for c in "${commands[@]}"; do
    if ! declare -F "cmd_$c" >/dev/null; then
        echo "unknown command: $c (tools, build, test, pack)" >&2
        exit 2
    fi
    say "$c"
    "cmd_$c"
done
