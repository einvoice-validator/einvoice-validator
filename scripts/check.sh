#!/usr/bin/env bash
# The local checks: formatting, lints, tests, the dependency policy and, when
# something is staged, the privacy guard.
#
#   scripts/check.sh                # every job: fmt clippy test deny privacy
#   scripts/check.sh clippy test    # selected jobs
#
# The Rust jobs use the toolchain pinned in rust-toolchain.toml (installed
# with rustup when missing), so the lint set only moves with a reviewed bump.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
# No warning is tolerated. The workspace lints already deny every warning
# (Cargo.toml [workspace.lints]); the flags make sure nothing can relax that.
export RUSTFLAGS="-D warnings" RUSTDOCFLAGS="-D warnings"

say() { printf '\n\033[1m== %s\033[0m\n' "$*"; }

toolchain_channel() { sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml; }

# The toolchain pinned in rust-toolchain.toml, installed if missing.
need_toolchain() {
    local channel
    channel=$(toolchain_channel)
    [ -n "$channel" ] || { echo "no channel in rust-toolchain.toml" >&2; return 1; }
    if ! rustup toolchain list | grep -q "^$channel-"; then
        rustup toolchain install "$channel" --profile minimal --component rustfmt,clippy
    fi
    rustup component add --toolchain "$channel" rustfmt clippy >/dev/null 2>&1
    rustc --version
    cargo --version
}

job_fmt() { need_toolchain && cargo fmt --all --check; }
job_clippy() { need_toolchain && cargo clippy --workspace --all-targets --all-features --locked -- -D warnings; }
job_test() { need_toolchain && cargo test --workspace --all-features --locked; }
# cargo-deny is not part of the toolchain: it runs when installed, and is
# skipped with a notice otherwise (CI installs it).
job_deny() {
    if ! cargo deny --version >/dev/null 2>&1; then
        echo "SKIPPED: cargo-deny is not installed (cargo install --locked cargo-deny)"
        return 0
    fi
    cargo deny --all-features check
}
# The pre-commit guard on the index; nothing staged, nothing to check.
job_privacy() {
    if git diff --cached --quiet; then
        echo "nothing staged: privacy guard not run"
        return 0
    fi
    scripts/privacy-guard.sh staged && echo "privacy guard: staged content clean"
}

ALL=(fmt clippy test deny privacy)

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
