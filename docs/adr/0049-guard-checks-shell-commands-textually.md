---
id: ADR-0049
title: The guard checks shell commands as text at three anchors; heredoc bodies are data unless an interpreter reads them
status: accepted
date: 2026-10-14
crates: [xtask]
tags: [quality, ai, security]
supersedes: []
superseded_by: null
---

## Context

ADR-0043 decision 4 says "paths are normalised (`.` and `..` collapsed) before any check". The
guard does this for the paths of Edit/Write calls. A shell command has no single path: the
guard matches its text at the three anchors of ADR-0043 decision 3 and never collapses
`name/..` in it. The sentence read as if `rm -rf missing/../xtask` were caught; it is not.

Bare harness directory names (`rm -rf xtask`) count as harness paths in shell commands. That made
commit messages and notes written through a heredoc (`git commit -F - <<'EOF'` with the subject
`xtask: …`) count as harness writes. A heredoc body is data for `cat` or `git`, but code for
`bash <<EOF` or `cat <<EOF | python3`.

## Decision

1. **Collapsing applies to Edit/Write paths only.** Shell commands are checked as text: a
   harness fragment counts at the start of a token, after a `./` that starts a token, or right
   after the repository root (ADR-0043 decision 3). A detour through `..` in a shell command
   (`rm -rf missing/../xtask`) is a known gap, like the `cd xtask && …` indirection.
2. **Comments and heredoc bodies of the safe shape are data:** a bare harness directory
   name in them is no path. A path with its slash (`xtask/…`) still counts everywhere. A
   comment starts only where the shell starts one: at an unquoted, unescaped `#` that starts
   a word (`$(true)#`, `\ #` and `'a'#` start none), and never inside an open `${…}`, also
   nested and inside double quotes or `$(…)` (`${x:- #}` is one word). A heredoc body is
   data only when its logical line (comments removed, backslash-newlines joined) has a
   single heredoc and nothing after `<<MARKER` (also `<<-`, `<<'M'`, `<<"M"`) but at most
   one output redirection to a file (`> path`, `>> path`). Anything else after the marker
   (a pipe, `;`, `&&`, `&`, a subshell or brace group, `>(…)`, a second redirection,
   another word) keeps the body as shell text, so `cat <<EOF | tee /tmp/x` is checked too.
3. **Only the exact message forms make a body data**, where the body ends up in a message
   or a file and never runs (programs also by path):
   - `git commit` / `git tag` reading it with `-F -` (`--file=-`, `--file -`), or as
     `-m "$(cat <<'EOF' …)"` / `--message="$(cat <<…)"`. No global option before the
     subcommand (`git -C …`, `git -c …`); options after it (`--amend`, `-a`) are fine. A
     `-F <file>` other than `-` keeps the body.
   - `gh pr create|edit`, `gh issue create|edit` with `--body-file -` or
     `--body "$(cat <<'EOF' …)"`.
   - `cat` writing one regular file: `cat > path <<EOF`, `cat <<EOF > path`,
     `cat >> path`. `cat` and exactly one redirection `>` or `>>` without a descriptor
     number (not `1>`, `2>`, `>|`), before or after the marker, and no other word. The
     target is literal (no `$`, `*`, `?`, `[`, `{` or backtick, which the shell expands:
     `/de?/stdout`, `$TTY`; `~` is fine) and has no `dev` or `proc` path component,
     compared ignoring case (`/dev/stdout`, `//dev/tty`, `/./dev/stdout`,
     `../../dev/stdout`, `/DEV/stdout`, `/proc/self/fd/1`); `/tmp/devnotes` or
     `/tmp/proc.md` are files. `tee` is no message form: it copies the body to stdout as
     well.

   Only that command's words precede the marker, counted from the start of the logical line
   or, for the `$(cat <<EOF` forms, the `git`/`gh` command before the `$(`. No `;`, `&`, `|`,
   `(`, `)`, `{`, `}` besides that `$(`, no other program in front (`env cat`, `X=1 git`,
   `cd /tmp && cat`). Every other heredoc (`bash <<EOF`, `{ bash; } <<EOF`,
   `while read …; done <<EOF`, `git rm --pathspec-from-file=- <<EOF`, `gh api … <<EOF`,
   `cat <<EOF` without a file) keeps its body as shell text, which is then checked like
   inline code with the cargo exceptions (`cargo xtask …`, `-p xtask`) as elsewhere. This
   allowlist replaces a list of interpreters, which could never be complete, and a list of
   reader programs, whose other subcommands read a heredoc as paths, config or code.

   **The message form stands alone:** the whole Bash command is the message-form command.
   Outside comments and blank lines there is nothing before the heredoc's logical line
   (backslash-newline joins lines) and nothing after the terminator line. For the
   `"$(cat <<'EOF'` forms the closing line `)"` may follow, optionally with further plain
   arguments on that line (closed quotes; no `;`, `&`, `|`, `<`, `>`, `(`, `)`, backtick,
   `$` or backslash). Otherwise the body is shell text. A command before the heredoc can
   route its output into a shell (`exec > >(bash)`, a fifo read by `bash`, a symlink to
   `/dev/stdout`), a command after it can run the file it wrote (`bash /tmp/s`); the rule
   also denies harmless neighbours (`git add -A` newline `git commit -F -`), which is the
   price of not modelling them.

   **An unquoted marker expands the body:** bash and zsh run `$(…)` and `${…}` in the body
   of `<<EOF` before git, gh or cat reads it. Under an unquoted marker (the marker word has
   no `'`, `"` or `\`) the body is data only if it contains no `$(` and no `${`; an escaped
   `\$(` counts too. Quoted markers (`<<'EOF'`, `<<"EOF"`, `<<\EOF`, `<<E'O'F`,
   `<<-'EOF'`) keep the body literal. `$NAME` and `$5` run nothing and stay data;
   backticks and `$((` fail closed (decision 4). A body under an unquoted marker with a
   line ending in a backslash is never data: bash and zsh join it with the next line before
   expanding (`$\` newline `(` is a substitution) and before comparing it with the
   terminator (`EO\` newline `F` ends the body, the lines after it run).

   **Early terminators inside `$(…)`:** bash (and sh) end a heredoc inside an open `$(` at a
   line such as `EOF)`, `EOF )`, `EOFX)`, bash 3.2 also after leading blanks; zsh does not.
   So inside `$(…)` the body is data only if no body line before the terminator starts,
   after optional blanks and tabs, with the marker word (fail closed). Outside `$(…)` no
   shell does this and nothing changes.
4. **Fail closed:** comments and heredoc bodies are removed only when the guard models
   every construct of the command. A command that contains anywhere a backtick, ANSI-C
   quoting `$'`, a redirection with `&` (`|&`, `>&`, `&>`, `<&`, so also `2>&1` and `>&2`), or
   arithmetic `((` (also `$((`, whose `<<` is a shift and whose `$(…)` runs) keeps its prose:
   every word is checked, as if there were no comments or heredocs. The same holds when any
   heredoc keeps its body as shell text or has no terminator: the quote and comment tracking
   cannot follow a body (`it's`), so no comment and no other body is removed. A heredoc
   inside a compound command (`{` newline `cat <<EOF` … `}` `| bash`) never stands alone
   (decision 3), so its body is shell text. Rare spellings are denied rather than parsed;
   the word rules (`xtask:`, the cargo exceptions) still apply.
5. **sed scripts that write or run are writers**, like `sed -i`. `sed` and `gsed`, also by
   path, count as writing when a script contains the command `w file`/`W file` (with any
   address, inside `{…}`, after `;` or a newline), the `w file` flag of `s` (also after
   other flags, any delimiter), or GNU's command `e` or `e` flag of `s`, which runs a shell
   command. A script is the first non-option argument before any `-e`/`-f` (BSD sed's
   script) and the `-e`/`--expression` arguments joined by newlines. The command is split
   into words quote-aware (quotes, backslashes, comments, redirections, `$(…)` and
   backticks also inside double quotes), and the script is read with the grammar of BSD
   and GNU sed: a `w`/`e` in a regex, replacement, `a`/`i`/`c` text, `y` list or label is
   none. The regex is read twice, with and without BSD's bracket expressions (in BSD sed
   `s/[/]/x/` has the regex `[/]`), and either reading counts. A syntax error ends the
   scan, because sed then runs nothing.

   **A script the guard cannot read counts as writing:** a script file (`-f`, `--file`,
   `--file=`, also in a group like `-nf`), a script word the shell expands (a `$` outside
   single quotes, a backtick, `$'…'`; `'$p'` is literal), and xargs' and find's
   placeholder `{}` as the script. Only the script words count: a `$` in a file argument
   (`sed -n 1p "$f"`) makes no writer.
6. **In-place editors and inline code count inside substitutions.** The programs of the
   in-place and inline-code tables (`sed -i`, `perl -pi`, `ruby -e`, `bash -c` …) are also
   recognised as the first word inside `$(…)`, backticks and `(…)` subshells, using the
   same quote-aware split into simple commands as decision 5. `echo $(sed -n 1p f)` only
   reads and passes.

## Consequences

- Commit messages, PR bodies and notes through a heredoc no longer trip the guard.
- `bash <<EOF`, `cat <<EOF | sh`, `{ bash; } <<EOF`, `while read … done <<EOF` and friends
  cannot hide a harness write in the body.
- The heuristic stays a heuristic: a body read through an indirection the guard does not
  see is not checked. Examples are a script written first and run by a later Bash call
  (`cat > /tmp/x.sh <<EOF` …, then `bash /tmp/x.sh`), a symlink or fifo prepared by an
  earlier Bash call (`cat > /tmp/p <<EOF` whose target is read by a process started
  before), and a commit message run as code afterwards. The `$(cat <<EOF` heredocs of the
  message forms stay data, because the commit and PR body forms
  `git commit -m "$(cat <<'EOF' …)"` and `gh pr create --body "$(cat <<'EOF' …)"` must pass;
  an unknown command in front of the `$(` (`rm -rf $(cat <<EOF`) keeps the body. CI
  and CODEOWNERS remain the wall (ADR-0037).
- A commit message or note that names a harness directory is denied when the same command
  uses an unmodelled construct (`2>&1`, backticks, `$((…))`), pipes the heredoc on
  (`cat <<EOF | tee /tmp/x`), puts anything in front of or after the command
  (`cd /tmp && cat`, `git add -A` on the line before) or uses another form
  (`git -C repo commit -F -`, `tee path <<EOF`, `cat <<EOF` to the terminal or a device).
  Agents write the note with `cat > file <<EOF`, `git commit -F - <<'EOF'` or split the
  command into separate Bash calls.
- sed with a script file or an expanded script is denied next to a harness path even when
  the script only reads; agents write the script literally in the command.
- Clarifies ADR-0043 decision 4 without changing it for Edit/Write.

## Rejected alternatives

- **Collapsing `..` in shell commands:** the guard does not know the working directory of
  each word or which words are paths; collapsing text would invent paths and still miss `cd`.
- **Checking every heredoc body:** denies ordinary commit messages and notes that name the
  harness, which pushes agents towards workarounds.
- **Parsing the rare constructs too** (backticks, `$'…'`, `|&`, pipelines continued after the
  body over comment and blank lines): each one was a way to hide a harness write behind
  "prose", and every parser addition is a new place for such a gap. Denying them costs little.

## References

- Code: `xtask/src/guard.rs` (`models_prose`, `without_prose`, `is_message_form`,
  `stands_alone`, `expands`, `ends_early`, `writes_regular_file`, `only_redirects_to_file`,
  `simple_commands`, `sed_writes`, `sed_scripts`, `runs_with_flag`, `harness_in_shell`)
- Sabotage cases: `harness/sabotage/guard-harness-shell-after-heredoc.case`,
  `guard-harness-shell-interpreter-heredoc.case`, `guard-prose-heredoc-body-checked.case`,
  `guard-harness-shell-fail-closed-prose.case`, `guard-harness-shell-backtick-comment.case`,
  `guard-harness-shell-heredoc-{tee,cat-stderr,device,not-alone}.case`,
  `guard-harness-shell-heredoc-{unquoted-substitution,cat-target-glob}.case`,
  `guard-harness-shell-heredoc-{split-substitution,paren-terminator}.case`,
  `guard-harness-shell-sed-{write,execute,expanded-script,script-file}.case`,
  `guard-harness-shell-in-place-in-substitution.case`
- Related: ADR-0037, ADR-0043
