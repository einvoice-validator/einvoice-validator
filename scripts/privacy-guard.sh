#!/usr/bin/env bash
# Privacy guard: this repository is public. It must never contain private
# terms (names of downstream users, their projects, people, providers) nor
# local paths, in files, file names, commit messages, tags, notes, ref names,
# or the text published on GitHub (issues, pull requests, comments, releases).
#
# The list of private terms is deliberately NOT in the repository (that would
# publish it). It is read from, in order:
#   1. $PRIVATE_TERMS             (newline-separated; the CI repository secret)
#   2. $PRIVATE_TERMS_FILE        (relative to the current directory)
#   3. ${XDG_CONFIG_HOME:-~/.config}/einvoice-validator/denylist
# Blank lines and lines starting with '#' are ignored; surrounding whitespace
# is trimmed; each line is a fixed string matched case-insensitively. A term
# of several words also matches when its words are split across lines (in
# commit messages, published text and staged files).
#
# Generic patterns (absolute home paths on Linux, macOS and Windows) are
# public and checked as well.
#
# The guard fails closed: exit 0 only when it could read everything it was
# asked to check and found nothing; exit 1 when it found something; exit 2
# when it could not look (no term list, unreadable input, empty text, a git
# or grep error, ...).
#
# Hits are reported as file:line, commit id, line number or entry number
# only, never with the matching text, so that CI logs do not publish what
# they caught. A location that itself contains a private term (a file name)
# is withheld.
#
# Usage:
#   scripts/privacy-guard.sh staged          # index content, names, identity (pre-commit hook)
#   scripts/privacy-guard.sh message FILE    # a commit message (commit-msg hook)
#   scripts/privacy-guard.sh tree            # HEAD's tracked files
#   scripts/privacy-guard.sh history         # every commit, tag, note and ref name (pre-push hook, CI)
#   scripts/privacy-guard.sh text            # text on stdin: titles, bodies, comments, commands
set -Eeuo pipefail
shopt -s inherit_errexit

die() {
    echo "privacy-guard: $*; refusing to pass" >&2
    exit 2
}
# Any command that fails unexpectedly stops the guard with exit 2: a failure
# must never turn into a pass, nor into a misleading "found" (exit 1).
trap 'die "unexpected error (line $LINENO)"' ERR

usage() {
    echo "usage: $0 staged | message FILE | tree | history | text" >&2
    exit 2
}

mode=${1:-}
case "$mode" in
staged | tree | history | text) [ $# -eq 1 ] || usage ;;
message) [ $# -eq 2 ] || usage ;;
*) usage ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# readable <description> <path>: the path must be a readable regular file.
readable() {
    [ -f "$2" ] || die "$1 is not a file: $2"
    [ -r "$2" ] || die "$1 is not readable: $2"
}

# Inputs are read before anything changes the current directory, so that
# relative paths mean what the caller meant.
case "$mode" in
message)
    readable "commit message file" "$2"
    cat -- "$2" >"$tmp/input" || die "cannot read the commit message file"
    ;;
text)
    [ ! -t 0 ] || die "text mode reads the text on stdin, not from a terminal"
    cat >"$tmp/input" || die "cannot read stdin"
    [ -s "$tmp/input" ] || die "no text on stdin"
    ;;
esac

config_list="${XDG_CONFIG_HOME:-$HOME/.config}/einvoice-validator/denylist"
if [ -n "${PRIVATE_TERMS:-}" ]; then
    printf '%s\n' "$PRIVATE_TERMS" >"$tmp/terms.raw"
elif [ -n "${PRIVATE_TERMS_FILE:-}" ]; then
    readable "PRIVATE_TERMS_FILE" "$PRIVATE_TERMS_FILE"
    cat -- "$PRIVATE_TERMS_FILE" >"$tmp/terms.raw" || die "cannot read PRIVATE_TERMS_FILE"
elif [ -e "$config_list" ]; then
    readable "the private term list" "$config_list"
    cat -- "$config_list" >"$tmp/terms.raw" || die "cannot read the private term list"
else
    die "no private term list found (PRIVATE_TERMS, PRIVATE_TERMS_FILE or ~/.config/einvoice-validator/denylist)"
fi
list="$tmp/terms"
# Drop CRs, surrounding whitespace, comments and blank lines (an empty pattern
# would match everything); join each whitespace run inside a term into one
# space, the form the line-joined scans compare against.
sed -e 's/\r$//' -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' \
    -e '/^#/d' -e '/^$/d' -e 's/[[:space:]]\{1,\}/ /g' "$tmp/terms.raw" >"$list"
[ -s "$list" ] || die "the private term list is empty"

# Public, generic patterns: absolute paths into someone's home. Each is
# written so that its own source text does not match it (the bracketed
# letter), otherwise this file would trip the guard.
generic='(/hom[e]/[A-Za-z0-9_.-]+|/User[s]/[A-Za-z0-9_.-]+|[A-Za-z]:\\User[s]\\[A-Za-z0-9_.-]+)'

# has <grep arguments>: 0 when grep matches, 1 when it does not; a grep
# error (exit 2: unreadable file, bad pattern, ...) stops the guard.
has() {
    local status=0
    grep -q "$@" || status=$?
    [ "$status" -le 1 ] || die "grep failed (exit $status)"
    return "$status"
}
has_term() { has -aiF -f "$list" -- "$1"; }
has_home() { has -aE -- "$generic" "$1"; }
# A term split across lines: the same file with every whitespace run
# (newlines included) joined into one space.
has_term_joined() {
    tr -s '[:space:]' ' ' <"$1" >"$tmp/joined"
    has_term "$tmp/joined"
}

# report runs at the end of pipelines, i.e. in a subshell: it records a hit
# in a file, since a shell variable set there would be lost. A location that
# contains a private term or a home path itself (a file name) is withheld.
report() { # report <label> ; reads locations on stdin
    local label=$1 loc
    while IFS= read -r loc; do
        [ -n "$loc" ] || continue
        printf '%s\n' "$loc" >"$tmp/location"
        if has_term "$tmp/location" || has_home "$tmp/location"; then
            loc="(location withheld: it contains a private term or a home path)"
        fi
        echo "privacy-guard: $label: $loc" >&2
        : >"$tmp/found"
    done
}

# search <label> <fields> <git grep arguments>: report the first <fields>
# colon-separated fields of every hit (file:line or rev:file:line, never the
# matched text). git grep exits 1 when nothing matches and above 1 when it
# fails; it exits 0 or 1 when it cannot read an object, though, and only
# says so on stderr: any message there stops the guard as well.
search() {
    local label=$1 fields=$2 status=0
    shift 2
    git grep "$@" >"$tmp/hits" 2>"$tmp/grep.err" || status=$?
    [ ! -s "$tmp/grep.err" ] || die "$label: git grep could not read everything (exit $status; see git fsck)"
    case $status in
    0) cut -d: -f"1-$fields" "$tmp/hits" | report "$label" ;;
    1) ;;
    *) die "$label: git grep failed (exit $status)" ;;
    esac
}

# symlinks <where> <file of "mode oid<TAB>path" lines>: git grep does not
# search symbolic links, so their targets are read here.
symlinks() {
    local where=$1 meta path mode oid
    while IFS=$'\t' read -r meta path; do
        read -r mode oid _ <<<"$meta"
        [ "$mode" = 120000 ] || continue
        git cat-file blob "$oid" >"$tmp/link"
        whole "the target of a symbolic link $where" "$path" "$tmp/link"
    done <"$2"
}

# entries <tree-ish> <file>: "mode oid<TAB>path" for every file of the tree.
entries() {
    git ls-tree -r --format='%(objectmode) %(objectname)%x09%(path)' "$1" >"$2"
}

# lines <label> <file> <grep arguments>: report the numbers of the lines of
# <file> that match (never the lines: they may be file or ref names).
lines() {
    local label=$1 file=$2 status=0
    shift 2
    grep -n "$@" -- "$file" >"$tmp/hits" || status=$?
    case $status in
    0) cut -d: -f1 "$tmp/hits" | report "$label" ;;
    1) ;;
    *) die "$label: grep failed (exit $status)" ;;
    esac
}

# prose <what> <file>: a commit message, published text, a tag: line numbers
# of terms and home paths, plus terms split across lines.
prose() {
    local what=$1 file=$2
    lines "private term in $what, line" "$file" -aiF -f "$list"
    lines "home path in $what, line" "$file" -aE "$generic"
    if ! has_term "$file" && has_term_joined "$file"; then
        echo "(split across lines)" | report "private term in $what"
    fi
}

# whole <label> <id> <file>: report <id> when the file holds a term or a home
# path anywhere, split across lines included.
whole() {
    local label=$1 id=$2 file=$3
    if has_term "$file" || has_term_joined "$file"; then echo "$id" | report "private term in $label"; fi
    if has_home "$file"; then echo "$id" | report "home path in $label"; fi
}

case "$mode" in
staged | tree | history)
    root=$(git rev-parse --show-toplevel) || die "not inside a git work tree"
    cd "$root"
    ;;
esac

case "$mode" in
text)
    prose "text" "$tmp/input"
    ;;
message)
    # Every line, '#' lines included: `git commit -m` and the verbatim and
    # whitespace cleanup modes keep them in the commit.
    prose "commit message" "$tmp/input"
    ;;
staged)
    search "private term in staged file" 2 --cached -naiF -f "$list"
    search "home path in staged file" 2 --cached -naE "$generic"
    # Terms split across lines, in the files this commit adds or changes.
    git diff --cached --raw -z --no-abbrev --no-renames --diff-filter=AM >"$tmp/changed"
    while IFS= read -r -d '' header && IFS= read -r -d '' path; do
        read -r _ newmode _ newoid _ <<<"$header"
        [ "$newmode" != 160000 ] || continue # a submodule: no blob to read
        git cat-file blob "$newoid" >"$tmp/blob"
        if ! has_term "$tmp/blob" && has_term_joined "$tmp/blob"; then
            echo "$path" | report "private term split across lines in staged file"
        fi
    done <"$tmp/changed"
    git ls-files --cached >"$tmp/names"
    lines "private term in a staged file name, entry of git ls-files" "$tmp/names" -aiF -f "$list"
    git ls-files --stage >"$tmp/entries" # mode oid stage<TAB>path
    symlinks "in the index" "$tmp/entries"
    # The identity the commit will carry is published too.
    git var GIT_AUTHOR_IDENT >"$tmp/ident" || die "cannot determine the author identity (set user.name and user.email)"
    git var GIT_COMMITTER_IDENT >>"$tmp/ident" || die "cannot determine the committer identity (set user.name and user.email)"
    if has_term "$tmp/ident"; then
        echo "git config user.name / user.email" | report "private term in author/committer identity"
    fi
    ;;
tree)
    git rev-parse --verify --quiet 'HEAD^{commit}' >/dev/null || die "no HEAD commit to scan"
    search "private term in file" 3 -naiF -f "$list" HEAD
    search "home path in file" 3 -naE "$generic" HEAD
    git ls-tree -r --name-only HEAD >"$tmp/names"
    lines "private term in a file name, entry of git ls-tree -r --name-only HEAD" "$tmp/names" -aiF -f "$list"
    entries HEAD "$tmp/entries"
    symlinks "in HEAD" "$tmp/entries"
    ;;
history)
    git rev-list --all >"$tmp/revs"
    mapfile -t revs <"$tmp/revs"
    if [ ${#revs[@]} -gt 0 ]; then
        # Every tree of every ref, refs/notes/* (git notes) included.
        search "private term in file" 3 -naiF -f "$list" "${revs[@]}"
        search "home path in file" 3 -naE "$generic" "${revs[@]}"
        for rev in "${revs[@]}"; do
            git ls-tree -r --name-only "$rev" >"$tmp/names"
            lines "private term in a file name of $rev, entry of git ls-tree -r --name-only" "$tmp/names" -aiF -f "$list"
            entries "$rev" "$tmp/entries"
            symlinks "in $rev" "$tmp/entries"
            git log -1 --format='%B' "$rev" >"$tmp/msg"
            whole "commit message" "$rev" "$tmp/msg"
            git log -1 --format='%an%n%ae%n%cn%n%ce' "$rev" >"$tmp/ident"
            if has_term "$tmp/ident"; then echo "$rev" | report "private term in author/committer identity"; fi
        done
    fi
    # Annotated tags: message and tagger, reported by tag object id.
    git for-each-ref --format='%(objecttype) %(objectname)' refs/tags >"$tmp/tags"
    while read -r kind oid; do
        [ "$kind" = tag ] || continue
        git cat-file tag "$oid" >"$tmp/tag"
        whole "annotated tag" "$oid" "$tmp/tag"
    done <"$tmp/tags"
    # Branch and tag names are published by a push.
    git for-each-ref --format='%(refname)' >"$tmp/refs"
    lines "private term in a ref name, entry of git for-each-ref" "$tmp/refs" -aiF -f "$list"
    ;;
esac

if [ -e "$tmp/found" ]; then
    echo "privacy-guard: FAILED. This repository is public: remove the private references above." >&2
    exit 1
fi
echo "privacy-guard: ok ($mode)"
