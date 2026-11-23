---
id: ADR-0004
title: OCCT 7.8.1 is built from source and linked statically through a minimal opencascade-sys fork; all OCCT calls go through our own exception-safe cxx wrappers
status: accepted
date: 2026-11-23
crates: [qoob3d-kernel]
tags: [kernel, ffi, dependencies, build, ci]
supersedes: []
superseded_by: null
---

## Context

We need a B-rep kernel with Boolean operations, fillets and chamfers, offsets, sweeps, STEP
and tessellation. OpenCascade (OCCT) is the only mature open-source candidate; pure Rust
kernels (Truck, Fornjot) are too immature for Booleans, fillets and STEP. `opencascade-rs`
(bschwind) provides `occt-sys` (the OCCT 7.8.1 sources and a CMake build) and
`opencascade-sys` (cxx bindings).

Facts that shape the integration:

- An OCCT exception (`Standard_Failure`, which does **not** derive from `std::exception`)
  that crosses the FFI boundary aborts the process. Some OCCT operations do not throw but
  segfault (`BRepOffsetAPI_MakeOffset` on concentric circles); only process isolation
  (ADR-0053) protects against those.
- OCCT takes about 4 minutes to build locally and about 20 minutes on a CI runner.
- Cargo runs a package's build script after those of its dependencies only if they declare
  `links`. `opencascade-sys` 0.3.0 does not, so on a clean checkout our build script looked
  for the OCCT headers before OCCT existed. Warm local target directories hid this.
- OCCT and opencascade-rs are LGPL-2.1; we link statically.

This topic absorbs the former decisions 0014, 0047, 0050 and 0028.

## Decision

### Static OCCT from source

The kernel depends on `opencascade-sys` 0.3 with the feature `builtin`: OCCT is built from
the `occt-sys` sources with CMake (Release, static) into `target/OCCT` and linked
statically. This is reproducible on macOS and Linux and independent of any system OCCT.
CMake 4 needs `CMAKE_POLICY_VERSION_MINIMUM=3.5` (`.cargo/config.toml`). The kernel uses
`opencascade-sys` only for the OCCT build and the `TopoDS_Shape` type; the high-level
`opencascade` crate is not used.

### Own exception-safe wrappers

- Every OCCT operation is our own C++ function in `crates/qoob3d-kernel/cpp/qoob3d_occt.cpp`,
  bridged by cxx in `src/occt.rs`; the bridge reuses `opencascade_sys::topo_ds::TopoDS_Shape`.
- Every function body runs inside `guarded(...)`, which catches `Standard_Failure` and
  rethrows it as `std::runtime_error`; cxx turns that into an `Err`
  (`KernelError::Occt`). Null results become errors as well.
- Results cross the boundary as flat buffers (`rust::Vec`), one FFI crossing per operation.
  Logic stays in Rust as far as possible; the C++ is kept short because clippy does not see
  it.
- `qoob3d_tessellate` meshes with `BRepMesh_IncrementalMesh`, flips winding and normals of
  reversed faces, skips seams and degenerate edges, and takes the edges from
  `PolygonOnTriangulation`, so edge points are identical to mesh vertices and every triangle
  carries its face id.
- OCCT types never leave the kernel crate (ADR-0036).

### opencascade-sys as a minimal fork

`opencascade-sys` 0.3.0 is vendored in `third_party/opencascade-sys` and patched in with
`[patch.crates-io]`. The fork declares `links = "occt"` and prints `cargo:include=<dir>`, so
Cargo runs it before the kernel's build script and hands the include directory over as
`DEP_OCCT_INCLUDE`. It passes `-Wno-deprecated-declarations` because Cargo replays the
build-script warnings of path dependencies on every build. Every change is marked
`// QOOB3D:` and listed in the fork's `README.md` (LGPL obligation).

### Skip the OCCT build when the pinned install exists; CI caches it

- The fork pins `occt-sys = "=7.8.1"` and skips `occt_sys::build_occt()` when
  `target/OCCT` already holds a complete install of that version
  (`OCC_VERSION_COMPLETE "7.8.1"` in `Standard_Version.hxx`, every library of `OCCT_LIBS` in
  `lib/`). `BUILTIN_OCCT_VERSION` and the pin are bumped together.
- CI restores `target/OCCT` with `actions/cache`, keyed by runner OS, architecture, runner
  image, the `occt-sys` version from `Cargo.lock` and a hash of the fork's `build.rs`. A
  separate step `cargo build -p opencascade-sys` builds or skips OCCT; on a cache miss the
  job drops `target/OCCT/build` (2 GB) and saves the install (about 180 MB) even when the
  check is red. The check jobs set `CARGO_PROFILE_DEV_DEBUG=line-tables-only`.

### Planar regions via General Fuse

Profiles that may intersect themselves (`polygon`) get their face from a region search,
`qoob3d_profile_faces`: a helper face larger than all curves is split by all curve edges with
`BOPAlgo_Builder` (General Fuse, fuzzy value 1e-7); the history maps each split edge to its
curve id; split faces that touch the helper boundary are the outside; every other face is a
region keyed by the sorted ids of its curves. Simple outlines (`rect`, `circle`, `slot`) skip
the search (`qoob3d_outline_face`).

## Consequences

- New OCCT capabilities cost a small C++ wrapper each, exception-safe from the start, plus an
  analytic volume test.
- Clean builds are ordered correctly; with a warm cache CI does not compile OCCT.
- A second-hand crate to maintain: updating opencascade-sys means re-applying four small
  changes. `links = "occt"` reserves that name in the dependency graph.
- Deleting `target/OCCT` (or `cargo clean`) rebuilds OCCT; deleting `target/debug` does not.
  A truncated library that still passes the install check is not detected.
- Islands and self-intersections come out of the region search without our own 2D
  arrangement code; a self-intersecting polygon yields several regions and is refused.
- Static LGPL linking must be clarified before binaries are distributed (open item).
- The Linux link order of the static libraries is not tested yet.

## Rejected alternatives

- **Homebrew or system OCCT** – version drift between machines, dynamic libraries, a
  different version than the pinned 7.8.1.
- **The high-level `opencascade` crate** – encapsulates its handles so our wrappers cannot
  reach them, and its mesher returns wrong normals and no face ids; it was used at first,
  then forked, then dropped.
- **Waiting or polling for the headers in our build script** – a race against an unrelated
  build script.
- **Building OCCT in our build script too** – two build scripts would run CMake on the same
  directory concurrently.
- **Our cxx bridge inside the opencascade-sys fork** – mixes our code into the vendored crate
  and blurs the LGPL boundary.
- **sccache or rust-cache for OCCT** – thousands of objects hit the cache rate limits, and the
  rust-cache key churns with every `Cargo.lock` change and drops `target/OCCT`.
- **Our own 2D arrangement in Rust for regions** – circle intersections and tolerances are
  error-prone; OCCT solves this robustly.

## References

- Code: `crates/qoob3d-kernel/{Cargo.toml,build.rs,src/occt.rs,src/tessellate.rs}`,
  `crates/qoob3d-kernel/cpp/qoob3d_occt.{hxx,cpp}`, `crates/qoob3d-kernel/src/script/profile.rs`
- Fork: `third_party/opencascade-sys/` (`README.md`, `build.rs`, `Cargo.toml`), `Cargo.toml`
  (`[patch.crates-io]`), `.cargo/config.toml`, `.github/workflows/ci.yml`, `deny.toml`
- Related: ADR-0036, ADR-0053
