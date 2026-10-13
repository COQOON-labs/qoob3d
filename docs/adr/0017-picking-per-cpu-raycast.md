---
id: ADR-0017
title: Picking vorerst per CPU-Raycast gegen das Dreiecksnetz
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-core, qoob3d-render, qoob3d-app]
tags: [picking, navigation]
supersedes: []
superseded_by: null
---

## Kontext

Fusion orbitet um den Modellpunkt unter dem Cursor. Ab M3/M4 kommen Auswahl von Flächen,
Profilen und Kanten dazu. Möglich sind ein CPU-Raycast oder ein GPU-ID-Puffer.

## Entscheidung

- `Camera::ray(ndc, aspect)` (Perspektive und Ortho) plus `TriMesh::raycast` (Möller–Trumbore,
  beide Seiten, nächster Treffer) in `qoob3d-core`.
- Der Treffer liefert das Dreieck. Über `TriMesh::face_ids` (ADR-0014) ist die B-Rep-Face bekannt.
- Orbit-Pivot = Trefferpunkt beim Start des Orbits, sonst das Kameraziel. Der Pivot wird als
  Mint-Punkt angezeigt.
- Wechsel auf einen GPU-ID-Puffer oder eine BVH erst, wenn Messungen es verlangen
  (lineare Suche reicht für MVP-Modelle).

## Konsequenzen

- Picking ist ohne GPU testbar (Unit-Tests in core, Smoke-Test prüft den Pivot).
- Für Kanten-Picking braucht es später eine Toleranz in Pixeln. Die kommt mit der Auswahl.

## Verworfene Alternativen

- **GPU-ID-Puffer sofort** – zusätzlicher Pass und Readback-Latenz, für Pivot und MVP-Auswahl unnötig.

## Verweise

- `crates/qoob3d-core/src/mesh.rs` (`raycast`), `crates/qoob3d-render/src/camera.rs` (`ray`),
  `crates/qoob3d-app/src/navigation.rs`
