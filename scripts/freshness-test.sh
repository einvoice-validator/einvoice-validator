#!/usr/bin/env bash
# Tests of scripts/freshness.sh. Offline by default: the upstream.toml reader
# (what it accepts and refuses), the pins check (drift is caught), and the
# Cargo dependency check against a fixture workspace and a curl stub serving
# fixture crates.io records (major, compatible, current, yanked, pre-release,
# network and API failures, rate limit, User-Agent, what the manifest and
# Cargo.lock reader refuses). With --online, also the queries: an outdated
# official rule set exits 1, a pinned version upstream does not publish and an
# unreachable network exit 2 and are never reported as current.
#
#   scripts/freshness-test.sh            # offline cases
#   scripts/freshness-test.sh --online   # offline and online cases (GitHub, Maven Central, crates.io)
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

valid_entry | sed 's/^severity = "info"$/severity = "high"\nrelease-name = "Rust"/' | case_toml
expect "release-name on severity high" 0 "pins: 1 pins of 1 upstreams" pins

valid_entry | sed 's/^severity = "info"$/severity = "info"\nrelease-name = "Rust"/' | case_toml
expect "release-name on severity info" 2 "release-name is for severity high only" pins

valid_entry | sed 's/^severity = "info"$/severity = "high"\nrelease-name = ""/' | case_toml
expect "empty release-name" 2 "release-name must not be empty" pins

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

# --- Cargo dependencies (offline: curl stub, fixture workspace and records) ----

# A stub of curl in front of PATH: serves $STUB_FIXTURES/NAME.json for
# https://crates.io/api/v1/crates/NAME (or the HTTP status in NAME.json.status)
# and fails like curl without network for anything else. Logs the time, the
# URL and the User-Agent of every call to $STUB_LOG.
mkdir -p "$tmp/bin"
cat >"$tmp/bin/curl" <<'STUB'
#!/usr/bin/env bash
url=${!#} ua= prev=
for arg in "$@"; do
    if [ "$prev" = --user-agent ]; then ua=$arg; fi
    prev=$arg
done
printf '%s\t%s\t%s\n' "${EPOCHREALTIME:-$(date +%s)}" "$url" "$ua" >>"$STUB_LOG"
file=
case $url in https://crates.io/api/v1/crates/*) file=$STUB_FIXTURES/${url##*/}.json ;; esac
if [ -n "$file" ] && [ -f "$file.status" ]; then
    echo "curl: (22) The requested URL returned error: $(cat "$file.status")" >&2
    exit 22
fi
if [ -z "$file" ] || [ ! -f "$file" ]; then
    host=${url#https://}
    echo "curl: (6) Could not resolve host: ${host%%/*}" >&2
    exit 6
fi
cat "$file"
STUB
chmod +x "$tmp/bin/curl"

# stubbed CMD...: runs CMD against the stub, the fixture workspace $tmp/ws and
# the records in $tmp/crates.
stubbed() {
    PATH=$tmp/bin:$PATH FRESHNESS_WORKSPACE=$tmp/ws STUB_FIXTURES=$tmp/crates STUB_LOG=$tmp/curl.log "$@"
}

# check NAME COMMAND...: one more test case, passed when COMMAND succeeds.
check() {
    local name=$1
    shift
    if "$@" >/dev/null 2>&1; then
        printf 'ok    %s\n' "$name"
        passed=$((passed + 1))
    else
        printf 'FAIL  %s\n' "$name"
        sed 's/^/      | /' "$tmp/out"
        failed=$((failed + 1))
    fi
}
# has TEXT: the output of the last case contains TEXT.
has() { grep -qF -- "$1" "$tmp/out"; }
# json FILTER: FILTER holds on the --json output of the last case.
json() { jq -e "$1" "$tmp/out.json"; }

# requests_spaced N: the stub got N requests, each one second or more after the previous.
requests_spaced() {
    awk -F '\t' -v n="$1" 'NR > 1 && $1 - last < 1 { bad = 1 } { last = $1 } END { exit bad || NR != n }' \
        "$tmp/curl.log"
}
# user_agent_ok: every request named the tool and the repository, and no e-mail address.
user_agent_ok() {
    awk -F '\t' '$3 != "einvoice-validator-freshness (+https://github.com/einvoice-validator/einvoice-validator)" \
        || $3 ~ /@/ { bad = 1 } END { exit bad || NR == 0 }' "$tmp/curl.log"
}

# crate_json NAME VERSION[:yanked]...: the crates.io record of a crate.
crate_json() {
    local name=$1
    shift
    printf '%s\n' "$@" | jq -Rn --arg name "$name" \
        '[inputs | split(":") | {num: .[0], yanked: (.[1] == "yanked")}]
         | {crate: {name: $name, num_versions: length}, versions: .}' >"$tmp/crates/$name.json"
}

# A workspace: member a (num-bigint, dev proptest, path b), member b.
workspace_ws() {
    rm -rf "$tmp/ws" "$tmp/crates" "$tmp/curl.log"
    mkdir -p "$tmp/ws/crates/a" "$tmp/ws/crates/b" "$tmp/crates"
    cat >"$tmp/ws/Cargo.toml" <<'TOML'
[workspace]
resolver = "3"
members = ["crates/*"]

[workspace.lints.clippy]
all = { level = "deny", priority = -1 }
TOML
    cat >"$tmp/ws/crates/a/Cargo.toml" <<'TOML'
[package]
name = "a"
version = "0.0.0"
keywords = [
    "one", # a comment
    "two",
]

[dependencies]
b = { path = "../b", version = "0.0.0" }
num-bigint = "0.4.8" # a comment

[dev-dependencies]
proptest = { version = "1.12.0", default-features = false, features = ["std"] }

[lints]
workspace = true
TOML
    cat >"$tmp/ws/crates/b/Cargo.toml" <<'TOML'
[package]
name = "b"
version = "0.0.0"
TOML
    cat >"$tmp/ws/Cargo.lock" <<'LOCK'
# This file is automatically @generated by Cargo.
# It is not intended for manual editing.
version = 4

[[package]]
name = "a"
version = "0.0.0"
dependencies = [
 "b",
 "num-bigint",
 "proptest",
]

[[package]]
name = "b"
version = "0.0.0"

[[package]]
name = "num-bigint"
version = "0.4.8"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "0000000000000000000000000000000000000000000000000000000000000000"

[[package]]
name = "proptest"
version = "1.12.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "0000000000000000000000000000000000000000000000000000000000000000"
LOCK
    crate_json proptest 1.11.0 1.12.0
}

crates_args=(crates --json "$tmp/out.json" --markdown "$tmp/out.md")

workspace_ws
crate_json num-bigint 0.4.7 0.4.8 0.4.9 0.5.0 0.5.1
stubbed expect "crates: a major update is reported, exit 0" 0 \
    "locked 0.4.8, newest compatible 0.4.9, newest 0.5.1 (major, manual migration)" "${crates_args[@]}"
check "crates: major update is actionable (opens the tracking issue)" \
    json '.[] | select(.name == "crate num-bigint") | .status == "major update" and .actionable and .severity == "info"'
check "crates: the markdown table has the row" grep -qF '| [crate num-bigint](https://crates.io/crates/num-bigint/versions) | info | 0.4.8 | 0.5.1 | major update |' "$tmp/out.md"
check "crates: dev-dependency checked, path dependency skipped" \
    json '(map(.name) == ["crate num-bigint", "crate proptest"]) and (.[1].note | endswith("used by a (dev)"))'
check "crates: 2 dependencies of 2 members" has "crates: 2 crates.io dependencies (normal, dev and build) of 2 workspace members"
check "crates: one request per crate, at most one per second" requests_spaced 2
check "crates: User-Agent names the repository, no e-mail" user_agent_ok

workspace_ws
crate_json num-bigint 0.4.8 0.4.9
stubbed expect "crates: only a compatible update" 0 \
    "locked 0.4.8, newest compatible 0.4.9, newest 0.4.9 (proposed by Dependabot)" "${crates_args[@]}"
check "crates: a compatible update is not actionable (Dependabot proposes it)" \
    json '.[] | select(.name == "crate num-bigint") | .status == "compatible update" and (.actionable | not)'

workspace_ws
crate_json num-bigint 0.3.3 0.4.0 0.4.8
stubbed expect "crates: current" 0 "locked 0.4.8, newest compatible 0.4.8, newest 0.4.8; used by a" "${crates_args[@]}"
check "crates: current is not actionable" json 'all(.[]; .status == "current" and (.actionable | not))'

workspace_ws
crate_json num-bigint 0.4.8 0.4.9:yanked 0.5.1:yanked
stubbed expect "crates: yanked newer versions are ignored" 0 "locked 0.4.8, newest compatible 0.4.8, newest 0.4.8;" \
    "${crates_args[@]}"
check "crates: yanked newer versions leave it current" json '.[0].status == "current" and .[0].latest == "0.4.8"'

workspace_ws
crate_json num-bigint 0.4.8 0.4.9-rc.1 0.5.0-alpha.1
stubbed expect "crates: pre-releases are ignored" 0 "locked 0.4.8, newest compatible 0.4.8, newest 0.4.8;" \
    "${crates_args[@]}"
check "crates: pre-releases leave it current" json '.[0].status == "current" and .[0].latest == "0.4.8"'

workspace_ws
crate_json num-bigint 0.4.8:yanked 0.4.9
stubbed expect "crates: a yanked locked version is reported" 0 "; locked version yanked" "${crates_args[@]}"
check "crates: a yanked locked version is actionable" json '.[0].status == "locked version yanked" and .[0].actionable'

workspace_ws
stubbed expect "crates: no network exits 2" 2 "cannot fetch https://crates.io/api/v1/crates/num-bigint: curl: (6)" \
    "${crates_args[@]}"
check "crates: no network, never current" json '.[] | select(.name == "crate num-bigint") | .status == "ERROR" and .latest == "?"'
check "crates: no network, the error is summed up" has "could not be checked (ERROR above)"

workspace_ws
echo 404 >"$tmp/crates/num-bigint.json.status"
stubbed expect "crates: an HTTP error exits 2" 2 "error: 404" "${crates_args[@]}"

workspace_ws
echo '<html>maintenance</html>' >"$tmp/crates/num-bigint.json"
stubbed expect "crates: a response that is not JSON exits 2" 2 "unexpected response from https://crates.io/api/v1/crates/num-bigint" \
    "${crates_args[@]}"

workspace_ws
echo '{"errors": [{"detail": "Not Found"}]}' >"$tmp/crates/num-bigint.json"
stubbed expect "crates: an API error record exits 2" 2 "not a crates.io crate record" "${crates_args[@]}"

workspace_ws
crate_json num-bigint 0.4.8 0.5.1
jq '.crate.num_versions = 40' "$tmp/crates/num-bigint.json" >"$tmp/x.json" && mv "$tmp/x.json" "$tmp/crates/num-bigint.json"
stubbed expect "crates: an incomplete version list exits 2" 2 "2 versions listed of 40" "${crates_args[@]}"

workspace_ws
crate_json num-bigint 0.4.8
jq '.crate.name = "num-integer"' "$tmp/crates/num-bigint.json" >"$tmp/x.json" && mv "$tmp/x.json" "$tmp/crates/num-bigint.json"
stubbed expect "crates: the record of another crate exits 2" 2 "the record of num-integer, not of num-bigint" "${crates_args[@]}"

workspace_ws
crate_json num-bigint 0.4.7 0.5.1
stubbed expect "crates: a locked version crates.io does not publish exits 2" 2 "locked 0.4.8 is not among the 2 versions" \
    "${crates_args[@]}"

workspace_ws
crate_json num-bigint 0.4.8 v0.4.9
stubbed expect "crates: a version that is not semver exits 2" 2 "not semantic versions: v0.4.9" "${crates_args[@]}"

# The reader of the manifests and Cargo.lock.

workspace_ws
crate_json num-bigint 0.4.8
sed -i.bak 's/^num-bigint = .*/bigint = { package = "num-bigint", version = "0.4" }/' "$tmp/ws/crates/a/Cargo.toml"
stubbed expect "crates: a renamed dependency is checked under its crate name" 0 "crate num-bigint" "${crates_args[@]}"

workspace_ws
crate_json num-bigint 0.4.8
crate_json cc 1.0.0 2.0.0
printf '\n[build-dependencies]\ncc = "1"\n' >>"$tmp/ws/crates/b/Cargo.toml"
awk '{ print } /^name = "b"$/ { getline; print; print "dependencies = ["; print " \"cc\","; print "]" }' \
    "$tmp/ws/Cargo.lock" >"$tmp/x.lock" && mv "$tmp/x.lock" "$tmp/ws/Cargo.lock"
cat >>"$tmp/ws/Cargo.lock" <<'LOCK'

[[package]]
name = "cc"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
LOCK
stubbed expect "crates: build-dependencies are checked" 0 "newest 2.0.0 (major, manual migration); used by b (build)" \
    "${crates_args[@]}"

workspace_ws
sed -i.bak 's/^num-bigint = .*/num-bigint.workspace = true/' "$tmp/ws/crates/a/Cargo.toml"
stubbed expect "crates: workspace inheritance is refused" 2 "crates/a/Cargo.toml:11: dotted key num-bigint.workspace" crates

workspace_ws
printf '\n[workspace.dependencies]\nnum-bigint = "0.4.8"\n' >>"$tmp/ws/Cargo.toml"
stubbed expect "crates: [workspace.dependencies] is refused" 2 "Cargo.toml:8: unsupported table [workspace.dependencies]" crates

workspace_ws
crate_json num-bigint 0.4.8
crate_json libc 0.2.0 0.3.0
printf "\n[target.'cfg(not(target_family = \"wasm\"))'.dev-dependencies]\nlibc = \"0.2\"\n" \
    >>"$tmp/ws/crates/b/Cargo.toml"
awk '{ print } /^name = "b"$/ { getline; print; print "dependencies = ["; print " \"libc\","; print "]" }' \
    "$tmp/ws/Cargo.lock" >"$tmp/x.lock" && mv "$tmp/x.lock" "$tmp/ws/Cargo.lock"
cat >>"$tmp/ws/Cargo.lock" <<'LOCK'

[[package]]
name = "libc"
version = "0.2.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
LOCK
stubbed expect "crates: target-specific dependencies are checked" 0 "newest 0.3.0 (major, manual migration); used by b (dev)" \
    "${crates_args[@]}"

workspace_ws
printf "\n[target.'cfg(unix)'.dependencies.libc]\nversion = \"0.2\"\n" >>"$tmp/ws/crates/b/Cargo.toml"
stubbed expect "crates: a target-specific dependency subtable is refused" 2 "unsupported table [target.'cfg(unix)'.dependencies.libc]" crates

workspace_ws
printf '\n[dependencies.serde]\nversion = "1"\n' >>"$tmp/ws/crates/b/Cargo.toml"
stubbed expect "crates: a dependency subtable is refused" 2 "unsupported table [dependencies.serde]" crates

workspace_ws
printf '\n[patch.crates-io]\nnum-bigint = { path = "../nb" }\n' >>"$tmp/ws/Cargo.toml"
stubbed expect "crates: [patch] is refused" 2 "it changes what Cargo.lock records" crates

workspace_ws
sed -i.bak 's|^num-bigint = .*|num-bigint = { git = "https://example.org/num-bigint" }|' "$tmp/ws/crates/a/Cargo.toml"
stubbed expect "crates: a git dependency is refused" 2 "unsupported dependency key git" crates

workspace_ws
sed -i.bak 's/^num-bigint = .*/num-bigint = "=0.4.8"/' "$tmp/ws/crates/a/Cargo.toml"
stubbed expect "crates: a non-caret requirement is refused" 2 'unsupported version requirement "=0.4.8"' crates

workspace_ws
sed -i.bak 's/^num-bigint = .*/num-bigint = { version = "0.4.8",/' "$tmp/ws/crates/a/Cargo.toml"
stubbed expect "crates: a multi-line inline table is refused" 2 "multi-line inline tables are not supported" crates

workspace_ws
printf 'description = """\n[dependencies]\n"""\n' >>"$tmp/ws/crates/b/Cargo.toml"
stubbed expect "crates: a multi-line string is refused" 2 "multi-line strings are not supported" crates

workspace_ws
printf 'authors = [\n    { name = "x" },\n]\n' >>"$tmp/ws/crates/b/Cargo.toml"
stubbed expect "crates: a multi-line array of non-strings is refused" 2 "unsupported line in a multi-line array" crates

workspace_ws
printf '"stray",\n' >>"$tmp/ws/crates/b/Cargo.toml"
stubbed expect "crates: a line that is not key = value is refused" 2 "unsupported line" crates

workspace_ws
sed -i.bak 's/^num-bigint = .*/num-bigint = "0.4.9"/' "$tmp/ws/crates/a/Cargo.toml"
stubbed expect "crates: a Cargo.lock that does not satisfy the requirement exits 2" 2 \
    'has no version of num-bigint that satisfies "0.4.9" for a (found: 0.4.8); run cargo update' crates

workspace_ws
sed -i.bak '/^ "proptest",$/d' "$tmp/ws/Cargo.lock"
stubbed expect "crates: a dependency missing from Cargo.lock exits 2" 2 "has no version of proptest" crates

workspace_ws
mkdir "$tmp/ws/crates/c"
printf '[package]\nname = "c"\n' >"$tmp/ws/crates/c/Cargo.toml"
stubbed expect "crates: a member missing from Cargo.lock exits 2" 2 "Cargo.lock does not match the workspace" crates

workspace_ws
mkdir "$tmp/ws/crates/empty"
stubbed expect "crates: a member directory without Cargo.toml exits 2" 2 "workspace member crates/empty has no Cargo.toml" crates

workspace_ws
printf '\n[metadata]\nx = "y"\n' >>"$tmp/ws/Cargo.lock"
stubbed expect "crates: an unknown Cargo.lock table exits 2" 2 "Cargo.lock:30: unsupported table [metadata]" crates

workspace_ws
sed -i.bak 's/^version = 4$/version = 2/' "$tmp/ws/Cargo.lock"
stubbed expect "crates: an old Cargo.lock format exits 2" 2 "unsupported Cargo.lock format 2" crates

workspace_ws
sed -i.bak 's/^source = .*/source = "git+https:\/\/example.org\/num-bigint#0000"/' "$tmp/ws/Cargo.lock"
stubbed expect "crates: a dependency locked outside crates.io exits 2" 2 "is locked from git+https://example.org/num-bigint#0000, not crates.io" crates

workspace_ws
rm "$tmp/ws/Cargo.lock"
stubbed expect "crates: no Cargo.lock exits 2" 2 "Cargo.lock not found" crates

# A whole query run without network: upstream.toml entries and crates alike
# are errors, nothing is current.
workspace_ws
rm "$tmp/crates/proptest.json"
cp upstream.toml "$tmp/case.toml"
stubbed expect "query run without network exits 2" 2 "could not be checked" --json "$tmp/out.json"
check "query run without network: crates are queried too" json 'any(.[]; .name == "crate num-bigint")'
check "query run without network: nothing is current" json 'all(.[]; .status == "ERROR")'

# The repository's own workspace is read (no network: every crate errs).
rm -f "$tmp/curl.log"
PATH=$tmp/bin:$PATH STUB_FIXTURES=$tmp/none STUB_LOG=$tmp/curl.log \
    expect "the repository's manifests and Cargo.lock are read" 2 "crates.io dependencies (normal, dev and build) of" \
    crates --json "$tmp/out.json"
check "the repository's dependencies include num-bigint" json 'any(.[]; .name == "crate num-bigint")'

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
