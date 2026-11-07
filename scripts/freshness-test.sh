#!/usr/bin/env bash
# Tests of scripts/freshness.sh. Offline by default: the upstream.toml reader
# (what it accepts and refuses) and the pins check (drift is caught). With
# --online, also the queries: an outdated official rule set exits 1, a pinned
# version upstream does not publish and an unreachable network exit 2 and are
# never reported as current.
#
#   scripts/freshness-test.sh            # offline cases
#   scripts/freshness-test.sh --online   # offline and online cases (GitHub, Maven Central)
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
online=
case ${1:-} in
'') ;;
--online) online=1 ;;
*)
    echo "usage: scripts/freshness-test.sh [--online]" >&2
    exit 2
    ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
passed=0 failed=0

# expect NAME STATUS TEXT [ARGS...]: runs scripts/freshness.sh ARGS on the pin
# file $tmp/case.toml; the exit status must be STATUS and the output (stdout
# and stderr) must contain TEXT. TEXT may be empty.
expect() {
    local name=$1 status=$2 text=$3 actual=0
    shift 3
    FRESHNESS_PINFILE=$tmp/case.toml scripts/freshness.sh "$@" >"$tmp/out" 2>&1 || actual=$?
    if [ "$actual" -ne "$status" ]; then
        printf 'FAIL  %s: exit %s, expected %s\n' "$name" "$actual" "$status"
        sed 's/^/      | /' "$tmp/out"
        failed=$((failed + 1))
    elif [ -n "$text" ] && ! grep -qF -- "$text" "$tmp/out"; then
        printf 'FAIL  %s: output lacks "%s"\n' "$name" "$text"
        sed 's/^/      | /' "$tmp/out"
        failed=$((failed + 1))
    else
        printf 'ok    %s\n' "$name"
        passed=$((passed + 1))
    fi
}

# case_toml: the pin file of the next case, from standard input.
case_toml() { cat >"$tmp/case.toml"; }

# A valid entry with one pin, the base of most cases.
valid_entry() {
    cat <<'TOML'
[[upstream]]
name = "Rust toolchain"
severity = "info"
pinned = "1.99.1"
source = "github-release"
repository = "rust-lang/rust"

[[upstream.pin]]
file = "rust-toolchain.toml"
pattern = 'channel = "(?<v>[^"]+)"'
TOML
}

# --- the real pin file ---------------------------------------------------------

cp upstream.toml "$tmp/case.toml"
expect "upstream.toml: every pin matches the repository" 0 "pins of" pins
expect "--help" 0 "Exit status" --help
expect "unknown argument" 2 "unknown argument" --nope

# --- the reader ----------------------------------------------------------------

valid_entry | case_toml
expect "minimal valid file" 0 "pins: 1 pins of 1 upstreams" pins

: | case_toml
expect "empty file" 2 "no [[upstream]]" pins

{ valid_entry; echo 'colour = "blue"'; } | case_toml
expect "unknown key in a pin" 2 "unknown key colour" pins

{ echo '[[upstream]]'; echo 'colour = "blue"'; } | case_toml
expect "unknown key in an upstream" 2 "unknown key colour" pins

{ valid_entry; echo 'pinned = 1.99'; } | case_toml
expect "unquoted value" 2 "expected [[upstream]]" pins

{ valid_entry; echo 'note = "a \"quoted\" word"'; } | case_toml
expect "escape in a basic string" 2 "expected [[upstream]]" pins

{ valid_entry; echo 'pattern = """x"""'; } | case_toml
expect "multi-line string" 2 "expected [[upstream]]" pins

{ valid_entry; echo '[upstream]'; } | case_toml
expect "plain table header" 2 "expected [[upstream]]" pins

valid_entry | sed 's/^severity = "info"$/severity = "info"\nseverity = "high"/' | case_toml
expect "duplicate key" 2 "duplicate key severity" pins

{ echo '[[upstream.pin]]'; valid_entry; } | case_toml
expect "pin before any upstream" 2 "before any [[upstream]]" pins

{ echo 'name = "x"'; valid_entry; } | case_toml
expect "key outside a table" 2 "key outside a table" pins

valid_entry | sed '/^pinned = /d' | case_toml
expect "missing pinned" 2 "are required" pins

valid_entry | sed 's/^severity = "info"$/severity = "medium"/' | case_toml
expect "bad severity" 2 "severity must be" pins

valid_entry | sed 's/^pinned = "1.99.1"$/pinned = "stable"/' | case_toml
expect "pinned not a version" 2 "pinned must be a version" pins

valid_entry | sed 's/^source = "github-release"$/source = "gitlab"/' | case_toml
expect "unknown source" 2 "source must be" pins

valid_entry | sed '/^repository = /d' | case_toml
expect "github-release without repository" 2 "needs repository" pins

valid_entry | sed 's|^repository = .*|repository = "rust-lang/rust"\ngroup = "org.example"|' | case_toml
expect "github-release with a maven key" 2 "takes none of" pins

valid_entry | sed 's|^source = "github-release"$|source = "listing"|; /^repository = /d; s|^pinned = .*|&\nurl = "https://example.org/"|' |
    case_toml
expect "listing without version-pattern" 2 "needs url and version-pattern" pins

valid_entry | sed "s|^repository = .*|&\nversion-pattern = '^v?([0-9.]+)\$'|" | case_toml
expect "version-pattern without group v" 2 "needs a group named v" pins

valid_entry | sed 's/^severity = "info"$/severity = "high"\nhold = "on purpose"/' | case_toml
expect "hold on severity high" 2 "hold is for severity info only" pins

valid_entry | sed 's|^repository = .*|&\nunused = "not used"|' | case_toml
expect "unused with pins" 2 "unused, yet" pins

valid_entry | sed '/^\[\[upstream.pin\]\]/,$d' | case_toml
expect "neither pins nor unused" 2 "no [[upstream.pin]]" pins

{ valid_entry; echo; valid_entry; } | case_toml
expect "duplicate name" 2 "duplicate name Rust toolchain" pins

valid_entry | sed 's/^file = .*/file = "rust-toolchain.toml"\ncompare = "fuzzy"/' | case_toml
expect "bad compare" 2 "compare must be" pins

valid_entry | sed "s/^pattern = .*/pattern = 'channel = \"([^\"]+)\"'/" | case_toml
expect "pin pattern without group v" 2 "needs a group named v" pins

# --- the pins check ------------------------------------------------------------

valid_entry | sed 's/^pinned = "1.99.1"$/pinned = "1.98.0"/' | case_toml
expect "pinned differs from the file" 2 "found 1.99.1, expected 1.98.0" pins

valid_entry | sed 's/^file = .*/file = "no-such-file.toml"/' | case_toml
expect "pin file missing" 2 "not a file tracked by git" pins

cp rust-toolchain.toml "$tmp/untracked.toml"
valid_entry | sed "s|^file = .*|file = \"$tmp/untracked.toml\"|" | case_toml
expect "pin file not tracked" 2 "not a file tracked by git" pins

valid_entry | sed "s/^pattern = .*/pattern = 'nightly-(?<v>[0-9.]+)'/" | case_toml
expect "pattern without a match" 2 "no match of" pins

valid_entry | sed "s/^pattern = .*/pattern = 'channel = \"(?<v>[^\"]+'/" | case_toml
expect "invalid regular expression" 2 "cannot apply the pattern" pins

valid_entry | sed "s/^file = .*/file = \"Cargo.toml\"/; s/^pattern = .*/pattern = 'rust-version = \"(?<v>[^\"]+)\"'\ncompare = \"prefix\"/" |
    case_toml
expect "prefix compare accepts 1.99 for 1.99.1" 0 "1 match(es)" pins

valid_entry | sed "s/^pinned = .*/pinned = \"1.990.1\"/; s/^file = .*/file = \"Cargo.toml\"/; s/^pattern = .*/pattern = 'rust-version = \"(?<v>[^\"]+)\"'\ncompare = \"prefix\"/" |
    case_toml
expect "prefix compare stops at a dot: 1.99 is not 1.990.1" 2 "found 1.99, expected 1.990.1" pins

valid_entry | sed "s/^file = .*/file = \"Cargo.toml\"/; s/^pattern = .*/pattern = 'rust-version = \"(?<v>[^\"]+)\"'/" |
    case_toml
expect "equal compare refuses 1.99 for 1.99.1" 2 "found 1.99, expected 1.99.1" pins

valid_entry | sed 's/^pinned = "1.99.1"$/pinned = "1.98.0"/' | case_toml
expect "a query run checks the pins first" 2 "do not match the repository"

# --- queries (network) ---------------------------------------------------------

if [ -n "$online" ]; then
    cat <<'TOML' | case_toml
[[upstream]]
name = "CEN EN 16931 validation artefacts"
severity = "high"
pinned = "1.3.16"
source = "github-release"
repository = "ConnectingEurope/eInvoicing-EN16931"
version-pattern = '^validation-(?<v>[0-9]+(?:\.[0-9]+)*)$'
unused = "test"
TOML
    expect "online: an outdated official rule set exits 1" 1 "OUTDATED" --json "$tmp/out.json"
    if jq -e '.[0].actionable == true and .[0].status == "OUTDATED"' "$tmp/out.json" >/dev/null; then
        echo "ok    online: --json marks it actionable"
        passed=$((passed + 1))
    else
        echo "FAIL  online: --json does not mark it actionable"
        failed=$((failed + 1))
    fi

    sed 's/^severity = "high"$/severity = "info"/' "$tmp/case.toml" >"$tmp/info.toml"
    mv "$tmp/info.toml" "$tmp/case.toml"
    expect "online: a newer tool version is info only, exit 0" 0 "newer exists"

    sed 's/^pinned = .*/pinned = "1.3.99"/' "$tmp/case.toml" >"$tmp/x.toml"
    mv "$tmp/x.toml" "$tmp/case.toml"
    expect "online: a pinned version not published upstream exits 2" 2 "is not among the"

    cat <<'TOML' | case_toml
[[upstream]]
name = "Saxon-HE"
severity = "info"
pinned = "9.9.1-8"
source = "maven"
group = "net.sf.saxon"
artifact = "Saxon-HE"
series = "99"
unused = "test"
TOML
    expect "online: a series nobody publishes exits 2" 2 "no 99.x version"

    cat <<'TOML' | case_toml
[[upstream]]
name = "OASIS UBL"
severity = "info"
pinned = "2.1"
source = "listing"
url = "https://docs.oasis-open.org/ubl/"
version-pattern = 'href="no-such-dir-(?<v>[0-9.]+)/"'
unused = "test"
TOML
    expect "online: a listing without any match exits 2" 2 "no published version matches"

    cat <<'TOML' | case_toml
[[upstream]]
name = "missing repository"
severity = "info"
pinned = "1.0.0"
source = "github-release"
repository = "einvoice-validator/no-such-repository-for-freshness-tests"
unused = "test"
TOML
    expect "online: an HTTP error (404 from GitHub) exits 2" 2 "error: 404"

    cat <<'TOML' | case_toml
[[upstream]]
name = "missing artifact"
severity = "info"
pinned = "1.0.0"
source = "maven"
group = "net.sf.saxon"
artifact = "no-such-artifact-for-freshness-tests"
unused = "test"
TOML
    expect "online: an HTTP error (404 from Maven Central) exits 2" 2 "error: 404"

    cp upstream.toml "$tmp/case.toml"
    # A proxy that refuses every connection: every query fails.
    https_proxy=http://127.0.0.1:9 HTTPS_PROXY=http://127.0.0.1:9 \
        expect "online: no network exits 2, nothing reported current" 2 "could not be checked"
    if grep -q ' current' "$tmp/out"; then
        echo "FAIL  online: an unreachable upstream was reported current"
        failed=$((failed + 1))
    else
        echo "ok    online: no upstream reported current without network"
        passed=$((passed + 1))
    fi
fi

echo
echo "freshness-test: $passed passed, $failed failed"
[ "$failed" -eq 0 ]
