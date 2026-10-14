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
| [ADR-0003](0003-workspace-und-crate-grenzen.md) | Cargo-Workspace mit vier Crates und festen Abhängigkeitsrichtungen | ersetzt | 2026-10-13 | qoob3d-core, qoob3d-kernel, qoob3d-render, qoob3d-app |
| [ADR-0004](0004-opencascade-ueber-opencascade-rs.md) | OpenCascade über opencascade-rs von crates.io mit statisch gebautem OCCT 7.8 | akzeptiert | 2026-10-13 | qoob3d-kernel |
| [ADR-0005](0005-kernel-im-worker-thread.md) | OCCT-Objekte leben ausschließlich in einem eigenen Kernel-Worker-Thread | akzeptiert | 2026-10-13 | qoob3d-kernel, qoob3d-app |
| [ADR-0006](0006-koordinatensystem-z-oben-mm.md) | Weltkoordinaten sind rechtshändig mit +Z oben, Einheit Millimeter | akzeptiert | 2026-10-13 | qoob3d-core, qoob3d-kernel, qoob3d-render |
| [ADR-0007](0007-offscreen-rendering-reverse-z.md) | Der Viewport rendert offscreen mit 4× MSAA und Reverse-Z und wird als egui-Textur angezeigt | ersetzt | 2026-10-13 | qoob3d-render, qoob3d-app |
| [ADR-0008](0008-kamera-und-navigation-wie-fusion.md) | Kamera als Orbit-Kamera mit Quaternion, Navigation 1:1 wie Fusion 360 | akzeptiert | 2026-10-13 | qoob3d-render, qoob3d-app |
| [ADR-0009](0009-projektformat-qoob3d-json.md) | Projektdateien .qoob3d sind versioniertes JSON und enthalten nur die Feature-Liste | ersetzt | 2026-10-13 | qoob3d-core |
| [ADR-0010](0010-skizzen-grundformen-ohne-solver.md) | Skizzen bestehen im MVP aus Grundformen mit eigenen Maßen, ohne Constraint-Solver | ersetzt | 2026-10-13 | qoob3d-core |
| [ADR-0011](0011-profilreferenzen-ueber-entity-ids.md) | Profile werden über die Menge ihrer begrenzenden Entity-IDs referenziert | akzeptiert | 2026-10-13 | qoob3d-core, qoob3d-kernel |
| [ADR-0012](0012-inkrementeller-rebuild-hash-kette.md) | Der Rebuild ist inkrementell über eine Hash-Kette pro Feature | akzeptiert | 2026-10-13 | qoob3d-kernel |
| [ADR-0013](0013-qualitaets-harness.md) | Ein Befehl definiert „fertig“ – cargo xtask check mit Smoke-Test, Render-CLI und Geometrie-Prüfungen | akzeptiert | 2026-10-13 | xtask, qoob3d-app, qoob3d-core, qoob3d-render |
| [ADR-0014](0014-eigene-occt-wrapper-und-fork.md) | Eigene exception-sichere OCCT-Wrapper im Kernel-Crate, opencascade als minimaler Fork | akzeptiert | 2026-10-13 | qoob3d-kernel |
| [ADR-0015](0015-overlays-mit-egui.md) | ViewCube und Viewport-Overlays werden mit egui gezeichnet, nur die Szene mit wgpu | akzeptiert | 2026-10-13 | qoob3d-app |
| [ADR-0016](0016-druckbett-und-druckerprofile.md) | Druckbett aus Druckerprofilen, zentriert um den Ursprung, im Raster-Shader gezeichnet | ersetzt | 2026-10-13 | qoob3d-core, qoob3d-render, qoob3d-app |
| [ADR-0017](0017-picking-per-cpu-raycast.md) | Picking vorerst per CPU-Raycast gegen das Dreiecksnetz | akzeptiert | 2026-10-13 | qoob3d-core, qoob3d-render, qoob3d-app |
| [ADR-0018](0018-ui-tests-und-screenshots-mit-kittest.md) | UI-Tests und Screenshots der echten App headless mit egui_kittest und wgpu | akzeptiert | 2026-10-13 | qoob3d-app, xtask |
| [ADR-0019](0019-viewport-textur-farbraum.md) | Viewport offscreen in sRGB rendern, egui eine Unorm-View derselben Textur geben | akzeptiert | 2026-10-13 | qoob3d-render, qoob3d-app |
| [ADR-0020](0020-englisch-als-basissprache-mit-sprachpaketen.md) | Englisch ist die Basissprache der UI, weitere Sprachen kommen als Fluent-Sprachpakete | akzeptiert | 2026-10-13 | qoob3d-app, xtask |
| [ADR-0021](0021-druckerdatenbank-aus-orcaslicer.md) | Druckerdatenbank aus OrcaSlicer-Profilen generiert, mit Sperrzonen und Düsenbereichen | akzeptiert | 2026-10-13 | qoob3d-core, qoob3d-render, qoob3d-app, xtask |
| [ADR-0022](0022-linksziehen-orbit-fuer-drei-finger.md) | Ziehen mit linker Maustaste (macOS Drei-Finger-Ziehen) dreht die Ansicht, abschaltbar | akzeptiert | 2026-10-13 | qoob3d-app |
| [ADR-0023](0023-meine-drucker.md) | „Meine Drucker“ als Favoriten oben in der Liste, andere optional ausblendbar | akzeptiert | 2026-10-13 | qoob3d-app |
| [ADR-0024](0024-skizzen-punkte-als-entitaeten.md) | Skizzen bestehen aus Punkten als eigenen Entitäten und Kurven, die Punkte referenzieren | akzeptiert | 2026-10-13 | qoob3d-core, qoob3d-sketch |
| [ADR-0025](0025-reihenfolge-m3a-m4-m3b-m3c.md) | Constraint-Solver kommt in den MVP, Reihenfolge M3a → M4 → M3b → M3c | akzeptiert | 2026-10-13 | – |
| [ADR-0026](0026-crate-grenzen-mit-skizzen-crate.md) | Cargo-Workspace mit fünf Crates – Skizzen-Logik in eigenem, reinem Crate qoob3d-sketch | ersetzt | 2026-10-13 | qoob3d-core, qoob3d-kernel, qoob3d-render, qoob3d-sketch, qoob3d-app, xtask |
| [ADR-0027](0027-skizzier-interaktion-und-darstellung.md) | Skizziermodus – linke Maustaste zeichnet, Look-At und Fangen an, Skizzen als egui-Overlay | akzeptiert | 2026-10-13 | qoob3d-app, qoob3d-sketch |
| [ADR-0028](0028-profilberechnung-general-fuse.md) | Profile entstehen im Kernel durch Teilen einer Hilfsfläche mit allen Skizzenkurven | akzeptiert | 2026-10-13 | qoob3d-kernel, qoob3d-core |
| [ADR-0029](0029-undo-ueber-dokument-snapshots.md) | Undo/Redo über Snapshots des ganzen Dokuments, eine Skizzensitzung als eine Timeline-Einheit | akzeptiert | 2026-10-13 | qoob3d-app, qoob3d-core |
| [ADR-0030](0030-projektdateien-qbd.md) | Projektdateien heißen .qbd, sind versioniertes JSON und enthalten nur die Feature-Liste | akzeptiert | 2026-10-13 | qoob3d-core, qoob3d-app |
| [ADR-0031](0031-koerper-features-und-rebuild.md) | Körper-Features (Extrusion, Drehung, Erhebung, Ebene) mit Rebuild über Hash-Kette und Booleans gegen berührende Körper | akzeptiert | 2026-10-14 | qoob3d-core, qoob3d-kernel |
| [ADR-0032](0032-generierte-features-zahnrad-und-spirale.md) | Zahnrad und Spirale sind parametrische Features; ihre Geometrie entsteht in reinem Rust (core::generate) | akzeptiert | 2026-10-14 | qoob3d-core, qoob3d-kernel |
| [ADR-0033](0033-referenzmodelle-als-qualitaets-gate.md) | Referenzmodelle mit analytischen Erwartungen sind ein Qualitäts-Gate; die CLI kann Dateien bauen, prüfen und exportieren | akzeptiert | 2026-10-14 | qoob3d-core, qoob3d-app, xtask |
| [ADR-0034](0034-export-stl-und-3mf.md) | Export als binäres STL und 3MF mit eigenen, abhängigkeitsfreien Writern | akzeptiert | 2026-10-14 | qoob3d-core, qoob3d-kernel, qoob3d-app |
| [ADR-0035](0035-feature-dialoge-mit-live-vorschau.md) | Feature-Befehle sind Dialoge mit Auswahl im Viewport und Live-Vorschau über den normalen Rebuild | akzeptiert | 2026-10-14 | qoob3d-app |
| [ADR-0036](0036-crate-grenzen-xtask-nutzt-core.md) | Crate-Grenzen – wie ADR-0026, zusätzlich darf xtask den reinen qoob3d-core nutzen | akzeptiert | 2026-10-14 | qoob3d-core, qoob3d-kernel, qoob3d-render, qoob3d-sketch, qoob3d-app, xtask |
<!-- adr-index:end -->
