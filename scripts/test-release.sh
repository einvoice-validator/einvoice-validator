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
# dry run (nothing changes), two real releases (versions, Cargo.lock,
# changelog sections and links, signed commit and tag, nothing pushed),
# --verify, the rule sets noted in the release (release-name for a name that
# ends with a version), and the pull-request check of changelog.sh. The repository's
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
mkdir -p scripts crates/alpha/src crates/beta/src crates/beta/tests rules
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
echo 'pub fn a() {}' >crates/alpha/src/lib.rs
echo 'pub fn b() { alpha::a() }' >crates/beta/src/lib.rs
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
check "0.1.0: only the version files changed" test "$(git diff --name-only HEAD~1 HEAD | tr '\n' ' ')" = "CHANGELOG.md Cargo.lock Cargo.toml crates/beta/Cargo.toml "
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
