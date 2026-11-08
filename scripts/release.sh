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
#   scripts/release.sh --publish TAG [CRATE...]
#                                          publish on crates.io, from the tagged
#                                          commit, every publishable crate (or the
#                                          CRATEs) whose version is not there yet;
#                                          used by the release workflow
#   scripts/release.sh --trusted-publishing [CRATE...]
#                                          configure crates.io trusted publishing
#                                          from the release workflow for every
#                                          publishable crate (or the CRATEs), then
#                                          list the configurations back
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
# it too (inline tables: `name = { path = "...", version = "X" }`), and so does
# every tracked npm package manifest (package.json, its top-level "version":
# the npm package built from a crate is released with the crates). The new
# section starts with the official rule sets the release was tested against:
# the upstreams of upstream.toml with severity "high" that are in use, each
# as its `release-name` (or else its `name`) and its pinned version.
#
# The changes are prepared and checked in a temporary worktree first, so a
# failure leaves the working tree untouched.
#
# --publish checks that HEAD is the tagged commit and that the versions agree
# with the tag, then reads the crates.io index: a crate whose version is
# already there is skipped, so a failed run can simply be run again. Without
# CRATE it publishes every publishable crate of the workspace (publish not
# set to false) with `cargo publish --locked --workspace`, in dependency
# order, and refuses a crate that is not on crates.io yet: trusted publishing
# can only publish new versions of an existing crate, so the first version of
# a crate is published by hand, by naming it (`cargo publish --locked -p`,
# with the maintainer's own token). A refusal of crates.io's rate limits
# (HTTP 429) is retried after the time crates.io gives, else with an
# exponential backoff, at most 10 attempts and only until
# RELEASE_PUBLISH_DEADLINE seconds after the start (default 1500: the token of
# trusted publishing lives 30 minutes); when cargo finds a version already
# published that the index read did not show, the index is read again after
# 30 s. Any other failure stops at once. The token is cargo's (CARGO_REGISTRY_TOKEN in the workflow).
#
# --trusted-publishing adds, for each crate, the configuration that lets the
# workflow .github/workflows/release.yml of the GitHub repository named by
# `repository` in [workspace.package], in its environment "release", publish
# the crate; an identical configuration is kept as it is. It authenticates
# with CARGO_REGISTRY_TOKEN, else the token of [registry] in
# $CARGO_HOME/credentials.toml (cargo login), which must be allowed the
# trusted-publishing scope. Only owners of a crate can configure it, and only
# once the crate exists on crates.io.
#
# Exit status: 0 done, 1 refused or failed, 2 usage error.
set -euo pipefail

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

# 0 if the strings $1 and $2 differ and $1 sorts after $2 in ASCII order.
# Only `sort` gets LC_ALL=C: the locale of the release, and of the
# scripts/check.sh it runs, is left as it is.
ascii_gt() {
    local first
    first=$(printf '%s\n%s\n' "$1" "$2" | LC_ALL=C sort | sed -n 1p)
    [ "$1" != "$2" ] && [ "$first" = "$2" ]
}

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
            ascii_gt "${a_core[i]}" "${b_core[i]}"
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
            ascii_gt "$x" "$y"
            return
        fi
        # A numeric identifier is lower than an alphanumeric one.
        [[ $x =~ ^[0-9]+$ ]] && return 1
        [[ $y =~ ^[0-9]+$ ]] && return 0
        ascii_gt "$x" "$y"
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

# The tracked npm package manifests of the tree $1 (package.json, at any depth).
npm_manifests() { git -C "$1" ls-files -- 'package.json' '*/package.json'; }

# The top-level "version" of the npm package manifest $1: its single line
# `  "version": "X"` (the member at two spaces of indentation, as npm writes
# it); fails unless there is exactly one.
npm_version() {
    awk '
        /^  "version" *: *"/ { v = $0; sub(/^  "version" *: *"/, "", v); sub(/".*/, "", v); print v; n++ }
        END { if (n != 1) exit 1 }
    ' "$1"
}

# Sets the workspace version of the tree $1 to $2, the version of every path
# dependency in its manifests, and the top-level version of its npm package
# manifests.
set_version() {
    local dir=$1 version=$2 manifest tmp
    tmp=$(mktemp)
    awk -v v="$version" '
        /^\[/ { inside = ($0 == "[workspace.package]") }
        inside && /^version *= *"/ { print "version = \"" v "\""; next }
        { print }
    ' "$dir/Cargo.toml" >"$tmp"
    cat "$tmp" >"$dir/Cargo.toml"
    local -a manifests
    mapfile -t manifests < <(member_manifests "$dir")
    for manifest in Cargo.toml "${manifests[@]}"; do
        sed -E -i.bak "/path *= *\"/s/(version *= *\")[^\"]*\"/\\1$version\"/" "$dir/$manifest"
        rm -f "$dir/$manifest.bak"
    done
    mapfile -t manifests < <(npm_manifests "$dir")
    for manifest in "${manifests[@]}"; do
        npm_version "$dir/$manifest" >/dev/null || {
            rm -f "$tmp"
            die "$manifest: no single top-level \"version\" line"
        }
        awk -v v="$version" '
            /^  "version" *: *"/ { sub(/"version" *: *"[^"]*"/, "\"version\": \"" v "\"") }
            { print }
        ' "$dir/$manifest" >"$tmp"
        cat "$tmp" >"$dir/$manifest"
    done
    rm -f "$tmp"
}

# Checks that every version recorded in the tree $1 is $2: the workspace
# version, every member's (inherited), every path dependency, the workspace
# packages in Cargo.lock and the npm package manifests.
check_versions() {
    local dir=$1 version=$2 current manifest names="" name bad="" npm
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
    mapfile -t manifests < <(npm_manifests "$dir")
    for manifest in "${manifests[@]}"; do
        npm=$(npm_version "$dir/$manifest") || die "$manifest: no single top-level \"version\" line"
        [ "$npm" = "$version" ] || die "$manifest: version $npm, not $version"
    done
}

# The official rule sets the release is tested against: "NAME VERSION, ..."
# for the upstreams of upstream.toml with severity "high" and no `unused`,
# NAME being `release-name` when set (for a name that ends with a version of
# its own: "Peppol BIS Billing 3.0", release 3.0.22), `name` otherwise.
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
                out = out (out == "" ? "" : ", ") (label != "" ? label : name) " " pinned
        }
        /^[ \t]*\[\[upstream\]\]/ { flush(); upstream = 1; pin = 0; name = label = severity = pinned = unused = ""; next }
        /^[ \t]*\[\[upstream\.pin\]\]/ { pin = 1; next }
        upstream && !pin && /^[ \t]*name[ \t]*=/ { name = value($0) }
        upstream && !pin && /^[ \t]*severity[ \t]*=/ { severity = value($0) }
        upstream && !pin && /^[ \t]*pinned[ \t]*=/ { pinned = value($0) }
        upstream && !pin && /^[ \t]*unused[ \t]*=/ { unused = value($0) }
        upstream && !pin && /^[ \t]*release-name[ \t]*=/ { label = value($0) }
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

# ---- crates.io ----------------------------------------------------------------

crates_io=https://crates.io
crates_index=https://index.crates.io
# The workflow and environment that trusted publishing accepts.
release_workflow=release.yml
release_environment=release
user_agent="einvoice-validator-release/1 (scripts/release.sh; +https://github.com/einvoice-validator/einvoice-validator)"
publish_attempts=10

need_tools() {
    local tool
    for tool in "$@"; do
        command -v "$tool" >/dev/null 2>&1 || die "$tool is not installed"
    done
}

# The publishable crates of the workspace: `publish` unset or naming crates-io.
publishable_crates() {
    cargo metadata --locked --no-deps --format-version 1 |
        jq -r '.packages[] | select(.publish == null or (.publish | index("crates-io") != null)) | .name'
}

# select_crates NAME...: sets `selection` to the NAMEs, each of which must be
# a publishable crate, or to every publishable crate without NAME.
select_crates() {
    local list name
    list=$(publishable_crates) || die "cargo metadata failed"
    [ -n "$list" ] || die "the workspace has no publishable crate"
    if [ $# -eq 0 ]; then
        mapfile -t selection <<<"$list"
        return
    fi
    for name in "$@"; do
        grep -qxF -- "$name" <<<"$list" || die "$name is not a publishable crate of the workspace"
    done
    selection=("$@")
}

# index_state NAME VERSION: "published" if the crates.io index has VERSION of
# the crate NAME (yanked or not), "pending" if it has the crate but not that
# version, "new" if it does not have the crate; fails if it cannot be read.
index_state() {
    local name=$1 version=$2 lower path body code versions
    lower=$(printf '%s' "$name" | tr '[:upper:]' '[:lower:]')
    case ${#lower} in
    1) path=1/$lower ;;
    2) path=2/$lower ;;
    3) path=3/${lower:0:1}/$lower ;;
    *) path=${lower:0:2}/${lower:2:2}/$lower ;;
    esac
    body=$(mktemp)
    code=$(curl --proto '=https' --tlsv1.2 -sS --retry 5 --retry-delay 2 -A "$user_agent" \
        -o "$body" -w '%{http_code}' "$crates_index/$path") || {
        rm -f "$body"
        return 1
    }
    case $code in
    200)
        versions=$(jq -r '.vers' <"$body") || {
            rm -f "$body"
            echo "the crates.io index entry of $name is not JSON" >&2
            return 1
        }
        if grep -qxF -- "$version" <<<"$versions"; then echo published; else echo pending; fi
        ;;
    404) echo new ;;
    *)
        rm -f "$body"
        echo "the crates.io index answered HTTP $code for $name" >&2
        return 1
        ;;
    esac
    rm -f "$body"
}

# rate_limit_wait LOG LIMITED NOW: the seconds to wait after the rate limit
# refusal in LOG, the LIMITED-th of the run: until the time crates.io gives
# ("Please try again after <HTTP date>") plus 5 s, else 60 s doubled at each
# refusal, at most 600 s.
rate_limit_wait() {
    local log=$1 limited=$2 now=$3 when until wait
    when=$(sed -n 's/.*[Tt]ry again after \([A-Z][a-z][a-z], [0-9][0-9]* [A-Z][a-z][a-z] [0-9]\{4\} [0-9][0-9]:[0-9][0-9]:[0-9][0-9] GMT\).*/\1/p' "$log" | tail -n 1)
    if [ -n "$when" ] && until=$(date -u -d "$when" +%s 2>/dev/null); then
        wait=$((until - now + 5))
        [ "$wait" -ge 5 ] || wait=5
    else
        wait=$((60 << (limited - 1)))
        [ "$wait" -le 600 ] || wait=600
    fi
    echo "$wait"
}

publish() {
    local tag=$1 version tagged name state attempt status now wait reason deadline limited=0 log
    shift
    local -a named=("$@") selection published pending new args
    version=${tag#v}
    if [[ $tag != v* ]] || ! is_semver "$version"; then
        die "the tag must be vX.Y.Z (SemVer, no build metadata): $tag"
    fi
    [[ ${RELEASE_PUBLISH_DEADLINE:-1500} =~ ^[0-9]+$ ]] || die "RELEASE_PUBLISH_DEADLINE is not a number of seconds"
    deadline=$(($(date +%s) + ${RELEASE_PUBLISH_DEADLINE:-1500}))
    cd "$(git rev-parse --show-toplevel)"
    need_tools cargo jq curl
    tagged=$(git rev-parse --verify --quiet "refs/tags/$tag^{commit}") || die "no tag $tag in this clone"
    [ "$(git rev-parse HEAD)" = "$tagged" ] || die "HEAD is not the commit of $tag: git checkout $tag"
    check_versions . "$version"
    select_crates "$@"
    log=$(mktemp)
    # shellcheck disable=SC2064 # expand now: the path is fixed
    trap "rm -f '$log'" EXIT

    for ((attempt = 1; attempt <= publish_attempts; attempt++)); do
        published=() pending=() new=()
        for name in "${selection[@]}"; do
            state=$(index_state "$name" "$version") || die "cannot read the crates.io index entry of $name"
            case $state in
            published) published+=("$name") ;;
            pending) pending+=("$name") ;;
            *) new+=("$name") ;;
            esac
        done
        if [ ${#named[@]} -eq 0 ] && [ ${#new[@]} -gt 0 ]; then
            die "not on crates.io yet: ${new[*]}. Trusted publishing only publishes new versions of existing crates:" \
                "publish the first version by hand from $tag (scripts/release.sh --publish $tag ${new[*]}, with your own token)," \
                "configure trusted publishing (scripts/release.sh --trusted-publishing ${new[*]}), then run this again;" \
                "nothing was published (RELEASING.md)"
        fi
        pending+=("${new[@]}")
        [ ${#published[@]} -eq 0 ] || echo "release: already on crates.io at $version, skipped: ${published[*]}"
        if [ ${#pending[@]} -eq 0 ]; then
            echo "release: every crate is on crates.io at $version"
            return 0
        fi
        echo "release: publishing at $version, attempt $attempt of $publish_attempts: ${pending[*]}"
        if [ ${#named[@]} -eq 0 ]; then
            args=(--workspace)
            for name in "${published[@]}"; do args+=(--exclude "$name"); done
        else
            args=()
            for name in "${pending[@]}"; do args+=(-p "$name"); done
        fi
        status=0
        cargo publish --locked "${args[@]}" 2>&1 | tee "$log" || status=$?
        if [ "$status" -eq 0 ]; then
            echo "release: published at $version: ${pending[*]}"
            return 0
        fi
        now=$(date +%s)
        if grep -qE 'status 429|Too Many Requests' "$log"; then
            limited=$((limited + 1))
            wait=$(rate_limit_wait "$log" "$limited" "$now")
            reason="crates.io rate limit (HTTP 429)"
        elif grep -qE 'already (exists|uploaded)' "$log"; then
            wait=30
            reason="a version is already on crates.io but not yet in the index read"
        else
            die "cargo publish failed (exit $status), not on a rate limit. If crates.io refused the token for a crate," \
                "configure trusted publishing for it (scripts/release.sh --trusted-publishing NAME); then run this again:" \
                "the versions already published are skipped"
        fi
        if [ $((now + wait)) -gt "$deadline" ]; then
            die "$reason: waiting ${wait}s would pass the deadline (RELEASE_PUBLISH_DEADLINE);" \
                "run this again later: the versions already published are skipped"
        fi
        [ "$attempt" -lt "$publish_attempts" ] || break
        echo "release: $reason; attempt $((attempt + 1)) in ${wait}s"
        sleep "$wait"
    done
    die "not published after $publish_attempts attempts; run this again later: the versions already published are skipped"
}

# The crates.io token: CARGO_REGISTRY_TOKEN, else the token of [registry] in
# cargo's credentials file.
registry_token() {
    local home=${CARGO_HOME:-$HOME/.cargo} file
    if [ -n "${CARGO_REGISTRY_TOKEN:-}" ]; then
        printf '%s\n' "$CARGO_REGISTRY_TOKEN"
        return
    fi
    for file in "$home/credentials.toml" "$home/credentials"; do
        [ -f "$file" ] || continue
        awk '
            /^\[/ { inside = ($0 == "[registry]") }
            inside && /^token *= *"/ { t = $0; sub(/^token *= *"/, "", t); sub(/".*/, "", t); print t; n++ }
            END { exit n != 1 }
        ' "$file"
        return
    done
    return 1
}

# api METHOD PATH [BODY]: a request to the crates.io API with the token in
# $token (sent on curl's standard input, never on its command line), BODY a
# JSON file; prints the response, fails with the API's error unless 2xx.
api() {
    local method=$1 path=$2 data=${3:-} body code detail
    local -a args=(--proto '=https' --tlsv1.2 -sS -A "$user_agent" -X "$method" -H @-)
    if [ -n "$data" ]; then
        args+=(-H 'Content-Type: application/json' --data-binary "@$data")
    else
        args+=(--retry 3 --retry-delay 2)
    fi
    body=$(mktemp)
    code=$(printf 'Authorization: %s\n' "$token" |
        curl "${args[@]}" -o "$body" -w '%{http_code}' "$crates_io$path") || {
        rm -f "$body"
        echo "crates.io: $method $path: request failed" >&2
        return 1
    }
    case $code in
    2??) cat "$body" ;;
    *)
        detail=$(jq -r '[.errors[]?.detail] | join("; ")' <"$body" 2>/dev/null || true)
        echo "crates.io: $method $path: HTTP $code${detail:+: $detail}" >&2
        rm -f "$body"
        return 1
        ;;
    esac
    rm -f "$body"
}

trusted_publishing() {
    local repository owner repo token name list id request filter
    local -a selection
    cd "$(git rev-parse --show-toplevel)"
    need_tools cargo jq curl
    repository=$(awk '
        /^\[/ { inside = ($0 == "[workspace.package]") }
        inside && /^repository *= *"/ { r = $0; sub(/^repository *= *"/, "", r); sub(/".*/, "", r); print r }
    ' Cargo.toml)
    [[ $repository =~ ^https://github\.com/([A-Za-z0-9-]+)/([A-Za-z0-9._-]+)$ ]] ||
        die "Cargo.toml: the repository of [workspace.package] is not https://github.com/OWNER/NAME: $repository"
    owner=${BASH_REMATCH[1]} repo=${BASH_REMATCH[2]%.git}
    [ -f ".github/workflows/$release_workflow" ] || die "no workflow .github/workflows/$release_workflow"
    select_crates "$@"
    token=$(registry_token) || die "no crates.io token: set CARGO_REGISTRY_TOKEN or run cargo login"
    [ -n "$token" ] || die "the crates.io token is empty"
    request=$(mktemp)
    # shellcheck disable=SC2064 # expand now: the path is fixed
    trap "rm -f '$request'" EXIT
    # GitHub owner and repository names are case-insensitive.
    # shellcheck disable=SC2016 # a jq program: $o, $r, $w and $e are jq's
    filter='[.github_configs[] | select((.repository_owner | ascii_downcase) == ($o | ascii_downcase)
        and (.repository_name | ascii_downcase) == ($r | ascii_downcase)
        and .workflow_filename == $w and .environment == $e) | .id] | first // empty'
    echo "release: trusted publishing from $owner/$repo, workflow $release_workflow, environment $release_environment"
    for name in "${selection[@]}"; do
        list=$(api GET "/api/v1/trusted_publishing/github_configs?crate=$name") ||
            die "$name: cannot list its trusted publishing configurations (is the crate on crates.io, and the token allowed the trusted-publishing scope?)"
        id=$(jq -r --arg o "$owner" --arg r "$repo" --arg w "$release_workflow" --arg e "$release_environment" \
            "$filter" <<<"$list") || die "$name: unexpected answer from crates.io"
        if [ -n "$id" ]; then
            echo "$name: already configured (id $id)"
        else
            jq -n --arg c "$name" --arg o "$owner" --arg r "$repo" --arg w "$release_workflow" --arg e "$release_environment" \
                '{github_config: {crate: $c, repository_owner: $o, repository_name: $r, workflow_filename: $w, environment: $e}}' \
                >"$request"
            sleep 1
            api POST /api/v1/trusted_publishing/github_configs "$request" >/dev/null ||
                die "$name: cannot add the configuration"
            sleep 1
            list=$(api GET "/api/v1/trusted_publishing/github_configs?crate=$name") ||
                die "$name: cannot list its trusted publishing configurations"
            id=$(jq -r --arg o "$owner" --arg r "$repo" --arg w "$release_workflow" --arg e "$release_environment" \
                "$filter" <<<"$list") || die "$name: unexpected answer from crates.io"
            [ -n "$id" ] || die "$name: the configuration added is not listed back"
            echo "$name: configured (id $id)"
        fi
        jq -r '.github_configs[] | "  id \(.id): \(.repository_owner)/\(.repository_name), workflow \(.workflow_filename), environment \(.environment // "any"), created \(.created_at)"' \
            <<<"$list"
        sleep 1
    done
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
--publish)
    [ $# -ge 2 ] || {
        usage >&2
        exit 2
    }
    shift
    publish "$@"
    ;;
--trusted-publishing)
    shift
    trusted_publishing "$@"
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
