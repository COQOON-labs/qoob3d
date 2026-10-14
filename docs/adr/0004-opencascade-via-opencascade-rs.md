---
id: ADR-0004
title: OpenCascade via opencascade-rs from crates.io with statically built OCCT 7.8
status: accepted
date: 2026-10-13
crates: [qoob3d-kernel]
tags: [kernel, dependencies, ffi]
supersedes: []
superseded_by: null
---

## Context

We need a B-rep kernel with Boolean operations, chamfers and fillets as well as
STEP and STL export. OpenCascade (OCCT) is the only mature open-source candidate.
`opencascade-rs` (bschwind) offers cxx bindings (`opencascade-sys`) and a high-level API
(`opencascade`). Version 0.3.0 has been on crates.io since August 2026 and bundles OCCT 7.8.1
via `occt-sys`.

## Decision

- We use `opencascade = "0.3.0"` from crates.io with the default feature `builtin`:
  OCCT is built from source and linked **statically**. This is reproducible on macOS and
  Linux, and there is no dependency on a system OCCT.
- We add missing functions as our own cxx wrappers following the same pattern. As soon as
  the first one is needed, we take over the repo as a fork in `third_party/opencascade-rs`
  (path dependency) – a separate ADR follows then.
- **Every own wrapper catches C++ exceptions** (`try/catch` around `Standard_Failure`) and
  returns a `Result` via cxx. Otherwise an exception across the FFI boundary aborts the process.
- OCCT types and the `glam 0.24` used by opencascade (for us `occt-glam`) never leave
  the kernel crate.

## Consequences

- The first build takes about 4 minutes. Changing `[profile.dev.package."*"]` triggers a
  complete OCCT rebuild.
- CMake ≥ 4 needs `CMAKE_POLICY_VERSION_MINIMUM=3.5` (`.cargo/config.toml`).
- OCCT is licensed under LGPL-2.1. Because of the static linking, the license must be
  clarified before distributing binaries (open point).
- Known defect: the `Mesher` of opencascade-rs returns inward-pointing normals for
  reversed faces. We correct this in `tessellate.rs`; an own mesher follows.

## Rejected alternatives

- **Homebrew or system OCCT** – version drift between machines, distribution-dependent on Linux.
- **Git dependency on opencascade-rs** – unnecessary as long as crates.io is current.
- **Truck / Fornjot (pure Rust)** – still too immature for Boolean operations, chamfers and STEP.

## References

- `crates/qoob3d-kernel/Cargo.toml`, `.cargo/config.toml`, `crates/qoob3d-kernel/src/tessellate.rs`
- ADR-0005
