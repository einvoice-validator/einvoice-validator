#!/usr/bin/env bash
# The checks, run the same way locally and in CI (.github/workflows/ci.yml,
# oracle.yml and peppol.yml call the jobs below): formatting, lints, docs,
# tests, the crates without their default features, the WebAssembly build and
# examples, the packages of the publishable crates, the dependency policy, the workflow linters, the upstream pins, the shell
# scripts and their tests, the reference oracles and, when something is
# staged, the privacy guard.
#
#   scripts/check.sh                # every job: fmt clippy doc test features wasm package deny actionlint zizmor pins guards oracle peppol privacy
#   scripts/check.sh clippy test    # selected jobs
#   scripts/check.sh toolchain      # only install the pinned toolchain
#
# The Rust jobs use the toolchain pinned in rust-toolchain.toml (installed
# with rustup when missing), so the lint set only moves with a reviewed bump.
# The wasm job needs the wasm32-unknown-unknown target of that toolchain
# (rustup target add wasm32-unknown-unknown) and wasmtime to run the wasm
# examples. cargo-deny, actionlint, zizmor, wasmtime, jq (pins) and ShellCheck
# (guards) are not part of the toolchain: each runs when installed and prints
# SKIPPED otherwise. In CI (CI set) a missing wasm target, wasmtime, jq,
# ShellCheck, xmllint or Java is a failure, not a skip; CI installs cargo-deny,
# actionlint, zizmor and wasmtime at pinned versions, so nothing is skipped
# there. The oracle job needs xmllint (libxml2) for the oracle of
# einvoice-xsd, and Java (any recent JRE) and network access the first time,
# to fetch Saxon-HE; locally, without them, it checks what it can and skips
# the rest. The peppol job needs Java and network access the first time too
# (Saxon-HE, the Peppol release, Maven and the Schematron compiler); without
# them it prints SKIPPED, except in CI, where it fails.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
# No warning is tolerated. The workspace lints already deny every warning
# (Cargo.toml [workspace.lints]); the flags make sure nothing can relax that.
export RUSTFLAGS="-D warnings" RUSTDOCFLAGS="-D warnings"

say() { printf '\n\033[1m== %s\033[0m\n' "$*"; }

# missing WHAT: a prerequisite WHAT is missing. SKIPPED locally (status 0), a
# failure in CI (status 1).
missing() {
    if [ -n "${CI:-}" ]; then
        echo "$1: required in CI" >&2
        return 1
    fi
    echo "SKIPPED: $1"
}

toolchain_channel() { sed -n 's/^channel *= *"\([^"]*\)".*/\1/p' rust-toolchain.toml; }

# The toolchain pinned in rust-toolchain.toml, installed if missing.
need_toolchain() {
    local channel
    channel=$(toolchain_channel)
    [ -n "$channel" ] || { echo "no channel in rust-toolchain.toml" >&2; return 1; }
    if ! rustup toolchain list | grep -q "^$channel-"; then
        rustup toolchain install "$channel" --profile minimal --component rustfmt,clippy --no-self-update
    fi
    rustup component add --toolchain "$channel" rustfmt clippy >/dev/null 2>&1
    rustc --version
    cargo --version
}

job_toolchain() { need_toolchain; }
job_fmt() { need_toolchain && cargo fmt --all --check; }
job_clippy() { need_toolchain && cargo clippy --workspace --all-targets --all-features --locked -- -D warnings; }
job_doc() { need_toolchain && cargo doc --workspace --no-deps --all-features --locked; }
# Unit and integration tests, then the doctests (--all-targets leaves them out).
job_test() {
    need_toolchain &&
        cargo test --workspace --all-targets --all-features --locked &&
        cargo test --workspace --doc --all-features --locked
}
# The library without its default features: no built-in rule set, bundles only.
job_features() {
    need_toolchain &&
        cargo clippy -p einvoice-validator --no-default-features --all-targets --locked -- -D warnings &&
        cargo test -p einvoice-validator --no-default-features --locked
}
# wasm_leb FILE OFFSET: the unsigned LEB128 number at byte OFFSET of FILE, and
# its length in bytes.
wasm_leb() {
    local value=0 shift=0 length=0 byte
    for byte in $(od -An -v -tu1 -j "$2" -N 5 "$1"); do
        value=$((value | (byte & 127) << shift))
        length=$((length + 1))
        shift=$((shift + 7))
        [ "$byte" -lt 128 ] && break
    done
    echo "$value $length"
}

# wasm_imports FILE: the number of imports of the WebAssembly module FILE (the
# count of its import section, 0 without one).
wasm_imports() {
    local file=$1 size offset=8 id length n count _
    if [ "$(od -An -tx1 -N 8 "$file" | tr -d ' \n')" != 0061736d01000000 ]; then
        echo "$file: not a WebAssembly 1.0 module" >&2
        return 1
    fi
    size=$(wc -c <"$file")
    while [ "$offset" -lt "$size" ]; do
        id=$(od -An -tu1 -j "$offset" -N 1 "$file" | tr -d ' ')
        read -r length n <<<"$(wasm_leb "$file" $((offset + 1)))"
        if [ "$id" -eq 2 ]; then
            read -r count _ <<<"$(wasm_leb "$file" $((offset + 1 + n)))"
            echo "$count"
            return
        fi
        offset=$((offset + 1 + n + length))
    done
    echo 0
}

# The library builds for WebAssembly (wasm32-unknown-unknown); then the
# wasm-smoke example of einvoice-xpath-eval (parse and evaluate) and the
# wasm-validate example of einvoice-validator (the built-in rule set on two
# embedded corpus documents) are built in release mode for that target, must
# import nothing, and must return 0 under wasmtime (WASMTIME, default wasmtime
# on PATH). Without wasmtime, the runs are SKIPPED, except in CI, where it
# fails (CI installs a pinned release).
job_wasm() {
    need_toolchain || return 1
    local target=wasm32-unknown-unknown
    if ! rustup target list --installed --toolchain "$(toolchain_channel)" | grep -qx "$target"; then
        missing "the $target target is not installed (rustup target add $target)"
        return
    fi
    cargo build --locked --target "$target" -p einvoice-validator || return 1
    local status=0 package example wasm imports result
    local wasmtime=${WASMTIME:-wasmtime}
    if ! command -v "$wasmtime" >/dev/null 2>&1; then
        missing "the wasm examples under wasmtime (wasmtime is not installed)" || status=1
        wasmtime=
    else
        "$wasmtime" --version
    fi
    for package in einvoice-xpath-eval:wasm-smoke einvoice-validator:wasm-validate; do
        example=${package#*:}
        package=${package%%:*}
        cargo build --locked --release --target "$target" -p "$package" --example "$example" || return 1
        wasm=target/$target/release/examples/$example.wasm
        imports=$(wasm_imports "$wasm") || return 1
        echo "$package $example: $wasm, $(wc -c <"$wasm") bytes, $imports imports"
        if [ "$imports" != 0 ]; then
            echo "$wasm: imports $imports functions or objects; it must import nothing" >&2
            status=1
        fi
        [ -n "$wasmtime" ] || continue
        # The example's main(argc, argv) returns 0 when every check passes.
        result=$("$wasmtime" run --invoke main "$wasm" 0 0 2>/dev/null) || {
            echo "$wasm: wasmtime failed" >&2
            status=1
            continue
        }
        if [ "$result" = 0 ]; then
            echo "$package $example: 0 under wasmtime"
        else
            echo "$wasm: returned ${result:-nothing} under wasmtime (the first failing check)" >&2
            status=1
        fi
    done
    return "$status"
}
# The publishable crates (every member whose publish is not false), packaged
# as crates.io would receive them, then each built from its own package
# (cargo package, with the workspace's packages standing in for crates.io);
# every term of each crate's licence expression has its text,
# LICENSES/<term>.txt, in the package, LICENSES/MIT.txt being the repository's
# LICENSE; and each has the metadata crates.io shows: a description, its
# README (in the package), one to five keywords and one to five categories.
# Cargo takes registry packages to be immutable: it reuses the copies of the
# workspace crates it unpacks for the verification (CARGO_HOME/registry/src/
# -<hash>/<name>-<version>, the local registry standing in for crates.io) and
# their builds by name and version alone, so a crate changed without a version
# change would be verified from a stale copy. Each run therefore removes those
# copies and verifies in its own, fresh, target directory (target/package-check).
# Needs jq.
job_package() {
    need_toolchain || return 1
    if ! command -v jq >/dev/null 2>&1; then
        missing "package (jq is not installed)"
        return
    fi
    local status=0 name version dir license description readme keywords categories term list packages=()
    local cargo_home=${CARGO_HOME:-$HOME/.cargo}
    while IFS=$'\t' read -r name version dir license description readme keywords categories; do
        packages+=(-p "$name")
        # Only the local registry's copies: its source directory has an empty name.
        rm -rf "$cargo_home"/registry/src/-*/"$name-$version"
        list=$(cargo package --locked --allow-dirty --no-verify --list -p "$name") || {
            status=1
            continue
        }
        for term in ${license// AND / }; do
            if [[ ! $term =~ ^[A-Za-z0-9.+-]+$ ]]; then
                echo "$name: licence expression \"$license\": only terms joined by AND are checked" >&2
                status=1
            elif ! grep -qxF "LICENSES/$term.txt" <<<"$list"; then
                echo "$name: LICENSES/$term.txt is not in the package (licence $license)" >&2
                status=1
            elif [ "$term" = MIT ] && ! cmp -s LICENSE "$dir/LICENSES/MIT.txt"; then
                echo "$name: LICENSES/MIT.txt differs from LICENSE" >&2
                status=1
            fi
        done
        if [ "$description" -eq 0 ]; then
            echo "$name: no description" >&2
            status=1
        fi
        if [ "$readme" = - ] || ! grep -qxF "$readme" <<<"$list"; then
            echo "$name: no readme in the package (readme = \"$readme\")" >&2
            status=1
        fi
        if [ "$keywords" -lt 1 ] || [ "$keywords" -gt 5 ] || [ "$categories" -lt 1 ] || [ "$categories" -gt 5 ]; then
            echo "$name: $keywords keywords and $categories categories (1 to 5 of each)" >&2
            status=1
        fi
        echo "$name: $license, $readme, $keywords keywords, $categories categories, $(wc -l <<<"$list") files"
    done < <(cargo metadata --locked --no-deps --format-version 1 |
        jq -r '.packages[] | select(.publish != []) | [.name, .version, (.manifest_path | rtrimstr("/Cargo.toml")),
            .license, (.description // "" | length), (.readme // "-"), (.keywords | length), (.categories | length)] | @tsv')
    [ ${#packages[@]} -gt 0 ] || { echo "no publishable crate" >&2; return 1; }
    rm -rf target/package-check
    cargo package --locked --allow-dirty --target-dir target/package-check "${packages[@]}" || status=1
    return "$status"
}
job_deny() {
    if ! cargo deny --version >/dev/null 2>&1; then
        echo "SKIPPED: cargo-deny is not installed (cargo install --locked cargo-deny)"
        return 0
    fi
    cargo deny --all-features check
}
# GitHub Actions workflows: syntax, expressions and embedded shell scripts
# (checked with ShellCheck when it is installed).
job_actionlint() {
    if ! command -v actionlint >/dev/null 2>&1; then
        echo "SKIPPED: actionlint is not installed (https://github.com/rhysd/actionlint)"
        return 0
    fi
    actionlint -version | head -n 1
    actionlint
}
# GitHub Actions security audit, pedantic persona, as in CI. The online audits
# need a GitHub token: GH_TOKEN, else the gh CLI's; without one, offline only.
job_zizmor() {
    if ! command -v zizmor >/dev/null 2>&1; then
        echo "SKIPPED: zizmor is not installed (cargo install --locked zizmor)"
        return 0
    fi
    zizmor --version
    local token=${GH_TOKEN:-${GITHUB_TOKEN:-}}
    if [ -z "$token" ] && command -v gh >/dev/null 2>&1; then
        token=$(gh auth token 2>/dev/null || true)
    fi
    if [ -n "$token" ]; then
        GH_TOKEN=$token zizmor --persona=pedantic .github/workflows .github/dependabot.yml
    else
        echo "no GitHub token: online audits not run"
        zizmor --persona=pedantic --offline .github/workflows .github/dependabot.yml
    fi
}
# Every pin of upstream.toml matches the files that record it (offline), and
# the tests of scripts/freshness.sh. Needs jq.
job_pins() {
    if ! command -v jq >/dev/null 2>&1; then
        missing "pins (jq is not installed)"
        return
    fi
    scripts/freshness.sh pins && scripts/freshness-test.sh
}
# The shell scripts and git hooks: ShellCheck, then the tests of the privacy
# guard and of the release scripts (synthetic data, throw-away repositories),
# which need jq.
job_guards() {
    local status=0
    if command -v shellcheck >/dev/null 2>&1; then
        shellcheck --version | sed -n 2p
        shellcheck scripts/*.sh .githooks/* && echo "shellcheck: scripts and hooks clean" || status=1
    else
        missing "ShellCheck (shellcheck is not installed)" || status=1
    fi
    if command -v jq >/dev/null 2>&1; then
        scripts/test-privacy-guard.sh || status=1
        scripts/test-release.sh || status=1
    else
        missing "tests of the privacy guard and release scripts (jq is not installed)" || status=1
    fi
    return "$status"
}
# The test corpus and its reference oracles (corpus/README.md): the vendored
# artefacts against their SHA256SUMS, the extracted corpus against the vendored
# test sets, the goldens against the VEFA expectations; then xmllint (libxml2)
# on every case of the oracle of einvoice-xsd (its results must not change,
# every disagreement must be explained); then Saxon-HE (fetched from Maven
# Central, checksum verified) re-run on every case, on the atomic value probes
# of einvoice-xdm, on the expression probes of einvoice-xpath-eval and on the
# stylesheet probes of einvoice-xslt-exec: the goldens must not change.
# Without xmllint or Java, that run is SKIPPED, except in CI, where it fails.
job_oracle() {
    need_toolchain || return 1
    cargo xtask manifest || return 1
    cargo xtask corpus || return 1
    cargo xtask crosscheck || return 1
    local xmllint=${XMLLINT:-xmllint}
    if command -v "$xmllint" >/dev/null 2>&1; then
        XMLLINT=$xmllint cargo run --locked --release -p einvoice-xsd --example xmllint-oracle || return 1
    else
        missing "xmllint oracle of einvoice-xsd (xmllint is not installed)" || return 1
    fi
    if ! command -v "${JAVA:-java}" >/dev/null 2>&1; then
        missing "Saxon-HE run (java is not installed)"
        return
    fi
    "${JAVA:-java}" -version 2>&1 | head -n 1
    if [ -n "${SAXON_JAR:-}" ]; then
        echo "using SAXON_JAR=$SAXON_JAR (checksum verified by the oracle)"
    else
        scripts/fetch-saxon.sh || return 1
    fi
    cargo xtask oracle || return 1
    cargo run --locked -p einvoice-xdm --example saxon-oracle || return 1
    cargo run --locked -p einvoice-xpath-eval --example saxon-oracle || return 1
    cargo run --locked -p einvoice-xslt-exec --example saxon-probes
}
# The Peppol BIS Billing 3.0 corpus (corpus/README.md), which is not committed
# because the Peppol files carry no licence: scripts/fetch-peppol.sh downloads
# the pinned release and compiles its two stylesheets into target/peppol/
# (checksums verified), `cargo xtask peppol` extracts the cases, runs Saxon-HE
# with both stylesheets and cross-checks the goldens with the VEFA
# expectations, then the engine must give the Saxon-HE result on every case
# (tests/peppol_equivalence.rs of einvoice-xslt-exec, and
# tests/mutation_equivalence.rs for the mutation cases), and einvoice-validate
# with the bundle that scripts/fetch-peppol.sh writes must give the Saxon-HE
# verdicts and findings (tests/peppol_bundle.rs of einvoice-cli); these tests
# are required to run here.
# Without Java or without network access (and nothing cached yet), SKIPPED,
# except in CI, where it fails.
job_peppol() {
    need_toolchain || return 1
    # skipped REASON: SKIPPED locally, a failure in CI.
    skipped() {
        if [ -n "${CI:-}" ]; then
            echo "$1: the Peppol corpus cannot be built" >&2
            return 1
        fi
        echo "SKIPPED: Peppol corpus and equivalence ($1)"
    }
    if ! command -v "${JAVA:-java}" >/dev/null 2>&1; then
        skipped "java is not installed"
        return
    fi
    "${JAVA:-java}" -version 2>&1 | head -n 1
    local status=0
    if [ -n "${SAXON_JAR:-}" ]; then
        echo "using SAXON_JAR=$SAXON_JAR (checksum verified by xtask)"
    else
        scripts/fetch-saxon.sh || status=$?
    fi
    [ "$status" -eq 0 ] && { scripts/fetch-peppol.sh || status=$?; }
    if [ "$status" -eq 75 ]; then
        skipped "a download failed: no network access?"
        return
    fi
    [ "$status" -eq 0 ] || return 1
    cargo xtask peppol || return 1
    EINVOICE_PEPPOL_REQUIRED=1 cargo test --locked --all-features -p einvoice-xslt-exec \
        --test peppol_equivalence --test mutation_equivalence -- --nocapture || return 1
    EINVOICE_PEPPOL_REQUIRED=1 cargo test --locked --all-features -p einvoice-cli \
        --test peppol_bundle -- --nocapture
}
# The pre-commit guard on the index; nothing staged, nothing to check.
job_privacy() {
    if git diff --cached --quiet; then
        echo "nothing staged: privacy guard not run"
        return 0
    fi
    scripts/privacy-guard.sh staged && echo "privacy guard: staged content clean"
}

ALL=(fmt clippy doc test features wasm package deny actionlint zizmor pins guards oracle peppol privacy)

jobs=("$@")
[ ${#jobs[@]} -eq 0 ] && jobs=("${ALL[@]}")

failed=()
for j in "${jobs[@]}"; do
    if ! declare -F "job_$j" >/dev/null; then
        echo "unknown job: $j" >&2
        exit 2
    fi
    say "$j"
    if ! "job_$j"; then failed+=("$j"); fi
done

if [ ${#failed[@]} -gt 0 ]; then
    printf '\n\033[31mFAILED: %s\033[0m\n' "${failed[*]}"
    exit 1
fi
printf '\n\033[32mOK: %s\033[0m\n' "${jobs[*]}"
