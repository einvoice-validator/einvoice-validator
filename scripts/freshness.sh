#!/usr/bin/env bash
# Freshness of every upstream the repository pins (upstream.toml): official
# rule sets, standards and tools. Needs bash, curl and jq.
#
#   scripts/freshness.sh pins                  offline: check every pin of upstream.toml
#                                              against the files that record it
#   scripts/freshness.sh                       the pins check, then query every upstream
#                                              and print pinned / latest / status
#   scripts/freshness.sh --json FILE --markdown FILE
#                                              also write the results as JSON and as a
#                                              Markdown table
#
# Statuses: "current"; "OUTDATED" (severity high: an official rule set has a
# newer release); "newer exists" (severity info); "pinned, newer exists"
# (severity info, held on purpose: `hold` in upstream.toml); "ERROR".
#
# Exit status of a query run:
#   0  nothing with severity high is outdated (info items are printed, never fail)
#   1  an upstream with severity high has a newer release
#   2  error: invalid upstream.toml, a pin that does not match the repository,
#      a network or API failure, an unexpected response, a pinned version that
#      upstream does not publish. An upstream that cannot be queried is never
#      reported as current.
# The pins check exits 0 when every pin matches, 2 otherwise.
#
# GitHub is queried through its REST API, with GITHUB_TOKEN (else GH_TOKEN)
# when set: without a token the limit is 60 requests per hour, enough for one
# run. Maven Central is read from its maven-metadata.xml files.
set -euo pipefail

# FRESHNESS_PINFILE replaces upstream.toml (tests: scripts/freshness-test.sh);
# the files its pins name stay relative to the repository root.
pinfile=upstream.toml
if [ -n "${FRESHNESS_PINFILE:-}" ]; then
    case $FRESHNESS_PINFILE in
    /*) pinfile=$FRESHNESS_PINFILE ;;
    *) pinfile=$PWD/$FRESHNESS_PINFILE ;;
    esac
fi

cd "$(git rev-parse --show-toplevel)"
# Releases are listed 100 per page; paging stops at the pinned version.
max_pages=10

usage() { sed -n '2,/^set -euo pipefail$/{/^set /d;s/^# \{0,1\}//;p;}' "$0"; }
die() {
    printf 'freshness: %s\n' "$*" >&2
    exit 2
}

mode=query json_out='' md_out=''
while [ $# -gt 0 ]; do
    case $1 in
    pins) mode=pins ;;
    --json | --markdown)
        [ $# -ge 2 ] || die "$1 needs a file name"
        if [ "$1" = --json ]; then json_out=$2; else md_out=$2; fi
        shift
        ;;
    -h | --help)
        usage
        exit 0
        ;;
    *) die "unknown argument: $1 (see --help)" ;;
    esac
    shift
done

for tool in curl jq; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool is not installed"
done

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# --- upstream.toml ------------------------------------------------------------

# Reads the TOML subset described at the top of upstream.toml into a JSON
# array of entries, refusing anything else, then validates the keys.
read -r -d '' parse_jq <<'JQ' || true
def fail($n; $msg): error("\($file):\($n): \($msg)");
def kv:
  [capture("^(?<k>[a-z][a-z0-9-]*)[ \t]*=[ \t]*(?:\"(?<b>[^\"\\\\]*)\"|'(?<l>[^']*)')[ \t]*(?:#.*)?$")]
  | first;
def known($list): . as $x | any($list[]; . == $x);
def default_pattern($source):
  if $source == "github-release" then "^v?(?<v>[0-9]+(?:\\.[0-9]+)*)$"
  elif $source == "maven" then "^(?<v>[0-9]+(?:\\.[0-9]+)*(?:-[0-9]+)?)$"
  else null end;

# Parse.
(reduce inputs as $raw ({n: 0, entries: [], in: null};
  .n += 1
  | .n as $n
  | ($raw | sub("^[ \t]+"; "") | sub("[ \t\r]+$"; "")) as $t
  | if $t == "" or ($t | startswith("#")) then .
    elif $t == "[[upstream]]" then
      .entries += [{line: $n, keys: {}, pins: []}] | .in = "upstream"
    elif $t == "[[upstream.pin]]" then
      if .in == null then fail($n; "[[upstream.pin]] before any [[upstream]]")
      else .entries[(.entries | length) - 1].pins += [{line: $n, keys: {}}] | .in = "pin" end
    else
      ($t | kv) as $m
      | if $m == null then
          fail($n; "expected [[upstream]], [[upstream.pin]], a comment or key = \"string\"")
        elif .in == null then fail($n; "key outside a table")
        else
          (.entries | length - 1) as $e
          | ($m.b // $m.l) as $v
          | if .in == "upstream" then
              if .entries[$e].keys | has($m.k) then fail($n; "duplicate key \($m.k)")
              else .entries[$e].keys[$m.k] = $v end
            else
              (.entries[$e].pins | length - 1) as $p
              | if .entries[$e].pins[$p].keys | has($m.k) then fail($n; "duplicate key \($m.k)")
                else .entries[$e].pins[$p].keys[$m.k] = $v end
            end
        end
    end)
 | .entries)
# Validate and normalise.
| if length == 0 then error("\($file): no [[upstream]]") else . end
| map(
    .line as $n | .keys as $k
    | ($k | keys | map(select(known(["name", "severity", "pinned", "source", "repository",
        "group", "artifact", "url", "version-pattern", "series", "hold", "unused"]) | not)))
        as $unknown
    | if ($unknown | length) > 0 then fail($n; "unknown key \($unknown | join(", "))")
      elif ([$k.name, $k.severity, $k.pinned, $k.source] | any(. == null or . == "")) then
        fail($n; "name, severity, pinned and source are required")
      elif ($k.severity | known(["high", "info"]) | not) then
        fail($n; "severity must be \"high\" or \"info\"")
      elif ($k.pinned | test("^[0-9][0-9A-Za-z.+-]*$") | not) then
        fail($n; "pinned must be a version, not \"\($k.pinned)\"")
      elif ($k.source | known(["github-release", "maven", "listing"]) | not) then
        fail($n; "source must be github-release, maven or listing")
      else . end
    | (if $k.source == "github-release" then [["repository"], ["group", "artifact", "url"]]
       elif $k.source == "maven" then [["group", "artifact"], ["repository", "url"]]
       else [["url", "version-pattern"], ["repository", "group", "artifact"]] end) as [$need, $deny]
    | ($k["version-pattern"] // default_pattern($k.source)) as $pattern
    | if any($need[]; $k[.] == null or $k[.] == "") then
        fail($n; "source \($k.source) needs \($need | join(" and "))")
      elif any($deny[]; $k[.] != null) then
        fail($n; "source \($k.source) takes none of \($deny | join(", "))")
      elif ($pattern | contains("(?<v>") | not) then
        fail($n; "version-pattern needs a group named v: (?<v>...)")
      elif $k.hold != null and $k.severity != "info" then
        fail($n; "hold is for severity info only")
      elif $k.unused != null and (.pins | length) > 0 then
        fail($n; "unused, yet [[upstream.pin]] tables follow")
      elif $k.unused == null and (.pins | length) == 0 then
        fail($n; "no [[upstream.pin]]: name the files that record the pin, or say unused = \"...\"")
      else . end
    | {
        line: $n, name: $k.name, severity: $k.severity, pinned: $k.pinned, source: $k.source,
        repository: $k.repository, group: $k.group, artifact: $k.artifact, url: $k.url,
        pattern: $pattern, series: $k.series, hold: $k.hold, unused: $k.unused,
        query: (if $k.source == "github-release" then "https://api.github.com/repos/\($k.repository)/releases"
                elif $k.source == "maven" then
                  "https://repo1.maven.org/maven2/\($k.group | gsub("\\."; "/"))/\($k.artifact)/maven-metadata.xml"
                else $k.url end),
        link: (if $k.source == "github-release" then "https://github.com/\($k.repository)/releases"
               elif $k.source == "maven" then "https://central.sonatype.com/artifact/\($k.group)/\($k.artifact)/versions"
               else $k.url end),
        pins: (.pins | map(
          .line as $pn | .keys as $pk
          | ($pk | keys | map(select(known(["file", "pattern", "compare"]) | not))) as $unknown
          | if ($unknown | length) > 0 then fail($pn; "unknown key \($unknown | join(", "))")
            elif $pk.file == null or $pk.pattern == null then fail($pn; "file and pattern are required")
            elif ($pk.pattern | contains("(?<v>") | not) then fail($pn; "pattern needs a group named v: (?<v>...)")
            elif ($pk.compare // "equal" | known(["equal", "prefix"]) | not) then
              fail($pn; "compare must be \"equal\" or \"prefix\"")
            else {line: $pn, file: $pk.file, pattern: $pk.pattern, compare: ($pk.compare // "equal")} end))
      })
| (group_by(.name) | map(select(length > 1) | .[1])) as $dups
| if ($dups | length) > 0 then error("\($file):\($dups[0].line): duplicate name \($dups[0].name)") else . end
JQ

entries=$tmp/entries.json
[ -f "$pinfile" ] || die "$pinfile not found"
jq -nR --arg file "$pinfile" "$parse_jq" "$pinfile" >"$entries" 2>"$tmp/parse.err" ||
    die "$(sed 's/^jq: error (at [^)]*): //' "$tmp/parse.err")"
count=$(jq length "$entries")

# --- the pins against the repository ------------------------------------------

# Every match of a pin's pattern in its file, compared with the pinned version.
read -r -d '' pin_jq <<'JQ' || true
[match($pattern; "g") | .captures[] | select(.name == "v" and .string != null) | .string] as $found
| [$found[] | . as $v | select($v != $pinned and ($compare != "prefix" or ($pinned | startswith($v + ".") | not)))]
  as $bad
| if ($found | length) == 0 then "no match of \($pattern)"
  elif ($bad | length) > 0 then "found \($bad | unique | join(", ")), expected \($pinned)"
  else "ok \($found | length)" end
JQ

# Prints one line per pin with --verbose; returns 1 if any pin fails.
check_pins() {
    local verbose=${1:-} i j n name pinned file pattern compare line result failed=0 total=0
    for ((i = 0; i < count; i++)); do
        name=$(jq -r --argjson i "$i" '.[$i].name' "$entries")
        pinned=$(jq -r --argjson i "$i" '.[$i].pinned' "$entries")
        n=$(jq --argjson i "$i" '.[$i].pins | length' "$entries")
        if [ "$n" -eq 0 ]; then
            [ -z "$verbose" ] || printf 'unused   %-36s %-10s %s\n' "$name" "$pinned" \
                "$(jq -r --argjson i "$i" '.[$i].unused' "$entries")"
            continue
        fi
        for ((j = 0; j < n; j++)); do
            file=$(jq -r --argjson i "$i" --argjson j "$j" '.[$i].pins[$j].file' "$entries")
            pattern=$(jq -r --argjson i "$i" --argjson j "$j" '.[$i].pins[$j].pattern' "$entries")
            compare=$(jq -r --argjson i "$i" --argjson j "$j" '.[$i].pins[$j].compare' "$entries")
            line=$(jq -r --argjson i "$i" --argjson j "$j" '.[$i].pins[$j].line' "$entries")
            total=$((total + 1))
            if ! git ls-files --error-unmatch -- "$file" >/dev/null 2>&1; then
                result="$file is not a file tracked by git"
            elif ! result=$(jq -Rsr --arg pattern "$pattern" --arg pinned "$pinned" \
                --arg compare "$compare" "$pin_jq" "$file" 2>&1); then
                result="cannot apply the pattern: $result"
            fi
            case $result in
            ok\ *) [ -z "$verbose" ] || printf 'ok       %-36s %-10s %s: %s match(es) (%s:%s)\n' "$name" \
                "$pinned" "$file" "${result#ok }" "$pinfile" "$line" ;;
            *)
                printf 'DRIFT    %-36s %-10s %s: %s (%s:%s)\n' "$name" "$pinned" "$file" "$result" \
                    "$pinfile" "$line" >&2
                failed=$((failed + 1))
                ;;
            esac
        done
    done
    if [ "$failed" -gt 0 ]; then
        printf 'freshness: %d of %d pins do not match the repository; update %s or the files\n' \
            "$failed" "$total" "$pinfile" >&2
        return 1
    fi
    printf 'pins: %d pins of %d upstreams match the repository\n' "$total" "$count"
}

if [ "$mode" = pins ]; then
    check_pins verbose || exit 2
    exit 0
fi
check_pins || exit 2

# --- querying the upstreams ---------------------------------------------------

token=${GITHUB_TOKEN:-${GH_TOKEN:-}}

# The content of a message file on one line.
oneline() { tr -s '\n' ' ' <"$1" | sed 's/ *$//'; }

# GET over HTTPS only; the token, when any, goes to api.github.com only and
# through curl's standard input, never on its command line.
fetch() {
    local url=$1
    local -a args=(--proto '=https' --proto-redir '=https' --tlsv1.2 --fail --silent --show-error
        --location --retry 3 --connect-timeout 10 --max-time 60
        --user-agent einvoice-validator-freshness)
    case $url in
    https://api.github.com/*)
        args+=(--header 'Accept: application/vnd.github+json' --header 'X-GitHub-Api-Version: 2022-11-28')
        if [ -n "$token" ]; then
            printf 'header = "Authorization: Bearer %s"\n' "$token" | curl "${args[@]}" --config - "$url"
            return
        fi
        ;;
    esac
    curl "${args[@]}" "$url"
}

# Candidate versions of one upstream, as a JSON array into $2.
read -r -d '' release_page_jq <<'JQ' || true
if type != "array" then error("not a list of releases")
else $acc[0] + [.[] | select((.draft | not) and (.prerelease | not)) | .tag_name
     | [match($pattern) | .captures[] | select(.name == "v") | .string] | first // empty]
end
JQ

versions_github() {
    local i=$1 out=$2 base pattern pinned page=1 items found
    base=$(jq -r --argjson i "$i" '.[$i].query' "$entries")
    pattern=$(jq -r --argjson i "$i" '.[$i].pattern' "$entries")
    pinned=$(jq -r --argjson i "$i" '.[$i].pinned' "$entries")
    echo '[]' >"$out"
    while :; do
        if ! fetch "$base?per_page=100&page=$page" >"$tmp/page.json" 2>"$tmp/curl.err"; then
            local hint=
            if [ -z "$token" ] && grep -qE 'error: (403|429)' "$tmp/curl.err"; then
                hint=" (no GITHUB_TOKEN: the API allows 60 requests per hour)"
            fi
            echo "cannot list $base (page $page): $(oneline "$tmp/curl.err")$hint"
            return 1
        fi
        if ! jq --arg pattern "$pattern" --slurpfile acc "$out" "$release_page_jq" \
            "$tmp/page.json" >"$tmp/acc.json" 2>"$tmp/jq.err"; then
            echo "unexpected response from $base: $(oneline "$tmp/jq.err")"
            return 1
        fi
        mv "$tmp/acc.json" "$out"
        items=$(jq length "$tmp/page.json")
        found=$(jq --arg v "$pinned" 'any(.[]; . == $v)' "$out")
        if [ "$found" = true ] || [ "$items" -lt 100 ] || [ "$page" -ge "$max_pages" ]; then
            return 0
        fi
        page=$((page + 1))
    done
}

# maven-metadata.xml lists one <version> per line; a listing page is matched whole.
read -r -d '' maven_jq <<'JQ' || true
if test("<metadata[ >]") | not then error("not a maven-metadata.xml")
else [match("<version>([^<]*)</version>"; "g") | .captures[0].string
      | [match($pattern) | .captures[] | select(.name == "v") | .string] | first // empty]
end
JQ
read -r -d '' listing_jq <<'JQ' || true
[match($pattern; "g") | .captures[] | select(.name == "v" and .string != null) | .string]
JQ

versions_page() {
    local i=$1 out=$2 program=$3 url pattern
    url=$(jq -r --argjson i "$i" '.[$i].query' "$entries")
    pattern=$(jq -r --argjson i "$i" '.[$i].pattern' "$entries")
    if ! fetch "$url" >"$tmp/page.txt" 2>"$tmp/curl.err"; then
        echo "cannot fetch $url: $(oneline "$tmp/curl.err")"
        return 1
    fi
    if ! jq -Rs --arg pattern "$pattern" "$program" "$tmp/page.txt" >"$out" 2>"$tmp/jq.err"; then
        echo "unexpected content at $url: $(oneline "$tmp/jq.err")"
        return 1
    fi
}

# One result row from the entry and its candidate versions (or an error).
read -r -d '' assess_jq <<'JQ' || true
def key: [scan("[0-9]+") | tonumber];
$entries[$i] as $e
| {name: $e.name, severity: $e.severity, pinned: $e.pinned, link: $e.link, hold: $e.hold}
| if $error != "" then . + {latest: "?", status: "ERROR", note: $error, actionable: false}
  else
    ($versions | unique | sort_by(key)) as $s
    | ($s | map(select(key > ($e.pinned | key))) | length) as $newer
    | (if $e.series == null then null
       else $s | map(select(startswith($e.series + "."))) | last end) as $series_latest
    | if ($s | length) == 0 then
        . + {latest: "?", status: "ERROR", actionable: false,
             note: "no published version matches version-pattern at \($e.query)"}
      elif (any($s[]; . == $e.pinned) | not) then
        . + {latest: $s[-1], status: "ERROR", actionable: false,
             note: "pinned \($e.pinned) is not among the \($s | length) versions published at \($e.query)"}
      elif $e.series != null and $series_latest == null then
        . + {latest: $s[-1], status: "ERROR", actionable: false,
             note: "no \($e.series).x version published at \($e.query)"}
      else
        . + {latest: $s[-1]}
        | . + (if ($s[-1] | key) == ($e.pinned | key) then {status: "current", actionable: false}
               elif $e.severity == "high" then {status: "OUTDATED", actionable: true}
               elif $e.hold != null then {status: "pinned, newer exists", actionable: false}
               else {status: "newer exists", actionable: true} end)
        | .note = ([if $newer > 0 then "\($newer) newer" else empty end,
                    if $series_latest != null then "latest \($e.series).x: \($series_latest)" else empty end]
                   | join("; "))
      end
  end
JQ

rows=$tmp/rows.jsonl
: >"$rows"
for ((i = 0; i < count; i++)); do
    source=$(jq -r --argjson i "$i" '.[$i].source' "$entries")
    versions=$tmp/versions.json
    echo '[]' >"$versions"
    error=
    case $source in
    github-release) error=$(versions_github "$i" "$versions") || true ;;
    maven) error=$(versions_page "$i" "$versions" "$maven_jq") || true ;;
    listing) error=$(versions_page "$i" "$versions" "$listing_jq") || true ;;
    esac
    jq -nc --slurpfile entries "$entries" --argjson i "$i" --slurpfile versions "$versions" \
        --arg error "$error" '$entries[0] as $entries | $versions[0] as $versions | '"$assess_jq" >>"$rows"
done

results=$tmp/results.json
jq -s . "$rows" >"$results"

# --- report -------------------------------------------------------------------

jq -r '
  def pad($w): if length >= $w then . else . + (" " * ($w - length)) end;
  [["UPSTREAM", "SEVERITY", "PINNED", "LATEST", "STATUS", "NOTE"]]
  + map([.name, (.severity | ascii_upcase), .pinned, .latest, .status, .note])
  | . as $t
  | [range(0; 5) as $c | [$t[][$c] | length] | max] as $w
  | $t[] | [range(0; 5) as $c | .[$c] | pad($w[$c])] + [.[5]] | join("  ") | sub(" +$"; "")
' "$results"

if [ -n "$json_out" ]; then
    cp "$results" "$json_out"
fi
if [ -n "$md_out" ]; then
    jq -r '
      def cell: tostring | gsub("\\|"; "\\|") | gsub("\n"; " ");
      "| Upstream | Severity | Pinned | Latest | Status | Note |",
      "|---|---|---|---|---|---|",
      (.[] | "| [\(.name | cell)](\(.link)) | \(.severity) | \(.pinned | cell) | \(.latest | cell) | \(.status) | \(.note | cell) |"),
      (map(select(.hold != null)) | if length > 0 then
         "", "Held on purpose (`hold` in `upstream.toml`):", "",
         (.[] | "- \(.name | cell): \(.hold | cell)")
       else empty end)
    ' "$results" >"$md_out"
fi

errors=$(jq '[.[] | select(.status == "ERROR")] | length' "$results")
outdated=$(jq '[.[] | select(.status == "OUTDATED")] | length' "$results")
newer=$(jq '[.[] | select(.status == "newer exists" or .status == "pinned, newer exists")] | length' "$results")
echo
if [ "$errors" -gt 0 ]; then
    echo "freshness: $errors upstream(s) could not be checked (ERROR above): nothing is known about them" >&2
    exit 2
fi
if [ "$outdated" -gt 0 ]; then
    echo "freshness: $outdated official rule set(s) OUTDATED; $newer tool(s) or standard(s) with a newer version"
    exit 1
fi
echo "freshness: no official rule set outdated; $newer tool(s) or standard(s) with a newer version (info)"
