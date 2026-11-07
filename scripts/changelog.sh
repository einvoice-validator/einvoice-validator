#!/usr/bin/env bash
# Reads and rewrites CHANGELOG.md (Keep a Changelog 1.1: `## [Unreleased]`,
# then `## [X.Y.Z] - YYYY-MM-DD` sections, newest first, and link reference
# definitions `[name]: URL` at the end). Used by scripts/release.sh, the
# release workflow and the changelog check on pull requests.
#
#   scripts/changelog.sh [-f FILE] section NAME
#       print the body of section NAME (`Unreleased` or a version), without
#       leading and trailing blank lines; exit 1 if there is no such section
#   scripts/changelog.sh [-f FILE] has-entries NAME
#       exit 0 if section NAME has at least one list item, 1 otherwise
#   scripts/changelog.sh [-f FILE] link NAME
#       print the URL of the link reference definition [NAME]; exit 1 if none
#   scripts/changelog.sh [-f FILE] release VERSION DATE [NOTE]
#       print the changelog with the entries of Unreleased moved to a new
#       section `## [VERSION] - DATE` (NOTE, if given, as its first
#       paragraph), Unreleased left empty, and the compare links updated:
#       [Unreleased] -> vVERSION...HEAD, [VERSION] -> vPREVIOUS...vVERSION
#       (or the release page of vVERSION for the first release)
#   scripts/changelog.sh guard BASE HEAD
#       pull request check: fails if a file other than documentation or tests
#       changed between the commits BASE and HEAD while the Unreleased section
#       of CHANGELOG.md did not (see is_exempt below for what is exempt)
#
# FILE defaults to CHANGELOG.md in the current directory. Exit status: 0 ok,
# 1 the answer is no (missing section, no entries, guard failed), 2 error.
set -euo pipefail

die() {
    printf 'changelog: %s\n' "$*" >&2
    exit 2
}
usage() { sed -n '2,/^set -euo pipefail$/{/^set /d;s/^# \{0,1\}//;p;}' "$0"; }

file=CHANGELOG.md
if [ "${1:-}" = -f ]; then
    [ $# -ge 2 ] || die "-f needs a file"
    file=$2
    shift 2
fi
[ $# -ge 1 ] || {
    usage >&2
    exit 2
}
command=$1
shift

# Body of section $1 of file $2: the lines after its heading, up to the next
# `## ` heading or the first link reference definition, trimmed of blank lines
# at both ends. Exit 1 if the heading is absent, 2 if it appears twice.
section_of() {
    awk -v name="$1" '
        function is_heading(line) {
            return line == "## [" name "]" || index(line, "## [" name "] - ") == 1
        }
        is_heading($0) { found++; inside = 1; next }
        inside && (/^## / || /^\[[^]]+\]: /) { inside = 0 }
        inside { lines[++n] = $0 }
        END {
            if (found == 0) exit 1
            if (found > 1) exit 2
            first = 1
            while (first <= n && lines[first] ~ /^[[:space:]]*$/) first++
            last = n
            while (last >= first && lines[last] ~ /^[[:space:]]*$/) last--
            for (i = first; i <= last; i++) print lines[i]
        }
    ' "$2"
}

has_entries() {
    local body
    body=$(section_of "$1" "$2") || return $?
    grep -Eq '^[[:space:]]*[-*] +[^[:space:]]' <<<"$body"
}

link_of() {
    awk -v name="$1" '
        index($0, "[" name "]: ") == 1 { found++; url = substr($0, length(name) + 5) }
        END {
            if (found != 1) exit 1
            print url
        }
    ' "$2"
}

# Files exempt from the changelog requirement: documentation and tests.
is_exempt() {
    case $1 in
    CHANGELOG.md) return 0 ;;
    *.md | docs/* | LICENSE | LICENSE.* | LICENSE-*) return 0 ;;
    tests/* | */tests/*) return 0 ;;
    *) return 1 ;;
    esac
}

need_file() { [ -f "$file" ] || die "$file: no such file"; }

case $command in
section)
    [ $# -eq 1 ] || die "usage: section NAME"
    need_file
    status=0
    section_of "$1" "$file" || status=$?
    case $status in
    0) ;;
    1)
        printf 'changelog: %s: no section [%s]\n' "$file" "$1" >&2
        exit 1
        ;;
    *) die "$file: section [$1] appears more than once" ;;
    esac
    ;;
has-entries)
    [ $# -eq 1 ] || die "usage: has-entries NAME"
    need_file
    status=0
    has_entries "$1" "$file" || status=$?
    case $status in
    0) ;;
    1) exit 1 ;;
    *) die "$file: section [$1] appears more than once" ;;
    esac
    ;;
link)
    [ $# -eq 1 ] || die "usage: link NAME"
    need_file
    link_of "$1" "$file" || {
        printf 'changelog: %s: no single link reference [%s]\n' "$file" "$1" >&2
        exit 1
    }
    ;;
release)
    [ $# -eq 2 ] || [ $# -eq 3 ] || die "usage: release VERSION DATE [NOTE]"
    version=$1 date=$2 note=${3:-}
    need_file
    [[ $date =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die "date must be YYYY-MM-DD: $date"
    [ "$(grep -c '^## \[Unreleased\]$' "$file")" -eq 1 ] ||
        die "$file: there must be exactly one '## [Unreleased]' heading"
    if section_of "$version" "$file" >/dev/null 2>&1; then
        die "$file: a section [$version] already exists"
    fi
    has_entries Unreleased "$file" || die "$file: the Unreleased section has no entries"
    unreleased_link=$(link_of Unreleased "$file") ||
        die "$file: no single link reference [Unreleased]"
    [[ $unreleased_link =~ ^(https://github\.com/[A-Za-z0-9._-]+/[A-Za-z0-9._-]+)/(compare/v[^/]+\.\.\.HEAD|commits/[A-Za-z0-9._/-]+)$ ]] ||
        die "$file: [Unreleased] must link to https://github.com/OWNER/REPO/compare/vX.Y.Z...HEAD or .../commits/BRANCH"
    repo=${BASH_REMATCH[1]}
    # The latest release: the first version heading after Unreleased.
    previous=$(awk '/^## \[/ && $0 != "## [Unreleased]" {
        sub(/^## \[/, ""); sub(/\].*/, ""); print; exit }' "$file")
    if [ -n "$previous" ]; then
        link_of "$previous" "$file" >/dev/null ||
            die "$file: no single link reference [$previous]"
    fi
    awk -v ver="$version" -v date="$date" -v note="$note" -v repo="$repo" -v prev="$previous" '
        $0 == "## [Unreleased]" {
            print
            print ""
            print "## [" ver "] - " date
            if (note != "") { print ""; print note }
            next
        }
        index($0, "[Unreleased]: ") == 1 {
            print "[Unreleased]: " repo "/compare/v" ver "...HEAD"
            if (prev != "") print "[" ver "]: " repo "/compare/v" prev "...v" ver
            else print "[" ver "]: " repo "/releases/tag/v" ver
            next
        }
        { print }
    ' "$file"
    ;;
guard)
    [ $# -eq 2 ] || die "usage: guard BASE HEAD"
    base=$1 head=$2
    git rev-parse --verify --quiet "$base^{commit}" >/dev/null || die "not a commit: $base"
    git rev-parse --verify --quiet "$head^{commit}" >/dev/null || die "not a commit: $head"
    changed=$(git diff --no-renames --name-only "$base" "$head") || die "git diff failed"
    needs=()
    while IFS= read -r path; do
        [ -n "$path" ] || continue
        is_exempt "$path" || needs+=("$path")
    done <<<"$changed"
    if [ ${#needs[@]} -eq 0 ]; then
        echo "changelog: only documentation and tests changed; no entry needed"
        exit 0
    fi
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    # Unreleased before and after; a missing file or section counts as empty.
    for side in base head; do
        rev=$base
        [ $side = head ] && rev=$head
        : >"$tmp/$side"
        if git cat-file -e "$rev:CHANGELOG.md" 2>/dev/null; then
            git show "$rev:CHANGELOG.md" >"$tmp/$side.md" || die "git show $rev:CHANGELOG.md failed"
            status=0
            section_of Unreleased "$tmp/$side.md" >"$tmp/$side" || status=$?
            case $status in
            0 | 1) ;;
            *) die "CHANGELOG.md at $rev: section [Unreleased] appears more than once" ;;
            esac
        fi
    done
    if cmp -s "$tmp/base" "$tmp/head"; then
        echo "changelog: these files changed, but the Unreleased section of CHANGELOG.md did not:" >&2
        printf '  %s\n' "${needs[@]}" >&2
        echo "Add an entry under [Unreleased], or, for a change users cannot notice, ask a" >&2
        echo "maintainer to add the label no-changelog to the pull request." >&2
        exit 1
    fi
    if ! has_entries Unreleased "$tmp/head.md"; then
        echo "changelog: the Unreleased section of CHANGELOG.md changed but has no entries" >&2
        exit 1
    fi
    echo "changelog: the Unreleased section of CHANGELOG.md was updated"
    ;;
-h | --help | help)
    usage
    ;;
*)
    usage >&2
    exit 2
    ;;
esac
