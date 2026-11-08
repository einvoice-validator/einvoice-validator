#!/usr/bin/env bash
# Tests of scripts/release.sh and scripts/changelog.sh, in throw-away
# repositories under a temporary directory: a small Cargo workspace with a
# bare "origin", its own git configuration (GIT_CONFIG_GLOBAL; the user's is
# never read) and a throw-away SSH key for signing. Nothing is pushed
# anywhere but to that temporary origin.
#
#   scripts/test-release.sh
#
# Covers every refusal of release.sh (usage, branch, clean tree, origin,
# version order, existing tag, empty Unreleased, upstream pins, check.sh), a
# dry run (nothing changes), two real releases (versions, Cargo.lock, the npm
# package manifest, changelog sections and links, signed commit and tag,
# nothing pushed),
# --verify, the rule sets noted in the release (release-name for a name that
# ends with a version), --publish (crates already published skipped, a new
# crate refused, the first version of a named crate, rate limits retried with
# the time crates.io gives or a backoff, the deadline and the attempts, other
# failures) and --trusted-publishing (configurations added, kept, listed back,
# refused; the token), against stubs of curl, cargo publish and sleep (no
# network), and the pull-request check of changelog.sh. The repository's
# scripts/check.sh is replaced by a stub there; scripts/freshness.sh is the
# real one. Needs git (2.34 or later), ssh-keygen, cargo, jq and curl (the
# last two for scripts/freshness.sh, which stays offline here).
set -euo pipefail

source_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
for tool in git ssh-keygen cargo jq curl; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "test-release: $tool is not installed" >&2
        exit 2
    }
done

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
passed=0 failed=0

# Isolated git: no user or system configuration, a throw-away signing key.
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=$tmp/gitconfig
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE
ssh-keygen -q -t ed25519 -N '' -C release-test -f "$tmp/key"
printf 'release-test@example.invalid %s\n' "$(cat "$tmp/key.pub")" >"$tmp/allowed_signers"
cat >"$GIT_CONFIG_GLOBAL" <<EOF
[user]
	name = Release Test
	email = release-test@example.invalid
	signingkey = $tmp/key
[gpg]
	format = ssh
[gpg "ssh"]
	program = ssh-keygen
	allowedSignersFile = $tmp/allowed_signers
[init]
	defaultBranch = main
[advice]
	detachedHead = false
EOF

# The stub of scripts/check.sh: exits with CHECK_STATUS (default 0), logs
# each run to CHECK_LOG and writes the locale it was given to CHECK_ENV.
export CHECK_LOG=$tmp/check.log CHECK_ENV=$tmp/check.env
: >"$CHECK_LOG"
# What the stub must see: the locale of this script, unchanged by release.sh.
expected_env="LC_ALL=${LC_ALL-<unset>} LANG=${LANG-<unset>}"

ok() {
    printf 'ok    %s\n' "$1"
    passed=$((passed + 1))
}
fail() {
    printf 'FAIL  %s\n' "$1"
    [ -f "$tmp/out" ] && sed 's/^/      | /' "$tmp/out"
    failed=$((failed + 1))
}

# expect NAME STATUS TEXT CMD...: CMD exits with STATUS and its output (stdout
# and stderr) contains TEXT (fixed string; may be empty). The output is kept
# in $tmp/last for the checks that follow.
expect() {
    local name=$1 status=$2 text=$3 actual=0
    shift 3
    "$@" >"$tmp/out" 2>&1 || actual=$?
    cp "$tmp/out" "$tmp/last"
    if [ "$actual" -ne "$status" ]; then
        fail "$name: exit $actual, expected $status"
    elif [ -n "$text" ] && ! grep -qF -- "$text" "$tmp/out"; then
        fail "$name: output lacks \"$text\""
    else
        ok "$name"
    fi
}

# check NAME CMD...: CMD succeeds (an assertion on the state).
check() {
    local name=$1
    shift
    if "$@" >"$tmp/out" 2>&1; then ok "$name"; else fail "$name"; fi
}

# ---- The repository under test -----------------------------------------------

origin=$tmp/origin.git
repo=$tmp/repo
git init --quiet --bare "$origin"
git init --quiet "$repo"
cd "$repo"
git remote add origin "$origin"
mkdir -p scripts crates/alpha/src crates/beta/src crates/beta/tests crates/gamma/src rules .github/workflows
cp "$source_dir/release.sh" "$source_dir/changelog.sh" "$source_dir/freshness.sh" scripts/
cat >scripts/check.sh <<'EOF'
#!/usr/bin/env bash
echo "check.sh stub: status ${CHECK_STATUS:-0}"
echo run >>"$CHECK_LOG"
echo "LC_ALL=${LC_ALL-<unset>} LANG=${LANG-<unset>}" >"$CHECK_ENV"
exit "${CHECK_STATUS:-0}"
EOF
chmod +x scripts/*.sh
printf '/target\n' >.gitignore
cat >Cargo.toml <<'EOF'
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
version = "0.0.0"
edition = "2021"
license = "MIT"
repository = "https://github.com/example/project"
EOF
cat >crates/alpha/Cargo.toml <<'EOF'
[package]
name = "alpha"
version.workspace = true
edition.workspace = true
license.workspace = true
EOF
cat >crates/beta/Cargo.toml <<'EOF'
[package]
name = "beta"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
alpha = { path = "../alpha", version = "0.0.0" }
EOF
# An npm package built from a crate: its top-level version follows the
# workspace's; a nested "version" member is left alone.
mkdir -p crates/beta/npm
cat >crates/beta/npm/package.json <<'EOF'
{
  "name": "beta",
  "version": "0.0.0",
  "config": {
    "version": "keep"
  }
}
EOF
echo 'pub fn a() {}' >crates/alpha/src/lib.rs
echo 'pub fn b() { alpha::a() }' >crates/beta/src/lib.rs
# A crate that is not published: --publish and --trusted-publishing leave it out.
cat >crates/gamma/Cargo.toml <<'EOF'
[package]
name = "gamma"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false
EOF
echo 'pub fn g() {}' >crates/gamma/src/lib.rs
echo 'name: release' >.github/workflows/release.yml
echo '#[test] fn t() { beta::b() }' >crates/beta/tests/t.rs
echo 'Rule set 1.2.3' >rules/README.md
echo 'Billing rules 2.0.7' >rules/billing.md
# "Billing rules 2.0" ends with a version of its own: its release-name keeps
# the note from reading "Billing rules 2.0 2.0.7".
cat >upstream.toml <<'EOF'
# Test pins.
[[upstream]]
name = "Rule set"
severity = "high"
pinned = "1.2.3"
source = "github-release"
repository = "example/rules"

[[upstream.pin]]
file = "rules/README.md"
pattern = 'Rule set (?<v>[0-9.]+)'

[[upstream]]
name = "Billing rules 2.0"
severity = "high"
pinned = "2.0.7"
release-name = "Billing rules"
source = "github-release"
repository = "example/billing"

[[upstream.pin]]
file = "rules/billing.md"
pattern = 'Billing rules (?<v>[0-9]+\.[0-9]+\.[0-9]+)'

[[upstream]]
name = "Future rule set"
severity = "high"
pinned = "3.0.0"
source = "github-release"
repository = "example/future"
unused = "not used yet"

[[upstream]]
name = "Tool"
severity = "info"
pinned = "9.9.9"
source = "github-release"
repository = "example/tool"
unused = "not used yet"
EOF
cat >CHANGELOG.md <<'EOF'
# Changelog

Intro.

## [Unreleased]

### Added

- The first feature.

[Unreleased]: https://github.com/example/project/commits/main
EOF
(cargo generate-lockfile --offline --quiet)
git add -A
git commit --quiet -m "Initial commit"
git push --quiet origin main

# commit_all MESSAGE: commit every change and push it to origin.
commit_all() {
    git add -A
    git commit --quiet -m "$1"
    git push --quiet origin main
}

release() { scripts/release.sh "$@"; }
tree_clean() { [ -z "$(git status --porcelain --untracked-files=all)" ]; }
head_is() { [ "$(git rev-parse HEAD)" = "$1" ]; }
no_tags() { [ -z "$(git tag)" ]; }
one_worktree() { [ "$(git worktree list | wc -l)" -eq 1 ]; }
check_runs() { [ "$(wc -l <"$CHECK_LOG")" -eq "$1" ]; }
lacks() { ! grep -qF -- "$1" "$2"; }
# note_is TEXT FILE: every "Tested against" line of FILE (diff lines too) is TEXT.
note_is() { [ "$(grep -F 'Tested against' "$2" | sed 's/^+//' | sort -u)" = "$1" ]; }
unreleased_empty() { [ -z "$(scripts/changelog.sh section Unreleased)" ]; }
# The rule sets every release notes: those in use, in the order of
# upstream.toml, each as its release-name or its name, then its version.
note_text="Rule set 1.2.3, Billing rules 2.0.7."

start=$(git rev-parse HEAD)

# ---- Usage and version syntax ------------------------------------------------

expect "no argument" 2 "Prepares a release" release
expect "--dry-run without version" 2 "" release --dry-run
expect "two versions" 2 "" release 0.1.0 0.2.0
expect "unknown option" 2 "" release --force 0.1.0
expect "version with v" 1 "not a SemVer version" release v0.1.0
expect "two components" 1 "not a SemVer version" release 0.1
expect "leading zero" 1 "not a SemVer version" release 01.0.0
expect "build metadata" 1 "not a SemVer version" release 0.1.0+build.1
expect "empty pre-release identifier" 1 "not a SemVer version" release 0.1.0-alpha..1

# ---- Refusals before anything is checked or changed --------------------------

git checkout --quiet -b topic
expect "not on main" 1 "releases are made from main" release 0.1.0
git checkout --quiet main
git branch --quiet -D topic
git checkout --quiet --detach
expect "detached HEAD" 1 "HEAD is detached" release 0.1.0
git checkout --quiet main

echo change >>rules/README.md
expect "modified file" 1 "not clean" release 0.1.0
git checkout --quiet -- rules/README.md
touch untracked.txt
expect "untracked file" 1 "not clean" release 0.1.0
rm untracked.txt
echo '- staged' >>CHANGELOG.md
git add CHANGELOG.md
expect "staged change" 1 "not clean" release 0.1.0
git reset --quiet --hard HEAD

echo '// local' >>crates/alpha/src/lib.rs
git commit --quiet -am "Local commit"
expect "ahead of origin" 1 "not identical to origin/main" release 0.1.0
git reset --quiet --hard "$start"
git clone --quiet "$origin" "$tmp/other"
(
    cd "$tmp/other"
    echo '// other' >>crates/alpha/src/lib.rs
    git commit --quiet -am "Commit from another clone"
    git push --quiet origin main
)
expect "behind origin" 1 "not identical to origin/main" release 0.1.0
git pull --quiet --ff-only origin main
start=$(git rev-parse HEAD)

expect "same version" 1 "0.0.0 is not greater than the current version 0.0.0" release 0.0.0

git tag v0.1.0
expect "tag exists locally" 1 "tag v0.1.0 already exists" release 0.1.0
git push --quiet origin v0.1.0
git tag -d v0.1.0 >/dev/null
expect "tag exists on origin" 1 "tag v0.1.0 already exists on origin" release 0.1.0
git push --quiet origin :refs/tags/v0.1.0

sed -i.bak '/^- The first feature\.$/d' CHANGELOG.md && rm CHANGELOG.md.bak
commit_all "Empty the Unreleased section"
expect "empty Unreleased" 1 "the Unreleased section has no entries" release 0.1.0
git revert --quiet --no-edit HEAD >/dev/null
git push --quiet origin main

echo 'Rule set 1.2.4' >rules/README.md
commit_all "Drift from the pin"
expect "upstream pin drift" 1 "scripts/freshness.sh pins failed" release 0.1.0
git revert --quiet --no-edit HEAD >/dev/null
git push --quiet origin main

check "no check.sh run so far" check_runs 0
before=$(git rev-parse HEAD)
expect "check.sh fails" 1 "scripts/check.sh failed" env CHECK_STATUS=1 scripts/release.sh 0.1.0
check "check.sh ran once" check_runs 1
check "after a refusal: HEAD unchanged" head_is "$before"
check "after a refusal: tree clean" tree_clean
check "after a refusal: no tag" no_tags

# ---- Dry run -----------------------------------------------------------------

today=$(date -u +%F)
expect "dry run" 0 "## [0.1.0] - $today" release --dry-run 0.1.0
check "dry run: rule sets noted" grep -qF "Tested against the official rule sets: $note_text" "$tmp/last"
check "dry run: unused rule sets left out" note_is "Tested against the official rule sets: $note_text" "$tmp/last"
check "dry run: release-name for a name ending with a version" lacks "Billing rules 2.0 2.0.7" "$tmp/last"
check "dry run: Cargo.lock in the diff" grep -qF "Cargo.lock" "$tmp/last"
check "dry run: push command printed" grep -qF "git push --atomic origin main v0.1.0" "$tmp/last"
check "dry run: check.sh ran" check_runs 2
check "dry run: HEAD unchanged" head_is "$before"
check "dry run: tree clean" tree_clean
check "dry run: no tag" no_tags
check "dry run: worktree removed" one_worktree

# ---- First release -----------------------------------------------------------

expect "release 0.1.0" 0 "git push --atomic origin main v0.1.0" release 0.1.0
check "0.1.0: check.sh ran" check_runs 3
check "0.1.0: check.sh got the caller's locale" test "$(cat "$CHECK_ENV")" = "$expected_env"
check "0.1.0: one new commit" test "$(git rev-parse HEAD~1)" = "$before"
check "0.1.0: commit message" test "$(git log -1 --format=%s)" = "Release 0.1.0"
check "0.1.0: commit signed and valid" git verify-commit HEAD
check "0.1.0: tag annotated" test "$(git cat-file -t v0.1.0)" = tag
check "0.1.0: tag signed and valid" git verify-tag v0.1.0
check "0.1.0: tag points at the release commit" test "$(git rev-parse 'v0.1.0^{commit}')" = "$(git rev-parse HEAD)"
check "0.1.0: tag message" grep -qF "einvoice-validator 0.1.0" <(git cat-file tag v0.1.0)
check "0.1.0: tag message keeps headings" grep -qF "### Added" <(git cat-file tag v0.1.0)
check "0.1.0: tree clean" tree_clean
check "0.1.0: worktree removed" one_worktree
check "0.1.0: only the version files changed" test "$(git diff --name-only HEAD~1 HEAD | tr '\n' ' ')" = "CHANGELOG.md Cargo.lock Cargo.toml crates/beta/Cargo.toml crates/beta/npm/package.json "
check "0.1.0: npm package version" grep -qx '  "version": "0.1.0",' crates/beta/npm/package.json
check "0.1.0: nested npm member kept" grep -qx '    "version": "keep"' crates/beta/npm/package.json
check "0.1.0: workspace version" grep -qx 'version = "0.1.0"' Cargo.toml
check "0.1.0: path dependency" grep -qF 'alpha = { path = "../alpha", version = "0.1.0" }' crates/beta/Cargo.toml
check "0.1.0: Cargo.lock" test "$(grep -A1 -E '^name = "(alpha|beta)"$' Cargo.lock | grep -c '^version = "0.1.0"$')" -eq 2
check "0.1.0: Cargo.lock consistent" cargo metadata --locked --offline --format-version 1
check "0.1.0: Unreleased emptied" unreleased_empty
check "0.1.0: section heading" grep -qx "## \[0.1.0\] - $today" CHANGELOG.md
check "0.1.0: section entries" grep -qF -- "- The first feature." <(scripts/changelog.sh section 0.1.0)
check "0.1.0: Unreleased link" grep -qx '\[Unreleased\]: https://github.com/example/project/compare/v0.1.0...HEAD' CHANGELOG.md
check "0.1.0: version link" grep -qx '\[0.1.0\]: https://github.com/example/project/releases/tag/v0.1.0' CHANGELOG.md
check "0.1.0: nothing pushed (main)" test "$(git ls-remote "$origin" refs/heads/main | cut -f1)" = "$before"
check "0.1.0: nothing pushed (tag)" test -z "$(git ls-remote --tags "$origin")"

expect "verify v0.1.0" 0 "agree" release --verify v0.1.0 "$tmp/notes.md"
check "notes: rule sets" note_is "Tested against the official rule sets: $note_text" "$tmp/notes.md"
check "notes: entries" grep -qF -- "- The first feature." "$tmp/notes.md"
check "notes: full changelog link" grep -qF "**Full changelog**: https://github.com/example/project/releases/tag/v0.1.0" "$tmp/notes.md"
expect "verify to stdout" 0 "### Added" release --verify v0.1.0
expect "verify: wrong version" 1 "the workspace version is 0.1.0, not 0.2.0" release --verify v0.2.0
expect "verify: tag without v" 1 "the tag must be vX.Y.Z" release --verify 0.1.0

sed -i.bak 's/^version.workspace = true$/version = "0.1.0"/' crates/alpha/Cargo.toml && rm crates/alpha/Cargo.toml.bak
expect "verify: member version not inherited" 1 "must inherit the version" release --verify v0.1.0
git checkout --quiet -- crates/alpha/Cargo.toml
sed -i.bak 's/version = "0.1.0" }/version = "0.0.9" }/' crates/beta/Cargo.toml && rm crates/beta/Cargo.toml.bak
expect "verify: path dependency version" 1 "path dependency not at version 0.1.0" release --verify v0.1.0
git checkout --quiet -- crates/beta/Cargo.toml
sed -i.bak '/^name = "alpha"$/{n;s/.*/version = "0.0.9"/;}' Cargo.lock && rm Cargo.lock.bak
expect "verify: Cargo.lock version" 1 "alpha@0.0.9" release --verify v0.1.0
git checkout --quiet -- Cargo.lock
sed -i.bak 's/^  "version": "0.1.0",$/  "version": "0.0.9",/' crates/beta/npm/package.json && rm crates/beta/npm/package.json.bak
expect "verify: npm package version" 1 "crates/beta/npm/package.json: version 0.0.9, not 0.1.0" release --verify v0.1.0
git checkout --quiet -- crates/beta/npm/package.json
sed -i.bak '/^  "version": "0.1.0",$/d' crates/beta/npm/package.json && rm crates/beta/npm/package.json.bak
expect "verify: npm package without version" 1 "no single top-level \"version\" line" release --verify v0.1.0
git checkout --quiet -- crates/beta/npm/package.json
sed -i.bak "s/^## \[0.1.0\] - $today\$/## [0.1.0]/" CHANGELOG.md && rm CHANGELOG.md.bak
expect "verify: section without date" 1 "no single heading" release --verify v0.1.0
git checkout --quiet -- CHANGELOG.md
sed -i.bak '/^\[0.1.0\]: /d' CHANGELOG.md && rm CHANGELOG.md.bak
expect "verify: no version link" 1 "no single link reference [0.1.0]" release --verify v0.1.0
git checkout --quiet -- CHANGELOG.md
check "verify cases left the tree clean" tree_clean

# ---- Second release, version order -------------------------------------------

git push --quiet --atomic origin main v0.1.0
expect "after a release: Unreleased empty" 1 "the Unreleased section has no entries" release 0.2.0
sed -i.bak 's/^## \[Unreleased\]$/## [Unreleased]\
\
### Fixed\
\
- A fix./' CHANGELOG.md && rm CHANGELOG.md.bak
echo '// fix' >>crates/alpha/src/lib.rs
commit_all "A fix"
expect "older version" 1 "0.0.5 is not greater than the current version 0.1.0" release 0.0.5
expect "pre-release of the current version" 1 "is not greater" release 0.1.0-rc.1
expect "release 0.2.0-rc.2" 0 "v0.2.0-rc.2 created" release 0.2.0-rc.2
check "0.2.0-rc.2: compare link from 0.1.0" grep -qx '\[0.2.0-rc.2\]: https://github.com/example/project/compare/v0.1.0...v0.2.0-rc.2' CHANGELOG.md
check "0.2.0-rc.2: Unreleased link" grep -qx '\[Unreleased\]: https://github.com/example/project/compare/v0.2.0-rc.2...HEAD' CHANGELOG.md
check "0.2.0-rc.2: 0.1.0 link kept" grep -qx '\[0.1.0\]: https://github.com/example/project/releases/tag/v0.1.0' CHANGELOG.md
check "0.2.0-rc.2: sections in order" test "$(grep '^## \[' CHANGELOG.md | tr '\n' '|')" = "## [Unreleased]|## [0.2.0-rc.2] - $today|## [0.1.0] - $today|"
check "0.2.0-rc.2: fix in its section" grep -qF -- "- A fix." <(scripts/changelog.sh section 0.2.0-rc.2)
check "0.2.0-rc.2: fix not in 0.1.0" lacks "A fix." <(scripts/changelog.sh section 0.1.0)
check "0.2.0-rc.2: path dependency" grep -qF 'version = "0.2.0-rc.2" }' crates/beta/Cargo.toml
check "0.2.0-rc.2: npm package version" grep -qx '  "version": "0.2.0-rc.2",' crates/beta/npm/package.json
expect "verify v0.2.0-rc.2" 0 "agree" release --verify v0.2.0-rc.2
git push --quiet --atomic origin main v0.2.0-rc.2
sed -i.bak 's/^## \[Unreleased\]$/## [Unreleased]\
\
### Changed\
\
- A change./' CHANGELOG.md && rm CHANGELOG.md.bak
commit_all "A change"
# SemVer 2.0.0 precedence: 0.2.0-rc.2 < 0.2.0-rc.10 < 0.2.0 < 0.10.0.
expect "rc.1 < rc.2" 1 "is not greater" release 0.2.0-rc.1
expect "rc < rc.2 (fewer identifiers)" 1 "is not greater" release 0.2.0-rc
expect "beta < rc.2" 1 "is not greater" release 0.2.0-beta.9
expect "1 < rc (numeric lower)" 1 "is not greater" release 0.2.0-1
expect "RC.3 < rc.2 (ASCII order, not the locale's)" 1 "is not greater" release 0.2.0-RC.3
expect "rc.10 > rc.2 (numeric order)" 0 "dry run: nothing changed" release --dry-run 0.2.0-rc.10
expect "rc.2.1 > rc.2" 0 "dry run: nothing changed" release --dry-run 0.2.0-rc.2.1
expect "0.2.0 > 0.2.0-rc.2" 0 "dry run: nothing changed" release --dry-run 0.2.0
expect "0.10.0 > 0.2.0-rc.2" 0 "dry run: nothing changed" release --dry-run 0.10.0
check "dry runs: tree clean" tree_clean
check "dry runs: worktrees removed" one_worktree

# ---- Publishing on crates.io (--publish, --trusted-publishing) ---------------

# Stubs, first on PATH for these tests only: curl answers from $stub (the
# crates.io index from $stub/index/NAME, the API from $stub/http/METHOD-KEY,
# then -KEY.1, -KEY.2... in turn when present: "STATUS" on the first line,
# the body after it) and logs every request; cargo runs the real cargo except
# for publish, which follows $stub/plan one line per run ("ok", or "KIND N
# MESSAGE": the first N crates are published, then MESSAGE and exit 101) and
# adds what it publishes to the index; sleep only logs its argument.
stub=$tmp/stub
mkdir -p "$stub/bin" "$stub/index" "$stub/http"
real_cargo=$(command -v cargo)
cat >"$stub/bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$STUB/argv.log"
method=GET out=/dev/stdout write="" data="" auth=none url=""
while [ $# -gt 0 ]; do
    case $1 in
    -X) method=$2; shift ;;
    -o) out=$2; shift ;;
    -w) write=$2; shift ;;
    -A | --proto | --retry | --retry-delay) shift ;;
    -H)
        if [ "$2" = @- ]; then
            auth=$(sed -n 's/^Authorization: //p')
        fi
        shift
        ;;
    --data-binary) data=${2#@}; shift ;;
    -*) ;;
    *) url=$1 ;;
    esac
    shift
done
printf '%s %s auth=%s data=%s\n' "$method" "$url" "$auth" "$( [ -z "$data" ] || tr -d '\n ' <"$data")" >>"$STUB/requests.log"
status=404 body=""
case $url in
https://index.crates.io/*)
    name=${url##*/}
    if [ -f "$STUB/index/$name.status" ]; then
        status=$(cat "$STUB/index/$name.status")
    elif [ -f "$STUB/index/$name" ]; then
        status=200 body=$(cat "$STUB/index/$name")
    fi
    ;;
*)
    key=$STUB/http/$method-$(printf '%s' "${url#https://}" | tr '/?=&' '____')
    file=""
    for n in 1 2 3 4 5 6 7 8 9; do
        if [ -f "$key.$n" ]; then file=$key.$n; break; fi
    done
    [ -n "$file" ] || { [ ! -f "$key" ] || file=$key; }
    if [ -n "$file" ]; then
        status=$(sed -n 1p "$file") body=$(sed 1d "$file")
        case $file in *.[0-9]) rm -f "$file" ;; esac
    fi
    ;;
esac
[ "$status" != 000 ] || { echo "curl: (7) Failed to connect" >&2; exit 7; }
printf '%s\n' "$body" >"$out"
[ "$write" != '%{http_code}' ] || printf '%s' "$status"
EOF
cat >"$stub/bin/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[ "${1:-}" = publish ] || exec "$REAL_CARGO" "$@"
shift
echo "cargo publish $*" >>"$STUB/cargo.log"
selected=() excluded=" " workspace=0
while [ $# -gt 0 ]; do
    case $1 in
    --locked) ;;
    --workspace) workspace=1 ;;
    --exclude) excluded="$excluded$2 "; shift ;;
    -p) selected+=("$2"); shift ;;
    *) echo "cargo stub: unexpected $1" >&2; exit 2 ;;
    esac
    shift
done
if [ "$workspace" = 1 ]; then
    for name in $STUB_CRATES; do
        [[ $excluded == *" $name "* ]] || selected+=("$name")
    done
fi
step=ok
if [ -s "$STUB/plan" ]; then
    step=$(sed -n 1p "$STUB/plan")
    sed -i.bak 1d "$STUB/plan" && rm -f "$STUB/plan.bak"
fi
read -r kind count message <<<"$step"
[ "$kind" != ok ] || count=${#selected[@]}
for ((i = 0; i < count && i < ${#selected[@]}; i++)); do
    echo "   Uploading ${selected[i]} v$STUB_VERSION"
    printf '{"name":"%s","vers":"%s"}\n' "${selected[i]}" "$STUB_VERSION" >>"$STUB/index/${selected[i]}"
done
[ "$kind" != ok ] || exit 0
printf 'error: failed to publish to registry at https://crates.io\n\nCaused by:\n  %s\n' "$message"
exit 101
EOF
cat >"$stub/bin/sleep" <<'EOF'
#!/usr/bin/env bash
echo "$*" >>"$STUB/sleep.log"
EOF
chmod +x "$stub/bin/"*
export STUB=$stub REAL_CARGO=$real_cargo STUB_CRATES="alpha beta"

# reset_stub: no request, run or sleep logged, no plan, an empty index.
reset_stub() {
    rm -rf "$stub/index" "$stub/http"
    mkdir -p "$stub/index" "$stub/http"
    rm -f "$stub/requests.log" "$stub/argv.log" "$stub/cargo.log" "$stub/sleep.log" "$stub/plan"
    touch "$stub/requests.log" "$stub/argv.log" "$stub/cargo.log" "$stub/sleep.log"
}
# indexed NAME VERSION...: the crates.io index has these versions of NAME.
indexed() {
    local name=$1 version
    shift
    for version in "$@"; do
        printf '{"name":"%s","vers":"%s","yanked":false}\n' "$name" "$version" >>"$stub/index/$name"
    done
}
# plan LINE...: what the next runs of cargo publish do.
plan() { printf '%s\n' "$@" >"$stub/plan"; }
# respond METHOD URL STATUS BODY [N]: the stub API's answer (the N-th one).
respond() {
    local key
    key=$stub/http/$1-$(printf '%s' "${2#https://}" | tr '/?=&' '____')
    printf '%s\n%s\n' "$3" "$4" >"$key${5:+.$5}"
}
stubbed() { env PATH="$stub/bin:$PATH" "$@"; }
publish() { stubbed scripts/release.sh --publish "$@"; }
cargo_runs() { [ "$(wc -l <"$stub/cargo.log")" -eq "$1" ]; }
cargo_run_is() { [ "$(sed -n "$1p" "$stub/cargo.log")" = "$2" ]; }
sleeps_are() { [ "$(tr '\n' ' ' <"$stub/sleep.log")" = "$1" ]; }
requested() { grep -qF -- "$1" "$stub/requests.log"; }
in_index() { grep -qF "\"vers\":\"$2\"" "$stub/index/$1"; }
# http_date SECONDS: the HTTP date SECONDS from now, as crates.io writes it.
http_date() { LC_ALL=C date -u -d "@$(($(date +%s) + $1))" '+%a, %d %b %Y %H:%M:%S GMT'; }
limit_message() {
    printf 'the remote server responded with an error (status 429 Too Many Requests): You have published too many new crates in a short period of time. Please try again after %s or email help@crates.io to have your limit increased.' "$1"
}

v=0.2.0-rc.2
export STUB_VERSION=$v
reset_stub
expect "publish: no tag" 2 "Prepares a release" release --publish
expect "publish: tag syntax" 1 "the tag must be vX.Y.Z" publish 0.2.0
expect "publish: unknown tag" 1 "no tag v9.9.9 in this clone" publish v9.9.9
expect "publish: HEAD not at the tag" 1 "HEAD is not the commit of v$v" publish "v$v"
git checkout --quiet --detach "v$v"
git tag v0.3.0
expect "publish: tag and workspace version disagree" 1 "the workspace version is $v, not 0.3.0" publish v0.3.0
git tag -d v0.3.0 >/dev/null
expect "publish: bad deadline" 1 "RELEASE_PUBLISH_DEADLINE is not a number" env RELEASE_PUBLISH_DEADLINE=soon PATH="$stub/bin:$PATH" scripts/release.sh --publish "v$v"
expect "publish: crate not publishable" 1 "gamma is not a publishable crate" publish "v$v" gamma
expect "publish: unknown crate" 1 "delta is not a publishable crate" publish "v$v" alpha delta
check "publish: refusals ran no cargo publish" cargo_runs 0

reset_stub
indexed alpha 0.1.0
expect "publish: new crate refused" 1 "not on crates.io yet: beta" publish "v$v"
check "publish: new crate: first publish explained" grep -qF "scripts/release.sh --publish v$v beta" "$tmp/last"
check "publish: new crate: trusted publishing named" grep -qF "scripts/release.sh --trusted-publishing beta" "$tmp/last"
check "publish: new crate: nothing published" cargo_runs 0
check "publish: index path of a long name" requested "GET https://index.crates.io/al/ph/alpha auth=none"
check "publish: index path of the other crate" requested "GET https://index.crates.io/be/ta/beta auth=none"
check "publish: non-published crate not looked up" lacks "gamma" "$stub/requests.log"

reset_stub
indexed alpha 0.1.0
echo 500 >"$stub/index/beta.status"
expect "publish: index error" 1 "cannot read the crates.io index entry of beta" publish "v$v"
check "publish: index error: HTTP status shown" grep -qF "answered HTTP 500 for beta" "$tmp/last"
check "publish: index error: nothing published" cargo_runs 0
echo 000 >"$stub/index/beta.status"
expect "publish: index unreachable" 1 "cannot read the crates.io index entry of beta" publish "v$v"

reset_stub
indexed alpha 0.1.0
indexed beta 0.1.0
expect "publish: every crate" 0 "published at $v: alpha beta" publish "v$v"
check "publish: one cargo publish --workspace" cargo_run_is 1 "cargo publish --locked --workspace"
check "publish: one run" cargo_runs 1
check "publish: alpha in the index" in_index alpha "$v"
check "publish: beta in the index" in_index beta "$v"
check "publish: no wait" sleeps_are ""
expect "publish: run again, nothing left" 0 "every crate is on crates.io at $v" publish "v$v"
check "publish: run again: skipped listed" grep -qF "already on crates.io at $v, skipped: alpha beta" "$tmp/last"
check "publish: run again: no cargo publish" cargo_runs 1

reset_stub
indexed alpha 0.1.0 "$v"
indexed beta 0.1.0
expect "publish: one already published" 0 "skipped: alpha" publish "v$v"
check "publish: --exclude the published crate" cargo_run_is 1 "cargo publish --locked --workspace --exclude alpha"

reset_stub
indexed alpha 0.1.0
indexed beta 0.1.0
plan "429 1 $(limit_message "$(http_date 120)")" ok
expect "publish: rate limit, then published" 0 "published at $v: beta" publish "v$v"
check "publish: rate limit: waited until the time given" test "$(cat "$stub/sleep.log")" -ge 120 -a "$(cat "$stub/sleep.log")" -le 126
check "publish: rate limit: two runs" cargo_runs 2
check "publish: rate limit: first run" cargo_run_is 1 "cargo publish --locked --workspace"
check "publish: rate limit: retry skips what was published" cargo_run_is 2 "cargo publish --locked --workspace --exclude alpha"
check "publish: rate limit: reason shown" grep -qF "crates.io rate limit (HTTP 429); attempt 2" "$tmp/last"

reset_stub
indexed alpha 0.1.0
indexed beta 0.1.0
plan "429 0 the remote server responded with an error (status 429 Too Many Requests): slow down" \
    "429 0 status 429" "429 0 status 429" ok
expect "publish: rate limit without a time: backoff" 0 "published at $v: alpha beta" publish "v$v"
check "publish: backoff doubles" sleeps_are "60 120 240 "
check "publish: backoff: four runs" cargo_runs 4

reset_stub
indexed alpha 0.1.0
indexed beta 0.1.0
plan "429 0 status 429" "429 0 status 429" "429 0 status 429" "429 0 status 429" "429 0 status 429" \
    "429 0 status 429" "429 0 status 429" "429 0 status 429" "429 0 status 429" "429 0 status 429" ok
expect "publish: rate limit, attempts exhausted" 1 "not published after 10 attempts" env RELEASE_PUBLISH_DEADLINE=100000 PATH="$stub/bin:$PATH" scripts/release.sh --publish "v$v"
check "publish: attempts exhausted: ten runs" cargo_runs 10
check "publish: attempts exhausted: backoff capped at 600 s" sleeps_are "60 120 240 480 600 600 600 600 600 "

reset_stub
indexed alpha 0.1.0
indexed beta 0.1.0
plan "429 0 $(limit_message "$(http_date 3600)")" ok
expect "publish: rate limit past the deadline" 1 "would pass the deadline" env RELEASE_PUBLISH_DEADLINE=600 PATH="$stub/bin:$PATH" scripts/release.sh --publish "v$v"
check "publish: past the deadline: no wait" sleeps_are ""
check "publish: past the deadline: one run" cargo_runs 1
check "publish: past the deadline: run again explained" grep -qF "the versions already published are skipped" "$tmp/last"

reset_stub
indexed alpha 0.1.0
indexed beta 0.1.0
plan "exists 0 crate alpha@$v already exists on crates.io index" ok
expect "publish: already exists, retried" 0 "published at $v: alpha beta" publish "v$v"
check "publish: already exists: waited" sleeps_are "30 "

reset_stub
indexed alpha 0.1.0
indexed beta 0.1.0
plan "fail 1 the remote server responded with an error (status 403 Forbidden): this token does not have the required permissions" ok
expect "publish: other failure stops" 1 "not on a rate limit" publish "v$v"
check "publish: other failure: cargo's error shown" grep -qF "status 403 Forbidden" "$tmp/last"
check "publish: other failure: one run" cargo_runs 1
check "publish: other failure: no wait" sleeps_are ""
check "publish: other failure: trusted publishing named" grep -qF "scripts/release.sh --trusted-publishing NAME" "$tmp/last"

reset_stub
indexed alpha 0.1.0 "$v"
expect "publish: first version of a named crate" 0 "published at $v: beta" publish "v$v" beta
check "publish: named crate: cargo publish -p" cargo_run_is 1 "cargo publish --locked -p beta"
check "publish: named crate in the index" in_index beta "$v"
expect "publish: named crates, nothing left" 0 "every crate is on crates.io at $v" publish "v$v" alpha beta
check "publish: named crates: no second run" cargo_runs 1
git checkout --quiet main

tp_url=https://crates.io/api/v1/trusted_publishing/github_configs
empty_list='{"github_configs":[],"meta":{"total":0,"next_page":null}}'
# config ID CRATE OWNER WORKFLOW ENVIRONMENT: a configuration as the API lists it.
config() {
    printf '{"id":%s,"crate":"%s","repository_owner":"%s","repository_owner_id":1,"repository_name":"project","workflow_filename":"%s","environment":%s,"created_at":"2026-01-01T00:00:00Z"}' \
        "$1" "$2" "$3" "$4" "$5"
}
listing() { printf '{"github_configs":[%s],"meta":{"total":1,"next_page":null}}' "$1"; }
trusted() { stubbed scripts/release.sh --trusted-publishing "$@"; }
empty_home=$tmp/cargo-home-empty
file_home=$tmp/cargo-home
mkdir -p "$empty_home" "$file_home"
printf '[registry]\ntoken = "file-token"\n' >"$file_home/credentials.toml"

reset_stub
expect "trusted: no token" 1 "no crates.io token" env -u CARGO_REGISTRY_TOKEN CARGO_HOME="$empty_home" PATH="$stub/bin:$PATH" scripts/release.sh --trusted-publishing
expect "trusted: crate not publishable" 1 "gamma is not a publishable crate" env CARGO_REGISTRY_TOKEN=t PATH="$stub/bin:$PATH" scripts/release.sh --trusted-publishing gamma
check "trusted: refusals sent no request" test ! -s "$stub/requests.log"

reset_stub
respond GET "$tp_url?crate=alpha" 200 "$empty_list" 1
respond GET "$tp_url?crate=alpha" 200 "$(listing "$(config 7 alpha example release.yml '"release"')")" 2
respond POST "$tp_url" 200 "{\"github_config\":$(config 7 alpha example release.yml '"release"')}"
respond GET "$tp_url?crate=beta" 200 "$(listing "$(config 8 beta Example release.yml '"release"')")"
expect "trusted: every crate" 0 "alpha: configured (id 7)" env -u CARGO_REGISTRY_TOKEN CARGO_HOME="$file_home" PATH="$stub/bin:$PATH" scripts/release.sh --trusted-publishing
check "trusted: identical configuration kept" grep -qF "beta: already configured (id 8)" "$tmp/last"
check "trusted: configurations listed back" grep -qF "id 7: example/project, workflow release.yml, environment release" "$tmp/last"
check "trusted: the configuration added" requested "POST $tp_url auth=file-token data={\"github_config\":{\"crate\":\"alpha\",\"repository_owner\":\"example\",\"repository_name\":\"project\",\"workflow_filename\":\"release.yml\",\"environment\":\"release\"}}"
check "trusted: one configuration added" test "$(grep -c '^POST ' "$stub/requests.log")" -eq 1
check "trusted: token of the credentials file" test "$(grep -vc 'auth=file-token' "$stub/requests.log")" -eq 0
check "trusted: token not on curl's command line" lacks "file-token" "$stub/argv.log"

reset_stub
# Same workflow in any environment, or another workflow in the environment:
# neither is the configuration wanted.
others="$(config 9 beta example release.yml null),$(config 11 beta example ci.yml '"release"')"
respond GET "$tp_url?crate=beta" 200 "$(listing "$others")" 1
respond GET "$tp_url?crate=beta" 200 "$(listing "$others,$(config 10 beta example release.yml '"release"')")" 2
respond POST "$tp_url" 200 "{\"github_config\":$(config 10 beta example release.yml '"release"')}"
expect "trusted: other configurations are not enough" 0 "beta: configured (id 10)" env CARGO_REGISTRY_TOKEN=env-token PATH="$stub/bin:$PATH" scripts/release.sh --trusted-publishing beta
check "trusted: CARGO_REGISTRY_TOKEN used" requested "POST $tp_url auth=env-token"
check "trusted: other configurations listed" grep -qF "id 9: example/project, workflow release.yml, environment any" "$tmp/last"
check "trusted: other configurations listed (environment)" grep -qF "id 11: example/project, workflow ci.yml, environment release" "$tmp/last"
check "trusted: named crate only" lacks "crate=alpha" "$stub/requests.log"

reset_stub
respond GET "$tp_url?crate=alpha" 200 "$empty_list"
respond POST "$tp_url" 403 '{"errors":[{"detail":"this token does not have the trusted-publishing scope"}]}'
expect "trusted: refused by crates.io" 1 "HTTP 403: this token does not have the trusted-publishing scope" env CARGO_REGISTRY_TOKEN=t PATH="$stub/bin:$PATH" scripts/release.sh --trusted-publishing alpha
respond POST "$tp_url" 200 "{\"github_config\":$(config 7 alpha example release.yml '"release"')}"
expect "trusted: not listed back" 1 "the configuration added is not listed back" env CARGO_REGISTRY_TOKEN=t PATH="$stub/bin:$PATH" scripts/release.sh --trusted-publishing alpha
respond GET "$tp_url?crate=alpha" 404 '{"errors":[{"detail":"crate not found"}]}'
expect "trusted: crate not on crates.io" 1 "HTTP 404: crate not found" env CARGO_REGISTRY_TOKEN=t PATH="$stub/bin:$PATH" scripts/release.sh --trusted-publishing alpha

sed -i.bak 's|^repository = .*|repository = "https://gitlab.com/example/project"|' Cargo.toml && rm Cargo.toml.bak
expect "trusted: repository not on GitHub" 1 "is not https://github.com/OWNER/NAME" env CARGO_REGISTRY_TOKEN=t PATH="$stub/bin:$PATH" scripts/release.sh --trusted-publishing
git checkout --quiet -- Cargo.toml
check "publishing tests left the tree clean" tree_clean

# ---- changelog.sh ------------------------------------------------------------

expect "changelog: no section" 1 "no section [9.9.9]" scripts/changelog.sh section 9.9.9
expect "changelog: has-entries" 0 "" scripts/changelog.sh has-entries Unreleased
expect "changelog: release refuses an existing version" 2 "already exists" scripts/changelog.sh release 0.1.0 "$today"
expect "changelog: release refuses a bad date" 2 "YYYY-MM-DD" scripts/changelog.sh release 0.3.0 07/11/2026
expect "changelog: unknown command" 2 "" scripts/changelog.sh frobnicate

# guard: commits on a topic branch, compared with their parent.
g=$tmp/guard
git init --quiet "$g"
cd "$g"
mkdir -p src docs crates/x/tests tests
cp "$source_dir/changelog.sh" .
echo 'fn main() {}' >src/main.rs
git add -A
git commit --quiet -m "Without a changelog"
first=$(git rev-parse HEAD)
guard() { ./changelog.sh guard "$@"; }

echo '// code' >>src/main.rs
cat >CHANGELOG.md <<'EOF'
# Changelog

## [Unreleased]

### Added

- Something.
EOF
git add -A && git commit --quiet -m "Code and a new changelog"
expect "guard: new changelog with entries" 0 "was updated" guard "$first" HEAD

base=$(git rev-parse HEAD)
echo doc >README.md && echo doc >docs/guide.txt && echo t >tests/a.txt && echo t >crates/x/tests/b.rs
echo 'MIT' >LICENSE
git add -A && git commit --quiet -m "Docs and tests"
expect "guard: docs and tests only" 0 "only documentation and tests changed" guard "$base" HEAD

base=$(git rev-parse HEAD)
echo '// more' >>src/main.rs
git add -A && git commit --quiet -m "Code only"
expect "guard: code without entry" 1 "src/main.rs" guard "$base" HEAD
check "guard: message names the label" grep -qF "no-changelog" "$tmp/last"

git reset --quiet --hard "$base"
echo '// more' >>src/main.rs
sed -i.bak 's/^# Changelog$/# Changes/' CHANGELOG.md && rm CHANGELOG.md.bak
git add -A && git commit --quiet -m "Code, changelog edited outside Unreleased"
expect "guard: edit outside Unreleased" 1 "did not" guard "$base" HEAD

git reset --quiet --hard "$base"
echo '// more' >>src/main.rs
echo '- Something else.' >>CHANGELOG.md
git add -A && git commit --quiet -m "Code and an entry"
expect "guard: code with entry" 0 "was updated" guard "$base" HEAD

git reset --quiet --hard "$base"
echo '// more' >>src/main.rs
sed -i.bak '/^- Something\.$/d' CHANGELOG.md && rm CHANGELOG.md.bak
git add -A && git commit --quiet -m "Code, Unreleased emptied"
expect "guard: Unreleased emptied" 1 "has no entries" guard "$base" HEAD

git reset --quiet --hard "$base"
git mv src/main.rs src/lib.rs
git commit --quiet -m "Rename"
expect "guard: rename counts" 1 "src/lib.rs" guard "$base" HEAD
expect "guard: bad revision" 2 "not a commit" guard "$base" no-such-revision

printf '\n%d passed, %d failed\n' "$passed" "$failed"
[ "$failed" -eq 0 ]
