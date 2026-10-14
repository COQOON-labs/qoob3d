---
id: ADR-0039
title: Tests and production code are written in separate agent contexts and commits
status: accepted
date: 2026-10-14
crates: [xtask, qoob3d-core, qoob3d-sketch, qoob3d-kernel, qoob3d-render, qoob3d-app]
tags: [tests, ai, quality]
supersedes: []
superseded_by: null
---

## Context

If the same agent writes code and tests, the tests usually test the implementation instead of
the requirement: misunderstandings end up in both and confirm each other. Tests
should be an independent oracle. In Rust, unit tests idiomatically live in the same file
(`mod tests { … }`), so test and production code cannot be separated by file.

## Decision

1. **Tests in separate files:** unit tests live in `<module>/tests.rs`, included with
   `#[cfg(test)] mod tests;`. `cargo xtask hygiene` rejects inline test modules. Test files
   are `tests/**`, `**/tests.rs`, `proptest-regressions/` and `models/*.expect.json`
   (`paths::kind`).
2. **Roles:** the subagent `test-author` may only change tests, the subagent `implementer`
   only production code. Every other agent context (main session, other subagents) is
   pinned to one of the two kinds on its first write access (ledger in
   `.git/qoob3d/roles/`). The guard blocks switching.
3. **Commits:** a commit changes either tests or production code. Exceptions (e.g.
   renaming an API together with all callers) need the trailer
   `Mixed-Change: <reason>`. Checked in `pre-push` and CI.
4. **Order:** first tests from the specification, then the implementation (skill
   `deliver`). This fits "bugfix = a failing test first".
5. **Cross-check:** mutation tests (`cargo-mutants`) on the changed code of the pure crates
   in CI show whether the tests really catch errors.

## Consequences

- The implementer cannot "repair" a wrong test but has to report it.
- Separation by context is a brake locally (the ledger lives in the repo directory).
  Separation by commits is deterministic and checkable in CI.
- Tools that check production code via text search (`xtask i18n`) ignore `tests.rs`.
- The commit that moved the tests carries `Mixed-Change`.

## Rejected alternatives

- **Separation only by instruction in CLAUDE.md:** not checkable.
- **Session IDs as commit trailers:** forgeable, more effort, no gain over
  commit separation.
- **`#[path = "x_tests.rs"]` next to the source file:** unusual; `x/tests.rs` is
  standard module resolution.

## References

- Code: `xtask/src/paths.rs`, `xtask/src/guard.rs`, `xtask/src/commits.rs`,
  `xtask/src/hygiene.rs`, `.claude/agents/test-author.md`, `.claude/agents/implementer.md`
- Related: ADR-0037, ADR-0041
