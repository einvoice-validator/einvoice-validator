#!/usr/bin/env bash
# Privacy guard: this repository is public. It must never contain private
# terms (names of downstream users, their projects, people, providers) nor
# local paths, in files, file names or commit messages.
#
# The list of private terms is deliberately NOT in the repository (that would
# publish it). It is read from, in order:
#   1. $PRIVATE_TERMS             (newline-separated; the CI repository secret)
#   2. $PRIVATE_TERMS_FILE
#   3. ~/.config/einvoice-validator/denylist
# Blank lines and lines starting with '#' are ignored; each line is a fixed
# string matched case-insensitively. With no list found the guard FAILS:
# it never passes silently.
#
# Generic patterns (absolute home paths) are public and checked as well.
#
# Matches are reported as file:line (or commit id) only, never with the
# matching text, so that CI logs do not publish what they caught.
#
# Usage:
#   scripts/privacy-guard.sh staged          # index content (pre-commit hook)
#   scripts/privacy-guard.sh message FILE    # a commit message (commit-msg hook)
#   scripts/privacy-guard.sh tree            # HEAD's tracked files
#   scripts/privacy-guard.sh history         # every commit: content, paths, messages, authors
set -euo pipefail

mode=${1:-}
cd "$(git rev-parse --show-toplevel)"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
list="$tmp/terms"

if [ -n "${PRIVATE_TERMS:-}" ]; then
    printf '%s\n' "$PRIVATE_TERMS" >"$list.raw"
elif [ -n "${PRIVATE_TERMS_FILE:-}" ]; then
    cp "$PRIVATE_TERMS_FILE" "$list.raw"
elif [ -f "${XDG_CONFIG_HOME:-$HOME/.config}/einvoice-validator/denylist" ]; then
    cp "${XDG_CONFIG_HOME:-$HOME/.config}/einvoice-validator/denylist" "$list.raw"
else
    echo "privacy-guard: no private term list found (PRIVATE_TERMS, PRIVATE_TERMS_FILE or ~/.config/einvoice-validator/denylist); refusing to pass" >&2
    exit 2
fi
# Drop comments, blank lines and CRs: an empty pattern would match everything.
sed -e 's/\r$//' -e '/^[[:space:]]*#/d' -e '/^[[:space:]]*$/d' "$list.raw" >"$list"
if [ ! -s "$list" ]; then
    echo "privacy-guard: the private term list is empty; refusing to pass" >&2
    exit 2
fi

# Public, generic patterns: absolute paths into someone's home. Each is
# written so that its own source text does not match it (the bracketed
# letter), otherwise this file would trip the guard.
generic='(/hom[e]/[A-Za-z0-9_.-]+|/User[s]/[A-Za-z0-9_.-]+)'

# report runs at the end of pipelines, i.e. in a subshell: it records a hit
# in a file, since a shell variable set there would be lost.
report() { # report <label> ; reads locations on stdin
    local label=$1 loc
    while IFS= read -r loc; do
        [ -n "$loc" ] || continue
        echo "privacy-guard: $label: $loc" >&2
        : >"$tmp/found"
    done
}

# Keep only "file:line" (or "rev:file:line"): never print the matched text.
locations() { # locations <number of leading fields>
    cut -d: -f"1-$1"
}

case "$mode" in
staged)
    git grep --cached -nIiF -f "$list" | locations 2 | report "private term in staged file" || true
    git grep --cached -nIE "$generic" | locations 2 | report "home path in staged file" || true
    git diff --cached --name-only --diff-filter=ACR | grep -iF -f "$list" | report "private term in file name" || true
    # The identity the commit will carry is published too.
    ident=$(git var GIT_AUTHOR_IDENT; git var GIT_COMMITTER_IDENT)
    if printf '%s\n' "$ident" | grep -qiF -f "$list"; then echo "git config user.name / user.email" | report "private term in author/committer identity"; fi
    ;;
message)
    msg=${2:?commit message file}
    # Ignore git's own comment lines in the message template.
    grep -v '^#' "$msg" | grep -niF -f "$list" | locations 1 | sed 's/^/line /' | report "private term in commit message" || true
    grep -v '^#' "$msg" | grep -nE "$generic" | locations 1 | sed 's/^/line /' | report "home path in commit message" || true
    ;;
tree)
    git grep -nIiF -f "$list" HEAD | locations 3 | report "private term in file" || true
    git grep -nIE "$generic" HEAD | locations 3 | report "home path in file" || true
    git ls-tree -r --name-only HEAD | grep -iF -f "$list" | report "private term in file name" || true
    ;;
history)
    revs=$(git rev-list --all)
    if [ -n "$revs" ]; then
        # shellcheck disable=SC2086 # one argument per revision, on purpose
        git grep -nIiF -f "$list" $revs | locations 3 | report "private term in file" || true
        # shellcheck disable=SC2086
        git grep -nIE "$generic" $revs | locations 3 | report "home path in file" || true
        for rev in $revs; do
            git ls-tree -r --name-only "$rev" | grep -iF -f "$list" | sed "s/^/$rev:/" | report "private term in file name" || true
            # Captured first: `git log | grep -q` under pipefail could turn a
            # hit into a miss when grep exits early and git log gets SIGPIPE.
            msg=$(git log -1 --format='%B' "$rev")
            ident=$(git log -1 --format='%an%n%ae%n%cn%n%ce' "$rev")
            if printf '%s\n' "$msg" | grep -qiF -f "$list"; then echo "$rev" | report "private term in commit message"; fi
            if printf '%s\n' "$msg" | grep -qE "$generic"; then echo "$rev" | report "home path in commit message"; fi
            if printf '%s\n' "$ident" | grep -qiF -f "$list"; then echo "$rev" | report "private term in author/committer identity"; fi
        done
    fi
    ;;
*)
    echo "usage: $0 staged | message FILE | tree | history" >&2
    exit 2
    ;;
esac

if [ -e "$tmp/found" ]; then
    echo "privacy-guard: FAILED. This repository is public: remove the private references above." >&2
    exit 1
fi
echo "privacy-guard: ok ($mode)"
