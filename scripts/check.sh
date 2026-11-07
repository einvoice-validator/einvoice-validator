#!/usr/bin/env bash
# The checks, run the same way locally and in CI (.github/workflows/ci.yml
# calls the jobs below): formatting, lints, docs, tests, the dependency
# policy, the workflow linters and, when something is staged, the privacy
# guard.
#
#   scripts/check.sh                # every job: fmt clippy doc test deny actionlint zizmor oracle privacy
#   scripts/check.sh clippy test    # selected jobs
#   scripts/check.sh toolchain      # only install the pinned toolchain
#
# The Rust jobs use the toolchain pinned in rust-toolchain.toml (installed
# with rustup when missing), so the lint set only moves with a reviewed bump.
# cargo-deny, actionlint and zizmor are not part of the toolchain: each job
# runs when its tool is installed and prints SKIPPED otherwise. CI installs
# all three at pinned versions, so nothing is skipped there. The oracle job
# needs Java (any recent JRE) and network access the first time, to fetch
# Saxon-HE; without Java it checks what it can and skips the Saxon run.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
# No warning is tolerated. The workspace lints already deny every warning
# (Cargo.toml [workspace.lints]); the flags make sure nothing can relax that.
export RUSTFLAGS="-D warnings" RUSTDOCFLAGS="-D warnings"

say() { printf '\n\033[1m== %s\033[0m\n' "$*"; }

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
        GH_TOKEN=$token zizmor --persona=pedantic .github/workflows
    else
        echo "no GitHub token: online audits not run"
        zizmor --persona=pedantic --offline .github/workflows
    fi
}
# The test corpus and its reference oracle (corpus/README.md): the vendored
# artefacts against their SHA256SUMS, the extracted corpus against the vendored
# test sets, the goldens against the VEFA expectations, then Saxon-HE (fetched
# from Maven Central, checksum verified) re-run on every case: the goldens must
# not change. Without Java the Saxon run is SKIPPED, except in CI, where it fails.
job_oracle() {
    need_toolchain || return 1
    cargo xtask manifest || return 1
    cargo xtask corpus || return 1
    cargo xtask crosscheck || return 1
    if ! command -v "${JAVA:-java}" >/dev/null 2>&1; then
        if [ -n "${CI:-}" ]; then
            echo "java is not installed: the oracle cannot run" >&2
            return 1
        fi
        echo "SKIPPED: Saxon-HE run (java is not installed); manifest, corpus and cross-check done"
        return 0
    fi
    "${JAVA:-java}" -version 2>&1 | head -n 1
    if [ -n "${SAXON_JAR:-}" ]; then
        echo "using SAXON_JAR=$SAXON_JAR (checksum verified by the oracle)"
    else
        scripts/fetch-saxon.sh || return 1
    fi
    cargo xtask oracle
}
# The pre-commit guard on the index; nothing staged, nothing to check.
job_privacy() {
    if git diff --cached --quiet; then
        echo "nothing staged: privacy guard not run"
        return 0
    fi
    scripts/privacy-guard.sh staged && echo "privacy guard: staged content clean"
}

ALL=(fmt clippy doc test deny actionlint zizmor oracle privacy)

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
