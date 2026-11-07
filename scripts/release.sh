#!/usr/bin/env bash
# Prepares a release (see RELEASING.md): sets the workspace version, moves the
# Unreleased entries of CHANGELOG.md into a dated section for that version,
# then makes a signed commit and a signed annotated tag vVERSION. It never
# pushes: it prints the push command.
#
#   scripts/release.sh VERSION             prepare the release, commit and tag
#   scripts/release.sh --dry-run VERSION   every check, then show the changes
#                                          that would be committed; changes nothing
#   scripts/release.sh --verify TAG [FILE] check that the tag vX.Y.Z, the workspace
#                                          version and the CHANGELOG.md section agree,
#                                          and write the release notes to FILE
#                                          (stdout without FILE); used by the
#                                          release workflow, needs no Rust toolchain
#
# VERSION is a SemVer 2.0.0 version without the `v` and without build
# metadata, greater than the current workspace version. The release is
# refused unless HEAD is the branch main, the working tree is clean (no
# untracked files either), main is identical to origin/main after a fetch,
# the tag does not exist locally or on origin, the Unreleased section of
# CHANGELOG.md has entries, every pin of upstream.toml matches the repository
# (scripts/freshness.sh pins) and scripts/check.sh passes.
#
# The version lives in [workspace.package] of Cargo.toml; members inherit it
# (version.workspace = true) and their path dependencies on each other carry
# it too (inline tables: `name = { path = "...", version = "X" }`). The new
# section starts with the official rule sets the release was tested against:
# the upstreams of upstream.toml with severity "high" that are in use.
#
# The changes are prepared and checked in a temporary worktree first, so a
# failure leaves the working tree untouched.
#
# Exit status: 0 done, 1 refused or failed, 2 usage error.
set -euo pipefail
export LC_ALL=C

die() {
    printf 'release: %s\n' "$*" >&2
    exit 1
}
usage() { sed -n '2,/^set -euo pipefail$/{/^set /d;s/^# \{0,1\}//;p;}' "$0"; }
say() { printf '\n\033[1m== %s\033[0m\n' "$*"; }

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
changelog() { "$script_dir/changelog.sh" "$@"; }

semver_re='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(\.(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*)?$'
is_semver() { [[ $1 =~ $semver_re ]]; }

# 0 if version $1 has a higher precedence than version $2 (SemVer 2.0.0,
# section 11), 1 otherwise. Both must be valid.
semver_gt() {
    local a=$1 b=$2 a_pre="" b_pre="" i x y
    local -a a_core b_core a_ids b_ids
    [[ $a == *-* ]] && a_pre=${a#*-}
    [[ $b == *-* ]] && b_pre=${b#*-}
    IFS=. read -ra a_core <<<"${a%%-*}"
    IFS=. read -ra b_core <<<"${b%%-*}"
    for i in 0 1 2; do
        # Numeric identifiers have no leading zeros (checked by is_semver).
        if [ ${#a_core[i]} -ne ${#b_core[i]} ]; then
            [ ${#a_core[i]} -gt ${#b_core[i]} ]
            return
        fi
        if [[ ${a_core[i]} != "${b_core[i]}" ]]; then
            [[ ${a_core[i]} > ${b_core[i]} ]]
            return
        fi
    done
    # Same major.minor.patch: a pre-release is lower than the release.
    [ -z "$a_pre" ] && [ -z "$b_pre" ] && return 1
    [ -z "$a_pre" ] && return 0
    [ -z "$b_pre" ] && return 1
    IFS=. read -ra a_ids <<<"$a_pre"
    IFS=. read -ra b_ids <<<"$b_pre"
    for ((i = 0; ; i++)); do
        [ "$i" -ge ${#b_ids[@]} ] && { [ "$i" -lt ${#a_ids[@]} ]; return; }
        [ "$i" -ge ${#a_ids[@]} ] && return 1
        x=${a_ids[i]} y=${b_ids[i]}
        [[ $x == "$y" ]] && continue
        if [[ $x =~ ^[0-9]+$ ]] && [[ $y =~ ^[0-9]+$ ]]; then
            if [ ${#x} -ne ${#y} ]; then
                [ ${#x} -gt ${#y} ]
                return
            fi
            [[ $x > $y ]]
            return
        fi
        # A numeric identifier is lower than an alphanumeric one.
        [[ $x =~ ^[0-9]+$ ]] && return 1
        [[ $y =~ ^[0-9]+$ ]] && return 0
        [[ $x > $y ]]
        return
    done
}

# The version of [workspace.package] in $1/Cargo.toml.
workspace_version() {
    awk '
        /^\[/ { inside = ($0 == "[workspace.package]") }
        inside && /^version *= *"/ { v = $0; sub(/^version *= *"/, "", v); sub(/".*/, "", v); print v; n++ }
        END { if (n != 1) exit 1 }
    ' "$1/Cargo.toml"
}

# The tracked member manifests of the workspace in $1 (every Cargo.toml but
# the root one).
member_manifests() { git -C "$1" ls-files -- '*/Cargo.toml'; }

# Sets the workspace version of the tree $1 to $2, and the version of every
# path dependency in its manifests.
set_version() {
    local dir=$1 version=$2 manifest tmp
    tmp=$(mktemp)
    awk -v v="$version" '
        /^\[/ { inside = ($0 == "[workspace.package]") }
        inside && /^version *= *"/ { print "version = \"" v "\""; next }
        { print }
    ' "$dir/Cargo.toml" >"$tmp"
    cat "$tmp" >"$dir/Cargo.toml"
    rm -f "$tmp"
    local -a manifests
    mapfile -t manifests < <(member_manifests "$dir")
    for manifest in Cargo.toml "${manifests[@]}"; do
        sed -E -i.bak "/path *= *\"/s/(version *= *\")[^\"]*\"/\\1$version\"/" "$dir/$manifest"
        rm -f "$dir/$manifest.bak"
    done
}

# Checks that every version recorded in the tree $1 is $2: the workspace
# version, every member's (inherited), every path dependency, and the
# workspace packages in Cargo.lock.
check_versions() {
    local dir=$1 version=$2 current manifest names="" name bad=""
    local -a manifests
    current=$(workspace_version "$dir") || die "Cargo.toml: no single version in [workspace.package]"
    [ "$current" = "$version" ] || die "Cargo.toml: the workspace version is $current, not $version"
    mapfile -t manifests < <(member_manifests "$dir")
    [ ${#manifests[@]} -gt 0 ] || die "no member manifest (*/Cargo.toml) is tracked"
    for manifest in "${manifests[@]}"; do
        name=$(awk '
            /^\[/ { inside = ($0 == "[package]") }
            inside && /^name *= *"/ { n = $0; sub(/^name *= *"/, "", n); sub(/".*/, "", n); print n }
        ' "$dir/$manifest")
        [ -n "$name" ] || die "$manifest: no package name"
        names="$names $name"
        awk '/^\[/ { inside = ($0 == "[package]") } inside && /^version *= *"/ { found = 1 } END { exit !found }' \
            "$dir/$manifest" && die "$manifest: the package must inherit the version (version.workspace = true)"
        bad=$(grep -nE 'path *= *"' "$dir/$manifest" | grep -E 'version *= *"' |
            grep -vE "version *= *\"$version\"" || true)
        [ -z "$bad" ] || die "$manifest: path dependency not at version $version: $bad"
    done
    bad=$(grep -nE 'path *= *"' "$dir/Cargo.toml" | grep -E 'version *= *"' |
        grep -vE "version *= *\"$version\"" || true)
    [ -z "$bad" ] || die "Cargo.toml: path dependency not at version $version: $bad"
    [ -f "$dir/Cargo.lock" ] || die "Cargo.lock is missing"
    bad=$(awk -v want="$version" -v names="$names " '
        function flush() {
            if (name != "" && !registry && index(names, " " name " ")) {
                seen[name] = 1
                if (ver != want) out = out " " name "@" ver
            }
        }
        /^\[\[package\]\]$/ { flush(); name = ""; ver = ""; registry = 0; next }
        /^name = "/ { name = $0; sub(/^name = "/, "", name); sub(/"$/, "", name) }
        /^version = "/ { ver = $0; sub(/^version = "/, "", ver); sub(/"$/, "", ver) }
        /^source = / { registry = 1 }
        END {
            flush()
            n = split(names, list, " ")
            for (i = 1; i <= n; i++) if (!(list[i] in seen)) out = out " " list[i] "@missing"
            print out
        }
    ' "$dir/Cargo.lock")
    [ -z "$bad" ] || die "Cargo.lock: workspace packages not at version $version:$bad"
}

# The official rule sets the release is tested against: "NAME VERSION, ..."
# for the upstreams of upstream.toml with severity "high" and no `unused`.
# Reads the subset of TOML that scripts/freshness.sh accepts (it checks the
# file first): value() is the string of a key = "value" or 'value' line.
tested_rule_sets() {
    awk '
        function value(line,   quote, rest) {
            sub(/^[^=]*=[ \t]*/, "", line)
            quote = substr(line, 1, 1)
            rest = substr(line, 2)
            return substr(rest, 1, index(rest, quote) - 1)
        }
        function flush() {
            if (upstream && severity == "high" && unused == "" && name != "")
                out = out (out == "" ? "" : ", ") name " " pinned
        }
        /^[ \t]*\[\[upstream\]\]/ { flush(); upstream = 1; pin = 0; name = severity = pinned = unused = ""; next }
        /^[ \t]*\[\[upstream\.pin\]\]/ { pin = 1; next }
        upstream && !pin && /^[ \t]*name[ \t]*=/ { name = value($0) }
        upstream && !pin && /^[ \t]*severity[ \t]*=/ { severity = value($0) }
        upstream && !pin && /^[ \t]*pinned[ \t]*=/ { pinned = value($0) }
        upstream && !pin && /^[ \t]*unused[ \t]*=/ { unused = value($0) }
        END { flush(); print out }
    ' "$1/upstream.toml"
}

# Release notes of $1 from the CHANGELOG.md of tree $2: the section body and
# the link to the full changelog.
release_notes() {
    local version=$1 dir=$2 body link
    body=$(changelog -f "$dir/CHANGELOG.md" section "$version") || return 1
    link=$(changelog -f "$dir/CHANGELOG.md" link "$version") || return 1
    printf '%s\n\n**Full changelog**: %s\n' "$body" "$link"
}

# Checks that CHANGELOG.md of tree $2 has a dated section for $1 with entries
# and its compare link.
check_changelog_section() {
    local version=$1 dir=$2 file=$2/CHANGELOG.md link
    [ -f "$file" ] || die "CHANGELOG.md is missing"
    awk -v v="$version" '
        index($0, "## [" v "] - ") == 1 && substr($0, length(v) + 9) ~ /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]$/ { n++ }
        END { exit n != 1 }
    ' "$file" || die "CHANGELOG.md: no single heading '## [$version] - YYYY-MM-DD'"
    changelog -f "$file" has-entries "$version" || die "CHANGELOG.md: the section [$version] has no entries"
    link=$(changelog -f "$file" link "$version") || die "CHANGELOG.md: no single link reference [$version]"
    case $link in
    */compare/v*...v"$version" | */releases/tag/v"$version") ;;
    *) die "CHANGELOG.md: the link [$version] does not point at v$version: $link" ;;
    esac
}

verify() {
    local tag=$1 notes=${2:-} version
    version=${tag#v}
    if [[ $tag != v* ]] || ! is_semver "$version"; then
        die "the tag must be vX.Y.Z (SemVer, no build metadata): $tag"
    fi
    cd "$(git rev-parse --show-toplevel)"
    check_versions . "$version"
    check_changelog_section "$version" .
    if [ -n "$notes" ]; then
        release_notes "$version" . >"$notes" || die "cannot write the release notes"
    else
        release_notes "$version" . || die "cannot extract the release notes"
    fi
    echo "release: tag $tag, workspace version and CHANGELOG.md section agree" >&2
}

prepare() {
    local version=$1 dry_run=$2 tag current branch remote_head note today tmp wt
    tag=v$version
    is_semver "$version" || die "not a SemVer version (X.Y.Z or X.Y.Z-pre, no 'v', no build metadata): $version"
    cd "$(git rev-parse --show-toplevel)"
    command -v cargo >/dev/null 2>&1 || die "cargo is not installed"

    say "preconditions"
    branch=$(git symbolic-ref --quiet --short HEAD) || die "HEAD is detached; check out main"
    [ "$branch" = main ] || die "on branch $branch; releases are made from main"
    [ -z "$(git status --porcelain --untracked-files=all)" ] ||
        die "the working tree is not clean (git status): commit, stash or remove every change, untracked files included"
    git fetch --quiet origin main || die "git fetch origin main failed"
    remote_head=$(git rev-parse --verify --quiet refs/remotes/origin/main) || die "no origin/main after the fetch"
    [ "$(git rev-parse HEAD)" = "$remote_head" ] ||
        die "main is not identical to origin/main (git status -sb): pull or push first"
    current=$(workspace_version .) || die "Cargo.toml: no single version in [workspace.package]"
    check_versions . "$current"
    semver_gt "$version" "$current" || die "$version is not greater than the current version $current"
    if git rev-parse --verify --quiet "refs/tags/$tag" >/dev/null; then die "tag $tag already exists"; fi
    local remote_tags
    remote_tags=$(git ls-remote --tags origin "refs/tags/$tag") || die "git ls-remote origin failed"
    [ -z "$remote_tags" ] || die "tag $tag already exists on origin"
    changelog has-entries Unreleased || die "CHANGELOG.md: the Unreleased section has no entries"
    if changelog section "$version" >/dev/null 2>&1; then die "CHANGELOG.md already has a section [$version]"; fi
    echo "on main at origin/main, clean; $current -> $version; tag $tag is free; Unreleased has entries"

    say "upstream pins"
    scripts/freshness.sh pins || die "scripts/freshness.sh pins failed"
    note=$(tested_rule_sets .)
    [ -n "$note" ] || die "upstream.toml: no official rule set in use (severity \"high\" without \"unused\")"
    note="Tested against the official rule sets: $note."
    echo "$note"

    say "scripts/check.sh"
    scripts/check.sh || die "scripts/check.sh failed"
    [ -z "$(git status --porcelain --untracked-files=all)" ] ||
        die "scripts/check.sh left changes in the working tree (git status)"

    say "prepare $tag"
    today=$(date -u +%F)
    tmp=$(mktemp -d)
    wt=$tmp/worktree
    # shellcheck disable=SC2064 # expand now: the paths are fixed
    trap "git worktree remove --force '$wt' >/dev/null 2>&1 || true; rm -rf '$tmp'" EXIT
    git worktree add --quiet --detach "$wt" HEAD || die "git worktree add failed"
    set_version "$wt" "$version"
    (cd "$wt" && cargo update --workspace --offline --quiet) || die "cargo update --workspace failed"
    changelog -f "$wt/CHANGELOG.md" release "$version" "$today" "$note" >"$tmp/CHANGELOG.md" ||
        die "cannot rewrite CHANGELOG.md"
    cat "$tmp/CHANGELOG.md" >"$wt/CHANGELOG.md"
    check_versions "$wt" "$version"
    check_changelog_section "$version" "$wt"
    (cd "$wt" && cargo metadata --locked --offline --format-version 1 >/dev/null) ||
        die "Cargo.lock does not match the manifests after the version change"
    release_notes "$version" "$wt" >"$tmp/notes.md" || die "cannot extract the release notes"
    printf 'einvoice-validator %s\n\n' "$version" >"$tmp/tag-message"
    cat "$tmp/notes.md" >>"$tmp/tag-message"

    local -a changed
    mapfile -t changed < <(git -C "$wt" diff --name-only)
    [ ${#changed[@]} -gt 0 ] || die "nothing changed in the worktree"
    git -C "$wt" diff --stat

    if [ "$dry_run" = 1 ]; then
        say "dry run: the changes"
        git -C "$wt" --no-pager diff
        say "dry run: the tag message"
        cat "$tmp/tag-message"
        say "dry run: nothing changed"
        echo "Would commit \"Release $version\" (signed), tag $tag (signed, annotated), then print:"
        echo "  git push --atomic origin main $tag"
        return 0
    fi

    say "commit and tag"
    local path
    for path in "${changed[@]}"; do
        cp "$wt/$path" "$path"
    done
    git add -- "${changed[@]}"
    git commit --quiet -S -m "Release $version" || die "git commit failed; the changes are staged (git reset --hard HEAD drops them)"
    grep -q '^gpgsig' < <(git cat-file commit HEAD) ||
        die "the release commit is not signed; undo it with: git reset --hard HEAD~1"
    git tag -s -a --cleanup=whitespace -F "$tmp/tag-message" "$tag" ||
        die "git tag failed; the release commit stays (undo it with: git reset --hard HEAD~1)"
    grep -q -- '-----BEGIN .*SIGNATURE-----' < <(git cat-file tag "$tag") || {
        git tag -d "$tag" >/dev/null
        die "the tag was not signed and has been deleted; the release commit stays (undo it with: git reset --hard HEAD~1)"
    }
    git --no-pager log --oneline -1
    echo "release: $tag created. Check it, then push the commit and the tag together:"
    echo "  git push --atomic origin main $tag"
}

dry_run=0
case ${1:-} in
--verify)
    [ $# -eq 2 ] || [ $# -eq 3 ] || {
        usage >&2
        exit 2
    }
    verify "$2" "${3:-}"
    ;;
--dry-run)
    [ $# -eq 2 ] || {
        usage >&2
        exit 2
    }
    dry_run=1
    prepare "$2" "$dry_run"
    ;;
-h | --help)
    usage
    ;;
-* | "")
    usage >&2
    exit 2
    ;;
*)
    [ $# -eq 1 ] || {
        usage >&2
        exit 2
    }
    prepare "$1" "$dry_run"
    ;;
esac
