#!/usr/bin/env bash
# Tests of scripts/privacy-guard.sh (every mode), .githooks/pre-push and
# scripts/claude-hook-privacy.sh.
#
#   scripts/test-privacy-guard.sh
#
# The term list is synthetic (made-up words, never a real private term) and
# the git repositories are throw-away ones in a temporary directory, with the
# user's git configuration, hooks and signing switched off. Every case checks
# the exit code (0 clean, 1 found, 2 could not look) and that the output never
# repeats a term or a home directory name. Run as a normal user: the
# unreadable-file cases cannot work as root.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/.." && pwd)
guard="$here/privacy-guard.sh"
hook="$here/claude-hook-privacy.sh"
prepush="$root/.githooks/pre-push"

if [ "$(id -u)" = 0 ]; then
    echo "test-privacy-guard: run as a normal user (root reads unreadable files)" >&2
    exit 2
fi

# Synthetic terms, and home paths built in pieces so that this file does not
# match the guard's home path pattern itself.
term=zorblax
pair_a=quux
pair_b=widgetcorp
home_linux="/hom""e/qhomeuserq/notes"
home_mac="/User""s/qmacuserq/notes"
home_windows="C:\\User""s\\qwinuserq\\notes"
# Never in any output: the terms and the user names of the home paths.
leaks="zorblax|quux|widgetcorp|qhomeuserq|qmacuserq|qwinuserq"

work=$(mktemp -d)
trap 'chmod -R u+rwX "$work" 2>/dev/null; rm -rf "$work"' EXIT

# Isolation: no user or system git configuration (hooks, signing, identity),
# no default term list unless a case creates one.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME="Test Author" GIT_AUTHOR_EMAIL=author@example.org
export GIT_COMMITTER_NAME="Test Committer" GIT_COMMITTER_EMAIL=committer@example.org
export XDG_CONFIG_HOME="$work/xdg"
unset PRIVATE_TERMS PRIVATE_TERMS_FILE
# Comments, blank lines and surrounding blanks must be ignored.
printf '%s\n' "# synthetic test list" "" "   $term  " "$pair_a $pair_b" >"$work/terms"
export PRIVATE_TERMS_FILE="$work/terms"

passed=0
failed=0
# expect <exit code> <name> <command...>
expect() {
    local want=$1 name=$2 got=0
    shift 2
    "$@" >"$work/out" 2>&1 </dev/null || got=$?
    if [ "$got" != "$want" ]; then
        failed=$((failed + 1))
        echo "FAIL  $name: exit $got, expected $want"
        sed 's/^/      | /' "$work/out"
    elif grep -qiE "$leaks" "$work/out"; then
        failed=$((failed + 1))
        echo "FAIL  $name: the output repeats a term or a home path"
    else
        passed=$((passed + 1))
        printf 'ok    %-72s exit %s\n' "$name" "$got"
    fi
}
# The command reads its stdin from a string, a file, or a closed descriptor.
with_text() { printf '%b' "$1" | "${@:2}"; }
with_file() { "${@:2}" <"$1"; }
with_closed_stdin() { "$@" <&-; }
in_dir() { (cd "$1" && "${@:2}"); }
without_list() { (unset PRIVATE_TERMS PRIVATE_TERMS_FILE && "$@"); }

echo "== text"
expect 0 "clean text" with_text 'A clean title\nand a body.\n' "$guard" text
expect 1 "term" with_text "see $term here\n" "$guard" text
expect 1 "term, other case" with_text "see ${term^^} here\n" "$guard" text
expect 1 "two-word term on one line" with_text "the $pair_a $pair_b case\n" "$guard" text
expect 1 "two-word term split across lines" with_text "the $pair_a\n$pair_b case\n" "$guard" text
expect 1 "two-word term, tab and double space" with_text "the $pair_a \t $pair_b\n" "$guard" text
expect 1 "Linux home path" with_text "file $home_linux/x.md\n" "$guard" text
expect 1 "macOS home path" with_text "file $home_mac/x.md\n" "$guard" text
expect 1 "Windows home path" with_text "file ${home_windows//\\/\\\\}\n" "$guard" text
expect 0 "a path outside any home" with_text 'see /usr/share/doc and /tmp/x\n' "$guard" text
expect 2 "empty stdin" with_text '' "$guard" text
expect 2 "closed stdin" with_closed_stdin "$guard" text
expect 2 "extra argument" with_text 'x\n' "$guard" text extra
expect 2 "no mode" "$guard"
expect 2 "unknown mode" "$guard" everything

echo "== term list"
expect 2 "no list at all" with_text 'x\n' without_list "$guard" text
printf '%s\n' "# only a comment" "" "   " >"$work/empty-terms"
expect 2 "list of comments and blanks only" with_text 'x\n' env PRIVATE_TERMS_FILE="$work/empty-terms" "$guard" text
expect 2 "PRIVATE_TERMS_FILE missing" with_text 'x\n' env PRIVATE_TERMS_FILE="$work/missing" "$guard" text
expect 2 "PRIVATE_TERMS_FILE is a directory" with_text 'x\n' env PRIVATE_TERMS_FILE="$work" "$guard" text
cp "$work/terms" "$work/locked-terms"
chmod 000 "$work/locked-terms"
expect 2 "PRIVATE_TERMS_FILE unreadable" with_text 'x\n' env PRIVATE_TERMS_FILE="$work/locked-terms" "$guard" text
mkdir -p "$work/lists/sub"
cp "$work/terms" "$work/lists/rel-terms"
expect 1 "PRIVATE_TERMS_FILE relative, from a subdirectory" \
    in_dir "$work/lists/sub" with_text "$term\n" env PRIVATE_TERMS_FILE=../rel-terms "$guard" text
expect 1 "PRIVATE_TERMS (the CI secret) wins" with_text "$term\n" env PRIVATE_TERMS="$term" PRIVATE_TERMS_FILE="$work/missing" "$guard" text
expect 0 "PRIVATE_TERMS with other terms" with_text "$term\n" env PRIVATE_TERMS="unrelatedword" "$guard" text
mkdir -p "$work/xdg/einvoice-validator"
cp "$work/terms" "$work/xdg/einvoice-validator/denylist"
expect 1 "default list in the config directory" with_text "$term\n" without_list "$guard" text
chmod 000 "$work/xdg/einvoice-validator/denylist"
expect 2 "default list unreadable" with_text "$term\n" without_list "$guard" text
rm -f "$work/xdg/einvoice-validator/denylist"

echo "== message"
repo="$work/repo"
git init -q -b main "$repo"
mkdir -p "$repo/sub"
printf 'Fix the parser\n\nA clean body.\n' >"$repo/sub/clean-msg"
printf 'Fix the parser\n\nFor %s.\n' "$term" >"$repo/sub/term-msg"
printf 'Fix the parser\n# %s\n' "$term" >"$repo/sub/comment-msg"
printf 'Fix the parser\n\nSee %s.\n' "$home_linux" >"$repo/sub/home-msg"
printf 'Fix the parser for %s\n%s users\n' "$pair_a" "$pair_b" >"$repo/sub/split-msg"
expect 0 "clean message" "$guard" message "$repo/sub/clean-msg"
expect 1 "term in message" "$guard" message "$repo/sub/term-msg"
expect 1 "term in a '#' line (kept by git commit -m)" "$guard" message "$repo/sub/comment-msg"
expect 1 "home path in message" "$guard" message "$repo/sub/home-msg"
expect 1 "two-word term split across lines" "$guard" message "$repo/sub/split-msg"
expect 2 "missing message file" "$guard" message "$repo/sub/no-such-msg"
expect 2 "missing message file, relative" in_dir "$repo/sub" "$guard" message no-such-msg
expect 1 "relative path from a subdirectory, term" in_dir "$repo/sub" "$guard" message term-msg
expect 0 "relative path from a subdirectory, clean" in_dir "$repo/sub" "$guard" message clean-msg
expect 2 "message file is a directory" "$guard" message "$repo/sub"
cp "$repo/sub/clean-msg" "$work/locked-msg"
chmod 000 "$work/locked-msg"
expect 2 "message file unreadable" "$guard" message "$work/locked-msg"
expect 2 "no message file argument" "$guard" message
expect 0 "message outside a git repository" in_dir "$work" "$guard" message "$repo/sub/clean-msg"

echo "== staged"
expect 0 "nothing staged, no commit yet" in_dir "$repo" "$guard" staged
printf 'clean content\n' >"$repo/clean.txt"
git -C "$repo" add clean.txt
expect 0 "clean staged file" in_dir "$repo" "$guard" staged
expect 0 "clean staged file, from a subdirectory" in_dir "$repo/sub" "$guard" staged
printf 'line one\nmentions %s\n' "$term" >"$repo/term.txt"
git -C "$repo" add term.txt
expect 1 "term in staged file" in_dir "$repo" "$guard" staged
git -C "$repo" rm -q --cached term.txt
printf 'about %s\n%s here\n' "$pair_a" "$pair_b" >"$repo/split.md"
git -C "$repo" add split.md
expect 1 "two-word term split across lines in staged file" in_dir "$repo" "$guard" staged
git -C "$repo" rm -q --cached split.md
printf 'see %s/x\n' "$home_mac" >"$repo/home.txt"
git -C "$repo" add home.txt
expect 1 "home path in staged file" in_dir "$repo" "$guard" staged
git -C "$repo" rm -q --cached home.txt
printf 'clean\n' >"$repo/$term-notes.txt"
git -C "$repo" add "$term-notes.txt"
expect 1 "term in staged file name (name withheld)" in_dir "$repo" "$guard" staged
printf 'also %s\n' "$term" >"$repo/$term-notes.txt"
git -C "$repo" add "$term-notes.txt"
expect 1 "term in a file whose name has one (location withheld)" in_dir "$repo" "$guard" staged
git -C "$repo" rm -q --cached "$term-notes.txt"
printf 'binary\0%s\0data' "$term" >"$repo/blob.bin"
git -C "$repo" add blob.bin
expect 1 "term in a binary file" in_dir "$repo" "$guard" staged
git -C "$repo" rm -q --cached blob.bin
ln -s "$home_linux/target" "$repo/link"
git -C "$repo" add link
expect 1 "home path as a symlink target" in_dir "$repo" "$guard" staged
git -C "$repo" rm -q --cached link
expect 1 "term in author identity" in_dir "$repo" env GIT_AUTHOR_NAME="${term^} Person" "$guard" staged
expect 1 "term in committer email" in_dir "$repo" env GIT_COMMITTER_EMAIL="me@$term.example" "$guard" staged
printf 'not an index' >"$work/bad-index"
expect 2 "corrupt index (git grep fails)" in_dir "$repo" env GIT_INDEX_FILE="$work/bad-index" "$guard" staged
expect 2 "outside a git repository" in_dir "$work" "$guard" staged
expect 0 "clean index again" in_dir "$repo" "$guard" staged

echo "== tree"
expect 2 "no commit yet" in_dir "$repo" "$guard" tree
git -C "$repo" commit -q -m "Add a clean file"
expect 0 "clean tree" in_dir "$repo" "$guard" tree
printf 'mentions %s\n' "$term" >"$repo/term.txt"
git -C "$repo" add term.txt
git -C "$repo" commit -q -m "Add a file"
expect 1 "term in a file of HEAD" in_dir "$repo" "$guard" tree
git -C "$repo" rm -q term.txt
printf 'clean\n' >"$repo/$term.txt"
git -C "$repo" add "$term.txt"
git -C "$repo" commit -q -m "Rename"
expect 1 "term in a file name of HEAD (name withheld)" in_dir "$repo" "$guard" tree
git -C "$repo" rm -q "$term.txt"
git -C "$repo" commit -q -m "Remove"
expect 0 "clean tree again" in_dir "$repo" "$guard" tree
ln -s "$home_windows" "$repo/winlink"
git -C "$repo" add winlink
git -C "$repo" commit -q -m "Add a link"
expect 1 "home path as a symlink target in HEAD" in_dir "$repo" "$guard" tree
git -C "$repo" rm -q winlink
git -C "$repo" commit -q -m "Remove the link"
expect 0 "clean tree after removing the link" in_dir "$repo" "$guard" tree
printf 'about %s\n  %s here\n' "$pair_a" "$pair_b" >"$repo/split.md"
git -C "$repo" add split.md
git -C "$repo" commit -q -m "Add a note"
expect 1 "two-word term split across lines in a file of HEAD" in_dir "$repo" "$guard" tree
printf 'about %s \t %s here\n' "$pair_a" "$pair_b" >"$repo/split.md"
git -C "$repo" commit -q -am "Edit the note"
expect 1 "two-word term, tab and spaces, in a file of HEAD" in_dir "$repo" "$guard" tree
printf 'binary\0%s\r\n%s\0data' "$pair_a" "$pair_b" >"$repo/split.md"
git -C "$repo" commit -q -am "Edit the note"
expect 1 "two-word term split across lines in a binary file of HEAD" in_dir "$repo" "$guard" tree
git -C "$repo" mv split.md "sub/$term.md"
git -C "$repo" commit -q -m "Move the note"
expect 1 "split two-word term in a file named with a term (location withheld)" in_dir "$repo" "$guard" tree
git -C "$repo" mv "sub/$term.md" split.md
printf 'about %s\nthen %s, %s\n' "$pair_b" "$pair_a" "$pair_b" >"$repo/split.md"
git -C "$repo" commit -q -am "Keep the words apart"
expect 0 "the words of a two-word term apart, not as the term" in_dir "$repo" "$guard" tree
git -C "$repo" rm -q split.md
git -C "$repo" commit -q -m "Remove the note"
expect 0 "clean tree after removing the split term" in_dir "$repo" "$guard" tree

echo "== history"
# The history still holds the term (an old file and an old file name).
expect 1 "term in files of older commits" in_dir "$repo" "$guard" history
hist="$work/hist"
git init -q -b main "$hist"
expect 0 "empty repository" in_dir "$hist" "$guard" history
printf 'clean\n' >"$hist/a.txt"
git -C "$hist" add a.txt
git -C "$hist" commit -q -m "Initial commit"
expect 0 "clean history" in_dir "$hist" "$guard" history
git -C "$hist" commit -q --allow-empty -m "Fix for $term"
expect 1 "term in a commit message" in_dir "$hist" "$guard" history
git -C "$hist" reset -q --hard HEAD~1
git -C "$hist" commit -q --allow-empty -m "Fix for $pair_a" -m "$pair_b users"
expect 1 "two-word term split in a commit message" in_dir "$hist" "$guard" history
git -C "$hist" reset -q --hard HEAD~1
git -C "$hist" commit -q --allow-empty -m "Moved from $home_linux"
expect 1 "home path in a commit message" in_dir "$hist" "$guard" history
git -C "$hist" reset -q --hard HEAD~1
GIT_AUTHOR_EMAIL="dev@$term.example" git -C "$hist" commit -q --allow-empty -m "Change"
expect 1 "term in an author identity" in_dir "$hist" "$guard" history
git -C "$hist" reset -q --hard HEAD~1
git -C "$hist" reflog expire --expire=now --all
git -C "$hist" gc -q --prune=now
expect 0 "clean again after rewriting" in_dir "$hist" "$guard" history
git -C "$hist" tag -a v1 -m "Release for $term"
expect 1 "term in an annotated tag message" in_dir "$hist" "$guard" history
git -C "$hist" tag -d v1 >/dev/null
GIT_COMMITTER_NAME="${term^} Tagger" git -C "$hist" tag -a v1 -m "Release"
expect 1 "term in a tagger identity" in_dir "$hist" "$guard" history
git -C "$hist" tag -d v1 >/dev/null
git -C "$hist" branch "feature-$term"
expect 1 "term in a branch name (name withheld)" in_dir "$hist" "$guard" history
git -C "$hist" branch -q -D "feature-$term"
git -C "$hist" tag "$term-1"
expect 1 "term in a lightweight tag name" in_dir "$hist" "$guard" history
git -C "$hist" tag -d "$term-1" >/dev/null
git -C "$hist" notes add -m "reviewed by $term" HEAD
expect 1 "term in a git note" in_dir "$hist" "$guard" history
git -C "$hist" notes remove HEAD 2>/dev/null
git -C "$hist" update-ref -d refs/notes/commits
git -C "$hist" reflog expire --expire=now --all
git -C "$hist" gc -q --prune=now
expect 0 "clean again" in_dir "$hist" "$guard" history
# A missing object: git grep cannot read a tree, the guard must not pass.
broken="$work/broken"
git init -q -b main "$broken"
printf 'clean\n' >"$broken/a.txt"
git -C "$broken" add a.txt
git -C "$broken" commit -q -m "Initial commit"
blob=$(git -C "$broken" rev-parse HEAD:a.txt)
rm -f "$broken/.git/objects/${blob:0:2}/${blob:2}"
expect 2 "history with a missing object" in_dir "$broken" "$guard" history
expect 2 "tree with a missing object" in_dir "$broken" "$guard" tree
expect 2 "index with a missing object" in_dir "$broken" "$guard" staged
links="$work/links"
git init -q -b main "$links"
ln -s "../$term/shared" "$links/shared"
git -C "$links" add shared
git -C "$links" commit -q -m "Add a link"
git -C "$links" rm -q shared
git -C "$links" commit -q -m "Remove the link"
expect 0 "symlink with a term removed from HEAD: tree clean" in_dir "$links" "$guard" tree
expect 1 "symlink with a term in an older commit" in_dir "$links" "$guard" history
splits="$work/splits"
git init -q -b main "$splits"
printf 'clean\n' >"$splits/a.txt"
printf 'see %s\n' "$pair_b" >"$splits/one-word.txt"
git -C "$splits" add a.txt one-word.txt
git -C "$splits" commit -q -m "Initial commit"
expect 0 "one word of a two-word term alone in history" in_dir "$splits" "$guard" history
printf 'Notes for %s\n%s.\n' "${pair_a^}" "${pair_b^^}" >"$splits/notes.md"
git -C "$splits" add notes.md
git -C "$splits" commit -q -m "Add notes"
cp "$splits/notes.md" "$splits/copy.md"
git -C "$splits" add copy.md
git -C "$splits" commit -q -m "Copy the notes"
git -C "$splits" rm -q notes.md copy.md
git -C "$splits" commit -q -m "Remove the notes"
expect 0 "split two-word term removed from HEAD: tree clean" in_dir "$splits" "$guard" tree
expect 1 "two-word term split across lines in files of older commits" in_dir "$splits" "$guard" history
git -C "$splits" reset -q --hard HEAD~3
git -C "$splits" reflog expire --expire=now --all
git -C "$splits" gc -q --prune=now
expect 0 "clean history after rewriting" in_dir "$splits" "$guard" history

echo "== pre-push hook"
pushrepo="$work/push"
git init -q -b main "$pushrepo"
mkdir -p "$pushrepo/scripts"
cp "$guard" "$pushrepo/scripts/"
git -C "$pushrepo" add scripts
git -C "$pushrepo" commit -q -m "Add the guard"
head=$(git -C "$pushrepo" rev-parse HEAD)
zero=0000000000000000000000000000000000000000
expect 0 "push of main" in_dir "$pushrepo" with_text "refs/heads/main $head refs/heads/main $zero\n" "$prepush" origin url
expect 1 "push to a remote branch named with a term" \
    in_dir "$pushrepo" with_text "refs/heads/main $head refs/heads/$term $zero\n" "$prepush" origin url
expect 0 "nothing to push" in_dir "$pushrepo" with_text '' "$prepush" origin url
git -C "$pushrepo" commit -q --allow-empty -m "Note for $term"
expect 1 "push of a history with a term" in_dir "$pushrepo" with_text "refs/heads/main $head refs/heads/main $zero\n" "$prepush" origin url

echo "== Claude Code hook"
files="$work/files"
mkdir -p "$files/$term-dir"
printf 'A clean body.\n' >"$files/clean.md"
printf 'A clean body.\n' >"$files/$term-dir/clean.md"
printf 'Body with\n%s inside.\n' "${term^}" >"$files/dirty.md"
printf 'Body for %s\n%s.\n' "$pair_a" "$pair_b" >"$files/split.md"
: >"$files/empty.md"
# hook_json <command>: the hook input of a Bash tool call run in $files.
hook_json() { jq -n --arg c "$1" --arg d "$files" '{tool_name: "Bash", cwd: $d, tool_input: {command: $c}}'; }
run_hook() { hook_json "$1" | "$hook"; }
expect 0 "unrelated command" run_hook "ls -la $files"
expect 0 "read-only gh" run_hook "gh issue view 13 && cat $files/dirty.md"
expect 0 "gh api GET" run_hook "gh api repos/o/r/issues/3"
expect 0 "clean gh issue create" run_hook "gh issue create --title 'Fix the parser' --body-file $files/clean.md"
expect 2 "body file with a term" run_hook "gh issue create --title 'Fix the parser' --body-file $files/dirty.md"
expect 2 "body file with a split two-word term" run_hook "gh issue create --title t --body-file $files/split.md"
expect 2 "relative -F body file with a term" run_hook "gh issue comment 3 -F dirty.md"
expect 2 "cd, then a relative body file" run_hook "cd $files && gh pr comment 3 --body-file=dirty.md"
expect 0 "empty body file" run_hook "gh issue comment 3 --body-file $files/empty.md"
expect 2 "term in the title" run_hook "gh issue create --title 'Support ${term}' --body x"
expect 2 "home path in the body" run_hook "gh issue create --title t --body 'see $home_linux/x'"
expect 2 "body file that does not exist yet" run_hook "gh issue create --title t --body-file $files/missing.md"
expect 2 "body from a pipe" run_hook "cat $files/clean.md | gh issue create --title t -F -"
expect 0 "body from a redirected clean file" run_hook "gh issue create --title t -F - < $files/clean.md"
expect 2 "body from a redirected file with a term" run_hook "gh issue create --title t -F - < $files/dirty.md"
expect 0 "clean heredoc body with HTML" run_hook "$(printf '%s\n' "gh pr create --title t --body-file - <<'EOF'" '<code>ok</code> and cat /x/y' 'EOF')"
expect 2 "heredoc body with a split two-word term" run_hook "$(printf '%s\n' "gh pr create --title t -F - <<'EOF'" "about $pair_a" "$pair_b here" 'EOF')"
html_cmd=$(printf '%s\n' "gh pr create --title t --body \"\$(cat <<'EOF'" "<code>$home_linux</code>" "EOF" ")\"")
expect 2 "heredoc body with a home path in HTML" run_hook "$html_cmd"
expect 2 "body from cat of a file with a term" run_hook "gh pr create --title t --body \"\$(cat $files/dirty.md)\""
expect 2 "body from \$(< file) with a term" run_hook "gh pr create --title t --body \"\$(<$files/dirty.md)\""
expect 0 "path through a directory named with a term" run_hook "gh issue create --title t --body-file $files/$term-dir/clean.md"
expect 0 "cd into a directory named with a term" run_hook "cd $files/$term-dir && gh issue create --title t -F clean.md"
expect 2 "gh api field from a file with a term" run_hook "gh api repos/o/r/issues -f title=x -F body=@$files/dirty.md"
expect 0 "gh api field from a clean file" run_hook "gh api repos/o/r/issues -f title=x -F body=@$files/clean.md"
expect 2 "gh api literal field with a term" run_hook "gh api -X PATCH repos/o/r/issues/3 -f title='about $term'"
expect 2 "gh api graphql with a term" run_hook "gh api graphql -f query='mutation { x(body: \"$term\") }'"
expect 2 "release asset with a term" run_hook "gh release create v1 $files/dirty.md --notes ok"
expect 0 "clean release asset in a directory named with a term" run_hook "gh release create v1 $files/$term-dir/clean.md --notes-file $files/clean.md"
expect 2 "release notes file with a term" run_hook "gh release create v1 --notes-file $files/dirty.md"
expect 2 "gh repo edit description with a term" run_hook "gh repo edit --description 'for $term'"
expect 2 "git tag message with a term" run_hook "git tag -a v1 -m 'release for $term'"
expect 2 "git tag message file with a term" run_hook "git tag v1 -F $files/dirty.md"
expect 2 "git note with a term" run_hook "git -C $files notes add -m 'by $term'"
expect 0 "clean git note" run_hook "git -C $files notes add -m 'reviewed'"
expect 2 "no term list: blocked" without_list run_hook "gh issue create --title t --body x"
expect 2 "hook input that is not JSON" with_text 'not json' "$hook"
expect 0 "a tool call without a command" with_text '{"tool_name":"Bash","tool_input":{}}' "$hook"

echo
echo "$passed passed, $failed failed"
[ "$failed" -eq 0 ]
