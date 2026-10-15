# opencascade-sys (QOOB3D fork)

Vendored copy of the `opencascade-sys` crate 0.3.0 from
[opencascade-rs](https://github.com/bschwind/opencascade-rs) (commit `07e8feb`,
`crates/opencascade-sys`), licensed under LGPL-2.1 (see `LICENSE`). Patched in
via `[patch.crates-io]` in the workspace `Cargo.toml`. Only `build.rs`, `src/`,
`include/` and `OCCT/CMakeLists.txt` are taken over; examples and tests are not.

Why a fork: upstream declares no `links` key, so Cargo does not order the build
script of `qoob3d-kernel` after the one of this crate, and on a clean checkout
the kernel looked for the OCCT headers before OCCT was built. With
`links = "occt"` Cargo runs this build script first and hands its
`cargo:include=` to direct dependents as `DEP_OCCT_INCLUDE`. See
`docs/adr/0047-opencascade-sys-fork-with-links-key.md`.

## Modifications

1. `Cargo.toml` rewritten (crates.io-normalised manifest replaced,
   `publish = false`) and `links = "occt"` added. Feature `builtin` and the
   dependencies are unchanged, except the exact `occt-sys` pin of item 4.
2. `build.rs` prints `cargo:include=<OCCT include dir>` (the `include_dir` from
   `OcctConfig::detect()`, works for builtin and system OCCT). Marked with
   `// QOOB3D:`.
3. `build.rs` passes `-Wno-deprecated-declarations` to the cxx build. The OCCT
   headers trigger `sprintf` deprecation warnings, and Cargo shows build-script
   warnings of path dependencies on every build, which buries real warnings.
   Marked with `// QOOB3D:`.
4. With `builtin`, `build.rs` skips `occt_sys::build_occt()` when
   `occt_sys::occt_path()` already holds a complete install of the pinned OCCT
   (`BUILTIN_OCCT_VERSION` in `include/Standard_Version.hxx`, every library of
   `OCCT_LIBS` in `lib/`). For this, `Cargo.toml` pins `occt-sys = "=7.8.1"`;
   bump both together. CI restores that install from its cache, so OCCT is not
   rebuilt (`docs/adr/0050-ci-caches-the-occt-install-by-version.md`). Marked
   with `// QOOB3D:`.
