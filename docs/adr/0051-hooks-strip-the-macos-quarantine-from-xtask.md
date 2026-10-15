---
id: ADR-0051
title: The git hooks and check-rev build xtask first and remove the macOS quarantine attribute from it
status: accepted
date: 2026-10-15
crates: []
tags: [harness, git-hooks, macos]
supersedes: []
superseded_by: null
---

## Context

The git hooks (ADR-0037) run `cargo xtask …`; `check-rev` (ADR-0048) runs `cargo xtask check`
in a temporary worktree with its own target directory. Both rebuild the `xtask` binary when its
sources changed.

GUI git clients run the hooks as their child processes. Sourcetree is registered with
`LSFileQuarantineEnabled`, so macOS sets `com.apple.quarantine` (`0081;…;Sourcetree;`) on every
file those processes create, including a freshly linked `target/debug/xtask`. Gatekeeper then
refuses to start the binary ("xtask Not Opened … Apple could not verify"), the hook fails and
git aborts; on 2026-10-15 a merge was left half-done. From a terminal the same hooks work,
because the terminal does not quarantine.

## Decision

- `.githooks/lib.sh` holds the shared setup of the hooks (PATH, repository root) and builds
  xtask with `cargo build --quiet --package xtask` before a hook runs it. On macOS it removes
  `com.apple.quarantine` from `${CARGO_TARGET_DIR:-target}/debug/xtask` and ignores a missing
  attribute. `cargo xtask` then finds the binary fresh and starts it without relinking.
  `commit-msg` and `pre-push` source `lib.sh`.
- `check-rev` builds the worktree's xtask into the shared `check-rev` target directory
  (`worktree::build_xtask`) before it spawns `cargo xtask check`, and on macOS removes the
  attribute from that binary with `xattr -d`, ignoring a missing attribute.

## Consequences

- Commits, merges and pushes from Sourcetree (and other quarantining GUI clients) run the
  hooks like a terminal does.
- Every hook run starts with a `cargo build` of xtask; with an unchanged xtask that is a no-op
  of well under a second.
- Only the xtask binary is cleared. If a GUI client also makes Gatekeeper block other binaries
  that `cargo xtask check` builds and starts (test binaries, `qoob3d` for the smoke test),
  `check-rev` from such a client still fails; the terminal remains the fallback.
- The hooks remove a security attribute from a binary we build ourselves from the checked-out
  sources; they never touch downloaded files.

## Rejected alternatives

- **Tell users to commit and push from the terminal** – the failure appears in the middle of a
  merge or push and leaves git in a half-finished state; a rule nobody sees in the GUI does not
  prevent that.
- **Disable `LSFileQuarantineEnabled` for Sourcetree** – the flag sits in the signed app
  bundle; changing it breaks the signature and is undone by every update.
- **Sign the xtask binary** – needs an Apple developer identity on every machine and in every
  build, for an internal tool.

## References

- Code: `.githooks/lib.sh`, `.githooks/commit-msg`, `.githooks/pre-push`,
  `xtask/src/worktree.rs`, `xtask/src/main.rs` (`check_rev`)
- Related: ADR-0013, ADR-0037, ADR-0048
