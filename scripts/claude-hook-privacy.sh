#!/usr/bin/env bash
# Claude Code PreToolUse hook (matcher "Bash"): refuses a shell command that
# would publish text containing a private term or a home path, before it
# runs. Register it in .claude/settings.local.json:
#
#   {"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command",
#     "command": "\"$CLAUDE_PROJECT_DIR\"/scripts/claude-hook-privacy.sh"}]}]}}
#
# Input: the hook JSON on stdin; tool_input.command is the shell command.
# Commands that publish text:
#   gh issue|pr|release|repo|label|gist|discussion|project <verb>  (read-only verbs excepted)
#   gh workflow run;  gh api with a write method, fields, --input, or graphql
#   git tag;  git notes
# For those, the hook scans with `scripts/privacy-guard.sh text`:
#   - the command text, heredoc bodies included, where a path given as a
#     file operand (cd, -C, --body-file, --notes-file, --input, -F FILE,
#     field=@FILE, < FILE, > FILE, cat FILE, tee FILE) is replaced by PATH
#     (the path is not published), and the path of a release asset or gist
#     file is reduced to its file name (the name is published);
#   - the content of every file whose text gets published: --body-file,
#     --notes-file, --input, -F FILE, field=@FILE, < FILE, cat FILE, and the
#     files given to gh release create|upload and gh gist create|edit.
# Exit 0: allowed (other commands exit 0 at once). Exit 2: blocked, with a
# short reason on stderr that never repeats what matched. The hook fails
# closed: a body file that does not exist yet, text read from a stdin it
# cannot see, or any error blocks the command.
set -Eeuo pipefail
shopt -s inherit_errexit

block() {
    echo "privacy hook: $*" >&2
    exit 2
}
trap 'block "unexpected error (line $LINENO); command blocked"' ERR

command -v jq >/dev/null 2>&1 || block "jq is not installed: cannot read the command; blocked"
input=$(cat) || block "cannot read the hook input; blocked"
cmd=$(jq -r '.tool_input.command // ""' <<<"$input" 2>/dev/null) || block "the hook input is not valid JSON; blocked"
cwd=$(jq -r '.cwd // ""' <<<"$input")
[ -n "$cwd" ] || cwd=$PWD
[ -n "$cmd" ] || exit 0

# One line, for the command detection below.
flat=$(printf '%s' "$cmd" | tr '\n\t' '  ')

# found <ERE> <text>: grep -E on a string; 0 match, 1 none, error blocks.
found() {
    local status=0
    grep -qE -- "$1" <<<"$2" || status=$?
    [ "$status" -le 1 ] || block "grep failed (exit $status); blocked"
    return "$status"
}

start='(^|[^[:alnum:]_./-])' # a command word starts here
publishes=false
# Every `gh <group> <verb>` of the command.
gh_calls=$(grep -oE -- "${start}gh +[a-z-]+( +[^ ;&|]+)?" <<<"$flat" || [ $? -eq 1 ])
while read -r _ group verb; do
    case "$group $verb" in
    "issue view" | "issue list" | "issue status" | "pr view" | "pr list" | "pr status" | \
        "pr diff" | "pr checks" | "pr checkout" | "release view" | "release list" | \
        "release download" | "repo view" | "repo list" | "repo clone" | "label list" | \
        "gist view" | "gist list" | "gist clone") ;;
    "api "*)
        # gh api sends a GET unless given a method, fields or a body.
        if [ "$verb" = graphql ] ||
            found '(^| )(-X|--method)( +|=)?["'\'']?(POST|PATCH|PUT|DELETE|post|patch|put|delete)' "$flat" ||
            found '(^| )(-f|-F|--field|--raw-field|--input)( |=)' "$flat"; then
            publishes=true
        fi
        ;;
    "issue "* | "pr "* | "release "* | "repo "* | "label "* | "gist "* | "discussion "* | \
        "project "* | "workflow run") publishes=true ;;
    esac
done <<<"$gh_calls"
if found "${start}git( +-[cC] +[^ ]+)* +(tag|notes)( |$)" "$flat"; then publishes=true; fi
[ "$publishes" = true ] || exit 0

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
guard="$here/privacy-guard.sh"
[ -x "$guard" ] || block "scripts/privacy-guard.sh not found next to the hook; blocked"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# A path inside the command: no blanks, quotes, redirections or separators.
pathchars="[^[:space:]'\"<>|;&()]"
lastchars="[^[:space:]'\"<>|;&()/]" # the same, '/' excluded: a last path component
# What introduces a file operand. Redirections must follow a blank, a digit,
# '&' or '(' so that HTML in a body (<code>/path</code>) is not taken for one.
operand="((^|[[:space:];&|(])(cd|-C|--body-file|--notes-file|--input|-F|cat|tee)([[:space:]]+|=)|(^|[[:space:]0-9&(])[<>]{1,2}[[:space:]]*|=@)"

# Heredoc bodies are text, not shell: split the command into its shell
# lines (bodies blanked) and its heredoc bodies (shell lines blanked), line
# for line, so that `paste` can put them back together. Only the shell lines
# are parsed for operands and get their paths reduced.
awk -v shell="$tmp/shell" -v body="$tmp/body" '
    function out(s, b) { print s > shell; print b > body }
    head < tail {
        line = $0
        if (dash[head]) sub(/^\t+/, "", line)
        if (line == delim[head]) { head++; out($0, ""); next }
        out("", $0); next
    }
    {
        out($0, "")
        rest = $0
        gsub(/<<</, "   ", rest) # here-strings are not heredocs
        while (match(rest, /<<-?[ \t]*["\047]?[A-Za-z0-9_.-]+["\047]?/)) {
            word = substr(rest, RSTART, RLENGTH)
            rest = substr(rest, RSTART + RLENGTH)
            dash[tail] = (substr(word, 3, 1) == "-")
            sub(/^<<-?[ \t]*["\047]?/, "", word)
            sub(/["\047]$/, "", word)
            delim[tail++] = word
        }
    }
    END { if (tail > 0) print "heredoc" > (shell ".heredoc") }
' <<<"$cmd"
heredoc=false
[ ! -e "$tmp/shell.heredoc" ] || heredoc=true
shell_text=$(cat "$tmp/shell")
shell_flat=$(tr '\n\t' '  ' <"$tmp/shell")

# The directories a relative path may be relative to: the hook's cwd and
# every `cd DIR` of the command.
dirs=("$cwd")
cds=$(grep -oE -- "(^|[[:space:];&|(])cd[[:space:]]+['\"]?${pathchars}+" <<<"$shell_text" || [ $? -eq 1 ])
while read -r line; do
    [ -n "$line" ] || continue
    d=${line#*cd}
    d=${d#"${d%%[![:space:]]*}"}
    d=${d#[\'\"]}
    d=${d/#\~/$HOME}
    case $d in /*) ;; *) d="$cwd/$d" ;; esac
    [ -d "$d" ] && dirs+=("$d")
done <<<"$cds"

# resolve <path>: print the existing file it names, or nothing.
resolve() {
    local p=${1/#\~/$HOME} d
    case $p in
    /*) [ -e "$p" ] && printf '%s\n' "$p" ;;
    *) for d in "${dirs[@]}"; do
        if [ -e "$d/$p" ]; then
            printf '%s\n' "$d/$p"
            return 0
        fi
    done ;;
    esac
    return 0
}

files=() # files whose content is published
stdin_used=false
ops=$(grep -oE -- "${operand}['\"]?${pathchars}+" <<<"$shell_text" || [ $? -eq 1 ])
while read -r op; do
    [ -n "$op" ] || continue
    path=$(sed -E "s#^${operand}['\"]?##" <<<"$op")
    case $op in
    *cd[[:space:]]* | *-C[[:space:]]* | *-C=* | *tee[[:space:]]* | *tee=* | *">"*) continue ;; # not read
    esac
    case $path in
    *=@*) path=${path#*=@} ;; # gh api -F key=@FILE
    *=*) case $op in *-F[[:space:]=]*) continue ;; esac ;; # gh api -F key=value: a literal
    esac
    if [ "$path" = - ]; then
        stdin_used=true
        continue
    fi
    case $op in *"<<"*) continue ;; esac # a heredoc delimiter, not a file
    real=$(resolve "$path")
    [ -n "$real" ] || block "a file operand does not exist yet, so its text cannot be checked: create it first, publish in a separate command"
    files+=("$real")
done <<<"$ops"
if [ "$stdin_used" = true ] && [ "$heredoc" = false ] && ! found '(^|[[:space:]0-9&(])<[[:space:]]*[^<[:space:]]' "$shell_flat"; then
    block "the text comes from stdin, which the hook cannot see: write it to a file (or a heredoc) and pass that"
fi

# The command text, with every path given as a file operand in the shell
# lines replaced by PATH: where a body is read from, or where the command
# runs, is not published. (A file operand without '/' is kept and scanned.)
# -F takes a path only when its value has no '=' (key=value is a gh api
# field, published as is; key=@FILE is caught by the "=@" expression).
noeq="[^[:space:]'\"<>|;&()=]"
noeqlast="[^[:space:]'\"<>|;&()=/]"
sed -E \
    -e "s#(^|[[:space:];&|(])(cd|-C|--body-file|--notes-file|--input|cat|tee)([[:space:]]+|=)(['\"]?)${pathchars}*/${lastchars}*#\\1\\2\\3\\4PATH#g" \
    -e "s#(^|[[:space:]0-9&(])([<>]{1,2}[[:space:]]*)(['\"]?)${pathchars}*/${lastchars}*#\\1\\2\\3PATH#g" \
    -e "s#=@(['\"]?)${pathchars}*/${lastchars}*#=@\\1PATH#g" \
    -e "s#(^|[[:space:];&|(])(-F)([[:space:]]+|=)(['\"]?)${noeq}*/${noeqlast}*([[:space:]'\"]|\$)#\\1\\2\\3\\4PATH\\5#g" \
    "$tmp/shell" >"$tmp/shell.reduced"

# Files uploaded as release assets or gists: content scanned, path reduced.
if found "${start}gh +(release +(create|upload)|gist +(create|edit))( |$)" "$shell_flat"; then
    read -r -a words <<<"$shell_flat"
    for w in "${words[@]}"; do
        w=${w#[\'\"]}
        w=${w%[\'\"]}
        w=${w%%#*} # gh release upload FILE#label
        [ -n "$w" ] || continue
        real=$(resolve "$w")
        if [ -n "$real" ] && [ -f "$real" ]; then
            files+=("$real")
            base=${w##*/}
            # The x keeps trailing blank lines, which align the lines for paste.
            reduced=$(cat "$tmp/shell.reduced" && echo x)
            reduced=${reduced%x}
            printf '%s' "${reduced//"$w"/"$base"}" >"$tmp/shell.reduced"
        fi
    done
fi

# check <what> <file>: run the guard on the file; block on a hit or error.
check() {
    local status=0
    "$guard" text <"$2" >/dev/null 2>"$tmp/guard.err" || status=$?
    case $status in
    0) ;;
    1) block "private term or home path in $1 ($(grep -c 'privacy-guard: .*in text' "$tmp/guard.err" || true) hit(s)); remove it before publishing" ;;
    *) block "could not check $1 (privacy-guard exit $status: $(head -n 1 "$tmp/guard.err")); blocked" ;;
    esac
}

paste -d '' "$tmp/shell.reduced" "$tmp/body" >"$tmp/command"
check "the command text" "$tmp/command"
n=0
for f in "${files[@]}"; do
    n=$((n + 1))
    [ -f "$f" ] || block "file operand #$n is not a regular file; blocked"
    [ -r "$f" ] || block "file operand #$n is not readable; blocked"
    [ -s "$f" ] || continue # empty: nothing published
    check "file operand #$n" "$f"
done
exit 0
