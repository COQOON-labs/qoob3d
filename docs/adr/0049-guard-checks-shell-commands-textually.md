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
   a word (`$(true)#`, `\ #` and `'a'#` start none). A heredoc body is data only when its
   logical line (comments removed, backslash-newlines joined) has a single heredoc and
   nothing after `<<MARKER` (also `<<-`, `<<'M'`, `<<"M"`) but at most one output
   redirection to a file (`> path`, `>> path`). Anything else after the marker (a pipe,
   `;`, `&&`, `&`, a subshell or brace group, `>(…)`, a second redirection, another word)
   keeps the body as shell text, so `cat <<EOF | tee /tmp/x` is checked too.
3. **Only a known data reader makes a body data:** the simple command carrying the heredoc
   is `cat`, `tee`, `git` or `gh` (also by path), and only its words precede the marker,
   counted from the start of the logical line or from right after a `$(` that is still open
   (`git commit -m "$(cat <<'EOF'`, `gh pr create --body "$(cat <<'EOF'`; the command before
   the `$(` must be such a reader too). No `;`, `&`, `|`, `(`, `)`, `{`, `}` before the
   marker, no other program in front (`env cat`, `sudo cat`, `X=1 cat`, `cd /tmp && cat`).
   Every other heredoc (`bash <<EOF`, `{ bash; } <<EOF`, `while read …; done <<EOF`, an
   unknown reader) keeps its body as shell text, which is then checked like inline code with
   the cargo exceptions (`cargo xtask …`, `-p xtask`) as elsewhere. This allowlist replaces
   a list of interpreters, which could never be complete (dash, ksh, csh, groups, loops).
4. **Fail closed:** comments and heredoc bodies are removed only when the guard models
   every construct of the command. A command that contains anywhere a backtick, ANSI-C
   quoting `$'`, a redirection with `&` (`|&`, `>&`, `&>`, `<&`, so also `2>&1` and `>&2`), or
   arithmetic `((` (also `$((`, whose `<<` is a shift and whose `$(…)` runs) keeps its prose:
   every word is checked, as if there were no comments or heredocs. Rare spellings are
   denied rather than parsed; the word rules (`xtask:`, the cargo exceptions) still apply.

## Consequences

- Commit messages, PR bodies and notes through a heredoc no longer trip the guard.
- `bash <<EOF`, `cat <<EOF | sh`, `{ bash; } <<EOF`, `while read … done <<EOF` and friends
  cannot hide a harness write in the body.
- The heuristic stays a heuristic: a body read through an indirection the guard does not
  see is not checked. Examples are a reader inside a compound command opened on an earlier
  line whose output goes on (`{` newline `cat <<EOF` … `}` `| bash`), and a script written
  first and run later. Heredocs of a reader inside `$(…)` stay data, because the commit and
  PR body forms `git commit -m "$(cat <<'EOF' …)"` and `gh pr create --body "$(cat <<'EOF'
  …)"` must pass; an unknown command in front of the `$(` (`rm -rf $(cat <<EOF`) keeps the
  body. CI and CODEOWNERS remain the wall (ADR-0037).
- A commit message or note that names a harness directory is denied when the same command
  uses an unmodelled construct (`2>&1`, backticks, `$((…))`), pipes the heredoc on
  (`cat <<EOF | tee /tmp/x`) or puts anything in front of the reader (`cd /tmp && cat`).
  Agents write the note with `cat > file <<EOF`, `git commit -F - <<'EOF'` or split the
  command.
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

- Code: `xtask/src/guard.rs` (`models_prose`, `without_prose`, `reads_data`,
  `only_redirects_to_file`, `harness_in_shell`)
- Sabotage cases: `harness/sabotage/guard-harness-shell-after-heredoc.case`,
  `guard-harness-shell-interpreter-heredoc.case`, `guard-prose-heredoc-body-checked.case`,
  `guard-harness-shell-fail-closed-prose.case`, `guard-harness-shell-backtick-comment.case`
- Related: ADR-0037, ADR-0043
