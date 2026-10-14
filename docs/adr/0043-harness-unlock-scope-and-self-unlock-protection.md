---
id: ADR-0043
title: The harness unlock lifts only the harness-file protection; agents can never unlock themselves
status: accepted
date: 2026-10-14
crates: [xtask]
tags: [quality, ai, security]
supersedes: []
superseded_by: null
---

## Context

ADR-0037 lets the human unlock harness work by starting Claude Code with
`QOOB3D_HARNESS_UNLOCK=1`. Three gaps showed up in practice:

1. `guard.sh` exited before calling `cargo xtask guard` when the variable was set. An unlocked
   session therefore also lost fixture protection, the role separation of ADR-0039 and the
   bypass detection, although the unlock was only meant for harness files.
2. ADR-0037 assumed that an agent cannot set the environment of the hook process. It can:
   the `env` block of Claude settings files (user, project-local, managed) and the environment
   settings of the VS Code extension reach hooks, partly in the running session. Shell startup
   files and `launchctl setenv` reach every later session. An agent that writes one of these
   files unlocks itself, and an unlocked session could make the unlock permanent.
3. The shell heuristic matched harness path fragments anywhere in a command. `$HOME/.cargo/bin`
   (contains `.cargo/`) together with `2>&1` (contains `>`) blocked ordinary commands, and
   writes to `~/.claude/projects/…` were denied because of `.claude/`.

## Decision

1. **Unlock scope:** `cargo xtask guard` reads `QOOB3D_HARNESS_UNLOCK` itself (`Session`).
   The unlock lifts only the protection of harness files (Edit/Write and shell writes).
   Fixture protection, role separation and bypass detection stay active. `guard.sh` always
   runs the guard binary.
2. **Self-unlock protection, always active (also when unlocked):** Edit/Write is denied for
   Claude settings outside the repository (`~/.claude/settings*.json`, managed settings),
   `.claude/settings.local.json`, VS Code settings (`.vscode/settings.json`, the user
   settings of VS Code) and shell startup files (`~/.zshrc`, `~/.zprofile`, `~/.zshenv`,
   `~/.bashrc`, `~/.bash_profile`, `~/.profile`) and everything under `.git/` (e.g. the hooks
   path in `.git/config`). A shell command that mentions one of these files is denied unless it
   is a pure read (no redirection, every segment starts with a read-only program such as `cat`,
   `grep`, `head`). Commands that run `launchctl setenv`, that name `QOOB3D_HARNESS_UNLOCK` or
   `QOOB3D_SKIP_REVIEW`, or that bypass the git hooks (`--no-verify`, `git commit -n`,
   `core.hooksPath` in any case, abbreviations such as `--no-veri`) are denied. Matching removes
   shell quotes first and ignores case, because git keys and the macOS file system do, so
   `~/.zsh''env`, `~/.ZSHENV` or `core.hookspath` do not slip through.
3. **Harness paths in shell commands** count only if they refer to this repository: at the
   start of a token, after `./`, or right after the repository root (spelled out or as
   `$CLAUDE_PROJECT_DIR`/`$PWD`). Crate manifests (`crates/*/Cargo.toml`) are not harness
   files and no longer match.
4. **Paths are normalised** (`.` and `..` collapsed) before any check, so
   `missing/../xtask/x.rs` is recognised as a harness file.
5. If the guard binary does not build during harness work, `guard.sh` falls back to the last
   built binary instead of blocking every tool call; without any binary it still blocks.
6. **The guard binary lives under `.git/qoob3d/guard-target/`**, which agents can write
   neither with Edit/Write nor through the shell. A binary in `target/` could be replaced
   through `target/debug/deps/`, which cargo copies over `target/debug/xtask` on every build.
   The self-test runs the real hook script in its own copy of the repository.

## Consequences

- An unlocked session is still bound by fixture, role and bypass rules; harness work goes
  through `test-author` and `implementer` like any other change.
- **Limit:** an unlocked session may change the guard itself (`xtask/src/guard.rs`,
  `.claude/hooks/guard.sh`, `.claude/settings.json`), and the changed guard applies from the next
  tool call. "Always active" therefore holds against agents that do not rewrite the guard; an
  unlocked session trusts the agent with the guard. Such edits are visible in the diff and need
  the owner's review before they reach `main`, but their effect on files outside the repository
  is not reviewed.
- Settings changes outside the repository (permissions, env, editor settings) are made by the
  human. The repository's own `.claude/settings.json` stays a harness file (versioned, owner
  review).
- The shell heuristic stays a heuristic: a write through an indirection the guard does not
  understand (e.g. `cd xtask && …`, `sh -c "$(…)"`, base64) is not detected. CI and
  CODEOWNERS remain the wall; for files outside the repository (shell startup files) there is
  no wall, which is why reads are the only shell access allowed there.
- Refines ADR-0037 (decision 2 and the consequence about false positives).

## Rejected alternatives

- **`permissions.deny` for the settings files:** would also block the human's own sessions and
  cannot be lifted per session.
- **Checking the content of edits for the variable name:** easy to evade (string building),
  while denying the files themselves is simple and complete for the known paths.

## References

- Code: `xtask/src/guard.rs`, `.claude/hooks/guard.sh`
- Sabotage cases: `harness/sabotage/guard-unlocked-*.case`, `guard-sh-unlocked-fixture.case`
  (runs the real hook script), `guard-self-unlock-*.case`, `guard-launchctl-setenv.case`,
  `guard-git-config-edit.case`, `guard-dot-dot-harness-edit.case`,
  `guard-harness-shell-dot-slash.case`, `guard-commit-short-no-verify.case`,
  `guard-hookspath-lowercase.case`, `guard-abbreviated-no-verify.case`,
  `guard-deps-binary.case`, `guard-git-dir-shell.case`, `guard-skip-review.case`,
  `guard-admin-merge.case`
- Related: ADR-0037, ADR-0039, ADR-0042
