# Architecture Decision Records (ADRs)

Jede Architektur- oder Designentscheidung steht in einer eigenen Datei (siehe ADR-0001).
Neue ADR: `docs/adr/0000-vorlage.md` kopieren, nächste freie Nummer vergeben, danach
`cargo xtask docs --fix` ausführen. Das aktualisiert die Tabelle unten automatisch.

Status: `vorgeschlagen` → `akzeptiert` → (`ersetzt` | `verworfen`).
Akzeptierte ADRs werden nicht umgeschrieben, sondern durch eine neue ADR ersetzt.

<!-- adr-index:start (generiert von `cargo xtask docs --fix` – nicht von Hand bearbeiten) -->
| ADR | Titel | Status | Datum | Crates |
| --- | --- | --- | --- | --- |
| [ADR-0001](0001-dokumentation-fuer-menschen-und-ki.md) | Entscheidungen als ADRs, gestufte CLAUDE.md-Dateien und maschinell geprüfte Doku | akzeptiert | 2026-10-13 | xtask |
| [ADR-0002](0002-fdm-druck-als-produktfokus.md) | QOOB3D ist primär ein Konstruktionswerkzeug für den FDM-Druck | akzeptiert | 2026-10-13 | – |
| [ADR-0003](0003-workspace-und-crate-grenzen.md) | Cargo-Workspace mit vier Crates und festen Abhängigkeitsrichtungen | akzeptiert | 2026-10-13 | qoob3d-core, qoob3d-kernel, qoob3d-render, qoob3d-app |
| [ADR-0004](0004-opencascade-ueber-opencascade-rs.md) | OpenCascade über opencascade-rs von crates.io mit statisch gebautem OCCT 7.8 | akzeptiert | 2026-10-13 | qoob3d-kernel |
| [ADR-0005](0005-kernel-im-worker-thread.md) | OCCT-Objekte leben ausschließlich in einem eigenen Kernel-Worker-Thread | akzeptiert | 2026-10-13 | qoob3d-kernel, qoob3d-app |
| [ADR-0006](0006-koordinatensystem-z-oben-mm.md) | Weltkoordinaten sind rechtshändig mit +Z oben, Einheit Millimeter | akzeptiert | 2026-10-13 | qoob3d-core, qoob3d-kernel, qoob3d-render |
| [ADR-0007](0007-offscreen-rendering-reverse-z.md) | Der Viewport rendert offscreen mit 4× MSAA und Reverse-Z und wird als egui-Textur angezeigt | akzeptiert | 2026-10-13 | qoob3d-render, qoob3d-app |
| [ADR-0008](0008-kamera-und-navigation-wie-fusion.md) | Kamera als Orbit-Kamera mit Quaternion, Navigation 1:1 wie Fusion 360 | akzeptiert | 2026-10-13 | qoob3d-render, qoob3d-app |
| [ADR-0009](0009-projektformat-qoob3d-json.md) | Projektdateien .qoob3d sind versioniertes JSON und enthalten nur die Feature-Liste | akzeptiert | 2026-10-13 | qoob3d-core |
| [ADR-0010](0010-skizzen-grundformen-ohne-solver.md) | Skizzen bestehen im MVP aus Grundformen mit eigenen Maßen, ohne Constraint-Solver | akzeptiert | 2026-10-13 | qoob3d-core |
| [ADR-0011](0011-profilreferenzen-ueber-entity-ids.md) | Profile werden über die Menge ihrer begrenzenden Entity-IDs referenziert | akzeptiert | 2026-10-13 | qoob3d-core, qoob3d-kernel |
| [ADR-0012](0012-inkrementeller-rebuild-hash-kette.md) | Der Rebuild ist inkrementell über eine Hash-Kette pro Feature | akzeptiert | 2026-10-13 | qoob3d-kernel |
| [ADR-0013](0013-qualitaets-harness.md) | Ein Befehl definiert „fertig“ – cargo xtask check mit Smoke-Test, Render-CLI und Geometrie-Prüfungen | akzeptiert | 2026-10-13 | xtask, qoob3d-app, qoob3d-core, qoob3d-render |
<!-- adr-index:end -->
