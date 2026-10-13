# opencascade (QOOB3D fork)

Vendored copy of the `opencascade` crate 0.3.0 from
[opencascade-rs](https://github.com/bschwind/opencascade-rs) (commit `32758df`),
licensed under LGPL-2.1 (see `LICENSE`). `opencascade-sys` is used unmodified
from crates.io.

Why a fork: QOOB3D needs the raw OCCT handles to run its own exception-safe
cxx wrappers (tessellation with face ids and edges, later sketch/extrude/boolean
operations). See `docs/adr/0014-eigene-occt-wrapper-und-fork.md`.

## Modifications

1. `Shape::as_ffi()` and `Shape::from_ffi()` added (`src/primitives/shape.rs`).
2. KiCad support removed (`src/kicad.rs`, `Error::KicadReadFailed`, dependency
   `opencascade-kicad-parser`) – not needed and avoids an extra dependency.
3. `Cargo.toml` rewritten (crates.io-normalised manifest replaced, `publish = false`).
