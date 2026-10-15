---
id: ADR-0047
title: opencascade-sys as a minimal fork with `links = "occt"`, the kernel takes the OCCT headers from DEP_OCCT_INCLUDE
status: accepted
date: 2026-10-14
crates: [qoob3d-kernel]
tags: [kernel, ffi, dependencies, build]
supersedes: []
superseded_by: null
---

## Context

The kernel compiles its own cxx wrappers against the OCCT headers (ADR-0014). Its build script
found them through `occt_sys::occt_path()` and asserted that `TopoDS_Shape.hxx` exists. OCCT
itself is built by the build script of `opencascade-sys` (feature `builtin`).

Cargo runs the build script of a package after the build scripts of its dependencies only if
those declare `links`; otherwise it waits just for the build dependencies to compile.
`opencascade-sys` 0.3.0 declares no `links`. On a clean checkout (CI) the kernel's build script
therefore ran minutes before OCCT existed and failed with "OCCT headers not found". Warm local
target directories hid the bug.

ADR-0014 kept `opencascade-sys` unchanged from crates.io and took the include path from
`occt-sys` as a build dependency. This ADR refines both points.

## Decision

- `opencascade-sys` 0.3.0 is vendored as a minimal fork in `third_party/opencascade-sys`
  and patched in with `[patch.crates-io]` in the workspace `Cargo.toml`.
- The fork declares `links = "occt"` and its build script prints `cargo:include=<dir>`, the
  include directory it compiled against (builtin or system OCCT).
- The fork's build script passes `-Wno-deprecated-declarations` to its cxx build, as the
  kernel's build script does. Cargo shows build-script warnings of path dependencies on every
  build, and the OCCT headers' `sprintf` deprecation warnings would bury real warnings.
- The kernel's build script reads `DEP_OCCT_INCLUDE`. Because of `links`, Cargo runs it only
  after the build script of `opencascade-sys`, so the headers exist. The kernel no longer has a
  build dependency on `occt-sys`.
- The modifications are listed in `third_party/opencascade-sys/README.md` (LGPL-2.1
  obligation, as for `third_party/opencascade`).
- Proof: a build in a fresh `git worktree` with its own `CARGO_TARGET_DIR` fails before the
  change and succeeds after it.

## Consequences

- Clean builds are ordered correctly; the include path comes from the crate that built OCCT
  instead of a second crate guessing the same directory.
- A second vendored crate to maintain. Updating opencascade-sys means re-applying three small
  changes (manifest, one `println!`, one compiler flag).
- `links = "occt"` reserves the name: no other crate in the graph may declare it.
- Changing the dependency source re-runs the build script of opencascade-sys once; OCCT stays in
  `target/OCCT` and is only re-checked by CMake.

## Rejected alternatives

- **Wait or poll in the kernel's build script** for the headers – a race against an unrelated
  build script, slow and still not guaranteed.
- **Build OCCT in the kernel's build script as well** (`occt_sys::build_occt()`) – two build
  scripts would run CMake on the same directory concurrently.
- **Upstream the `links` key and wait** – CI stays red until a release; can be done in addition,
  the fork is dropped then.
- **Make the kernel's cxx bridge part of the opencascade-sys fork** – mixes our code into the
  vendored crate and blurs the LGPL boundary.

## References

- Code: `third_party/opencascade-sys/`, `crates/qoob3d-kernel/build.rs`, `Cargo.toml`
  (`[patch.crates-io]`)
- Related: ADR-0004, ADR-0014
