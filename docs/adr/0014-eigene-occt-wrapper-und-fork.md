---
id: ADR-0014
title: Eigene exception-sichere OCCT-Wrapper im Kernel-Crate, opencascade als minimaler Fork
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-kernel]
tags: [kernel, ffi, abhängigkeiten, tessellierung]
supersedes: []
superseded_by: null
---

## Kontext

Für M2 brauchen wir eine Tessellierung mit Face-IDs pro Dreieck, korrekt orientierten Normalen
und Feature-Kanten, die exakt auf den Flächen liegen. Der `Mesher` von opencascade-rs liefert
das nicht (falsche Normalen, keine IDs, keine Kanten). Außerdem kapselt die High-Level-Crate
`opencascade` ihre OCCT-Handles (`pub(crate) inner`), sodass eigene cxx-Wrapper sie nicht
erreichen. ADR-0004 sah dafür einen Fork vor.

## Entscheidung

- **Eigene cxx-Bridge im Kernel-Crate:** `crates/qoob3d-kernel/src/occt.rs` (Bridge),
  `cpp/qoob3d_occt.{hxx,cpp}` (C++). `build.rs` kompiliert sie gegen das OCCT, das
  `opencascade-sys` (Feature `builtin`) bereits gebaut hat. Den Include-Pfad liefert `occt-sys`
  als Build-Abhängigkeit.
- Die Bridge verwendet die OCCT-Typen aus `opencascade-sys` wieder
  (`type TopoDS_Shape = opencascade_sys::topo_ds::TopoDS_Shape;`). opencascade-sys bleibt
  **unverändert von crates.io**.
- **Jede C++-Funktion** fängt `Standard_Failure` (leitet nicht von `std::exception` ab!) ab,
  wirft sie als `std::runtime_error` weiter, und cxx macht daraus ein `Err`
  (`KernelError::Occt`). Ergebnisse kommen als flache Puffer (eine FFI-Grenze pro Operation).
- **`opencascade` (High-Level) wird minimal geforkt** nach `third_party/opencascade`
  (`[workspace] exclude`): nur `Shape::as_ffi()`/`from_ffi()` hinzu, KiCad entfernt.
  Die Änderungen stehen in `third_party/opencascade/README.md` (LGPL-Pflicht).
  Das präzisiert ADR-0004: Wir forken nicht das ganze Repo, sondern nur diese eine reine
  Rust-Crate.
- `qoob3d_tessellate` vernetzt mit `BRepMesh_IncrementalMesh`, dreht Wicklung und Normalen
  umgedrehter Faces und überspringt Nähte (`BRep_Tool::IsClosed`) sowie degenerierte Kanten.
  Kanten stammen aus `PolygonOnTriangulation`, die Punkte sind also identisch mit den
  Mesh-Vertices.

## Konsequenzen

- Neue Kernel-Operationen (Skizzen-Faces, Extrusion, Boolesche Operationen, Export) entstehen
  nach demselben Muster und sind von Anfang an exception-sicher.
- Der C++-Code ist Teil unseres Codes. Er läuft durch `cargo build`, wird aber (noch) nicht von
  clippy erfasst, daher gilt: kurz halten, Logik in Rust.
- Auf Linux kann die Link-Reihenfolge statischer Libs relevant werden (noch ungetestet).

## Verworfene Alternativen

- **Ganzes opencascade-rs-Repo forken** – mehr Pflegeaufwand, C++-Bindings müssten wir mitpflegen.
- **High-Level-Crate verwerfen, nur opencascade-sys nutzen** – widerspricht dem gewünschten
  Stack und verschenkt die bequeme API für Skizzen und Boolesche Operationen.
- **Faces einzeln mit dem vorhandenen Mesher vernetzen** – Ränder passen nicht garantiert
  zusammen, das Mesh wäre nicht dicht.

## Verweise

- `crates/qoob3d-kernel/{build.rs,src/occt.rs,cpp/}`, `third_party/opencascade/README.md`
- ADR-0004, ADR-0005
