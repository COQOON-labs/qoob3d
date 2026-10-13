---
id: ADR-0006
title: Weltkoordinaten sind rechtshändig mit +Z oben, Einheit Millimeter
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-core, qoob3d-kernel, qoob3d-render]
tags: [geometrie, konvention, fdm]
supersedes: []
superseded_by: null
---

## Kontext

CAD-Programme sind uneinheitlich: Fusion nutzt standardmäßig Y oben, ist aber umstellbar.
Slicer (Bambu Studio, PrusaSlicer) und STEP-Dateien arbeiten mit Z oben und Millimetern.

## Entscheidung

- Rechtshändig, **+Z oben**, die XY-Ebene ist das Druckbett.
- Die Einheit ist intern immer **Millimeter**: `f64` im Dokument und Kernel, `f32` erst auf
  der GPU und im `TriMesh`.
- Ansichten: Vorne schaut in +Y-Richtung (Kamera bei −Y), Rechts schaut in −X-Richtung
  (Kamera bei +X). Die Home-Ansicht ist vorne-rechts-oben.

## Konsequenzen

- Export ohne Achsentausch: Ein Teil liegt im Slicer so, wie es konstruiert wurde.
- Anzeige-Einheiten (z. B. Zoll) wären später eine reine UI-Umrechnung.

## Verworfene Alternativen

- **Y oben wie Fusion-Default** – passt nicht zu Slicern und STEP und erzwingt Umrechnungen.

## Verweise

- `crates/qoob3d-render/src/camera.rs`, `crates/qoob3d-core/src/mesh.rs`
- ADR-0002
