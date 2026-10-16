---
id: ADR-0051
title: The git hooks run an un-quarantined xtask directly, pre-push stops under a quarantining client
status: accepted
date: 2026-10-15
crates: []
tags: [harness, git-hooks, macos]
supersedes: []
superseded_by: null
---

## Context

The git hooks (ADR-0037) ran `cargo xtask …`; `check-rev` (ADR-0048) runs `cargo xtask check`
in a temporary worktree with its own target directory.

GUI git clients run the hooks as their child processes. Sourcetree is registered with
`LSFileQuarantineEnabled`, so macOS sets `com.apple.quarantine` (`0081;…;Sourcetree;`) on every
file those processes create, including a freshly linked `target/debug/xtask`. Gatekeeper then
refuses to start the binary ("xtask Not Opened … Apple could not verify"), the hook fails and
git aborts; on 2026-10-15 a merge was left half-done. From a terminal the same hooks work,
because the terminal does not quarantine.

A first fix built xtask, stripped the attribute and then called `cargo xtask`. A real push
from Sourcetree on 2026-10-15 showed that this cannot work: cargo re-creates the hardlink
`target/debug/xtask` on every `cargo build` or `cargo run`, even without recompiling, and the
client quarantines the new link again. The full check of `check-rev` also builds and at once
runs test binaries and build scripts, which nothing can un-quarantine in time.

## Decision

- `.githooks/lib.sh` holds the shared setup of the hooks (PATH, repository root, the path
  `xtask_binary="${CARGO_TARGET_DIR:-target}/debug/xtask"`); sourcing it builds nothing.
- Its function `build_xtask` runs `cargo build --quiet --package xtask` and on macOS removes
  `com.apple.quarantine` from the binary, ignoring a missing attribute; if the binary itself is
  missing, it says so on stderr instead of calling `xattr`. The hooks then run
  `"$xtask_binary"` directly, never `cargo xtask`, which would re-link and re-quarantine it.
- Its function `quarantining_client` creates a probe file with `mktemp`, tests it for
  `com.apple.quarantine` (macOS only) and removes it again. `pre-push` calls it before it
  builds anything; under a quarantining client it writes on stderr that the client quarantines
  newly built programs, that Gatekeeper would block the full check, and that the push must go
  from a terminal, and exits 1.
- `check-rev` does nothing about the quarantine.

## Consequences

- Commits and merges work from Sourcetree, as long as the dependencies of xtask (serde,
  serde_json, proc-macro2, thiserror) are built: their build scripts are compiled and run
  during `cargo build --package xtask`, before anything can be stripped. After a toolchain or
  dependency change, build xtask once from a terminal.
- Pushes need a terminal (or another client that does not quarantine). The stop comes before
  any build, so git is not left half-done and the message names the way out.
- Every hook run starts with a `cargo build` of xtask; with an unchanged xtask that is a no-op
  of well under a second.
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
- **Strip the quarantine in `check-rev`** – the check re-links xtask and builds and immediately
  runs test binaries and build scripts; none of them can be cleared before they start.
- **Skip `check-rev` under a quarantining client** – it would weaken the local brake before a
  push; the user chose to stop the push instead.

## References

- Code: `.githooks/lib.sh`, `.githooks/commit-msg`, `.githooks/pre-push`
- Related: ADR-0013, ADR-0037, ADR-0048
