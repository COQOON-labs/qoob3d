---
id: ADR-0048
title: The pre-push hook checks the tip of every pushed ref in a temporary worktree
status: accepted
date: 2026-10-14
crates: []
tags: [harness, git-hooks, ci]
supersedes: []
superseded_by: null
---

## Context

The pre-push hook (ADR-0037) ended with `cargo xtask check` in the working tree. The working tree
is not what gets pushed: uncommitted or untracked changes can make a broken commit green (a fix
not yet committed) or a good commit red, and pushing a branch other than the checked-out one
checked the wrong state entirely. The commit policy and the critic record already refer to the
pushed commits; only the definition of done did not.

## Decision

- `cargo xtask check-rev <rev>` creates a detached `git worktree` of `<rev>` in the system temp
  directory, runs `cargo xtask check` there and removes the worktree afterwards, also on failure.
- All worktree checks share one persistent target directory, `check-rev/` inside the cargo
  target directory of the checkout (`CARGO_TARGET_DIR`, default `target/`), so OCCT is built
  there once and then reused (ADR-0047). It is never the checkout's own target directory: cargo
  hashes path packages relative to the workspace root, so a worktree build in the checkout's
  target directory replaces the checkout's artifacts (the `xtask` binary, `qoob3d`) with the
  worktree's, and cargo then considers them fresh. xtask resolves built binaries (`qoob3d` for
  models, smoke test, screenshots) through `CARGO_TARGET_DIR`.
- `.githooks/pre-push` runs `cargo xtask check-rev "$local_sha"` for every pushed ref instead of
  one `cargo xtask check` in the working tree.

## Consequences

- What is checked is exactly what is pushed, independent of the working tree. Intermediate
  commits are not checked (a tests-first commit is red by design, ADR-0039); CI checks the head
  of a pull request the same way.
- The check of the pushed commit runs with that commit's harness, as in CI.
- The first `check-rev` builds OCCT once in `target/check-rev/` (about 10 minutes, a few GB);
  later ones reuse it. Workspace crates are compiled again for every worktree, because its files
  are newer. Pushing several refs checks each of them.
- Commits from before this ADR cannot be checked with `check-rev` (their xtask looks for
  binaries in the worktree's own `target/`); the hook checks only the tip of each pushed ref.
- The hook itself is shell; a test pins that it calls `check-rev` and no bare `check`.

## Rejected alternatives

- **Refuse pushes from a dirty working tree** – blocks a common workflow and still checks the
  wrong commit when another branch is pushed.
- **`git stash` around the check** – changes the user's working tree in a hook; untracked and
  ignored files still leak in.
- **Own target directory per worktree** – a full OCCT build on every push.
- **The checkout's own target directory** – corrupts the checkout's artifacts (see Decision).
- **`cargo clean -p <workspace crates>` after each check** – repairs the checkout but rebuilds
  all workspace crates there after every push, and races with a concurrent build.

## References

- Code: `xtask/src/worktree.rs`, `.githooks/pre-push`
- Related: ADR-0037, ADR-0041, ADR-0047
