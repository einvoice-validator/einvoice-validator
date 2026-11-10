#!/usr/bin/env bash
# The checks, run the same way locally and in CI (.github/workflows/ci.yml,
# oracle.yml and peppol.yml call the jobs below): formatting, lints, docs,
# tests, the crates without their default features, the WebAssembly build and
# examples, the npm package, the packages of the publishable crates, the
# dependency policy, the workflow linters, the upstream pins, the shell
# scripts and their tests, the reference oracles and, when something is
# staged, the privacy guard.
#
#   scripts/check.sh                # every job: fmt clippy doc test features wasm npm package deny actionlint zizmor pins guards oracle peppol facturx xrechnung fr-ctc rules privacy
#   scripts/check.sh rules-online   # on demand: every fetched rule set from its publisher
#   scripts/check.sh corpora rules  # on demand: the four fetched corpora, then the rules job
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
# them it prints SKIPPED, except in CI, where it fails. So do the facturx,
# xrechnung and fr-ctc jobs (the Factur-X, XRechnung and French corpora, built
# in target/ by their fetch scripts and cross-checked); they need xmllint too
# (the XML Schema step of their acceptance tables). The npm job needs the
# wasm target, Node.js and npm, and the tools that crates/einvoice-wasm/npm.sh
# tools installs (wasm-bindgen-cli, wasm-opt); without them it prints
# SKIPPED, except in CI, where it fails (npm.sh itself skips a missing
# browser or TypeScript locally and fails in CI).
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
    # Some tests read every package of Cargo.lock offline (cargo metadata and
    # cargo tree on all targets: the licence and supply-chain checks); a
    # build only downloads the host's: fetch them all first.
    need_toolchain &&
        cargo fetch --locked &&
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

# The library, the PDF reader and the streaming XML parser build for
# WebAssembly (wasm32-unknown-unknown); then the wasm-smoke example of
# einvoice-xpath-eval (parse and evaluate), the wasm-validate example of
# einvoice-validator (the built-in rule set on two embedded corpus documents),
# the wasm-extract example of einvoice-pdf (two PDF files built in memory) and
# the wasm-smoke example of einvoice-xml (well-formed and malformed documents)
# are built in release mode for that target, must import nothing, and must
# return 0 under wasmtime (WASMTIME, default wasmtime on PATH). Without
# wasmtime, the runs are SKIPPED, except in CI, where it fails (CI installs a
# pinned release).
job_wasm() {
    need_toolchain || return 1
    local target=wasm32-unknown-unknown
    if ! rustup target list --installed --toolchain "$(toolchain_channel)" | grep -qx "$target"; then
        missing "the $target target is not installed (rustup target add $target)"
        return
    fi
    cargo build --locked --target "$target" -p einvoice-validator -p einvoice-pdf -p einvoice-xml || return 1
    local status=0 package example wasm imports result
    local wasmtime=${WASMTIME:-wasmtime}
    if ! command -v "$wasmtime" >/dev/null 2>&1; then
        missing "the wasm examples under wasmtime (wasmtime is not installed)" || status=1
        wasmtime=
    else
        "$wasmtime" --version
    fi
    # One at a time: two examples share the name wasm-smoke, and so the file.
    for package in einvoice-xpath-eval:wasm-smoke einvoice-validator:wasm-validate \
        einvoice-pdf:wasm-extract einvoice-xml:wasm-smoke; do
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
# The npm package einvoice-validator (crates/einvoice-wasm/README.md):
# crates/einvoice-wasm/npm.sh builds it (opt-level 3, LTO, wasm-bindgen,
# wasm-opt), runs its Node.js tests (the API, the CEN UBL and CII corpora
# against the Saxon-HE goldens), the TypeScript check and the browser test
# (headless Chrome and Firefox) on the packed package, then npm pack
# --dry-run. Needs the wasm target, Node.js, npm and jq, and wasm-bindgen-cli
# and wasm-opt at their pinned versions (crates/einvoice-wasm/npm.sh tools
# installs them in EINVOICE_WASM_TOOLS; WASM_BINDGEN and WASM_OPT name others);
# without them SKIPPED, except in CI, where it fails.
job_npm() {
    need_toolchain || return 1
    local target=wasm32-unknown-unknown program
    local tools=${EINVOICE_WASM_TOOLS:-${XDG_CACHE_HOME:-$HOME/.cache}/einvoice-validator/wasm-tools}
    if ! rustup target list --installed --toolchain "$(toolchain_channel)" | grep -qx "$target"; then
        missing "the npm package (the $target target is not installed: rustup target add $target)"
        return
    fi
    for program in node npm jq; do
        if ! command -v "$program" >/dev/null 2>&1; then
            missing "the npm package ($program is not installed)"
            return
        fi
    done
    for program in wasm-bindgen:"${WASM_BINDGEN:-}" wasm-opt:"${WASM_OPT:-}"; do
        if [ -z "${program#*:}" ] && [ ! -x "$tools/bin/${program%%:*}" ] &&
            ! command -v "${program%%:*}" >/dev/null 2>&1; then
            missing "the npm package (${program%%:*} is not installed: crates/einvoice-wasm/npm.sh tools)"
            return
        fi
    done
    crates/einvoice-wasm/npm.sh build test pack
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
# The dependency policy: first the packages of the distributed graphs themselves (no C,
# C++ or assembly file, no build script or build dependency that builds native code,
# every crate, build script and use of inline assembly audited in docs/dependencies.md:
# tests/supply_chain.rs of einvoice-testkit), then cargo-deny (licences, advisories,
# sources, and the bans of deny.toml, which refuse the crates that build native code).
job_deny() {
    need_toolchain || return 1
    # The test reads every package offline: fetch them first (all targets).
    cargo fetch --locked || return 1
    cargo test --locked -p einvoice-testkit --test supply_chain || return 1
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
# The shell scripts (scripts/, and those of the crates: crates/einvoice-wasm/npm.sh)
# and git hooks: ShellCheck, then the tests of the privacy
# guard and of the release scripts (synthetic data, throw-away repositories),
# which need jq.
job_guards() {
    local status=0
    if command -v shellcheck >/dev/null 2>&1; then
        shellcheck --version | sed -n 2p
        shellcheck scripts/*.sh crates/*/*.sh .githooks/* && echo "shellcheck: scripts and hooks clean" || status=1
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
# stylesheet probes of einvoice-xslt-exec: the goldens must not change; and
# Saxon-HE on the hand-written cases of einvoice-xml (its saxon-check example:
# the same verdict and data, but for the differences it lists with their
# reasons); then, with both, the encodings other than UTF-8: Saxon-HE and
# xmllint on the encoding probes of einvoice-xml (its encoding-oracle example:
# their results must not change, and the decoder must agree with them but for
# the differences it lists; the results are those of the runner of oracle.yml,
# and outside that environment, those of a reference whose environment differs
# are listed as differences of the environment) and on the corpora re-encoded
# (the encoding-oracle example of einvoice-validator: the results of the
# documents in UTF-8).
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
    cargo run --locked -p einvoice-xslt-exec --example saxon-probes || return 1
    cargo run --locked -p einvoice-xml --example saxon-check || return 1
    if command -v "$xmllint" >/dev/null 2>&1; then
        XMLLINT=$xmllint cargo run --locked -p einvoice-xml --example encoding-oracle || return 1
        XMLLINT=$xmllint cargo run --locked --release -p einvoice-validator --example encoding-oracle
    else
        missing "encoding oracles (xmllint is not installed)"
    fi
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
# verdicts and findings (tests/peppol_bundle.rs of einvoice-cli), and the
# Schematron compiled in Rust must give ph-schematron's result at each step and
# the pinned stylesheets (tests/peppol.rs of einvoice-schematron, after its
# ph-schematron-oracle example, which also checks the goldens of its synthetic
# schemas against ph-schematron); these tests are required to run here.
# Without Java or without network access (and nothing cached yet), SKIPPED,
# except in CI, where it fails.
job_peppol() {
    local status=0
    EINVOICE_PEPPOL_TOOLS=1 build_corpus peppol scripts/fetch-peppol.sh || status=$?
    [ "$status" -ne 2 ] || return 0
    [ "$status" -eq 0 ] || return 1
    EINVOICE_PEPPOL_REQUIRED=1 cargo test --locked --all-features -p einvoice-xslt-exec \
        --test peppol_equivalence --test mutation_equivalence -- --nocapture || return 1
    EINVOICE_PEPPOL_REQUIRED=1 cargo test --locked --all-features -p einvoice-cli \
        --test peppol_bundle -- --nocapture || return 1
    # The Schematron compiled in Rust as ph-schematron compiles it: ph-schematron's
    # result of each step (its oracle, with the Maven of fetch-peppol.sh) must be the
    # goldens of the synthetic schemas of einvoice-schematron, and is written for the
    # Peppol ones; then the same steps and the pinned stylesheets from
    # einvoice-schematron.
    cargo run --locked --release -p einvoice-schematron --example ph-schematron-oracle &&
        cargo run --locked --release -p einvoice-schematron --example ph-schematron-oracle -- --peppol &&
        EINVOICE_SCHEMATRON_PEPPOL_REQUIRED=1 cargo test --locked --release -p einvoice-schematron \
            --test peppol -- --nocapture
}
# build_corpus NAME SCRIPT: the corpus of a family of fetched rule sets in target/:
# Saxon-HE (scripts/fetch-saxon.sh, unless SAXON_JAR names it), the fetch script
# SCRIPT (the pinned inputs, checksums verified), then `cargo xtask NAME` (the cases,
# the Saxon-HE goldens, the acceptance table, the cross-checks against the committed
# records). Status 0 when built, 1 on a failure, 2 when SKIPPED: without Java, or
# when a download fails (no network access, and nothing cached yet), which in CI is
# a failure (status 1).
build_corpus() {
    local name=$1 script=$2 status=0
    need_toolchain || return 1
    # skipped REASON: SKIPPED locally (status 2), a failure in CI (status 1).
    skipped() {
        if [ -n "${CI:-}" ]; then
            echo "$1: the $name corpus cannot be built" >&2
            return 1
        fi
        echo "SKIPPED: $name corpus ($1)"
        return 2
    }
    if ! command -v "${JAVA:-java}" >/dev/null 2>&1; then
        skipped "java is not installed"
        return
    fi
    # The acceptance tables run the XML Schema step of the reference validators
    # with xmllint (cargo xtask reads $XMLLINT, else xmllint): the schemas of the
    # KoSIT scenarios (XRechnung), of the Factur-X profiles and of the French
    # package.
    if [ "$name" != peppol ] && ! command -v "${XMLLINT:-xmllint}" >/dev/null 2>&1; then
        skipped "xmllint is not installed"
        return
    fi
    "${JAVA:-java}" -version 2>&1 | head -n 1
    if [ -n "${SAXON_JAR:-}" ]; then
        echo "using SAXON_JAR=$SAXON_JAR (checksum verified by xtask)"
    else
        scripts/fetch-saxon.sh || status=$?
    fi
    [ "$status" -eq 0 ] && { "$script" || status=$?; }
    if [ "$status" -eq 75 ]; then
        skipped "a download failed: no network access?"
        return
    fi
    [ "$status" -eq 0 ] || return 1
    cargo xtask "$name" || return 1
}
# A family of fetched rule sets (corpus/README.md): Factur-X, XRechnung or the
# French rules, whose files are not committed (see the README of each
# corpus/<rule set>/). The fetch script downloads the pinned inputs into
# target/ (checksums verified), then `cargo xtask <family>` extracts the
# cases, runs Saxon-HE with every stylesheet, writes the acceptance table
# (with the XML Schema step of the reference validator, run with xmllint: the
# schemas of the KoSIT scenarios for XRechnung, of the Factur-X profiles, and
# those of the French package) and cross-checks the goldens with the
# expectations (XML Mutate,
# accepted documents) against the committed records (build_corpus), then the
# equivalence test of the engine on the corpus (tests/<family>_equivalence.rs
# of einvoice-xslt-exec, required), then xmllint (libxml2) and einvoice-xsd on
# every case of the family's CII corpus against the CII D16B schemas
# (fetched_cii_corpora_agree_with_xmllint in tests/oracle.rs of einvoice-xsd,
# required: every difference must be explained).
# Without Java or without network access (and nothing cached yet), or without
# xmllint, SKIPPED, except in CI, where it fails; so is the xmllint run without
# xmllint.
#   fetched_family NAME SCRIPT CII_CORPUS
fetched_family() {
    local name=$1 script=$2 cii=$3 status=0
    build_corpus "$name" "$script" || status=$?
    [ "$status" -ne 2 ] || return 0
    [ "$status" -eq 0 ] || return 1
    # The engine on the corpus: every stylesheet, every case (a release build: some
    # ten thousand transformations).
    local test=${name//-/_} variable
    variable=EINVOICE_$(echo "$test" | tr '[:lower:]' '[:upper:]')_REQUIRED
    env "$variable=1" cargo test --locked --release -p einvoice-xslt-exec \
        --test "${test}_equivalence" -- --nocapture || return 1
    local xmllint=${XMLLINT:-xmllint}
    if command -v "$xmllint" >/dev/null 2>&1; then
        XMLLINT=$xmllint EINVOICE_XSD_FETCHED_REQUIRED=$cii cargo test --locked --release \
            -p einvoice-xsd --test oracle fetched_cii_corpora_agree_with_xmllint -- --nocapture
    else
        missing "xmllint on the $name CII corpus (xmllint is not installed)"
    fi
}
job_facturx() { fetched_family facturx scripts/fetch-facturx.sh factur-x-1.09.2; }
job_xrechnung() { fetched_family xrechnung scripts/fetch-xrechnung.sh xrechnung-cii-3.0.2; }
job_fr-ctc() { fetched_family fr-ctc scripts/fetch-fr-ctc.sh fr-ctc-cii-1.4.0.04; }
# The rule sets fetched on demand (`einvoice-validate rules`, the feature `fetch` of
# einvoice-validator): the pins embedded in the crate are the fetch scripts' pin blocks
# and the URLs the scripts download; the bundles `rules fetch` assembles from the
# scripts' downloads are, byte for byte, the bundles the scripts wrote
# (tests/rulesets.rs of einvoice-validator); the rules command works end to end against
# a local mirror (tests/rules.rs of einvoice-cli); and every corpus of the facturx,
# xrechnung, fr-ctc and peppol jobs, validated with `einvoice-validate --rules NAME`
# from the cache target/rules-cache/, gives the Saxon-HE results and the verdicts of
# the acceptance tables (tests/rules_equivalence.rs, a release build; the Peppol bundle
# is compiled there in Rust, from the downloads of scripts/fetch-peppol.sh). Runs after those
# jobs, or after the corpora job, on their outputs in target/: without them the tests
# say SKIPPED, except in CI, where they are required.
job_rules() {
    need_toolchain || return 1
    local required=()
    [ -z "${CI:-}" ] || required=(EINVOICE_RULES_REQUIRED=1)
    env ${required[@]+"${required[@]}"} cargo test --locked --all-features -p einvoice-validator \
        --test rulesets -- --nocapture &&
        env ${required[@]+"${required[@]}"} cargo test --locked --release --all-features -p einvoice-cli \
            --test rules --test rules_equivalence -- --nocapture
}
# On demand only (not part of a full run, where the peppol, facturx, xrechnung and
# fr-ctc jobs build them and test the engine on them too): the corpora of the four
# families of fetched rule sets, built as those jobs build them (build_corpus), for
# the rules job: `scripts/check.sh corpora rules` (CI's rulesets workflow).
job_corpora() {
    local status=0 family code
    for family in peppol:scripts/fetch-peppol.sh facturx:scripts/fetch-facturx.sh \
        xrechnung:scripts/fetch-xrechnung.sh fr-ctc:scripts/fetch-fr-ctc.sh; do
        code=0
        build_corpus "${family%%:*}" "${family#*:}" || code=$?
        [ "$code" -eq 0 ] || [ "$code" -eq 2 ] || status=1
    done
    return "$status"
}
# On demand only (not part of a full run, it downloads): fetches every rule set from
# its publisher, for real, into target/rules-cache/, and compares each bundle with the
# one its fetch script wrote in target/ (run the facturx, xrechnung, fr-ctc and peppol
# jobs first). EINVOICE_FETCH_FROM may name directories (':'-separated) holding
# published files to use instead of downloading them. Then a TLS handshake with each
# publisher's host through the client's cryptography provider, which prints what was
# negotiated (EINVOICE_TLS_PROBE_HOSTS may name other hosts, ','-separated).
job_rules-online() {
    need_toolchain || return 1
    cargo test --locked --all-features -p einvoice-validator --test rulesets -- \
        --ignored --nocapture &&
        cargo test --locked --all-features -p einvoice-validator --lib \
            rulesets::fetch::tls::tests::publishers_hosts_handshake -- --ignored --nocapture
}
# The pre-commit guard on the index; nothing staged, nothing to check.
job_privacy() {
    if git diff --cached --quiet; then
        echo "nothing staged: privacy guard not run"
        return 0
    fi
    scripts/privacy-guard.sh staged && echo "privacy guard: staged content clean"
}

ALL=(fmt clippy doc test features wasm npm package deny actionlint zizmor pins guards oracle peppol facturx xrechnung fr-ctc rules privacy)

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
