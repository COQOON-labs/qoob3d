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
2. **Heredoc bodies are data**, as are comments: a bare harness directory name in them is no
   path. A path with its slash (`xtask/…`) still counts everywhere. A comment starts only
   where the shell starts one: at an unquoted, unescaped `#` that starts a word (`$(true)#`,
   `\ #` and `'a'#` start none).
3. **Unless an interpreter reads the body:** when a pipeline with the heredoc names a shell
   or interpreter (sh, bash, zsh, python with any version, perl, ruby, node, deno, also by path),
   or `xargs`, `eval`, `source`, `.`, the body is checked like inline code, with the cargo
   exceptions (`cargo xtask …`, `-p xtask`) as elsewhere. Any word of the pipeline counts,
   not only a program (`env bash`, `FOO=1 bash`). The pipeline is read as the shell reads it:
   a backslash-newline joins lines, and a `|` that ends the heredoc line continues with the
   command after the terminator line, joined over further lines while one ends in `|` or a
   backslash (`cat <<EOF |` … `EOF` / `tee /tmp/y |` / `bash`).

## Consequences

- Commit messages, PR bodies and notes through a heredoc no longer trip the guard.
- `bash <<EOF`, `cat <<EOF | sh` and friends cannot hide a harness write in the body.
- The heuristic stays a heuristic: a body read through an indirection the guard does not
  see is not checked. Examples are `while read f; do rm -rf "$f"; done <<EOF`, a command
  substitution that turns the body into arguments (`rm -rf $(cat <<EOF` …), and a script
  written first and run later. Heredocs inside `$(…)` stay data, because the PR body form
  `gh pr create --body "$(cat <<'EOF' …)"` must pass. CI and CODEOWNERS remain the wall
  (ADR-0037).
- A word naming an interpreter anywhere in a continued pipeline makes the body code, also as
  a mere argument (`grep -c bash`); the guard denies rather than guesses.
- Clarifies ADR-0043 decision 4 without changing it for Edit/Write.

## Rejected alternatives

- **Collapsing `..` in shell commands:** the guard does not know the working directory of
  each word or which words are paths; collapsing text would invent paths and still miss `cd`.
- **Checking every heredoc body:** denies ordinary commit messages and notes that name the
  harness, which pushes agents towards workarounds.

## References

- Code: `xtask/src/guard.rs` (`without_prose`, `feeds_interpreter`, `harness_in_shell`)
- Sabotage cases: `harness/sabotage/guard-harness-shell-after-heredoc.case`,
  `guard-harness-shell-interpreter-heredoc.case`, `guard-prose-heredoc-body-checked.case`
- Related: ADR-0037, ADR-0043
