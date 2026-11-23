---
id: ADR-0037
title: A lean AI harness – local brakes, the wall in CI and GitHub, a commit policy, write protection for harness files and voluntary review tools
status: accepted
date: 2026-11-23
crates: [xtask]
tags: [quality, ai, security, ci, git, tests, style]
supersedes: []
superseded_by: null
---

## Context

AI agents write most of the code. Locally an agent has the human's rights: it can skip hooks,
change checks and loosen thresholds, usually not on purpose but to get green. AI tools add
`Co-Authored-By: Claude` trailers by default, and agents commit with whatever git identity the
machine provides; the maintainer has several. Rules only in CLAUDE.md turn into wishes, taste
decided anew by every agent leads to rework, and a check that never fires looks like a broken
one. At the same time the harness must not block the work it protects: "sufficiently safe,
not over-regulated", a check exists because something actually went wrong or because it is
cheap.

This topic absorbs the former decisions 0038–0044, 0048, 0049 and 0051.

## Decision

### Layers: brake locally, block in CI

| Layer | Where | Kind |
| --- | --- | --- |
| Write protection (`PreToolUse`, edit tools only) | `.claude/hooks/guard.sh` | brake |
| Stop hook | `.claude/hooks/check-on-stop.sh` → `cargo xtask check --fast` (ADR-0013) | brake |
| `commit-msg`, `pre-push` | `.githooks/`, activated by `cargo xtask setup` | brake |
| CI | `.github/workflows/ci.yml`: `policy`, `check (macOS)` | **wall** |
| GitHub | ruleset for `main`, required checks, CODEOWNERS review | **wall** |

Checks are deterministic where possible. Local layers only save a round trip; CI runs
everything again. `cargo xtask check` is red locally while the git hooks are not active.
`docs/harness.md` maps every rule to its checks.

### Commit policy: project identity, no AI attribution

`cargo xtask commits` (`xtask/src/commits.rs`) checks every non-merge commit:

- **Identity:** author and committer must equal the single identity in `harness/identity.txt`
  (`Name <mail>`). The `commit-msg` hook checks the commit being created
  (`git var GIT_AUTHOR_IDENT`/`GIT_COMMITTER_IDENT`), `pre-push` and CI `policy` every pushed
  commit. An identity that names an AI tool is rejected as well.
- **No AI attribution:** a message is rejected if it contains 🤖, or a `…-by:` trailer or a
  "Generated with/by" note that names an AI tool (`AI_MARKERS`). Naming a tool elsewhere in the
  text ("Claude Code hook") is fine. Tests and production code may share a commit.

### Git hooks

`commit-msg` runs the commit policy on the new commit; `pre-push` runs it on every pushed
commit and scans them with gitleaks (a warning if gitleaks is missing; CI scans anyway).
Neither builds or tests the product. Their shared setup `.githooks/lib.sh` builds xtask, on
macOS removes `com.apple.quarantine` from the binary and runs it directly: GUI clients such as
Sourcetree quarantine what their child processes create, Gatekeeper would refuse it, and
`cargo xtask` would re-link and re-quarantine it. So commits and pushes work from Sourcetree.

### CI and GitHub: the wall

- `policy` (Linux): commit policy for the range, gitleaks, `xtask hygiene`, `arch`, `docs`,
  `i18n`, cargo-machete, cargo-deny; on pull requests it lists changed harness files in the
  run summary (`cargo xtask harness-changes`).
- `check (macOS)`: `cargo xtask check`, the definition of done.
- `selftest`: weekly, on demand and on pull requests that change the harness.
- `mutants`: weekly report on `qoob3d-core`, never a gate.
- `check (Linux, experimental)`: not required until it has been green once. Actions are
  pinned by commit SHA, the token is read-only, `merge_group` is a trigger.
- GitHub (once, by the human): ruleset for `main` with pull requests, required checks
  `policy` and `check (macOS)`, merge queue, no force push or deletion, "Require review from
  Code Owners". Only merge commits: squash and rebase merges get the committer `GitHub` and
  fail the identity rule. Recommended: a separate agent identity without merge or admin rights.

### Harness files: write protection, unlock and code owners

The harness is `paths::HARNESS`: `.claude/`, `.githooks/`, `.github/`, `.cargo/`, `xtask/`,
`harness/`, `Cargo.toml`, `clippy.toml`, `deny.toml`, `rustfmt.toml`, `rust-toolchain.toml`
(compared ignoring case). `guard.sh` carries the same list and denies Edit, Write, MultiEdit
and NotebookEdit on these paths after collapsing `.` and `..`; it fails closed when it cannot
read the hook input. Shell commands are not inspected. Only the human unlocks, by starting
Claude Code with `QOOB3D_HARNESS_UNLOCK=1`; the unlock lifts exactly this protection.
`.github/CODEOWNERS` assigns `* @Ben-Gee` and lists every harness path explicitly;
`xtask hygiene` fails if one has no owner line. Harness changes go in their own pull requests.

### Lint policy and ratchet

- Workspace lints in `Cargo.toml`, inherited by every crate (`xtask hygiene`): `unsafe_code`
  denied, `unreachable_pub` (so `dead_code` finds unused parts), clippy `all`, `unwrap_used`
  outside tests, `dbg_macro`, `todo`, `unimplemented`, `ignore_without_reason`, a few style
  lints, `too_many_lines` (100) and `cognitive_complexity` (25, thresholds in `clippy.toml`).
- Suppressions only as `#[expect(lint, reason = "…")]` (`allow_attributes`,
  `allow_attributes_without_reason`); `expect` speaks up once the exception is unnecessary.
- **Ratchet:** `xtask hygiene` counts every lint in an `#[expect(…)]` (also in `cfg_attr`);
  the total may not exceed `harness/ratchet.txt`, whose bound only goes down.
- **Style register** `docs/design/code-style.md` (ST-NN): only what is written there applies;
  agents propose entries, the human decides, conversions go in separate commits.

### Tests in their own files

Unit tests live in `<module>/tests.rs` (`#[cfg(test)] mod tests;`); `xtask hygiene` rejects
inline test modules and test attributes elsewhere (`paths::is_test`). Separate files keep tests
apart for review, for the voluntary roles and for tools that scan production code (`xtask i18n`).

### Self-test

`harness/sabotage/*.case` holds deliberate violations: a patch, commit message, stdin or
environment, the command that must reject it and the expected output text, so a failure for
the wrong reason does not count. `cargo xtask selftest` plays every case in a copy of the
working tree as its own git repository under `target/selftest/`, committed with the project
identity (a case may set another with `identity:`). A new check comes with a sabotage case.
The skill `harness-audit` (subagent `harness-auditor`) reviews coverage, false alarms, run
times and real escapes and only proposes changes.

### Voluntary roles and critic

`/deliver` (subagents `test-author` for tests from the specification, `implementer` for the
code) gives an independent oracle where it pays off: geometry, file formats, bugfixes.
`/critic-review` runs the subagent `critic` with a fresh context against
`harness/critic-checklist.md` (K-NN, severities, JSON findings; taste without a style register
entry is no finding) and one `verifier` per `blocker`/`major` that tries to refute it.
Nothing enforces these tools; the result of a critic review belongs in the pull request
description.

## Consequences

- Locally a violation costs a detour; in CI it cannot be merged. Judgment (design,
  duplication, usability) is a matter for the pull request review.
- **Limits:** the write protection is not a sandbox; a shell command can still change a harness
  file. Such a change shows up in the diff, `policy` flags it and the code owner must approve
  it. Local hooks can be skipped; CI runs the same checks. The wall is only complete once the
  agent pushes with its own identity without merge and admin rights; until then CI is as strong
  as the rights the agent has. An OS sandbox for harness paths is an open item in CLAUDE.md.
- A human whose name contains an AI marker cannot be a co-author; accepted. Another person or
  a bot account needs `harness/identity.txt` to become a list; not needed yet.
- After a toolchain or dependency change, build xtask once from a terminal (its dependencies'
  build scripts run before the hooks can strip the quarantine). Sabotage patches need
  maintenance when their targets change ("patch no longer applies").

## Rejected alternatives

- **A heavier harness.** Until 2026-11-23 the harness also had a guard that parsed shell
  commands (heredocs, sed scripts, pure-read allowlists), a role ledger binding each agent
  context to tests or code, separate test and code commits with a `Mixed-Change` trailer, a
  critic record as a push condition, a pre-push `check-rev` of every pushed tip in a temporary
  worktree, a pre-push stop under quarantining clients, a rule register with its own check,
  and mutation tests as a required check. It blocked more work than it caught: xtask had about
  14.6k lines against about 21k lines of product code, the guard alone about 8.1k; the mutation
  job took 21–31 minutes and was red four times in a row on PR #6. It was cut to what has
  actually escaped or is cheap; the history is in Git.
- **`permissions.deny` in `.claude/settings.json`:** cannot be lifted per session, so not even
  an unlocked session could change the harness.
- **CI with `pull_request_target` and checks from `main`:** opens cache poisoning and is still
  circumvented with the same identity; a separate agent identity solves it at the root.
- **All of `clippy::pedantic`, or thresholds raised to today's maximum:** rework without
  benefit, or every function allowed the worst case; marked, capped exceptions are more honest.
- **Signed commits:** need key management on every machine and in CI; can come later.

## References

- Code: `.claude/settings.json`, `.claude/hooks/`, `.githooks/`, `.github/workflows/ci.yml`,
  `.github/CODEOWNERS`, `xtask/src/{main,commits,hygiene,selftest,paths}.rs`
- Harness data: `harness/identity.txt`, `harness/ratchet.txt`, `harness/sabotage/`,
  `harness/critic-checklist.md`, `harness/language-allowlist.txt`, `Cargo.toml`, `clippy.toml`
- Tools: `.claude/agents/`, `.claude/skills/{deliver,critic-review,harness-audit}/SKILL.md`
- Overview: `docs/harness.md`, style register: `docs/design/code-style.md`
- Related: ADR-0013, ADR-0036
