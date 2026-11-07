#!/usr/bin/env bash
# Freshness of every upstream the repository pins (upstream.toml): official
# rule sets, standards and tools; and of the Cargo dependencies, whose
# semver-incompatible ("major") updates Dependabot does not propose. Needs
# bash, curl and jq (no Rust toolchain).
#
#   scripts/freshness.sh pins                  offline: check every pin of upstream.toml
#                                              against the files that record it
#   scripts/freshness.sh                       the pins check, then query every upstream
#                                              and every Cargo dependency, and print
#                                              pinned / latest / status
#   scripts/freshness.sh crates                query the Cargo dependencies only
#   scripts/freshness.sh --json FILE --markdown FILE
#                                              also write the results as JSON and as a
#                                              Markdown table
#
# Statuses of upstream.toml entries: "current"; "OUTDATED" (severity high: an
# official rule set has a newer release); "newer exists" (severity info);
# "pinned, newer exists" (severity info, held on purpose: `hold` in
# upstream.toml); "ERROR".
#
# Cargo dependencies: every direct dependency, dev-dependency and
# build-dependency on crates.io of every workspace member, read from the
# manifests and Cargo.lock (a strict TOML subset; anything else is refused),
# then looked up on crates.io, yanked and pre-release versions ignored. One row
# per crate and locked version, severity info, note "locked L, newest compatible
# C, newest N". Statuses: "current"; "compatible update" (within the version
# requirement: Dependabot proposes it, so it opens no issue); "major update"
# (outside the requirement: Dependabot does not propose it, it is migrated by
# hand, so it opens the tracking issue); "locked version yanked" (also opens
# it); "ERROR". crates.io is queried at most once per second, with a
# User-Agent that names this repository (crates.io crawler policy).
#
# Exit status of a query run:
#   0  nothing with severity high is outdated (info items are printed, never fail)
#   1  an upstream with severity high has a newer release
#   2  error: invalid upstream.toml, a pin that does not match the repository,
#      a manifest or Cargo.lock outside the subset read here or out of date,
#      a network or API failure, an unexpected response, a pinned or locked
#      version that upstream does not publish. Nothing that cannot be queried
#      is ever reported as current.
# The pins check exits 0 when every pin matches, 2 otherwise.
#
# GitHub is queried through its REST API, with GITHUB_TOKEN (else GH_TOKEN)
# when set: without a token the limit is 60 requests per hour, enough for one
# run. Maven Central is read from its maven-metadata.xml files, crates.io from
# its API (https://crates.io/api/v1/crates/NAME).
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
# FRESHNESS_WORKSPACE replaces the repository root as the Cargo workspace
# whose dependencies are checked (tests).
workspace=.
if [ -n "${FRESHNESS_WORKSPACE:-}" ]; then
    case $FRESHNESS_WORKSPACE in
    /*) workspace=$FRESHNESS_WORKSPACE ;;
    *) workspace=$PWD/$FRESHNESS_WORKSPACE ;;
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
    crates) mode=crates ;;
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

# jq's error messages without their "jq: error (at ...): " prefix.
jq_error() { sed 's/^jq: error (at [^)]*): //' "$1"; }

entries=$tmp/entries.json
if [ "$mode" = crates ]; then
    echo '[]' >"$entries"
else
    [ -f "$pinfile" ] || die "$pinfile not found"
    jq -nR --arg file "$pinfile" "$parse_jq" "$pinfile" >"$entries" 2>"$tmp/parse.err" ||
        die "$(jq_error "$tmp/parse.err")"
fi
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
[ "$mode" = crates ] || check_pins || exit 2

# --- the Cargo workspace ------------------------------------------------------

# A Cargo.toml, read as a strict TOML subset: the [package] name, the
# [workspace] members, and every line of [dependencies], [dev-dependencies] and
# [build-dependencies] (`name = "requirement"` or a one-line inline table).
# Other tables are only checked to be `key = value` lines; what could change
# the dependencies without being read (target-specific or workspace dependency
# tables, dotted keys such as `x.workspace = true`, [patch], git or other
# registries, multi-line strings) is refused.
read -r -d '' manifest_jq <<'JQ' || true
def fail($n; $msg): error("\($file):\($n): \($msg)");
def key_re: "(?:[A-Za-z0-9_-]+|\"[^\"\\\\]*\"|'[^']*')";
def str_re: "(?:\"[^\"\\\\]*\"|'[^']*')";
def unquote: if startswith("\"") or startswith("'") then .[1:-1] else . end;
def known($list): . as $x | any($list[]; . == $x);
def dep_tables: ["dependencies", "dev-dependencies", "build-dependencies", "dev_dependencies",
  "build_dependencies"];
# "a.'b c'.d" -> ["a", "b c", "d"]
def dotted($n):
  . as $h
  | [match("\\s*(" + key_re + ")\\s*(\\.|$)"; "g")] as $m
  | if ($m | length) == 0 or ($m | map(.string) | add) != $h or $m[-1].captures[1].string != "" then
      fail($n; "unsupported key or table name \($h)")
    else $m | map(.captures[0].string | unquote) end;
# The strings of a one-line array body: "\"a\", 'b'" -> ["a", "b"]
def strings($n; $body):
  ($body | sub("^\\s+"; "") | sub("\\s+$"; "")) as $b
  | [$b | match("(" + str_re + ")\\s*(?:,\\s*|$)"; "g")] as $m
  | if $b == "" then []
    elif ($m | map(.string) | add) != $b then fail($n; "unsupported array [\($b)]: strings only")
    else $m | map(.captures[0].string | unquote) end;
# A one-line inline table of a dependency: {version, path, package}.
def inline($n; $body):
  ($body | sub("^\\s+"; "") | sub("\\s+$"; "")) as $b
  | [$b | match("(?<k>[A-Za-z0-9_-]+)\\s*=\\s*(?<v>" + str_re + "|true|false|\\[[^\\[\\]]*\\])\\s*(?:,\\s*|$)"; "g")]
    as $m
  | ($m | map({k: .captures[0].string, v: .captures[1].string})) as $items
  | if $b == "" or ($m | map(.string) | add) != $b then fail($n; "unsupported inline table { \($b) }")
    elif ($items | map(.k) | unique | length) != ($items | length) then fail($n; "duplicate key in { \($b) }")
    else
      ($items | map(.k | select(known(["version", "path", "package", "features", "default-features",
        "default_features", "optional"]) | not))) as $unknown
      | if ($unknown | length) > 0 then
          fail($n; "unsupported dependency key \($unknown | join(", ")): crates.io and path dependencies only")
        else
          reduce $items[] as $i ({};
            if ($i.k | known(["version", "path", "package"])) then
              if ($i.v | test("^" + str_re + "$")) then .[$i.k] = ($i.v | unquote)
              else fail($n; "\($i.k) must be a string") end
            elif ($i.k | known(["default-features", "default_features", "optional"])) then
              if ($i.v | known(["true", "false"])) then . else fail($n; "\($i.k) must be true or false") end
            elif ($i.v | startswith("[")) and (strings($n; $i.v[1:-1]) | length) >= 0 then .
            else fail($n; "features must be an array of strings") end)
        end
    end;

reduce inputs as $raw ({n: 0, table: null, kind: null, array: null,
                         package: null, workspace: null, deps: []};
  .n += 1
  | .n as $n
  | ($raw | sub("^[ \t]+"; "") | sub("[ \t\r]+$"; "")) as $t
  | if ($t | contains("\"\"\"") or contains("'''")) then fail($n; "multi-line strings are not supported")
    elif .array != null then
      if $t == "" or ($t | startswith("#")) then .
      elif ($t | test("^\\](\\s*#.*)?$")) then
        (if .table == ["workspace"] and (.array.key | known(["members", "exclude"])) then
           .workspace[.array.key] = .array.items
         else . end)
        | .array = null
      else
        ($t | [capture("^(?<b>" + str_re + "(?:\\s*,\\s*" + str_re + ")*)\\s*,?\\s*(?:#.*)?$")] | first) as $c
        | if $c == null then
            fail($n; "unsupported line in a multi-line array: strings only, and ] on its own line")
          else .array.items += strings($n; $c.b) end
      end
    elif $t == "" or ($t | startswith("#")) then .
    elif ($t | startswith("[")) then
      ($t | [capture("^(?<o>\\[\\[?)(?<h>[^\\[\\]]*)(?<c>\\]\\]?)\\s*(?:#.*)?$")] | first) as $m
      | if $m == null or ($m.o | length) != ($m.c | length) then fail($n; "unsupported table header \($t)")
        else
          ($m.h | dotted($n)) as $keys
          | if $m.o == "[" and ($keys | length) == 1 and ($keys[0] | known(dep_tables[0:3])) then
              .table = $keys
              | .kind = {"dependencies": "normal", "dev-dependencies": "dev", "build-dependencies": "build"}[$keys[0]]
            elif any($keys[]; known(dep_tables)) then
              fail($n; "unsupported table \($t): only [dependencies], [dev-dependencies] and [build-dependencies] are read")
            elif $keys[0] == "patch" or $keys == ["replace"] then
              fail($n; "unsupported table \($t): it changes what Cargo.lock records")
            elif $m.o == "[[" then .table = null | .kind = null
            elif $keys == ["workspace"] then
              .table = $keys | .kind = null | .workspace = (.workspace // {members: null, exclude: null})
            else .table = $keys | .kind = null end
        end
    else
      ($t | [capture("^(?<k>[^=]*?)\\s*=\\s*(?<v>.*)$")] | first) as $kv
      | if $kv == null then fail($n; "unsupported line: \($t)") else . end
      | ($kv.k | dotted($n)) as $keys
      | $kv.v as $v
      | if ($v | startswith("{")) and ($v | contains("}") | not) then
          fail($n; "multi-line inline tables are not supported")
        elif .kind != null then
          if ($keys | length) > 1 then
            fail($n; "dotted key \($kv.k) is not supported (nor is workspace = true inheritance)")
          else
            (if ($v | test("^" + str_re + "\\s*(#.*)?$")) then
               {version: ($v | capture("^(?<s>" + str_re + ")").s | unquote)}
             elif ($v | test("^\\{[^{}]*\\}\\s*(#.*)?$")) then
               inline($n; $v | capture("^\\{(?<b>[^{}]*)\\}").b)
             else fail($n; "unsupported dependency value \($v): a string or a one-line inline table") end) as $d
            | ($d.package // $keys[0]) as $crate
            | if $d.version == null and $d.path == null then fail($n; "dependency \($keys[0]) has no version")
              elif ($crate | test("^[A-Za-z0-9_-]+$") | not) then fail($n; "invalid crate name \($crate)")
              else .deps += [{key: $keys[0], crate: $crate, kind: .kind, req: $d.version, path: $d.path,
                               line: $n}] end
          end
        elif ($v | startswith("[")) and ($v | sub("\\s*#[^\"'\\]]*$"; "") | endswith("]") | not) then
          if ($v | sub("\\s*#.*$"; "")) != "[" then
            fail($n; "unsupported value \($v): a multi-line array opens with [ alone on its line")
          else .array = {key: $keys[0], items: []} end
        elif .table == ["package"] and $keys[0] == "name" then
          if ($keys | length) > 1 or .package != null then fail($n; "unsupported or duplicate package name")
          elif ($v | test("^\"[A-Za-z0-9_-]+\"\\s*(#.*)?$") | not) then fail($n; "unsupported package name \($v)")
          else .package = ($v | capture("^\"(?<s>[^\"]*)\"").s) end
        elif .table == ["workspace"] and ($keys[0] | known(["members", "exclude"])) then
          if ($keys | length) > 1 or ($v | test("^\\[[^\\[\\]]*\\]\\s*(#.*)?$") | not) then
            fail($n; "unsupported \($kv.k) = \($v)")
          else .workspace[$keys[0]] = strings($n; $v | capture("^\\[(?<b>[^\\[\\]]*)\\]").b) end
        else . end
    end)
| if .array != null then fail(.n; "unterminated multi-line array \(.array.key)") else . end
| {file: $file, package, workspace, deps}
JQ

# Cargo.lock (format 3 or 4): every [[package]] with its name, version, source
# and dependencies ("name", "name version" or "name version (source)").
read -r -d '' lock_jq <<'JQ' || true
def fail($n; $msg): error("\($file):\($n): \($msg)");
def str_re: "\"[^\"\\\\]*\"";
reduce inputs as $raw ({n: 0, version: null, array: false, packages: []};
  .n += 1
  | .n as $n
  | ($raw | sub("^[ \t]+"; "") | sub("[ \t\r]+$"; "")) as $t
  | (.packages | length - 1) as $p
  | if .array then
      if $t == "]" then .array = false
      elif ($t | test("^" + str_re + ",?$")) then .packages[$p].dependencies += [$t | sub(",$"; "") | .[1:-1]]
      else fail($n; "unsupported line in dependencies: \($t)") end
    elif $t == "" or ($t | startswith("#")) then .
    elif $t == "[[package]]" then
      .packages += [{line: $n, name: null, version: null, source: null, dependencies: []}]
    elif ($t | startswith("[")) then fail($n; "unsupported table \($t)")
    elif $p < 0 then
      if ($t | test("^version = [0-9]+$")) and .version == null then .version = ($t | ltrimstr("version = ") | tonumber)
      else fail($n; "unsupported line: \($t)") end
    else
      ($t | [capture("^(?<k>[a-z]+) = (?<v>.*)$")] | first) as $kv
      | if $kv == null then fail($n; "unsupported line: \($t)")
        elif ($kv.k | IN("name", "version", "source", "checksum")) then
          if ($kv.v | test("^" + str_re + "$") | not) then fail($n; "\($kv.k) must be a string")
          elif .packages[$p][$kv.k] != null then fail($n; "duplicate key \($kv.k)")
          else .packages[$p][$kv.k] = ($kv.v | .[1:-1]) end
        elif $kv.k == "dependencies" then
          if $kv.v == "[" then .array = true
          elif ($kv.v | test("^\\[(" + str_re + "(, " + str_re + ")*)?\\]$")) then
            .packages[$p].dependencies = [$kv.v | match(str_re; "g") | .string | .[1:-1]]
          else fail($n; "unsupported dependencies \($kv.v)") end
        else fail($n; "unsupported key \($kv.k)") end
    end)
| if .array then fail(.n; "unterminated dependencies array") else . end
| if .version != 3 and .version != 4 then fail(1; "unsupported Cargo.lock format \(.version // "none") (3 or 4)")
  else . end
| (.packages | map(select(.name == null or .version == null)) | first) as $bad
| if $bad != null then fail($bad.line; "[[package]] without name or version") else . end
| .packages | map(del(.checksum, .line))
JQ

# Semantic versions and Cargo's default (caret) requirements.
read -r -d '' semver_jq <<'JQ' || true
def semver:
  [capture("^(?<ma>0|[1-9][0-9]*)\\.(?<mi>0|[1-9][0-9]*)\\.(?<pa>0|[1-9][0-9]*)(?:-(?<pre>[0-9A-Za-z.-]+))?(?:\\+[0-9A-Za-z.-]+)?$")]
  | first // null
  | if . == null then null else {ma: (.ma | tonumber), mi: (.mi | tonumber), pa: (.pa | tonumber), pre} end;
# Sort key of a valid version string; a pre-release sorts before its release.
def vkey: semver | [.ma, .mi, .pa, (if .pre == null then 1 else 0 end)];
def req:
  [capture("^\\^?\\s*(?<ma>[0-9]+)(?:\\.(?<mi>[0-9]+)(?:\\.(?<pa>[0-9]+))?)?$")] | first // null
  | if . == null then null
    else {ma: (.ma | tonumber), mi: (.mi | if . == null then null else tonumber end),
          pa: (.pa | if . == null then null else tonumber end)} end;
# Does the version string . satisfy the caret requirement $r? (Pre-releases never do.)
def satisfies($r):
  semver as $v
  | $v != null and $v.pre == null
    and ([$v.ma, $v.mi, $v.pa] >= [$r.ma, ($r.mi // 0), ($r.pa // 0)])
    and (if $r.ma > 0 or $r.mi == null then $v.ma == $r.ma
         elif $r.mi > 0 or $r.pa == null then $v.ma == 0 and $v.mi == $r.mi
         else $v.ma == 0 and $v.mi == 0 and $v.pa == $r.pa end);
JQ

# The crates.io dependencies of the workspace, one per crate and locked
# version: {crate, locked, reqs, uses}. Every requirement must be a caret
# requirement, satisfied by exactly one crates.io version in Cargo.lock.
read -r -d '' deps_jq <<'JQ' || true
def registry: "registry+https://github.com/rust-lang/crates.io-index";
($manifests | map(.package) | sort) as $members
| ($lock | map(select(.source == null) | .name) | sort) as $locked_members
| if ($members | unique | length) != ($members | length) then
    error("two workspace members have the same name: \($members | group_by(.) | map(select(length > 1) | .[0]) | join(", "))")
  elif $members != $locked_members then
    error("\($lockfile) does not match the workspace: members \($members | join(", ")); local packages in \($lockfile) \($locked_members | join(", "))")
  else . end
| [$manifests[] as $m
   | ($lock | map(select(.source == null and .name == $m.package)) | first) as $lp
   | $m.deps[] | select(.path == null)
   | . as $d
   | ($d.req | req) as $r
   | if $r == null then
       error("\($m.file):\($d.line): unsupported version requirement \"\($d.req)\" of \($d.key): only caret requirements (\"1.2.3\", \"^1.2\") are read")
     else . end
   | [$lp.dependencies[]
      | capture("^(?<n>[^ ]+)(?: (?<v>[^ ]+))?(?: \\((?<s>.+)\\))?$")
      | select(.n == $d.crate)
      | . as $e
      | $lock[] | select(.name == $e.n and ($e.v == null or .version == $e.v) and ($e.s == null or .source == $e.s))]
     as $candidates
   | if any($candidates[]; .source != registry) then
       error("\($m.file):\($d.line): \($d.crate) is locked from \([$candidates[] | .source // "a local path"] | unique | join(", ")), not crates.io")
     else . end
   | [$candidates[] | select(.version | satisfies($r))] as $ok
   | if ($ok | length) == 0 then
       error("\($m.file):\($d.line): \($lockfile) has no version of \($d.crate) that satisfies \"\($d.req)\" for \($m.package) (found: \([$candidates[].version] | join(", ") | if . == "" then "none" else . end)); run cargo update")
     elif ($ok | length) > 1 then
       error("\($m.file):\($d.line): \($lockfile) has several versions of \($d.crate) that satisfy \"\($d.req)\": \([$ok[].version] | join(", "))")
     else {crate: $d.crate, locked: $ok[0].version, req: $r,
           use: ($m.package + (if $d.kind == "normal" then "" else " (\($d.kind))" end))} end]
| group_by([.crate, .locked])
| map({crate: .[0].crate, locked: .[0].locked, reqs: (map(.req) | unique), uses: (map(.use) | unique)})
JQ

crates=$tmp/crates.json
echo '[]' >"$crates"
if [ "$mode" = query ] || [ "$mode" = crates ]; then
    for file in Cargo.toml Cargo.lock; do
        [ -f "$workspace/$file" ] || die "$workspace/$file not found"
    done
    # parse_manifest PATH OUT: PATH relative to the workspace root.
    parse_manifest() {
        jq -nR --arg file "$1" "$manifest_jq" "$workspace/$1" >"$2" 2>"$tmp/parse.err" ||
            die "$(jq_error "$tmp/parse.err")"
    }
    parse_manifest Cargo.toml "$tmp/manifest-root.json"
    jq -e '.workspace.members != null' "$tmp/manifest-root.json" >/dev/null ||
        die "Cargo.toml: no [workspace] members"
    jq -e '(.workspace.exclude // []) == []' "$tmp/manifest-root.json" >/dev/null ||
        die "Cargo.toml: [workspace] exclude is not supported"
    member_dirs=()
    if jq -e '.package != null' "$tmp/manifest-root.json" >/dev/null; then
        member_dirs+=(.)
    fi
    while IFS= read -r pattern; do
        case $pattern in
        '' | /* | *..* | *[!A-Za-z0-9_./*-]*) die "Cargo.toml: unsupported workspace member \"$pattern\"" ;;
        *'*'*)
            dir=${pattern%/\*}
            case $dir in *'*'* | "$pattern") die "Cargo.toml: unsupported glob \"$pattern\" (only DIR/*)" ;; esac
            found=0
            for d in "$workspace/$dir"/*/; do
                [ -d "$d" ] || continue
                d=${d%/}
                found=1
                member_dirs+=("$dir/${d##*/}")
            done
            [ "$found" -eq 1 ] || die "Cargo.toml: workspace member \"$pattern\" matches no directory"
            ;;
        *) member_dirs+=("${pattern%/}") ;;
        esac
    done < <(jq -r '.workspace.members[]' "$tmp/manifest-root.json")
    : >"$tmp/manifests.jsonl"
    k=0
    while IFS= read -r d; do
        if [ "$d" = . ]; then
            cat "$tmp/manifest-root.json" >>"$tmp/manifests.jsonl"
            continue
        fi
        [ -f "$workspace/$d/Cargo.toml" ] || die "workspace member $d has no Cargo.toml"
        k=$((k + 1))
        parse_manifest "$d/Cargo.toml" "$tmp/manifest-$k.json"
        jq -e '.package != null' "$tmp/manifest-$k.json" >/dev/null || die "$d/Cargo.toml: no [package] name"
        jq -e '.workspace == null' "$tmp/manifest-$k.json" >/dev/null || die "$d/Cargo.toml: nested [workspace]"
        cat "$tmp/manifest-$k.json" >>"$tmp/manifests.jsonl"
    done < <(printf '%s\n' "${member_dirs[@]}" | sort -u)
    jq -nR --arg file Cargo.lock "$lock_jq" "$workspace/Cargo.lock" >"$tmp/lock.json" 2>"$tmp/parse.err" ||
        die "$(jq_error "$tmp/parse.err")"
    jq -n --slurpfile manifests "$tmp/manifests.jsonl" --slurpfile lock "$tmp/lock.json" --arg lockfile Cargo.lock \
        "$semver_jq"' $manifests as $manifests | $lock[0] as $lock | '"$deps_jq" >"$crates" 2>"$tmp/parse.err" ||
        die "$(jq_error "$tmp/parse.err")"
    printf 'crates: %d crates.io dependencies (normal, dev and build) of %d workspace members, from the manifests and Cargo.lock\n' \
        "$(jq length "$crates")" "$(jq -s length "$tmp/manifests.jsonl")"
fi

# --- querying the upstreams ---------------------------------------------------

token=${GITHUB_TOKEN:-${GH_TOKEN:-}}
# Names the tool and where to reach its maintainers (crates.io crawler policy).
user_agent='einvoice-validator-freshness (+https://github.com/einvoice-validator/einvoice-validator)'

# The content of a message file on one line.
oneline() { tr -s '\n' ' ' <"$1" | sed 's/ *$//'; }

# GET over HTTPS only; the token, when any, goes to api.github.com only and
# through curl's standard input, never on its command line.
fetch() {
    local url=$1
    local -a args=(--proto '=https' --proto-redir '=https' --tlsv1.2 --fail --silent --show-error
        --location --retry 3 --connect-timeout 10 --max-time 60
        --user-agent "$user_agent")
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

# --- querying crates.io -------------------------------------------------------

# The version list of one crate from its crates.io record, as a JSON array of
# {num, yanked}; refuses anything that is not a complete record of that crate.
read -r -d '' crate_record_jq <<'JQ' || true
if type != "object" or (.crate | type) != "object" or (.versions | type) != "array" then
  error("not a crates.io crate record")
elif .crate.name != $crate then error("the record of \(.crate.name), not of \($crate)")
elif any(.versions[]; (.num | type) != "string" or (.yanked | type) != "boolean") then
  error("a version without num or yanked")
elif (.versions | length) != .crate.num_versions then
  error("\(.versions | length) versions listed of \(.crate.num_versions)")
else [.versions[] | {num, yanked}] end
JQ

# One result row from a dependency and the versions crates.io publishes (or an error).
read -r -d '' crate_assess_jq <<'JQ' || true
def all_reqs($reqs): . as $v | all($reqs[]; . as $r | $v | satisfies($r));
$dep.crate as $crate | $dep.locked as $locked | $dep.reqs as $reqs
| {name: "crate \($crate)", severity: "info", pinned: $locked, hold: null,
   link: "https://crates.io/crates/\($crate)/versions"}
| ("used by \($dep.uses | join(", "))") as $uses
| if $error != "" then . + {latest: "?", status: "ERROR", note: $error, actionable: false}
  else
    ($versions | map(select(.num | semver == null)) | map(.num)) as $bad
    | ($versions | map(select(.num == $locked)) | first) as $lv
    | ($versions | map(select((.yanked | not) and (.num | semver.pre == null)))) as $stable
    | ($stable | max_by(.num | vkey) | .num) as $newest
    | ($stable | map(select(.num | all_reqs($reqs))) | max_by(.num | vkey) | .num) as $compatible
    | if ($bad | length) > 0 then
        . + {latest: "?", status: "ERROR", actionable: false,
             note: "crates.io lists versions that are not semantic versions: \($bad | join(", "))"}
      elif $lv == null then
        . + {latest: ($newest // "?"), status: "ERROR", actionable: false,
             note: "locked \($locked) is not among the \($versions | length) versions crates.io publishes"}
      else
        ($newest != null and ($newest | all_reqs($reqs) | not)
         and ($newest | vkey) > ($compatible // $locked | vkey)) as $major
        | ($compatible != null and ($compatible | vkey) > ($locked | vkey)) as $update
        | . + {latest: ($newest // "none")}
        | . + (if $major then {status: "major update", actionable: true}
               elif $lv.yanked then {status: "locked version yanked", actionable: true}
               elif $update then {status: "compatible update", actionable: false}
               else {status: "current", actionable: false} end)
        | .note = ("locked \($locked), newest compatible \($compatible // "none"), newest \($newest // "none")"
                   + (if $major then " (major, manual migration)"
                      elif $update then " (proposed by Dependabot)" else "" end)
                   + (if $lv.yanked then "; locked version yanked" else "" end)
                   + "; \($uses)")
      end
  end
JQ

versions_crate() {
    local crate=$1 out=$2 url
    url=https://crates.io/api/v1/crates/$crate
    if ! fetch "$url" >"$tmp/crate.json" 2>"$tmp/curl.err"; then
        echo "cannot fetch $url: $(oneline "$tmp/curl.err")"
        return 1
    fi
    if ! jq --arg crate "$crate" "$crate_record_jq" "$tmp/crate.json" >"$out" 2>"$tmp/jq.err"; then
        echo "unexpected response from $url: $(oneline "$tmp/jq.err")"
        return 1
    fi
}

# crates.io crawler policy: at most one request per second. Requests are
# sequential, so a one-second pause before each one but the first is enough
# (curl's own retries wait at least one second too).
crates_requests=0
ncrates=$(jq length "$crates")
for ((i = 0; i < ncrates; i++)); do
    crate=$(jq -r --argjson i "$i" '.[$i].crate' "$crates")
    versions=$tmp/versions.json
    echo '[]' >"$versions"
    error=
    # Each crate is fetched once, even when it is locked at several versions.
    if [ -f "$tmp/crate-$crate.json" ]; then
        cp "$tmp/crate-$crate.json" "$versions"
    else
        if [ "$crates_requests" -gt 0 ]; then sleep 1; fi
        crates_requests=$((crates_requests + 1))
        if error=$(versions_crate "$crate" "$versions"); then
            cp "$versions" "$tmp/crate-$crate.json"
        fi
    fi
    jq -nc --slurpfile crates "$crates" --argjson i "$i" --slurpfile versions "$versions" --arg error "$error" \
        "$semver_jq"' $crates[0][$i] as $dep | $versions[0] as $versions | '"$crate_assess_jq" >>"$rows"
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
major=$(jq '[.[] | select(.status == "major update" or .status == "locked version yanked")] | length' "$results")
compatible=$(jq '[.[] | select(.status == "compatible update")] | length' "$results")
summary="$newer tool(s) or standard(s) with a newer version; $major Cargo dependenc(ies) with a major update or a yanked lock (manual migration); $compatible with a compatible update (Dependabot)"
echo
if [ "$errors" -gt 0 ]; then
    echo "freshness: $errors upstream(s) or crate(s) could not be checked (ERROR above): nothing is known about them" >&2
    exit 2
fi
if [ "$outdated" -gt 0 ]; then
    echo "freshness: $outdated official rule set(s) OUTDATED; $summary"
    exit 1
fi
echo "freshness: no official rule set outdated; $summary (info)"
