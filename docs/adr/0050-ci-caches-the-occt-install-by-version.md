---
id: ADR-0050
title: CI caches the OCCT install by OCCT version; the opencascade-sys fork skips the OCCT build when the pinned install exists
status: accepted
date: 2026-10-15
crates: [qoob3d-kernel]
tags: [ci, build, dependencies, kernel]
supersedes: []
superseded_by: null
---

## Context

`check (macOS)` took 33 minutes, about 20 of them for the OCCT C++ build inside the build
script of `opencascade-sys` (`occt_sys::build_occt()`, CMake, Release, static); the Linux job
looked the same. `occt_sys::build_occt()` always runs CMake configure and build. It installs to
`target/OCCT` (`lib/`, `include/`, CMake package files, about 180 MB) next to a 2 GB CMake
`build/` directory.

`Swatinem/rust-cache` keys on `Cargo.lock` and the toolchain, so every dependency update
starts from an empty cache, and its cleanup keeps none of `target/OCCT`. Restoring the CMake
build directory would not help either: on a fresh runner the extracted OCCT sources are newer
than the restored objects, and CMake rebuilds everything.

OCCT itself only changes when the `occt-sys` version changes. crates.io versions are immutable,
so the version identifies the sources and build options completely.

## Decision

- The fork `third_party/opencascade-sys` pins `occt-sys = "=7.8.1"` and its build script
  (`builtin` feature) skips `occt_sys::build_occt()` when `occt_sys::occt_path()` already holds
  a complete install of that version: `include/Standard_Version.hxx` defines
  `OCC_VERSION_COMPLETE "7.8.1"` and every library in `OCCT_LIBS` exists in `lib/` (CMake
  installs the libraries last). Otherwise it builds as before. The pinned version is a constant
  next to `OCCT_VERSION` and must be bumped together with the pin. Marked `// QOOB3D:` and
  listed in `third_party/opencascade-sys/README.md` (ADR-0047).
- The CI jobs `check (macOS)` and `check (Linux, experimental)` restore `target/OCCT` with
  `actions/cache/restore`, keyed by runner OS, architecture, runner image (`ImageOS`), the
  `occt-sys` version from `Cargo.lock` and a hash of the fork's `build.rs`, not by
  `Cargo.lock` as a whole.
- A separate step `cargo build -p opencascade-sys` builds OCCT (or skips it) before
  `cargo xtask check`. When that step succeeded and the cache missed, the job removes
  `target/OCCT/build` and saves `target/OCCT` with `actions/cache/save` under `if: always()`,
  so a run whose check is red still fills the cache.
- The check jobs set `CARGO_PROFILE_DEV_DEBUG=line-tables-only` to cut the build and link time
  of the test binaries. Local builds keep full debug info.

## Consequences

- With a warm cache the check jobs no longer compile OCCT; only the fork's cxx bridge and our
  own code are built. Expected: from 33 to roughly 10–13 minutes on macOS.
- A new `occt-sys` version, a new runner image or a change to the fork's `build.rs` costs one
  full OCCT build per OS and architecture, then the cache is warm again.
- Locally, deleting only the Cargo build directories (`target/debug`) no longer rebuilds or
  re-checks OCCT; deleting `target/OCCT` or `cargo clean` still does.
- Bumping `occt-sys` means bumping the pin and `BUILTIN_OCCT_VERSION` together; a mismatch
  makes every build script run rebuild OCCT (slow, not wrong).
- A corrupted install that still passes the check (truncated library) is not detected; delete
  `target/OCCT` or change the cache key.
- CI backtraces keep file and line numbers, but debuggers in CI see no variables.

## Rejected alternatives

- **Homebrew (or apt) OCCT** – a different OCCT version than the pinned 7.8.1 and dynamic
  libraries; ADR-0047 builds against the `occt-sys` sources on purpose.
- **sccache with the GitHub Actions cache backend** – OCCT consists of thousands of object
  files; the per-object cache requests run into the GitHub cache rate limits and still need the
  CMake configure and link.
- **rust-cache only** – its key churns with every `Cargo.lock` change and its cleanup removes
  `target/OCCT/lib` and `include/`.
- **Cache the CMake build directory as well** – 2 GB, and CMake rebuilds anyway because the
  sources are newer than the objects on a fresh runner.

## References

- Code: `third_party/opencascade-sys/build.rs`, `third_party/opencascade-sys/Cargo.toml`,
  `.github/workflows/ci.yml`
- Related: ADR-0014, ADR-0037, ADR-0047
