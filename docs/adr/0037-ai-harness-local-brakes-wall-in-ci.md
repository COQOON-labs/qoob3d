---
id: ADR-0037
title: AI harness in layers – brakes locally, the wall stands in CI and in the GitHub rules
status: accepted
date: 2026-10-14
crates: [xtask]
tags: [quality, ai, security, ci]
supersedes: []
superseded_by: null
---

## Context

AI agents should write as much code as possible without quality dropping. Locally, an agent
runs with the same rights as the human: it can disable hooks, change checks,
loosen thresholds and use `--no-verify`. The stop hook from ADR-0013 gives up after three red
attempts. Whatever the agent can circumvent, it will circumvent at some point, usually not on
purpose, but because it wants to get green.

## Decision

Checks run in layers. Each is deterministic where possible:

| Layer | Where | Kind |
| --- | --- | --- |
| Guard (`PreToolUse`) | Claude Code, `.claude/hooks/guard.sh` → `cargo xtask guard` | brake |
| Stop hook | Claude Code, `check-on-stop.sh` → `cargo xtask check --fast` | brake |
| `commit-msg`, `pre-push` | `.githooks/`, activated by `cargo xtask setup` | brake |
| CI | `.github/workflows/ci.yml`: `policy`, `selftest`, `check (macOS)`, `mutants` | **wall** |
| GitHub rules | Branch protection `main`, required checks, CODEOWNERS review | **wall** |

1. **Harness files** (`paths::is_harness`: `.claude/`, `.githooks/`, `.github/`, `.cargo/`,
   `xtask/`, `harness/`, `Cargo.toml`, `clippy.toml`, `deny.toml`, `rustfmt.toml`,
   `rust-toolchain.toml`, the guard binary) are read-only for agents. The guard
   blocks Edit/Write on them and shell commands that could write them.
2. **Only the human can unlock:** they start Claude Code with `QOOB3D_HARNESS_UNLOCK=1`.
   An agent cannot set the environment of the hook process, and the guard rejects
   shell commands that name the variable. (Refined by ADR-0043: settings files, shell
   startup files and `launchctl` can set it, so the guard protects them as well.)
3. **Circumventions** (`--no-verify`, `core.hooksPath`, `gh pr merge --admin`) are blocked by the
   guard. `cargo xtask check` turns red if the Git hooks are not active locally.
4. **The guard fails closed:** if it cannot run, the call is blocked.
5. **CI** runs all local checks again and adds a secret scan (gitleaks),
   unused dependencies (cargo-machete), mutation tests and the self-test
   (ADR-0042). Actions are pinned by commit SHA, the token has read permissions only.
6. **GitHub settings** (once, by the human): ruleset for `main` with
   mandatory pull requests, required checks `policy`, `selftest`, `check (macOS)`, `mutants`,
   no force push, no bypass list, CODEOWNERS review for harness paths.

## Consequences

- Locally, a violation costs a detour; in CI it cannot be merged.
- **Limit:** if the agent works with the human's GitHub identity, it can also use their
  rights (merge, labels, settings). The wall is only complete once the agent
  has its own identity without admin and merge rights (bot account or GitHub App) and runs in
  an environment without the human's credentials (e.g. a devcontainer). Open point
  in CLAUDE.md.
- Harness changes need an unlocked session and belong in separate pull requests.
- The shell detection in the guard is a heuristic (path + write operation) and can trigger
  falsely. In that case the human runs the command.

## Rejected alternatives

- **`permissions.deny` in `.claude/settings.json`:** deny rules cannot be unlocked per session.
  Then even unlocked harness sessions could not change anything.
- **Sandbox with `denyWrite`:** only affects Bash, changes network and file access for all
  commands. Not for now, possible later as an additional layer.
- **CI with `pull_request_target` and check code from `main`:** protects against a
  manipulated harness in the PR, but opens up cache poisoning and can still be circumvented with
  the same identity. A separate agent identity solves the problem at the root.

## References

- Code: `xtask/src/guard.rs`, `xtask/src/paths.rs`, `.claude/hooks/guard.sh`, `.githooks/`,
  `.github/workflows/ci.yml`
- Overview: `docs/harness.md`, register: `harness/rules.md`
- Related: ADR-0013, ADR-0038, ADR-0039, ADR-0040, ADR-0041, ADR-0042
