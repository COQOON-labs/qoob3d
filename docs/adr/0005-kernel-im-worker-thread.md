---
id: ADR-0005
title: OCCT-Objekte leben ausschließlich in einem eigenen Kernel-Worker-Thread
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-kernel, qoob3d-app]
tags: [threading, kernel]
supersedes: []
superseded_by: null
---

## Kontext

Geometrie-Operationen (Boolesche Operationen, Tessellierung) können Sekunden dauern, und die
UI darf dabei nicht einfrieren. OCCT-Handles sind nicht `Send`.

## Entscheidung

- `qoob3d_kernel::Kernel::spawn(notify)` startet einen Thread (`qoob3d-kernel`), der **alle**
  OCCT-Objekte besitzt.
- Die Kommunikation läuft ausschließlich über Nachrichten mit reinen Daten:
  `submit(Request) -> RequestId` und `try_recv() -> Option<Response>`.
- Jede Antwort trägt die `RequestId` ihrer Anfrage. **Die UI verwirft veraltete Antworten.**
- Nach jeder Antwort ruft der Worker `notify()` auf (in der App: `ctx.request_repaint()`).
- Ab M4: Liegen mehrere Rebuild-Anfragen in der Warteschlange, wird nur die neueste
  bearbeitet. Bis zum neuen Ergebnis bleibt die alte Geometrie sichtbar.
- Wird der `Kernel`-Handle gedroppt, beendet sich der Worker.

## Konsequenzen

- Die UI blockiert nie auf Geometrie.
- Der Kernel-Cache (ADR-0012) lebt im Worker-Thread.
- Ein späterer Umzug in einen **separaten Prozess** (Absturzisolierung, untrusted STEP-Import)
  ändert nur den Transport, nicht das Protokoll.

## Verworfene Alternativen

- **Rayon/Tasks pro Anfrage** – OCCT-Objekte müssten zwischen Threads wandern (nicht `Send`).
- **Synchrone Aufrufe aus der UI** – friert bei komplexen Modellen ein.

## Verweise

- `crates/qoob3d-kernel/src/worker.rs`, `crates/qoob3d-app/src/app.rs`
