---
id: ADR-0004
title: OpenCascade über opencascade-rs von crates.io mit statisch gebautem OCCT 7.8
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-kernel]
tags: [kernel, abhängigkeiten, ffi]
supersedes: []
superseded_by: null
---

## Kontext

Wir brauchen einen B-Rep-Kernel mit Booleschen Operationen, Fasen und Rundungen sowie
STEP- und STL-Export. OpenCascade (OCCT) ist der einzige ausgereifte Open-Source-Kandidat.
`opencascade-rs` (bschwind) bietet cxx-Bindings (`opencascade-sys`) und eine High-Level-API
(`opencascade`). Version 0.3.0 liegt seit August 2026 auf crates.io und bündelt OCCT 7.8.1
über `occt-sys`.

## Entscheidung

- Wir nutzen `opencascade = "0.3.0"` von crates.io mit dem Default-Feature `builtin`:
  OCCT wird aus den Quellen gebaut und **statisch** gelinkt. Das ist auf macOS und Linux
  reproduzierbar, und es gibt keine Abhängigkeit von System-OCCT.
- Fehlende Funktionen ergänzen wir als eigene cxx-Wrapper nach demselben Muster. Sobald der
  erste gebraucht wird, übernehmen wir das Repo als Fork nach `third_party/opencascade-rs`
  (path dependency) – eine eigene ADR folgt dann.
- **Jeder eigene Wrapper fängt C++-Exceptions** (`try/catch` um `Standard_Failure`) und
  liefert über cxx ein `Result`. Eine Exception über die FFI-Grenze bricht sonst den Prozess ab.
- OCCT-Typen und die von opencascade benutzte `glam 0.24` (bei uns `occt-glam`) verlassen
  das Kernel-Crate nie.

## Konsequenzen

- Der erste Build dauert etwa 4 Minuten. `[profile.dev.package."*"]` zu ändern löst einen
  kompletten OCCT-Neubau aus.
- CMake ≥ 4 braucht `CMAKE_POLICY_VERSION_MINIMUM=3.5` (`.cargo/config.toml`).
- OCCT steht unter LGPL-2.1. Wegen des statischen Linkens muss die Lizenz vor der
  Weitergabe von Binaries geklärt werden (offener Punkt).
- Bekannter Mangel: Der `Mesher` von opencascade-rs liefert bei umgedrehten Faces nach
  innen zeigende Normalen. Wir korrigieren das in `tessellate.rs`, ein eigener Mesher folgt.

## Verworfene Alternativen

- **Homebrew- oder System-OCCT** – Versionsdrift zwischen Rechnern, auf Linux distributionsabhängig.
- **Git-Abhängigkeit auf opencascade-rs** – unnötig, solange crates.io aktuell ist.
- **Truck / Fornjot (reines Rust)** – noch zu unreif für Boolesche Operationen, Fasen und STEP.

## Verweise

- `crates/qoob3d-kernel/Cargo.toml`, `.cargo/config.toml`, `crates/qoob3d-kernel/src/tessellate.rs`
- ADR-0005
